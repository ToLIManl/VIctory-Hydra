//! A host list's D3D12 list, with barrier calls joined
//! (`VICTORY_JOIN_BARRIERS=1`; off by default; A/B-able with
//! `VICTORY_AB=<s>,VICTORY_JOIN_BARRIERS`).
//!
//! The state changes the host asks for (`crate::list::ensure_state`, the
//! aliasing before a texture's first upload) and the title's own barriers
//! wait here instead of going to the list at once; they go, in the order
//! they were asked for, in one call, at the next use of the list for
//! anything else. Every use goes through [`Rec`]'s `Deref`, so nothing is
//! recorded before a barrier asked for earlier. In a replay of capture
//! replay5 joining the calls that had no work between them saved 2.4 ms of
//! 79.5 (2,490 calls joined, 1,619 of them a texture's state change after
//! its upload followed by the next texture's aliasing).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use windows::Win32::Graphics::Direct3D12::*;

/// Whether barriers are held now (`VICTORY_JOIN_BARRIERS=1`, or its phase
/// of `VICTORY_AB`).
pub(crate) fn on() -> bool {
    #[cfg(feature = "diagnostics")]
    if let Some(on) = crate::ab::pick("VICTORY_JOIN_BARRIERS") {
        return on;
    }
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_JOIN_BARRIERS").is_some_and(|v| v == "1"))
}

/// Barriers held. Their resources are borrowed: the list's states hold the
/// textures (`HostList::transition`), and the title's barriers name what
/// the title keeps alive until the list has run.
struct Held(Vec<D3D12_RESOURCE_BARRIER>);

// SAFETY: borrowed interface pointers of free-threaded objects.
unsafe impl Send for Held {}

/// Since the last frame-rate line: barrier calls asked for while held, and
/// calls made of them.
pub(crate) static STATS: [std::sync::atomic::AtomicU64; 2] = [const { std::sync::atomic::AtomicU64::new(0) }; 2];

pub(crate) struct Rec {
    list: ID3D12GraphicsCommandList,
    any: AtomicBool,
    held: Mutex<Held>,
}

impl Rec {
    pub(crate) fn new(list: ID3D12GraphicsCommandList) -> Self {
        Self { list, any: AtomicBool::new(false), held: Mutex::new(Held(Vec::new())) }
    }

    /// Records `barriers` now, or holds them for the next use of the list.
    ///
    /// # Safety
    /// As `ResourceBarrier`'s: an open list, live resources.
    pub(crate) unsafe fn barriers(&self, barriers: &[D3D12_RESOURCE_BARRIER]) {
        if barriers.is_empty() {
            return;
        }
        if !on() {
            // (What was held while it was on goes first.)
            if self.any.load(Relaxed) {
                self.flush();
            }
            // SAFETY: the caller's.
            unsafe { self.list.ResourceBarrier(barriers) };
            return;
        }
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        // A resource whose state changes again: what is held goes first, so
        // that no call holds two changes of one resource.
        if barriers.iter().filter_map(named).any(|r| held.0.iter().filter_map(named).any(|h| h == r)) {
            // SAFETY: as in `flush`.
            unsafe { self.list.ResourceBarrier(&held.0) };
            held.0.clear();
            STATS[1].fetch_add(1, Relaxed);
        }
        for b in barriers {
            if let Some(uav_ptr) = uav_resource(b) {
                // If a global UAV barrier (`None`) is already queued with no
                // commands between, every subsequent UAV barrier is redundant.
                if held.0.iter().any(|h| uav_resource(h) == Some(0)) {
                    continue;
                }
                if uav_ptr == 0 {
                    // A global UAV barrier supersedes any narrower UAV barriers
                    // queued in the same batch (no commands ran between them).
                    held.0.retain(|h| uav_resource(h).is_none());
                } else if held.0.iter().any(|h| uav_resource(h) == Some(uav_ptr)) {
                    continue;
                }
            }
            // SAFETY: a plain copy of the description; nothing is released
            // through it (its pointers are borrowed, `ManuallyDrop`).
            held.0.push(unsafe { std::ptr::read(b) });
        }
        STATS[0].fetch_add(1, Relaxed);
        if !held.0.is_empty() {
            self.any.store(true, Relaxed);
        }
    }

    /// Emits a scoped D3D12 Enhanced Global Barrier (`ID3D12GraphicsCommandList7::Barrier`)
    /// after flushing any held legacy barriers, falling back to a global legacy UAV barrier
    /// when Enhanced Barriers are unavailable.
    ///
    /// # Safety
    /// As `Barrier`'s: an open command list.
    pub(crate) unsafe fn global_barrier(&self, barrier: D3D12_GLOBAL_BARRIER) {
        use windows::core::Interface;
        if self.any.load(Relaxed) {
            self.flush();
        }
        if crate::host::host_made().is_some_and(crate::host::Host::enhanced_barriers_supported)
            && let Ok(list7) = self.list.cast::<ID3D12GraphicsCommandList7>()
        {
            let group = [D3D12_BARRIER_GROUP {
                Type: D3D12_BARRIER_TYPE_GLOBAL,
                NumBarriers: 1,
                Anonymous: D3D12_BARRIER_GROUP_0 {
                    pGlobalBarriers: &barrier,
                },
            }];
            // SAFETY: open list supporting ID3D12GraphicsCommandList7 + EnhancedBarriersSupported.
            unsafe { list7.Barrier(&group) };
            STATS[0].fetch_add(1, Relaxed);
            STATS[1].fetch_add(1, Relaxed);
            return;
        }
        let uav = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_UAV,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                UAV: std::mem::ManuallyDrop::new(D3D12_RESOURCE_UAV_BARRIER {
                    pResource: std::mem::ManuallyDrop::new(None),
                }),
            },
        };
        // SAFETY: fallback legacy UAV barrier on an open list.
        unsafe { self.barriers(&[uav]) };
    }

    /// What is held goes to the list, in one call.
    fn flush(&self) {
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        self.any.store(false, Relaxed);
        if held.0.is_empty() {
            return;
        }
        // SAFETY: the barriers were asked for on this open list, in order;
        // their resources are alive (`Held`).
        unsafe { self.list.ResourceBarrier(&held.0) };
        held.0.clear();
        STATS[1].fetch_add(1, Relaxed);
    }
}

/// The resource a state change or aliasing names (its interface pointer).
fn named(b: &D3D12_RESOURCE_BARRIER) -> Option<usize> {
    // SAFETY: the union's member is the one `Type` says; a pointer read.
    unsafe {
        let p: Option<&ID3D12Resource> = match b.Type {
            D3D12_RESOURCE_BARRIER_TYPE_TRANSITION => b.Anonymous.Transition.pResource.as_ref(),
            D3D12_RESOURCE_BARRIER_TYPE_ALIASING => b.Anonymous.Aliasing.pResourceAfter.as_ref(),
            _ => None,
        };
        p.map(|r| windows::core::Interface::as_raw(r) as usize)
    }
}

/// For a UAV barrier, returns `Some(ptr)` (`0` for a global `NULL` UAV barrier);
/// `None` for transition/aliasing barriers.
fn uav_resource(b: &D3D12_RESOURCE_BARRIER) -> Option<usize> {
    if b.Type != D3D12_RESOURCE_BARRIER_TYPE_UAV {
        return None;
    }
    // SAFETY: `Type == UAV` selects the `UAV` union field; read raw pointer without cloning.
    unsafe {
        Some(
            b.Anonymous
                .UAV
                .pResource
                .as_ref()
                .map_or(0, |r| windows::core::Interface::as_raw(r) as usize),
        )
    }
}

impl std::ops::Deref for Rec {
    type Target = ID3D12GraphicsCommandList;

    /// The list, with what is held recorded first.
    fn deref(&self) -> &ID3D12GraphicsCommandList {
        if self.any.load(Relaxed) {
            self.flush();
        }
        &self.list
    }
}

/// For the frame-rate line.
pub(crate) fn line(frames: f64) {
    let [asked, made] = [0, 1].map(|i| STATS[i].swap(0, Relaxed) as f64 / frames);
    if asked > 0.0 {
        tracing::info!(
            "barrier calls per frame (VICTORY_JOIN_BARRIERS): {asked:.0} asked for, {made:.0} made of them"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uav(ptr: usize) -> D3D12_RESOURCE_BARRIER {
        let mut b = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_UAV,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            ..Default::default()
        };
        // SAFETY: constructing a raw-pointer UAV barrier for unit-testing `uav_resource`.
        unsafe {
            let slot: *mut usize = (&raw mut (*b.Anonymous.UAV).pResource).cast();
            slot.write(ptr);
        }
        b
    }

    #[test]
    fn uav_resource_distinguishes_global_scoped_and_transition() {
        let global = uav(0);
        let scoped = uav(0x1234_5678);
        let trans = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            ..Default::default()
        };
        assert_eq!(uav_resource(&global), Some(0));
        assert_eq!(uav_resource(&scoped), Some(0x1234_5678));
        assert_eq!(uav_resource(&trans), None);
    }
}
