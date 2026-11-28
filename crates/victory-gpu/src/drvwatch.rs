//! The graphics driver's errors, seen as the system records them.
//!
//! Ported from `realblastex/flamingo` (`drvwatch.rs`, GPL-3.0-or-later):
//! subscribes to Windows Event Log `System` channel for `nvlddmkm` (Event 153),
//! `amdkmdag`, `amdwddmg`, `igfx`, `igfxn`, and `Display`. Logs each event with
//! our clock and trips [`tripped`] so suspect fast paths (such as draw windows)
//! fall back to per-record clamping instead of triggering a full TDR device loss.
//!
//! On by default; `VICTORY_DRIVER_WATCH=0` subscribes to nothing.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

use windows_sys::Win32::System::EventLog::{
    EVT_HANDLE, EVT_SUBSCRIBE_NOTIFY_ACTION, EvtSubscribe, EvtSubscribeActionDeliver,
    EvtSubscribeToFutureEvents,
};

/// Entries of a display driver seen since the start.
static ERRORS: AtomicU64 = AtomicU64::new(0);
/// Whether one was seen ([`tripped`]).
static TRIPPED: AtomicBool = AtomicBool::new(false);

/// Whether the card's driver has reported an error in this run: what could
/// have caused it stops for the rest of the run.
pub(crate) fn tripped() -> bool {
    TRIPPED.load(Relaxed)
}

/// Driver errors seen since the start.
#[allow(dead_code)]
pub(crate) fn errors() -> u64 {
    ERRORS.load(Relaxed)
}

/// An entry of a display driver was recorded.
fn seen() {
    let n = ERRORS.fetch_add(1, Relaxed) + 1;
    let first = !TRIPPED.swap(true, Relaxed);
    tracing::error!(
        n,
        "the graphics driver reported an error to the system's event log{}",
        if first {
            ": from now on suspect fast paths (draw windows) fall back to per-record clamping"
        } else {
            ""
        }
    );
}

unsafe extern "system" fn on_event(
    action: EVT_SUBSCRIBE_NOTIFY_ACTION,
    _context: *const core::ffi::c_void,
    _event: EVT_HANDLE,
) -> u32 {
    if action == EvtSubscribeActionDeliver {
        seen();
    }
    0
}

/// Subscribes to the display drivers' entries in the system's event log,
/// once. Entries from before the call are not delivered.
pub(crate) fn start() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if std::env::var_os("VICTORY_DRIVER_WATCH").is_some_and(|v| v == "0") {
            return;
        }
        let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
        let channel = wide("System");
        let query = wide(
            "*[System[Provider[@Name='nvlddmkm' or @Name='amdkmdag' or @Name='amdwddmg' or @Name='igfx' or @Name='igfxn' \
             or @Name='Display']]]",
        );
        // SAFETY: valid null-terminated UTF-16 strings; `on_event` touches only
        // atomics/tracing; subscription stays active for the process lifetime.
        let subscription = unsafe {
            EvtSubscribe(
                0,
                std::ptr::null_mut(),
                channel.as_ptr(),
                query.as_ptr(),
                0,
                std::ptr::null(),
                Some(on_event),
                EvtSubscribeToFutureEvents,
            )
        };
        if subscription == 0 {
            tracing::warn!(
                error = %std::io::Error::last_os_error(),
                "no subscription to the system's event log: display driver errors are not watched"
            );
        } else {
            tracing::info!(
                "display driver event-log watchdog active (nvlddmkm/Display pre-TDR; VICTORY_DRIVER_WATCH=0 to disable)"
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_error_trips_the_switch_and_every_one_is_counted() {
        assert!(!tripped());
        assert_eq!(errors(), 0);
        seen();
        assert!(tripped());
        seen();
        assert_eq!(errors(), 2);
        assert!(tripped());
    }

    #[test]
    fn the_system_takes_the_subscription() {
        use windows_sys::Win32::System::EventLog::EvtClose;
        let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
        let (channel, query) = (
            wide("System"),
            wide("*[System[Provider[@Name='nvlddmkm' or @Name='Display']]]"),
        );
        // SAFETY: valid null-terminated UTF-16 strings; subscription is closed immediately.
        let subscription = unsafe {
            EvtSubscribe(
                0,
                std::ptr::null_mut(),
                channel.as_ptr(),
                query.as_ptr(),
                0,
                std::ptr::null(),
                Some(on_event),
                EvtSubscribeToFutureEvents,
            )
        };
        assert_ne!(subscription, 0, "{}", std::io::Error::last_os_error());
        // SAFETY: valid handle returned by `EvtSubscribe`.
        unsafe { EvtClose(subscription) };
    }
}
