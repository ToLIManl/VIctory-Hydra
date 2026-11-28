//! D3D12.X on host D3D12.
//!
//! D3D12.X records commands from code inlined into the title, which writes
//! packets into the command list while its put pointer is below a limit and
//! calls the list's method otherwise. Our lists keep the limits at zero, so
//! every command arrives here as a call, to be run on host D3D12. The
//! title's GPU addresses are CPU addresses (console memory is unified).
//!
//! [`device`] is the device object titles create, [`dxgi`] its adapter side,
//! [`resource`] heaps and resources, [`fence`] fences, [`queue`] command
//! queues, [`list`] command allocators and lists, [`descriptor`]
//! descriptor heaps, [`root`] root signatures, [`query`] query heaps,
//! [`signature`] command signatures, [`pipeline`] pipeline states,
//! [`frame`] frame pacing, [`object`] what all console graphics objects
//! share, [`console`] which console model the title sees. Planned:
//! `present` (swap chain).

#[cfg(windows)]
mod ahead;
#[cfg(windows)]
mod ambient;
#[cfg(windows)]
mod bench;
#[cfg(windows)]
mod coalesce;
#[cfg(windows)]
mod epoch;
#[cfg(windows)]
pub mod external;
/// True while the F1 settings menu is open so host keyboard input is kept from the title.
pub static SETTINGS_MENU_OPEN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(all(windows, feature = "diagnostics"))]
mod capture;
#[cfg(windows)]
mod compose;
pub mod console;
#[cfg(all(windows, feature = "diagnostics"))]
mod copystat;
#[cfg(all(windows, feature = "diagnostics"))]
mod faultbook;
#[cfg(windows)]
mod cores;
pub mod descriptor;
#[cfg(windows)]
mod detile;
pub mod device;
#[cfg(windows)]
mod draw;
pub mod dxgi;
pub mod fence;
pub mod frame;
#[cfg(all(windows, feature = "diagnostics"))]
mod book;
#[cfg(windows)]
mod host;
#[cfg(windows)]
pub mod hydra;
#[cfg(windows)]
mod indirect;
#[cfg(windows)]
mod inplace;
#[cfg(windows)]
mod port;
#[cfg(windows)]
mod port_cbv;
#[cfg(windows)]
pub mod port_descriptors;
#[cfg(windows)]
mod port_journal;
#[cfg(windows)]
mod land;
#[cfg(windows)]
mod launch;
pub mod list;
pub mod object;
pub mod pipeline;
#[cfg(windows)]
mod picked;
#[cfg(windows)]
mod pix;
#[cfg(windows)]
mod pool;
#[cfg(all(windows, feature = "diagnostics"))]
mod popwatch;
#[cfg(windows)]
mod primitive;
#[cfg(windows)]
mod prof;
pub mod query;
pub mod queue;
#[cfg(all(windows, feature = "diagnostics"))]
pub mod replay;
#[cfg(windows)]
mod residency;
mod rootsig;
pub mod resource;
pub mod root;
#[cfg(windows)]
mod safecopy;
#[cfg(windows)]
mod sampler;
#[cfg(windows)]
pub mod shader_cache;
#[cfg(windows)]
mod memory;
#[cfg(windows)]
mod shadow;
#[cfg(windows)]
mod tiles;
pub mod signature;
#[cfg(windows)]
pub mod streamline;
mod switch;
#[cfg(all(windows, feature = "diagnostics"))]
mod ab;
#[cfg(windows)]
mod tdr;
#[cfg(windows)]
mod texture;
pub mod timeline;
#[cfg(windows)]
pub mod titlewrite;
#[cfg(windows)]
mod upload;
#[cfg(windows)]
mod watch;
#[cfg(windows)]
mod texback;

#[cfg(windows)]
pub use host::on_failure;

/// Whether `switch` is on now, if it is one of those `VICTORY_AB` turns on and
/// off while the title runs (`ab`).
#[cfg(all(windows, feature = "diagnostics"))]
pub fn switched(switch: &str) -> Option<bool> {
    ab::pick(switch)
}

/// What the frame is at (`copystat::phase`), for the sampling profiler.
#[cfg(all(windows, feature = "diagnostics"))]
pub fn frame_phase() -> u32 {
    copystat::phase()
}

/// Frames the title has begun (its frame waits that ended), for logs.
pub fn frames_begun() -> u64 {
    frame::current()
}
#[cfg(windows)]
pub use inplace::{title_memory_freeing, title_memory_section, title_memory_section_gone, title_memory_view};
#[cfg(windows)]
pub use shadow::title_memory_write_combined;
#[cfg(windows)]
pub use watch::{
    Remapping, kernel_writing, title_memory_allocated, title_memory_freed, title_memory_reading,
    title_memory_remapped, title_memory_remapping, title_memory_remapping_range, title_memory_remapping_ranges,
    title_memory_splitting, title_memory_unmapping, title_memory_written, writing as kernel_will_write,
};

/// Probe the live host D3D12 device: loaded `D3D12Core.dll` path,
/// `ID3D12Device13`, ReBAR `GPU_UPLOAD` heap support/test, VRAM budget, and
/// `inplace::probe`.
#[cfg(windows)]
pub fn probe_host() -> String {
    use std::fmt::Write as _;
    use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};

    let Some(host) = crate::host::host() else {
        return "ERROR: failed to create host D3D12 device".to_owned();
    };
    let mut out = String::new();

    let name: Vec<u16> = "D3D12Core.dll\0".encode_utf16().collect();
    // SAFETY: `name` is null-terminated UTF-16; `GetModuleHandleW` returns the
    // already-loaded `D3D12Core.dll` handle without incrementing refcount.
    let module = unsafe { GetModuleHandleW(name.as_ptr()) };
    let dll_path = if !module.is_null() {
        let mut buf = vec![0u16; 1024];
        // SAFETY: `buf` is a writable UTF-16 slice of length `buf.len()`.
        let len = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
        if len > 0 && len < buf.len() {
            String::from_utf16_lossy(&buf[..len])
        } else {
            format!("<GetModuleFileNameW len={len}>")
        }
    } else {
        "<D3D12Core.dll not loaded>".to_owned()
    };
    let _ = writeln!(out, "D3D12Core.dll: {dll_path}");

    if let Some([budget, used, avail, reserved]) = host.video_memory() {
        let _ = writeln!(
            out,
            "VRAM local: budget={} MiB, used={} MiB, avail_res={} MiB, reserved={} MiB",
            budget >> 20,
            used >> 20,
            avail >> 20,
            reserved >> 20
        );
    }

    let rebar_supported = host.mapped_card_memory();
    let rebar_map_ok = if rebar_supported {
        if let Some((_buf, ptr)) = host.mapped_card_buffer(65536, true) {
            // SAFETY: `mapped_card_buffer(65536, true)` returned a valid mapped
            // pointer to 65536 bytes; test writing one byte.
            unsafe {
                ptr.write_volatile(0x5A);
            }
            true
        } else {
            false
        }
    } else {
        false
    };
    let _ = writeln!(
        out,
        "ReBAR GPUUploadHeapSupported: {rebar_supported}, mapped_card_buffer(64 KiB, uav): {rebar_map_ok}"
    );

    let probe = crate::inplace::probe(&host.device, &|| host.video_memory(), 64);
    for line in probe.lines {
        let _ = writeln!(out, "inplace: {line}");
    }
    out
}
