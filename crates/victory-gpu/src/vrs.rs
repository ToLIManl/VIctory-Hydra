//! Hardware Variable Rate Shading (VRS Tier 1 & Tier 2) support for D3D12.XS
//! (`ID3D12GraphicsCommandList5::RSSetShadingRate` / `RSSetShadingRateImage`,
//! `d3d12_xs.h:27484`) plus optional host-side foveated Tier 2 VRS (`VICTORY_VRS`).
//!
//! Modes (`VICTORY_VRS`, live-toggleable from the F1 menu):
//! - `0` / `off` (default): disabled (no host VRS calls).
//! - `title`: forwards the title's own `RSSetShadingRate` calls to the host.
//! - `soft` / `foveated` / `tier2`: keeps `1x1` shading rate inside the central
//!   ~65% screen region (focus & character), transitions to `1x2`/`2x1` in the
//!   outer ring and `2x2` in the extreme corners via a hardware Tier 2 screen-space
//!   shading-rate image (`R8_UINT`, 16x16 tiles on Ampere), applied only on main
//!   color render passes (`>= 640x360`, never depth-only shadows or small UI).
//! - `1x2` / `2x1` / `2x2`: uniform Tier 1 coarse shading rate on main color passes.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::core::Interface;

/// Configured VRS mode.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Off = 0,
    Title = 1,
    Soft = 2,
    Rate1x2 = 3,
    Rate2x1 = 4,
    Rate2x2 = 5,
}

impl Mode {
    pub(crate) fn parse(spec: &str) -> Self {
        match spec.trim().to_ascii_lowercase().as_str() {
            "" | "0" | "off" => Self::Off,
            "title" | "passthrough" => Self::Title,
            "1" | "soft" | "foveated" | "tier2" => Self::Soft,
            "1x2" => Self::Rate1x2,
            "2x1" => Self::Rate2x1,
            "2x2" => Self::Rate2x2,
            _ => Self::Off,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Title,
            2 => Self::Soft,
            3 => Self::Rate1x2,
            4 => Self::Rate2x1,
            5 => Self::Rate2x2,
            _ => Self::Off,
        }
    }
}

fn mode_cell() -> &'static AtomicU8 {
    static MODE: OnceLock<AtomicU8> = OnceLock::new();
    MODE.get_or_init(|| {
        let m = std::env::var("VICTORY_VRS")
            .ok()
            .map_or(Mode::Off, |v| Mode::parse(&v));
        AtomicU8::new(m as u8)
    })
}

/// Current VRS mode (single relaxed atomic load on hot path).
#[inline]
pub(crate) fn mode() -> Mode {
    Mode::from_u8(mode_cell().load(Ordering::Relaxed))
}

/// Live-updates `VICTORY_VRS` from the F1 menu.
pub(crate) fn set_live(spec: &str) {
    let m = Mode::parse(spec);
    mode_cell().store(m as u8, Ordering::Relaxed);
    tracing::info!(?m, spec, "VRS mode updated live from F1 menu");
}

/// Host hardware VRS capabilities queried from `D3D12_FEATURE_D3D12_OPTIONS6`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Caps {
    pub(crate) tier: u32,
    pub(crate) tile_size: u32,
}

pub(crate) fn caps(host: &crate::host::Host) -> Caps {
    static CAPS: OnceLock<Caps> = OnceLock::new();
    *CAPS.get_or_init(|| {
        let mut o = D3D12_FEATURE_DATA_D3D12_OPTIONS6::default();
        // SAFETY: plain CheckFeatureSupport query for OPTIONS6.
        let ok = unsafe {
            host.device.CheckFeatureSupport(
                D3D12_FEATURE_D3D12_OPTIONS6,
                (&raw mut o).cast(),
                size_of::<D3D12_FEATURE_DATA_D3D12_OPTIONS6>() as u32,
            )
        }
        .is_ok();
        let c = if ok {
            Caps {
                tier: o.VariableShadingRateTier.0 as u32,
                tile_size: o.ShadingRateImageTileSize,
            }
        } else {
            Caps {
                tier: 0,
                tile_size: 0,
            }
        };
        tracing::info!(
            tier = c.tier,
            tile_size = c.tile_size,
            "host Variable Rate Shading (VRS) capabilities"
        );
        c
    })
}

/// Builds a foveated screen-space `R8_UINT` shading rate map (`tiles_x * tiles_y`
/// bytes) using `D3D12_SHADING_RATE_*` encodings (`0x0 = 1x1`, `0x1 = 1x2`,
/// `0x4 = 2x1`, `0x5 = 2x2`).
///
/// Keeps `1x1` across the central ~65% of the frame (normalized radius `<= 0.68`)
/// so the character, crosshair, and main focus area remain full-resolution,
/// steps gently to directional `1x2`/`2x1` in the outer ring (`0.68..=0.92`),
/// and uses `2x2` only in the extreme periphery (`> 0.92`).
pub(crate) fn build_foveated_map(tiles_x: u32, tiles_y: u32) -> Vec<u8> {
    const R1X1: u8 = 0x0;
    const R1X2: u8 = 0x1;
    const R2X1: u8 = 0x4;
    const R2X2: u8 = 0x5;

    let mut out = vec![R1X1; (tiles_x as usize) * (tiles_y as usize)];
    if tiles_x == 0 || tiles_y == 0 {
        return out;
    }
    let cx = tiles_x as f32 * 0.5;
    let cy = tiles_y as f32 * 0.5;
    for y in 0..tiles_y {
        let ny = ((y as f32 + 0.5) - cy) / cy.max(1.0);
        for x in 0..tiles_x {
            let nx = ((x as f32 + 0.5) - cx) / cx.max(1.0);
            let r2 = nx * nx + ny * ny;
            let rate = if r2 <= 0.68 * 0.68 {
                R1X1
            } else if r2 <= 0.92 * 0.92 {
                // Use horizontal coarsening on left/right edges and vertical on top/bottom.
                if nx.abs() >= ny.abs() { R2X1 } else { R1X2 }
            } else {
                R2X2
            };
            out[(y * tiles_x + x) as usize] = rate;
        }
    }
    out
}

struct ImageEntry {
    resource: ID3D12Resource,
}

// SAFETY: `ID3D12Resource` is free-threaded and kept read-only in `SHADING_RATE_SOURCE`.
unsafe impl Send for ImageEntry {}

static IMAGES: Mutex<Option<HashMap<(u32, u32), ImageEntry>>> = Mutex::new(None);

/// Returns (or creates and uploads once) a foveated VRS Tier 2 shading-rate
/// image for the given `(tiles_x, tiles_y)` dimensions in state
/// `D3D12_RESOURCE_STATE_SHADING_RATE_SOURCE`.
fn foveated_image(host: &crate::host::Host, tiles_x: u32, tiles_y: u32) -> Option<ID3D12Resource> {
    let mut guard = IMAGES.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if let Some(entry) = map.get(&(tiles_x, tiles_y)) {
        return Some(entry.resource.clone());
    }
    let resource = create_foveated_image(host, tiles_x, tiles_y)
        .inspect_err(|e| tracing::warn!(tiles_x, tiles_y, error = %e, "VRS Tier 2 image creation failed"))
        .ok()?;
    map.insert(
        (tiles_x, tiles_y),
        ImageEntry {
            resource: resource.clone(),
        },
    );
    Some(resource)
}

fn create_foveated_image(
    host: &crate::host::Host,
    tiles_x: u32,
    tiles_y: u32,
) -> windows::core::Result<ID3D12Resource> {
    let data = build_foveated_map(tiles_x, tiles_y);
    let row_pitch = (tiles_x as usize).next_multiple_of(D3D12_TEXTURE_DATA_PITCH_ALIGNMENT as usize);
    let upload_bytes = (row_pitch * tiles_y as usize) as u64;

    // SAFETY: plain D3D12 resource creation, synchronous one-time upload of a tiny
    // (~8 KiB) R8_UINT shading-rate image, transitioned to SHADING_RATE_SOURCE.
    unsafe {
        let default_heap = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_DEFAULT,
            ..Default::default()
        };
        let tex_desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Alignment: 0,
            Width: u64::from(tiles_x),
            Height: tiles_y,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: DXGI_FORMAT_R8_UINT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
            Flags: D3D12_RESOURCE_FLAG_NONE,
        };
        let mut tex: Option<ID3D12Resource> = None;
        host.device.CreateCommittedResource(
            &default_heap,
            D3D12_HEAP_FLAG_NONE,
            &tex_desc,
            D3D12_RESOURCE_STATE_COPY_DEST,
            None,
            &mut tex,
        )?;
        let tex = tex.ok_or_else(windows::core::Error::empty)?;

        let upload_heap = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_UPLOAD,
            ..Default::default()
        };
        let buf_desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Alignment: 0,
            Width: upload_bytes,
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: DXGI_FORMAT_UNKNOWN,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            Flags: D3D12_RESOURCE_FLAG_NONE,
        };
        let mut upload: Option<ID3D12Resource> = None;
        host.device.CreateCommittedResource(
            &upload_heap,
            D3D12_HEAP_FLAG_NONE,
            &buf_desc,
            D3D12_RESOURCE_STATE_GENERIC_READ,
            None,
            &mut upload,
        )?;
        let upload = upload.ok_or_else(windows::core::Error::empty)?;

        let mut mapped = std::ptr::null_mut();
        upload.Map(0, None, Some(&mut mapped))?;
        for y in 0..tiles_y as usize {
            std::ptr::copy_nonoverlapping(
                data.as_ptr().add(y * tiles_x as usize),
                mapped.cast::<u8>().add(y * row_pitch),
                tiles_x as usize,
            );
        }
        upload.Unmap(0, None);

        let list = host.new_list()?;
        let dst = D3D12_TEXTURE_COPY_LOCATION {
            pResource: std::mem::ManuallyDrop::new(Some(tex.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
        };
        let src = D3D12_TEXTURE_COPY_LOCATION {
            pResource: std::mem::ManuallyDrop::new(Some(upload.clone())),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                    Offset: 0,
                    Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                        Format: DXGI_FORMAT_R8_UINT,
                        Width: tiles_x,
                        Height: tiles_y,
                        Depth: 1,
                        RowPitch: row_pitch as u32,
                    },
                },
            },
        };
        list.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
        // Drop the ManuallyDrop refs cleanly.
        let _ = std::mem::ManuallyDrop::into_inner(dst.pResource);
        let _ = std::mem::ManuallyDrop::into_inner(src.pResource);
        let barrier = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                Transition: std::mem::ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                    pResource: std::mem::ManuallyDrop::new(Some(tex.clone())),
                    Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                    StateBefore: D3D12_RESOURCE_STATE_COPY_DEST,
                    StateAfter: D3D12_RESOURCE_STATE_SHADING_RATE_SOURCE,
                }),
            },
        };
        list.list.ResourceBarrier(&[barrier]);
        list.close();
        host.execute_sync_internal(&list);
        drop(upload);
        tracing::info!(
            tiles_x,
            tiles_y,
            "VRS Tier 2 foveated shading-rate image ready"
        );
        Ok(tex)
    }
}

/// Applies the configured host VRS state to `list` for a graphics draw with
/// color target count `color_targets` and active viewport `(vp_w, vp_h)`.
///
/// Only applies coarse shading on main color passes (`color_targets > 0` and
/// `vp_w >= 640 && vp_h >= 360`); shadow/depth-only and small passes are always
/// kept at `1x1`.
pub(crate) fn apply_for_draw(
    list: &crate::host::HostList,
    color_targets: usize,
    vp_w: u32,
    vp_h: u32,
    last_key: &mut u64,
) {
    let m = mode();
    if matches!(m, Mode::Off | Mode::Title) {
        return;
    }
    let Some(host) = crate::host::host_made() else {
        return;
    };
    let c = caps(host);
    if c.tier == 0 {
        return;
    }
    let eligible = color_targets > 0 && vp_w >= 640 && vp_h >= 360;
    let (rate, image_dims) = if !eligible {
        (D3D12_SHADING_RATE_1X1, None)
    } else {
        match m {
            Mode::Soft if c.tier >= 2 && c.tile_size > 0 => (
                D3D12_SHADING_RATE_1X1,
                Some((vp_w.div_ceil(c.tile_size), vp_h.div_ceil(c.tile_size))),
            ),
            Mode::Soft => (D3D12_SHADING_RATE_1X1, None),
            Mode::Rate1x2 => (D3D12_SHADING_RATE_1X2, None),
            Mode::Rate2x1 => (D3D12_SHADING_RATE_2X1, None),
            Mode::Rate2x2 => (D3D12_SHADING_RATE_2X2, None),
            Mode::Off | Mode::Title => return,
        }
    };
    let key = (u64::from(m as u8) << 56)
        | (u64::from(rate.0 as u32) << 48)
        | match image_dims {
            Some((tx, ty)) => (u64::from(tx) << 24) | u64::from(ty),
            None => 0,
        };
    if *last_key == key {
        return;
    }
    let Ok(l5) = list.list.cast::<ID3D12GraphicsCommandList5>() else {
        return;
    };
    if let Some((tx, ty)) = image_dims
        && let Some(img) = foveated_image(host, tx, ty)
    {
        let combiners = [
            D3D12_SHADING_RATE_COMBINER_PASSTHROUGH,
            D3D12_SHADING_RATE_COMBINER_OVERRIDE,
        ];
        // SAFETY: open command list, resource in SHADING_RATE_SOURCE state.
        unsafe {
            l5.RSSetShadingRate(D3D12_SHADING_RATE_1X1, Some(combiners.as_ptr()));
            l5.RSSetShadingRateImage(&img);
        }
        list.keep(&img);
        *last_key = key;
        return;
    }
    let combiners = [
        D3D12_SHADING_RATE_COMBINER_PASSTHROUGH,
        D3D12_SHADING_RATE_COMBINER_PASSTHROUGH,
    ];
    // SAFETY: open command list; clears Tier 2 image if Tier 2 is supported.
    unsafe {
        l5.RSSetShadingRate(rate, Some(combiners.as_ptr()));
        if c.tier >= 2 {
            l5.RSSetShadingRateImage(None);
        }
    }
    *last_key = key;
}

/// Forwards a title's `RSSetShadingRate(base, combiners)` call when `VICTORY_VRS=title`.
pub(crate) fn forward_title_rate(
    list: &crate::host::HostList,
    base_rate: u32,
    combiners: Option<[u32; 2]>,
) {
    if mode() != Mode::Title {
        return;
    }
    let Some(host) = crate::host::host_made() else {
        return;
    };
    if caps(host).tier == 0 {
        return;
    }
    let Ok(l5) = list.list.cast::<ID3D12GraphicsCommandList5>() else {
        return;
    };
    let rate = D3D12_SHADING_RATE(base_rate as i32);
    let comb = combiners.map(|[a, b]| [D3D12_SHADING_RATE_COMBINER(a as i32), D3D12_SHADING_RATE_COMBINER(b as i32)]);
    // SAFETY: open list; validated D3D12 shading rate enum values.
    unsafe {
        l5.RSSetShadingRate(rate, comb.as_ref().map(|c| c.as_ptr()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foveated_map_keeps_center_full_res_and_coarsens_periphery() {
        // 1920x1080 with 16x16 tiles = 120x68 tiles.
        let (tx, ty) = (120, 68);
        let map = build_foveated_map(tx, ty);
        assert_eq!(map.len(), (tx * ty) as usize);
        // Center tile is 1x1 (0x0).
        assert_eq!(map[((ty / 2) * tx + (tx / 2)) as usize], 0x0);
        // Extreme corners are 2x2 (0x5).
        assert_eq!(map[0], 0x5);
        assert_eq!(map[(tx - 1) as usize], 0x5);
        assert_eq!(map[((ty - 1) * tx) as usize], 0x5);
        assert_eq!(map[(ty * tx - 1) as usize], 0x5);
        // Mid-right ring (nx ~ 0.80) is horizontally coarsened 2x1 (0x4);
        // mid-top ring (ny ~ -0.80) is vertically coarsened 1x2 (0x1).
        assert_eq!(map[((ty / 2) * tx + (tx * 9 / 10)) as usize], 0x4);
        assert_eq!(map[((ty / 10) * tx + (tx / 2)) as usize], 0x1);
    }

    #[test]
    fn mode_parsing_covers_all_options() {
        assert_eq!(Mode::parse(""), Mode::Off);
        assert_eq!(Mode::parse("0"), Mode::Off);
        assert_eq!(Mode::parse("title"), Mode::Title);
        assert_eq!(Mode::parse("soft"), Mode::Soft);
        assert_eq!(Mode::parse("tier2"), Mode::Soft);
        assert_eq!(Mode::parse("1x2"), Mode::Rate1x2);
        assert_eq!(Mode::parse("2x1"), Mode::Rate2x1);
        assert_eq!(Mode::parse("2x2"), Mode::Rate2x2);
    }
}
