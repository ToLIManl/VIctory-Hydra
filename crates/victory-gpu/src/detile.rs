//! Batched hardware GPU compute detiler for console GFX10 thin 2D textures
//! (`VICTORY_GPU_DETILE=1`, live-toggleable from the F1 menu).
//!
//! Moves scalar CPU tile-swizzle detiling (`victory_formats::texture::Layout::detile`)
//! off the CPU onto the host GPU's compute units. Supports all 6 thin 2D non-XOR
//! GFX10 swizzle modes (`SW_256B_S/D`, `SW_4KB_S/D`, `SW_64KB_S/D`), all 4/8/16-byte
//! elements (RGBA8/R32F/RG16F/RGBA16F/BC1/BC3/BC4/BC5/BC6H/BC7), full mip chains
//! including mip tails, and texture arrays — bit-exact against CPU `Layout::detile`.
//!
//! Uses root SRV (`t0`) + root UAV (`u0`) so it records cleanly onto prelude
//! command lists without touching or requiring any descriptor heap. All `gpu_raw`
//! uploads in one prelude are sub-allocated into disjoint 512B-aligned regions of a
//! shared `DEFAULT` UAV scratch buffer so all dispatches overlap on the GPU with
//! **one** `UAV -> COPY_SOURCE` barrier and **one** `COPY_SOURCE -> UAV` barrier
//! per chunk instead of `2*N` barriers.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Graphics::Direct3D12::*;
use windows::core::s;

use crate::host::{HostList, host, transition};

const MAX_BYTES: u64 = 64 * 1024 * 1024;
const INITIAL_SCRATCH: u64 = 16 * 1024 * 1024;
const SCRATCH_ALIGN: u64 = 4 * 1024 * 1024;
const PLACEMENT_ALIGN: u64 = 512;

static TEXTURES_DETILED: AtomicU64 = AtomicU64::new(0);
static BYTES_DETILED: AtomicU64 = AtomicU64::new(0);

fn on_cell() -> &'static AtomicBool {
    static ON: OnceLock<AtomicBool> = OnceLock::new();
    ON.get_or_init(|| {
        AtomicBool::new(std::env::var_os("VICTORY_GPU_DETILE").is_some_and(|v| v == "1"))
    })
}

/// Whether new eligible textures use GPU compute detiling.
pub(crate) fn on() -> bool {
    on_cell().load(Ordering::Relaxed)
}

/// Toggles `VICTORY_GPU_DETILE` live from the F1 menu (applies to newly staged textures).
pub(crate) fn set_live(enabled: bool) {
    on_cell().store(enabled, Ordering::Relaxed);
    tracing::info!(enabled, "GPU texture detile: updated live from F1 menu");
}

/// Minimum linear footprint bytes for GPU detile (`VICTORY_GPU_DETILE_MIN_KB`, default 16 KiB)
/// so tiny icons stay on the fast CPU path without dispatch overhead.
fn min_bytes() -> u64 {
    static MIN: OnceLock<u64> = OnceLock::new();
    *MIN.get_or_init(|| {
        std::env::var("VICTORY_GPU_DETILE_MIN_KB")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(16)
            * 1024
    })
}

/// Cap for the shared scratch buffer (`VICTORY_GPU_DETILE_BATCH_MB`, default 128 MiB,
/// at least `MAX_BYTES` so any single eligible texture fits in one chunk).
fn batch_cap() -> u64 {
    static CAP: OnceLock<u64> = OnceLock::new();
    *CAP.get_or_init(|| {
        let mb = std::env::var("VICTORY_GPU_DETILE_BATCH_MB")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(128)
            .clamp(64, 512);
        mb * 1024 * 1024
    })
}

/// Totals for periodic telemetry: `(textures_detiled, linear_bytes_detiled)`.
pub(crate) fn stats() -> (u64, u64) {
    (
        TEXTURES_DETILED.load(Ordering::Relaxed),
        BYTES_DETILED.load(Ordering::Relaxed),
    )
}

/// Whether a texture layout is eligible for bit-exact GPU detiling.
pub(crate) fn eligible(
    layout: &victory_formats::texture::Layout,
    on_demand: bool,
    total_linear_bytes: u64,
    raw_read: usize,
) -> bool {
    !on_demand
        && layout.gpu_detile_bits().is_some()
        && total_linear_bytes >= min_bytes()
        && total_linear_bytes <= MAX_BYTES
        && raw_read >= 4
        && (raw_read as u64) <= MAX_BYTES
        && pass().is_some()
}

const SOURCE: &str = r#"
cbuffer Detile : register(b0) {
    uint width;
    uint height;
    uint element_bytes;
    uint dst_pitch;
    uint src_base;
    uint dst_base;
    uint bw;
    uint bh;
    uint blocks_per_row;
    uint block_log2;
    uint tail_x;
    uint tail_y;
    uint x_bits;
    uint x_count;
    uint y_bits;
    uint y_count;
    uint src_size;
};
ByteAddressBuffer src : register(t0);
RWByteAddressBuffer dst : register(u0);

[numthreads(8, 8, 1)]
void detile(uint3 at : SV_DispatchThreadID) {
    uint x = at.x;
    uint y = at.y;
    if (x >= width || y >= height) return;
    uint sx = x + tail_x;
    uint sy = y + tail_y;
    uint blk = (sy / bh) * blocks_per_row + (sx / bw);
    uint lx = sx & (bw - 1u);
    uint ly = sy & (bh - 1u);
    uint ox = 0u;
    for (uint i = 0u; i < x_count; i++) {
        ox |= ((lx >> i) & 1u) << ((x_bits >> (i * 4u)) & 15u);
    }
    uint oy = 0u;
    for (uint j = 0u; j < y_count; j++) {
        oy |= ((ly >> j) & 1u) << ((y_bits >> (j * 4u)) & 15u);
    }
    uint src_off = src_base + (blk << block_log2) + (ox | oy);
    if (src_off + element_bytes > src_size) return;
    uint dst_off = dst_base + y * dst_pitch + x * element_bytes;
    if (element_bytes == 4u) {
        dst.Store(dst_off, src.Load(src_off));
    } else if (element_bytes == 8u) {
        dst.Store2(dst_off, src.Load2(src_off));
    } else {
        dst.Store4(dst_off, src.Load4(src_off));
    }
}
"#;

struct Scratch {
    buffer: ID3D12Resource,
    size: u64,
    /// Set once after the initial explicit `COMMON -> UNORDERED_ACCESS` transition;
    /// subsequent command lists decay back to `COMMON` at ECL end and legally
    /// implicitly promote to `UNORDERED_ACCESS` on first compute dispatch.
    /// Never shrinks for the lifetime of the run (bounded by `batch_cap()`).
    in_uav: bool,
}

struct Pass {
    root: ID3D12RootSignature,
    pso: ID3D12PipelineState,
    scratch: Mutex<Option<Scratch>>,
}

// SAFETY: D3D12 objects are free-threaded; mutable scratch state is behind a Mutex.
unsafe impl Send for Pass {}
// SAFETY: as above.
unsafe impl Sync for Pass {}

fn make(device: &ID3D12Device) -> windows::core::Result<Pass> {
    let code = crate::host::compile(SOURCE, s!("detile"), s!("cs_5_1"))?;
    // SAFETY: plain root signature and compute PSO creation.
    unsafe {
        use crate::rootsig::Param;
        let root = crate::rootsig::make(
            device,
            &[
                Param::Constants { register: 0, count: 17 },
                Param::Srv(0),
                Param::Uav(0),
            ],
            "GPU texture detile",
        )?;
        let desc = D3D12_COMPUTE_PIPELINE_STATE_DESC {
            pRootSignature: std::mem::transmute_copy(&root),
            CS: D3D12_SHADER_BYTECODE {
                pShaderBytecode: code.as_ptr().cast(),
                BytecodeLength: code.len(),
            },
            ..Default::default()
        };
        let pso = device.CreateComputePipelineState(&desc)?;
        Ok(Pass {
            root,
            pso,
            scratch: Mutex::new(None),
        })
    }
}

fn pass() -> Option<&'static Pass> {
    static PASS: OnceLock<Option<Pass>> = OnceLock::new();
    PASS.get_or_init(|| {
        let host = host()?;
        make(&host.device)
            .inspect_err(|e| tracing::warn!(error = %e, "GPU texture detile disabled: shader/PSO build failed"))
            .ok()
    })
    .as_ref()
}

/// Ensures the shared GPU scratch UAV buffer is at least `needed` bytes and returns
/// `(buffer, actual_size, needs_common_to_uav_transition)`.
fn acquire_scratch(pass: &Pass, needed: u64) -> Option<(ID3D12Resource, u64, bool)> {
    let mut guard = pass.scratch.lock().unwrap_or_else(|e| e.into_inner());
    let need_alloc = guard.as_ref().is_none_or(|s| s.size < needed);
    if need_alloc {
        let host = host()?;
        let size = needed.max(INITIAL_SCRATCH).next_multiple_of(SCRATCH_ALIGN);
        let buffer = host.uav_buffer(size)?;
        *guard = Some(Scratch {
            buffer,
            size,
            in_uav: false,
        });
    }
    let s = guard.as_mut()?;
    let promote = !s.in_uav;
    s.in_uav = true;
    Some((s.buffer.clone(), s.size, promote))
}

/// Inputs for one `gpu_raw` texture in [`record_batch`].
pub(crate) struct BatchItem<'a> {
    pub(crate) raw_buffer: &'a ID3D12Resource,
    pub(crate) raw_base: u64,
    pub(crate) raw_size: u32,
    pub(crate) layout: &'a victory_formats::texture::Layout,
    pub(crate) mips: u32,
    pub(crate) first: u32,
    pub(crate) level0: u32,
    pub(crate) footprints: &'a [D3D12_PLACED_SUBRESOURCE_FOOTPRINT],
    pub(crate) total_linear_bytes: u64,
    pub(crate) texture: &'a ID3D12Resource,
}

/// Dispatches batched GPU compute detiling for all `items` into disjoint
/// 512B-aligned regions of the shared `DEFAULT` UAV scratch buffer, emitting
/// **one** `UAV -> COPY_SOURCE` barrier and **one** `COPY_SOURCE -> UAV`
/// barrier per chunk instead of `2*N` per-texture barriers.
///
/// Returns the indices of any items that could not be recorded (e.g., scratch
/// OOM). Callers MUST NOT fall back to a linear copy for failed `gpu_raw`
/// items; instead mark them skipped so they retry on the next submission.
pub(crate) fn record_batch(
    host_list: &HostList,
    cmd_list: &ID3D12GraphicsCommandList,
    items: &[BatchItem<'_>],
) -> Vec<usize> {
    if items.is_empty() {
        return Vec::new();
    }
    let Some(pass) = pass() else {
        return (0..items.len()).collect();
    };

    let mut failed = Vec::new();
    let mut valid: Vec<(usize, u64, (u32, u32, u32, u32))> = Vec::with_capacity(items.len());
    let mut sum_bytes = 0u64;
    let mut max_item = 0u64;
    for (idx, item) in items.iter().enumerate() {
        let Some(bits) = item.layout.gpu_detile_bits() else {
            failed.push(idx);
            continue;
        };
        let region = item.total_linear_bytes.next_multiple_of(PLACEMENT_ALIGN);
        if region == 0 || region > MAX_BYTES {
            failed.push(idx);
            continue;
        }
        sum_bytes = sum_bytes.saturating_add(region);
        max_item = max_item.max(region);
        valid.push((idx, region, bits));
    }
    if valid.is_empty() {
        return failed;
    }

    let cap = batch_cap().max(max_item);
    let wanted = sum_bytes.min(cap).max(max_item);
    let Some((scratch, scratch_size, mut from_common)) = acquire_scratch(pass, wanted) else {
        return (0..items.len()).collect();
    };
    let chunk_cap = scratch_size.min(cap).max(max_item);

    let _pix = crate::pix::Scope::begin(cmd_list, c"VIctory: GPU Detile (batch)");
    let gpu_tok = crate::gpu_prof::begin(cmd_list, crate::gpu_prof::PassKind::Detile, 0, false);
    host_list.keep(&scratch);

    let mut detiled_count = 0u64;
    let mut detiled_bytes = 0u64;
    let mut pos = 0usize;
    let mut placements: Vec<(usize, u64, (u32, u32, u32, u32))> = Vec::with_capacity(valid.len());

    while pos < valid.len() {
        placements.clear();
        let mut cursor = 0u64;
        while pos < valid.len() {
            let (idx, region, bits) = valid[pos];
            if !placements.is_empty() && cursor + region > chunk_cap {
                break;
            }
            placements.push((idx, cursor, bits));
            cursor += region;
            pos += 1;
        }

        // SAFETY: open direct prelude list; every raw_buffer is a live UPLOAD buffer
        // in GENERIC_READ; `scratch` is a DEFAULT UAV buffer kept alive by `host_list`;
        // every destination `texture` is already in COPY_DEST; regions within a chunk
        // are pairwise disjoint so dispatches need no UAV barrier between them.
        unsafe {
            if from_common {
                cmd_list.ResourceBarrier(&[transition(
                    &scratch,
                    D3D12_RESOURCE_STATE_COMMON,
                    D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                )]);
                from_common = false;
            }
            cmd_list.SetComputeRootSignature(&pass.root);
            cmd_list.SetPipelineState(&pass.pso);
            cmd_list.SetComputeRootUnorderedAccessView(2, scratch.GetGPUVirtualAddress());

            for &(idx, base_off, (x_bits, x_count, y_bits, y_count)) in &placements {
                let item = &items[idx];
                cmd_list.SetComputeRootShaderResourceView(
                    1,
                    item.raw_buffer.GetGPUVirtualAddress() + item.raw_base,
                );
                let sub_mips = item.mips - item.first;
                let [bw, bh, _] = item.layout.block;
                let block_log2 = item.layout.alignment.trailing_zeros();

                for (i, footprint) in item.footprints.iter().enumerate() {
                    let mip = item.first + (i as u32 % sub_mips);
                    let slice = i as u32 / sub_mips;
                    let Some(m) = item.layout.mips.get(mip as usize) else {
                        continue;
                    };
                    let src_base = u64::from(slice) * item.layout.slice_size + m.offset;
                    let (tail_x, tail_y) = m.tail.map_or((0, 0), |t| (t.x, t.y));
                    let dst_base = base_off + footprint.Offset;
                    let words: [u32; 17] = [
                        m.width,
                        m.height,
                        item.layout.desc.element_bytes,
                        footprint.Footprint.RowPitch,
                        src_base as u32,
                        dst_base as u32,
                        bw,
                        bh,
                        (m.pitch / bw).max(1),
                        block_log2,
                        tail_x,
                        tail_y,
                        x_bits,
                        x_count,
                        y_bits,
                        y_count,
                        item.raw_size,
                    ];
                    cmd_list.SetComputeRoot32BitConstants(0, 17, words.as_ptr().cast(), 0);
                    cmd_list.Dispatch(m.width.div_ceil(8), m.height.div_ceil(8), 1);
                }
            }

            cmd_list.ResourceBarrier(&[transition(
                &scratch,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            )]);

            for &(idx, base_off, _) in &placements {
                let item = &items[idx];
                for (i, footprint) in item.footprints.iter().enumerate() {
                    let to = D3D12_TEXTURE_COPY_LOCATION {
                        pResource: std::mem::transmute_copy(item.texture),
                        Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                            SubresourceIndex: item.first - item.level0 + i as u32,
                        },
                    };
                    let from = D3D12_TEXTURE_COPY_LOCATION {
                        pResource: std::mem::transmute_copy(&scratch),
                        Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                            PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                                Offset: base_off + footprint.Offset,
                                ..*footprint
                            },
                        },
                    };
                    cmd_list.CopyTextureRegion(&to, 0, 0, 0, &from, None);
                }
                detiled_count += 1;
                detiled_bytes += item.total_linear_bytes;
            }

            // Keep this barrier: orders this chunk's copy reads before the next
            // chunk's dispatches AND before the next prelude sharing `scratch`
            // within the same ExecuteCommandLists batch.
            cmd_list.ResourceBarrier(&[transition(
                &scratch,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            )]);
        }
    }

    crate::gpu_prof::end(cmd_list, gpu_tok);
    TEXTURES_DETILED.fetch_add(detiled_count, Ordering::Relaxed);
    BYTES_DETILED.fetch_add(detiled_bytes, Ordering::Relaxed);
    failed
}

#[cfg(test)]
mod tests {
    use super::*;
    use victory_formats::texture::{Desc, Dimension, Layout, Swizzle};
    use windows::core::Interface;

    #[test]
    fn gpu_detile_matches_cpu_bit_exact() {
        let Some(host) = host() else {
            return;
        };
        let pass = make(&host.device).expect("detile compute PSO builds");

        let cases = [
            // (width, height, depth/slices, mips, element_bytes, block, swizzle)
            (256, 256, 1, 5, 8, 4, Swizzle::SW_64KB_D),  // BC1 with mip tail
            (256, 256, 1, 5, 16, 4, Swizzle::SW_64KB_D), // BC7 with mip tail
            (300, 196, 2, 4, 16, 4, Swizzle::SW_64KB_S), // NPOT BC7 array
            (128, 128, 1, 4, 8, 4, Swizzle::SW_4KB_S),   // BC1 4KB_S
            (192, 160, 1, 3, 4, 1, Swizzle::SW_64KB_D),  // RGBA8 4BPE
            (64, 64, 1, 3, 4, 1, Swizzle::SW_256B_D),    // RGBA8 256B_D
        ];

        // SAFETY: self-contained test command list, buffers, and fence on the host device.
        unsafe {
            let alloc: ID3D12CommandAllocator = host
                .device
                .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                .expect("allocator");
            let list: ID3D12GraphicsCommandList = host
                .device
                .CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &alloc, None)
                .expect("list");
            list.Close().expect("close");
            let fence: ID3D12Fence = host
                .device
                .CreateFence(0, D3D12_FENCE_FLAG_NONE)
                .expect("fence");
            let mut fence_val = 0u64;

            for (w, h, slices, mips, bpe, block, swizzle) in cases {
                let layout = Layout::new(&Desc {
                    dimension: Dimension::Tex2d,
                    width: w,
                    height: h,
                    depth: slices,
                    mips,
                    element_bytes: bpe,
                    block,
                    samples: 1,
                    swizzle,
                })
                .expect("layout");
                let (x_bits, x_count, y_bits, y_count) =
                    layout.gpu_detile_bits().expect("gpu_detile_bits");

                // Deterministic pseudo-random tiled input
                let raw_size = (layout.size as usize).next_multiple_of(4);
                let mut tiled = vec![0u8; raw_size];
                let mut seed = 0x1234_5678_9abc_def0u64 ^ u64::from(w) ^ (u64::from(swizzle.0) << 16);
                for b in &mut tiled {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    *b = (seed >> 56) as u8;
                }

                // Compute CPU golden output across all mips & slices
                let mut sub_offsets = Vec::new();
                let mut total_linear = 0u64;
                for slice in 0..slices {
                    for mip in 0..mips {
                        let m = &layout.mips[mip as usize];
                        let pitch = (m.width * bpe).next_multiple_of(256);
                        let off = total_linear.next_multiple_of(512);
                        let len = u64::from(pitch) * u64::from(m.height);
                        sub_offsets.push((mip, slice, off, pitch, len));
                        total_linear = off + len;
                    }
                }
                let mut cpu_out = vec![0u8; total_linear as usize];
                for &(mip, slice, off, pitch, len) in &sub_offsets {
                    layout
                        .detile(
                            &tiled,
                            mip,
                            slice,
                            &mut cpu_out[off as usize..(off + len) as usize],
                            pitch as usize,
                            len as usize,
                        )
                        .expect("cpu detile");
                }

                // Upload tiled bytes, run GPU detile shader into DEFAULT UAV, read back
                let (src_buf, src_ptr) = host.upload_buffer(raw_size as u64).expect("upload");
                std::ptr::copy_nonoverlapping(tiled.as_ptr(), src_ptr, tiled.len());
                let dst_buf = host.uav_buffer(total_linear).expect("uav");
                let rb_heap = D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_READBACK,
                    ..Default::default()
                };
                let rb_desc = D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                    Width: total_linear,
                    Height: 1,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                    ..Default::default()
                };
                let mut rb_buf: Option<ID3D12Resource> = None;
                host.device
                    .CreateCommittedResource(
                        &rb_heap,
                        D3D12_HEAP_FLAG_NONE,
                        &rb_desc,
                        D3D12_RESOURCE_STATE_COPY_DEST,
                        None,
                        &mut rb_buf,
                    )
                    .expect("readback");
                let rb_buf = rb_buf.unwrap();

                alloc.Reset().expect("alloc reset");
                list.Reset(&alloc, None).expect("list reset");
                list.ResourceBarrier(&[transition(
                    &dst_buf,
                    D3D12_RESOURCE_STATE_COMMON,
                    D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                )]);
                list.SetComputeRootSignature(&pass.root);
                list.SetPipelineState(&pass.pso);
                list.SetComputeRootShaderResourceView(1, src_buf.GetGPUVirtualAddress());
                list.SetComputeRootUnorderedAccessView(2, dst_buf.GetGPUVirtualAddress());

                let [bw, bh, _] = layout.block;
                let block_log2 = layout.alignment.trailing_zeros();
                for &(mip, slice, off, pitch, _) in &sub_offsets {
                    let m = &layout.mips[mip as usize];
                    let src_base = u64::from(slice) * layout.slice_size + m.offset;
                    let (tail_x, tail_y) = m.tail.map_or((0, 0), |t| (t.x, t.y));
                    let words: [u32; 17] = [
                        m.width,
                        m.height,
                        bpe,
                        pitch,
                        src_base as u32,
                        off as u32,
                        bw,
                        bh,
                        (m.pitch / bw).max(1),
                        block_log2,
                        tail_x,
                        tail_y,
                        x_bits,
                        x_count,
                        y_bits,
                        y_count,
                        raw_size as u32,
                    ];
                    list.SetComputeRoot32BitConstants(0, 17, words.as_ptr().cast(), 0);
                    list.Dispatch(m.width.div_ceil(8), m.height.div_ceil(8), 1);
                }

                list.ResourceBarrier(&[transition(
                    &dst_buf,
                    D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                    D3D12_RESOURCE_STATE_COPY_SOURCE,
                )]);
                list.CopyBufferRegion(&rb_buf, 0, &dst_buf, 0, total_linear);
                list.Close().expect("close");

                let cmd: ID3D12CommandList = list.cast().unwrap();
                host.queue().ExecuteCommandLists(&[Some(cmd)]);
                fence_val += 1;
                host.queue().Signal(&fence, fence_val).expect("signal");
                while fence.GetCompletedValue() < fence_val {
                    std::thread::yield_now();
                }

                let mut mapped = std::ptr::null_mut();
                rb_buf.Map(0, None, Some(&mut mapped)).expect("map");
                let gpu_out = std::slice::from_raw_parts(mapped as *const u8, total_linear as usize);
                // Compare every valid element in every row of every subresource
                for &(mip, slice, off, pitch, _) in &sub_offsets {
                    let m = &layout.mips[mip as usize];
                    let row_bytes = (m.width * bpe) as usize;
                    for y in 0..m.height as usize {
                        let start = off as usize + y * pitch as usize;
                        assert_eq!(
                            &gpu_out[start..start + row_bytes],
                            &cpu_out[start..start + row_bytes],
                            "mismatch for {w}x{h} slices={slices} bpe={bpe} swizzle={swizzle:?} mip={mip} slice={slice} row={y}"
                        );
                    }
                }
                rb_buf.Unmap(0, None);
            }
        }
    }
}
