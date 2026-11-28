//! Lightweight, release-safe live per-shader / per-pass profiler (`VICTORY_PROF=1`,
//! live-toggleable from the F1 menu).
//!
//! When disabled (`VICTORY_PROF=0`/unset), costs a single relaxed atomic load.
//! When enabled, samples 1 in 64 draws/dispatches per thread (`Instant` around
//! the draw/dispatch record call) and aggregates per-pipeline shader hash counts
//! and estimated CPU time, logging the Top-10 heaviest shaders every 5 seconds
//! so bottlenecks can be identified directly in `--release` runs (and optionally
//! paired with `VICTORY_PIX=1` for Nsight/PIX captures).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Sampling period: measure CPU time on 1 of every `SAMPLE_MASK + 1` draws.
pub(crate) const SAMPLE_MASK: u64 = 63;

fn on_cell() -> &'static AtomicBool {
    static ON: OnceLock<AtomicBool> = OnceLock::new();
    ON.get_or_init(|| {
        AtomicBool::new(std::env::var_os("VICTORY_PROF").is_some_and(|v| v == "1"))
    })
}

/// Whether live per-shader profiling is enabled.
#[inline]
pub(crate) fn on() -> bool {
    on_cell().load(Ordering::Relaxed)
}

/// Toggles `VICTORY_PROF` live from the F1 menu.
pub(crate) fn set_live(enabled: bool) {
    on_cell().store(enabled, Ordering::Relaxed);
    tracing::info!(enabled, "pass/shader profiler: updated live from F1 menu");
}

#[derive(Default, Clone, Copy)]
struct Entry {
    calls: u64,
    samples: u64,
    sample_ns: u64,
    is_compute: bool,
}

impl Entry {
    fn est_cpu_us(&self) -> f64 {
        if self.samples == 0 {
            return 0.0;
        }
        (self.sample_ns as f64 / self.samples as f64) * (self.calls as f64) / 1_000.0
    }
}

struct State {
    window_start: Instant,
    frames: u64,
    by_hash: HashMap<u64, Entry>,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(State {
            window_start: Instant::now(),
            frames: 0,
            by_hash: HashMap::with_capacity(256),
        })
    })
}

#[derive(Clone, Debug)]
pub(crate) struct TopRow {
    pub(crate) kind: &'static str,
    pub(crate) hash: u64,
    pub(crate) us_per_frame: f64,
    pub(crate) calls_per_frame: f64,
}

fn last_top() -> &'static Mutex<Vec<TopRow>> {
    static TOP: OnceLock<Mutex<Vec<TopRow>>> = OnceLock::new();
    TOP.get_or_init(|| Mutex::new(Vec::new()))
}

/// Returns the most recent Top-N shader CPU snapshot collected by `on_present`
/// (pure memory read, never blocks rendering).
pub(crate) fn top_snapshot(limit: usize) -> Vec<TopRow> {
    let guard = last_top().lock().unwrap_or_else(|e| e.into_inner());
    guard.iter().take(limit).cloned().collect()
}

/// Total sampled/counted draws seen by `prof` since start.
pub(crate) fn total_draws() -> u64 {
    TOTAL_DRAWS_PROFILED.load(Ordering::Relaxed)
}

static TOTAL_DRAWS_PROFILED: AtomicU64 = AtomicU64::new(0);

/// Records one sampled shader/pipeline invocation (`sample_ns = 0` when not
/// timed on this call, or elapsed nanoseconds when timed on the 1-in-64 sample).
/// Thread-local batching keeps global mutex overhead negligible.
pub(crate) fn record_shader(hash: u64, is_compute: bool, cpu_ns: Option<u64>) {
    if hash == 0 {
        return;
    }
    TOTAL_DRAWS_PROFILED.fetch_add(1, Ordering::Relaxed);

    thread_local! {
        static LOCAL: std::cell::RefCell<(u32, HashMap<u64, Entry>)> =
            std::cell::RefCell::new((0, HashMap::with_capacity(64)));
    }

    LOCAL.with(|cell| {
        let mut guard = cell.borrow_mut();
        let entry = guard.1.entry(hash).or_insert(Entry {
            calls: 0,
            samples: 0,
            sample_ns: 0,
            is_compute,
        });
        entry.calls += 1;
        if let Some(ns) = cpu_ns {
            entry.samples += 1;
            entry.sample_ns += ns;
        }
        guard.0 += 1;
        if guard.0 >= 256 {
            guard.0 = 0;
            let mut global = state().lock().unwrap_or_else(|e| e.into_inner());
            for (h, e) in guard.1.drain() {
                let g = global.by_hash.entry(h).or_insert(Entry {
                    calls: 0,
                    samples: 0,
                    sample_ns: 0,
                    is_compute: e.is_compute,
                });
                g.calls += e.calls;
                g.samples += e.samples;
                g.sample_ns += e.sample_ns;
            }
        }
    });
}

/// Called once per presented frame from `host.rs` / `queue.rs`: logs Top-10
/// heaviest shaders/passes every 5 seconds when `VICTORY_PROF=1`.
pub(crate) fn on_present() {
    if !on() {
        return;
    }
    let mut global = state().lock().unwrap_or_else(|e| e.into_inner());
    global.frames += 1;
    let elapsed = global.window_start.elapsed();
    if elapsed < Duration::from_secs(5) {
        return;
    }
    let frames = global.frames.max(1) as f64;
    let mut rows: Vec<(u64, Entry, f64)> = global
        .by_hash
        .drain()
        .map(|(h, e)| {
            let us_per_frame = e.est_cpu_us() / frames;
            (h, e, us_per_frame)
        })
        .collect();
    global.frames = 0;
    global.window_start = Instant::now();
    drop(global);

    rows.sort_by(|a, b| b.2.total_cmp(&a.2));
    if rows.is_empty() {
        return;
    }
    {
        let mut top = last_top().lock().unwrap_or_else(|e| e.into_inner());
        top.clear();
        for &(hash, e, us_frame) in rows.iter().take(20) {
            top.push(TopRow {
                kind: if e.is_compute { "CS" } else { "PS" },
                hash,
                us_per_frame: us_frame,
                calls_per_frame: e.calls as f64 / frames,
            });
        }
    }
    let mut summary = String::new();
    for (i, (hash, e, us_frame)) in rows.iter().take(10).enumerate() {
        if i > 0 {
            summary.push_str(", ");
        }
        let kind = if e.is_compute { "CS" } else { "PS" };
        let calls_f = e.calls as f64 / frames;
        summary.push_str(&format!("{kind}:{hash:016x}={us_frame:.0}us({calls_f:.0}x)"));
    }
    tracing::info!(
        frames = frames as u64,
        elapsed_ms = elapsed.as_millis() as u64,
        top = %summary,
        "prof: top shaders CPU/frame"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_estimates_cpu_microseconds() {
        let e = Entry {
            calls: 640,
            samples: 10,
            sample_ns: 20_000, // 2 us/call average
            is_compute: false,
        };
        assert!((e.est_cpu_us() - 1280.0).abs() < 1e-6);
    }
}
