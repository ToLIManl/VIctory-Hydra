//! Native D3D12 PIX / NVIDIA Nsight Graphics event markers (`VICTORY_PIX=1`).
//!
//! Uses `ID3D12GraphicsCommandList::BeginEvent` / `EndEvent` directly with
//! `PIX_EVENT_ANSI_VERSION = 1` (zero external DLLs, zero heap allocations,
//! single relaxed atomic load when disabled). Live-toggleable from the F1 menu.

use std::ffi::CStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Graphics::Direct3D12::ID3D12GraphicsCommandList;

const PIX_EVENT_ANSI_VERSION: u32 = 1;

fn enabled_cell() -> &'static AtomicBool {
    static ON: OnceLock<AtomicBool> = OnceLock::new();
    ON.get_or_init(|| AtomicBool::new(std::env::var_os("VICTORY_PIX").is_some_and(|v| v == "1")))
}

/// Whether native PIX / Nsight command-list markers are emitted.
#[inline]
pub(crate) fn on() -> bool {
    enabled_cell().load(Ordering::Relaxed)
}

/// Toggles native PIX / Nsight markers live from the F1 menu.
pub(crate) fn set_live(enabled: bool) {
    enabled_cell().store(enabled, Ordering::Relaxed);
    tracing::info!(enabled, "PIX/Nsight GPU markers: updated live from F1 menu");
}

/// RAII scope that pairs `BeginEvent` / `EndEvent` on `list` when [`on`] is true.
pub(crate) struct Scope<'a> {
    list: Option<&'a ID3D12GraphicsCommandList>,
}

impl<'a> Scope<'a> {
    #[inline]
    pub(crate) fn begin(list: &'a ID3D12GraphicsCommandList, name: &'static CStr) -> Self {
        if !on() {
            return Self { list: None };
        }
        let bytes = name.to_bytes_with_nul();
        // SAFETY: an open command list and a static NUL-terminated ASCII string.
        unsafe {
            list.BeginEvent(
                PIX_EVENT_ANSI_VERSION,
                Some(bytes.as_ptr().cast()),
                bytes.len() as u32,
            );
        }
        Self { list: Some(list) }
    }
}

impl Drop for Scope<'_> {
    #[inline]
    fn drop(&mut self) {
        if let Some(list) = self.list {
            // SAFETY: matched with `BeginEvent` on the same open command list.
            unsafe { list.EndEvent() };
        }
    }
}
