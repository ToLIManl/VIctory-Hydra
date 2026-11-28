//! Unattended world-gameplay benchmark & auto-exit harness (`VICTORY_BENCH_WORLD_S`).
//!
//! When `VICTORY_BENCH_WORLD_S=<seconds>` is set:
//! 1. Waits through boot/menus/loading screens until the first real 3D world frame
//!    (`draws_in_frame >= VICTORY_BENCH_MIN_DRAWS`, default `500`).
//! 2. Skips `VICTORY_BENCH_WARMUP_S` seconds (default `5.0`) for streaming settle.
//! 3. Optionally triggers `VICTORY_RECORD` by writing `<VICTORY_BENCH_RECORD_DIR>/go`.
//! 4. Measures steady-state world frametimes + draws for `<seconds>`, prints a machine-
//!    readable `BENCH_RESULT:` summary, and exits cleanly with code 0.
//!
//! When unset or `0`, adds a single relaxed branch per `Present`.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

struct Config {
    world_s: f64,
    warmup_s: f64,
    min_draws: u64,
    abort_s: f64,
    record_dir: Option<PathBuf>,
    start_file: Option<PathBuf>,
}

fn config() -> Option<&'static Config> {
    static CFG: OnceLock<Option<Config>> = OnceLock::new();
    CFG.get_or_init(|| {
        let world_s: f64 = std::env::var("VICTORY_BENCH_WORLD_S")
            .ok()?
            .trim()
            .parse()
            .ok()
            .filter(|&v: &f64| v.is_finite() && v > 0.0)?;
        let warmup_s = std::env::var("VICTORY_BENCH_WARMUP_S")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|&v| v.is_finite() && v >= 0.0)
            .unwrap_or(5.0);
        let min_draws = std::env::var("VICTORY_BENCH_MIN_DRAWS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(500);
        let abort_s = std::env::var("VICTORY_BENCH_ABORT_S")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|&v| v.is_finite() && v >= 30.0)
            .unwrap_or(240.0);
        let record_dir = std::env::var_os("VICTORY_BENCH_RECORD_DIR")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from);
        let start_file = std::env::var_os("VICTORY_BENCH_START_FILE")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from);
        Some(Config {
            world_s,
            warmup_s,
            min_draws,
            abort_s,
            record_dir,
            start_file,
        })
    })
    .as_ref()
}

/// Set once the first 3D world frame (`draws >= min_draws`) has been presented.
static WORLD_ACTIVE: AtomicBool = AtomicBool::new(false);

#[allow(dead_code)]
pub(crate) fn world_entered() -> bool {
    WORLD_ACTIVE.load(Relaxed)
}

struct State {
    boot: Instant,
    last_present: Instant,
    last_drawn: u64,
    world_at: Option<Instant>,
    locked_at: Option<Instant>,
    triggered_record: bool,
    samples: Vec<f32>,
    draws_sum: u64,
}

fn summarize(samples: &mut [f32]) -> (f32, f32, f32, f32) {
    if samples.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let mean = samples.iter().sum::<f32>() / samples.len() as f32;
    samples.sort_by(f32::total_cmp);
    let median = samples[samples.len() / 2];
    let p90 = samples[(samples.len() * 9) / 10];
    let p99 = samples[(samples.len() * 99) / 100];
    (mean, median, p90, p99)
}

/// Called once per presented frame from `queue::present`.
pub(crate) fn presented(frame: u64) {
    let drawn = crate::draw::drawn();
    let cfg = config();
    let ab_world = std::env::var_os("VICTORY_AB_WORLD").is_some_and(|v| v == "1");
    if cfg.is_none() && !ab_world {
        return;
    }
    static STATE: Mutex<Option<State>> = Mutex::new(None);
    let now = Instant::now();
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let st = guard.get_or_insert_with(|| State {
        boot: now,
        last_present: now,
        last_drawn: drawn,
        world_at: None,
        locked_at: None,
        triggered_record: false,
        samples: Vec::with_capacity(1024),
        draws_sum: 0,
    });
    let frame_draws = drawn.saturating_sub(st.last_drawn);
    let dt_ms = (now - st.last_present).as_secs_f32() * 1000.0;
    st.last_present = now;
    st.last_drawn = drawn;

    let min_draws = cfg.map_or(500, |c| c.min_draws);
    if st.world_at.is_none() && frame_draws >= min_draws {
        st.world_at = Some(now);
        WORLD_ACTIVE.store(true, Relaxed);
        let boot_s = (now - st.boot).as_secs_f64();
        let line = format!("BENCH_WORLD_ENTERED: frame={frame} boot_s={boot_s:.1} draws={frame_draws}");
        tracing::info!("{line}");
        println!("{line}");
    }

    let Some(cfg) = cfg else {
        return;
    };

    if let Some(world_at) = st.world_at {
        let anchor = if let Some(start_file) = &cfg.start_file {
            if st.locked_at.is_none() {
                if start_file.exists() {
                    st.locked_at = Some(now);
                    let line = format!("BENCH_LOCK_CONFIRMED: frame={frame} since_world_s={:.1}", (now - world_at).as_secs_f64());
                    tracing::info!("{line}");
                    println!("{line}");
                } else {
                    if (now - st.boot).as_secs_f64() >= cfg.abort_s {
                        let line = format!("BENCH_TIMEOUT: boot_s={:.1} frame={frame} waiting_for_lock=1", (now - st.boot).as_secs_f64());
                        tracing::error!("{line}");
                        println!("{line}");
                        let _ = std::io::stdout().flush();
                        let _ = std::io::stderr().flush();
                        std::process::exit(3);
                    }
                    return;
                }
            }
            st.locked_at.unwrap_or(world_at)
        } else {
            world_at
        };
        let world_s = (now - anchor).as_secs_f64();
        if world_s >= cfg.warmup_s {
            if !st.triggered_record
                && let Some(dir) = &cfg.record_dir
            {
                st.triggered_record = true;
                let _ = std::fs::create_dir_all(dir);
                let go = dir.join("go");
                if std::fs::write(&go, b"1").is_ok() {
                    tracing::info!(path = %go.display(), "BENCH_RECORD_TRIGGERED");
                }
            }
            if frame_draws >= cfg.min_draws && dt_ms > 0.0 && dt_ms < 10_000.0 {
                st.samples.push(dt_ms);
                st.draws_sum += frame_draws;
            }
        }
        if world_s >= cfg.warmup_s + cfg.world_s {
            let boot_s = (world_at - st.boot).as_secs_f64();
            let (mean_ms, median_ms, p90_ms, p99_ms) = summarize(&mut st.samples);
            let n = st.samples.len();
            let median_fps = if median_ms > 0.0 { 1000.0 / median_ms } else { 0.0 };
            let mean_fps = if mean_ms > 0.0 { 1000.0 / mean_ms } else { 0.0 };
            let avg_draws = st.draws_sum as f64 / n.max(1) as f64;
            let line = format!(
                "BENCH_RESULT: boot_s={boot_s:.1} world_s={:.1} frames={n} median_fps={median_fps:.2} mean_fps={mean_fps:.2} median_ms={median_ms:.1} mean_ms={mean_ms:.1} p90_ms={p90_ms:.1} p99_ms={p99_ms:.1} avg_draws={avg_draws:.0}",
                world_s - cfg.warmup_s
            );
            tracing::info!("{line}");
            println!("{line}");
            let _ = std::io::stdout().flush();
            let _ = std::io::stderr().flush();
            std::process::exit(0);
        }
    } else if (now - st.boot).as_secs_f64() >= cfg.abort_s {
        let line = format!(
            "BENCH_TIMEOUT: boot_s={:.1} frame={frame} last_draws={frame_draws}",
            (now - st.boot).as_secs_f64()
        );
        tracing::error!("{line}");
        println!("{line}");
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
        std::process::exit(3);
    }
}

#[cfg(test)]
mod tests {
    use super::summarize;

    #[test]
    fn summarizes_frametimes() {
        let mut s = [50.0, 20.0, 30.0, 40.0, 100.0];
        let (mean, med, p90, p99) = summarize(&mut s);
        assert!((mean - 48.0).abs() < 1e-4);
        assert_eq!(med, 40.0);
        assert_eq!(p90, 100.0);
        assert_eq!(p99, 100.0);
    }
}
