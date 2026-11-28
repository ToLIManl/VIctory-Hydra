//! Asynchronous, release-safe live GPU per-shader / per-pass timestamp profiler
//! (`VICTORY_GPU_PASS_PROF=1` or `<period_frames>`).
//!
//! Unlike `VICTORY_GPU_TIMING=1`, this profiler NEVER forces `late_stage=0`,
//! NEVER breaks `VICTORY_RELAY=2` / `VICTORY_ASYNC_SUBMIT=1`, and NEVER blocks
//! on the GPU. Instead, once every `period` world frames (default 60), it opens
//! a single-frame timestamp window, brackets draws/dispatches and engine passes
//! (`DLSS`, `Ambient`, `Compare`, `Hash`, `Detile`) with `EndQuery(TIMESTAMP)`,
//! resolves the query heap asynchronously at present, and reads the readback
//! buffer once the host queue fence has passed.
//!
//! When disabled (`VICTORY_GPU_PASS_PROF` unset or `0`), costs a single relaxed
//! atomic load per draw/dispatch.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Graphics::Direct3D12::*;

/// Maximum timestamp queries captured in a single profiled frame (8,192 spans).
const MAX_QUERIES: u32 = 16_384;

/// Category of a GPU-timed command or engine pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PassKind {
    Cs,
    Draw,
    Indexed,
    Mesh,
    Indirect,
    Copy,
    Clear,
    Ambient,
    Dlss,
    Compare,
    Hash,
    Detile,
}

impl PassKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Cs => "CS",
            Self::Draw => "DRAW",
            Self::Indexed => "IDX",
            Self::Mesh => "MESH",
            Self::Indirect => "IND",
            Self::Copy => "COPY",
            Self::Clear => "CLEAR",
            Self::Ambient => "AMBIENT",
            Self::Dlss => "DLSS",
            Self::Compare => "COMPARE",
            Self::Hash => "HASH",
            Self::Detile => "DETILE",
        }
    }

    fn is_engine(self) -> bool {
        matches!(
            self,
            Self::Ambient | Self::Dlss | Self::Compare | Self::Hash | Self::Detile
        )
    }
}

#[derive(Clone, Copy)]
struct SpanRecord {
    slot: u32,
    kind: PassKind,
    hash: u64,
    native_wave64: bool,
}

/// Capturing gate checked on the hot path (single relaxed atomic load when off).
static CAPTURING: AtomicBool = AtomicBool::new(false);
static NEXT_SLOT: AtomicU32 = AtomicU32::new(0);

/// Returns capture period in frames (`0` = disabled).
fn period_frames() -> u64 {
    static PERIOD: OnceLock<u64> = OnceLock::new();
    *PERIOD.get_or_init(|| {
        let Some(raw) = std::env::var("VICTORY_GPU_PASS_PROF").ok() else {
            return 0;
        };
        let s = raw.trim();
        if s.is_empty() || s == "0" || s.eq_ignore_ascii_case("off") {
            return 0;
        }
        if s == "1" || s.eq_ignore_ascii_case("on") {
            return 60;
        }
        s.parse::<u64>().unwrap_or(60).clamp(10, 600)
    })
}

/// Fast-path check for whether the current frame is recording GPU timestamps.
#[inline]
pub(crate) fn capturing() -> bool {
    CAPTURING.load(Ordering::Relaxed)
}

struct GpuResources {
    heap: ID3D12QueryHeap,
    back: ID3D12Resource,
    data_ptr: usize,
    ticks_per_ms: f64,
    resolve_alloc: ID3D12CommandAllocator,
    resolve_list: ID3D12GraphicsCommandList,
}

// SAFETY: D3D12 objects are free-threaded; `data_ptr` points to a persistently
// mapped READBACK buffer only read after the resolve fence completes.
unsafe impl Send for GpuResources {}
// SAFETY: `heap` is shared read-only for `EndQuery` on open command lists while
// capturing; all mutable state is behind `STATE`.
unsafe impl Sync for GpuResources {}

static RESOURCES: OnceLock<Option<GpuResources>> = OnceLock::new();

fn resources(host: &crate::host::Host) -> Option<&'static GpuResources> {
    RESOURCES
        .get_or_init(|| init_resources(host).inspect_err(|e| tracing::warn!(error = %e, "gpu_prof: disabled")).ok())
        .as_ref()
}

fn init_resources(host: &crate::host::Host) -> windows::core::Result<GpuResources> {
    // SAFETY: plain D3D12 object creation for timestamp profiling.
    unsafe {
        let mut heap: Option<ID3D12QueryHeap> = None;
        host.device.CreateQueryHeap(
            &D3D12_QUERY_HEAP_DESC {
                Type: D3D12_QUERY_HEAP_TYPE_TIMESTAMP,
                Count: MAX_QUERIES,
                NodeMask: 0,
            },
            &mut heap,
        )?;
        let heap = heap.ok_or_else(windows::core::Error::empty)?;
        let bytes = u64::from(MAX_QUERIES) * 8;
        let back = host
            .read_back_buffer(bytes)
            .ok_or_else(windows::core::Error::empty)?;
        let mut data = std::ptr::null_mut();
        back.Map(0, None, Some(&mut data))?;
        std::ptr::write_bytes(data.cast::<u8>(), 0, bytes as usize);
        let resolve_alloc: ID3D12CommandAllocator =
            host.device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)?;
        let resolve_list: ID3D12GraphicsCommandList = host.device.CreateCommandList(
            0,
            D3D12_COMMAND_LIST_TYPE_DIRECT,
            &resolve_alloc,
            None,
        )?;
        resolve_list.Close()?;
        let freq = host.queue().GetTimestampFrequency()?;
        if freq == 0 {
            return Err(windows::core::Error::empty());
        }
        Ok(GpuResources {
            heap,
            back,
            data_ptr: data as usize,
            ticks_per_ms: freq as f64 / 1000.0,
            resolve_alloc,
            resolve_list,
        })
    }
}

enum Phase {
    Idle { next_frame: u64 },
    Capturing { started_frame: u64 },
    WaitingResolve { frame: u64, fence: u64, count: u32 },
}

struct ProfState {
    phase: Phase,
    world_frames: u64,
    records: Vec<SpanRecord>,
}

static STATE: Mutex<ProfState> = Mutex::new(ProfState {
    phase: Phase::Idle { next_frame: 0 },
    world_frames: 0,
    records: Vec::new(),
});

/// Starts a GPU timestamp span on an open `DIRECT` command list if the current
/// frame is being profiled. Returns a token to pass to [`end`].
#[inline]
pub(crate) fn begin(
    list: &ID3D12GraphicsCommandList,
    kind: PassKind,
    hash: u64,
    native_wave64: bool,
) -> Option<u32> {
    if !CAPTURING.load(Ordering::Relaxed) {
        return None;
    }
    begin_slow(list, kind, hash, native_wave64)
}

#[cold]
fn begin_slow(
    list: &ID3D12GraphicsCommandList,
    kind: PassKind,
    hash: u64,
    native_wave64: bool,
) -> Option<u32> {
    let res = RESOURCES.get()?.as_ref()?;
    let slot = NEXT_SLOT.fetch_add(2, Ordering::Relaxed);
    if slot.checked_add(2).is_none_or(|end| end > MAX_QUERIES) {
        return None;
    }
    // SAFETY: `list` is an open DIRECT command list; `slot < MAX_QUERIES`.
    unsafe { list.EndQuery(&res.heap, D3D12_QUERY_TYPE_TIMESTAMP, slot) };
    STATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .records
        .push(SpanRecord {
            slot,
            kind,
            hash,
            native_wave64,
        });
    Some(slot)
}

/// Closes a GPU timestamp span started by [`begin`].
#[inline]
pub(crate) fn end(list: &ID3D12GraphicsCommandList, token: Option<u32>) {
    if let Some(slot) = token {
        end_slow(list, slot);
    }
}

#[cold]
fn end_slow(list: &ID3D12GraphicsCommandList, slot: u32) {
    let Some(res) = RESOURCES.get().and_then(|r| r.as_ref()) else {
        return;
    };
    // SAFETY: `list` is an open DIRECT command list; `slot + 1 < MAX_QUERIES`.
    unsafe { list.EndQuery(&res.heap, D3D12_QUERY_TYPE_TIMESTAMP, slot + 1) };
}

/// Advances the asynchronous 1-frame GPU capture state machine once per present.
pub(crate) fn on_present(host: &crate::host::Host, frame: u64, frame_draws: u64) {
    let period = period_frames();
    if period == 0 {
        return;
    }
    let Some(res) = resources(host) else {
        return;
    };
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if frame_draws >= 200 {
        st.world_frames += 1;
    }
    match st.phase {
        Phase::Idle { next_frame } => {
            // Wait until we are in the 3D world (at least 10 world frames) so we
            // don't waste captures on 1-draw splash screens.
            if st.world_frames >= 10 && frame >= next_frame {
                st.records.clear();
                NEXT_SLOT.store(0, Ordering::Release);
                CAPTURING.store(true, Ordering::Release);
                st.phase = Phase::Capturing { started_frame: frame };
            }
        }
        Phase::Capturing { started_frame } => {
            CAPTURING.store(false, Ordering::Release);
            let count = NEXT_SLOT.load(Ordering::Acquire).min(MAX_QUERIES);
            if count < 2 || st.records.is_empty() {
                st.records.clear();
                st.phase = Phase::Idle {
                    next_frame: frame + period,
                };
                return;
            }
            // Submit an async ResolveQueryData on the host queue without blocking.
            if let Some(fence) = host.submit_gpu_prof_resolve(
                &res.resolve_alloc,
                &res.resolve_list,
                &res.heap,
                &res.back,
                count,
            ) {
                st.phase = Phase::WaitingResolve {
                    frame: started_frame,
                    fence,
                    count,
                };
            } else {
                st.records.clear();
                st.phase = Phase::Idle {
                    next_frame: frame + period,
                };
            }
        }
        Phase::WaitingResolve {
            frame: captured_frame,
            fence,
            count,
        } => {
            if host.completed_fence() < fence {
                return;
            }
            let records = std::mem::take(&mut st.records);
            st.phase = Phase::Idle {
                next_frame: frame + period,
            };
            drop(st);
            report(res, captured_frame, count, &records);
        }
    }
}

#[derive(Default)]
struct Agg {
    calls: u32,
    ticks: u64,
    max_ticks: u64,
}

fn report(res: &GpuResources, frame: u64, count: u32, records: &[SpanRecord]) {
    // SAFETY: resolve fence has completed; buffer holds `MAX_QUERIES` u64 entries.
    let stamps: &[u64] =
        unsafe { std::slice::from_raw_parts(res.data_ptr as *const u64, count as usize) };
    let mut by_key: HashMap<(PassKind, u64, bool), Agg> = HashMap::with_capacity(256);
    let mut cat_ticks: HashMap<PassKind, u64> = HashMap::new();
    let mut wave64_ticks: u64 = 0;
    let mut wave32_ticks: u64 = 0;
    let mut engine_ticks: u64 = 0;
    let mut valid_spans: u64 = 0;

    // Cap single-span deltas at 250 ms to ignore cross-submission/unwritten noise.
    let max_reasonable = (res.ticks_per_ms * 250.0) as u64;

    for r in records {
        let s = r.slot as usize;
        if s + 1 >= stamps.len() {
            continue;
        }
        let (t0, t1) = (stamps[s], stamps[s + 1]);
        if t0 == 0 || t1 <= t0 {
            continue;
        }
        let dt = t1 - t0;
        if dt > max_reasonable {
            continue;
        }
        valid_spans += 1;
        *cat_ticks.entry(r.kind).or_default() += dt;
        if r.kind.is_engine() {
            engine_ticks += dt;
        } else if r.native_wave64 {
            wave64_ticks += dt;
        } else {
            wave32_ticks += dt;
        }
        let entry = by_key.entry((r.kind, r.hash, r.native_wave64)).or_default();
        entry.calls += 1;
        entry.ticks += dt;
        entry.max_ticks = entry.max_ticks.max(dt);
    }

    // Clear readback memory for the next capture window.
    // SAFETY: resolve fence is complete and next capture has not started yet.
    unsafe { std::ptr::write_bytes(res.data_ptr as *mut u8, 0, count as usize * 8) };

    let to_ms = |ticks: u64| ticks as f64 / res.ticks_per_ms;
    let total_ms = to_ms(wave64_ticks + wave32_ticks + engine_ticks);
    let cat_ms = |k: PassKind| to_ms(cat_ticks.get(&k).copied().unwrap_or(0));

    tracing::info!(
        "GPU_PASS_PROF frame={frame} spans={valid_spans}/{} sum={total_ms:.2}ms \
         (native_w64={:.2}ms, native_w32={:.2}ms, engine={:.2}ms) | \
         CS={:.2}ms DRAW={:.2}ms IDX={:.2}ms MESH={:.2}ms IND={:.2}ms COPY={:.2}ms CLEAR={:.2}ms | \
         DLSS={:.2}ms AMBIENT={:.2}ms COMPARE={:.2}ms HASH={:.2}ms DETILE={:.2}ms",
        records.len(),
        to_ms(wave64_ticks),
        to_ms(wave32_ticks),
        to_ms(engine_ticks),
        cat_ms(PassKind::Cs),
        cat_ms(PassKind::Draw),
        cat_ms(PassKind::Indexed),
        cat_ms(PassKind::Mesh),
        cat_ms(PassKind::Indirect),
        cat_ms(PassKind::Copy),
        cat_ms(PassKind::Clear),
        cat_ms(PassKind::Dlss),
        cat_ms(PassKind::Ambient),
        cat_ms(PassKind::Compare),
        cat_ms(PassKind::Hash),
        cat_ms(PassKind::Detile),
    );

    let mut rows: Vec<_> = by_key.into_iter().collect();
    rows.sort_by_key(|a| std::cmp::Reverse(a.1.ticks));
    for (rank, ((kind, hash, w64), agg)) in rows.into_iter().take(25).enumerate() {
        let ms = to_ms(agg.ticks);
        let max_ms = to_ms(agg.max_ticks);
        let wave_tag = if kind.is_engine() {
            "engine"
        } else if w64 {
            "W64"
        } else {
            "w32"
        };
        if hash != 0 {
            tracing::info!(
                "  GPU_TOP #{:02} {:<7} {:016x} [{:<6}] {:6.2} ms ({:4}x, max {:5.2} ms)",
                rank + 1,
                kind.label(),
                hash,
                wave_tag,
                ms,
                agg.calls,
                max_ms,
            );
        } else {
            tracing::info!(
                "  GPU_TOP #{:02} {:<7} {:<16} [{:<6}] {:6.2} ms ({:4}x, max {:5.2} ms)",
                rank + 1,
                kind.label(),
                "-",
                wave_tag,
                ms,
                agg.calls,
                max_ms,
            );
        }
    }
}
