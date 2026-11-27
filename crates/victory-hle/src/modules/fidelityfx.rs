//! `amd_fidelityfx_loader_xbox.dll`, redirected to a PC FidelityFX DLL.
//!
//! The Series S title upscales with AMD's FSR 4 through the FidelityFX API
//! (`ffxCreateContext`, `ffxDispatch`, ...). The console build and the PC
//! build export the same five functions. With `VICTORY_FSR_DLL=<path>` set
//! (`amd_fidelityfx_upscaler_dx12.dll`, or the `loader_dx12` beside it) the
//! title's loader is not mapped: its `LoadLibrary` gets this module, and
//! each call goes to the PC DLL, so tools that hook that DLL (OptiScaler:
//! NVIDIA DLSS / XeSS / FSR 3.1) see the title's upscaler like any PC game's.
//!
//! Without it, [`Interpose`] binds the title's imports of the loader to
//! DLSS / frame generation (`victory_gpu::streamline`) when those are on.

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

use victory_gpu::external;

use crate::HleModule;

/// The environment variable that turns the redirect on.
const VARIABLE: &str = "VICTORY_FSR_DLL";

/// The title DLLs the redirect answers for.
const DLLS: [&str; 3] = [
    "amd_fidelityfx_loader_xbox.dll",
    "amd_fidelityfx_upscaler_xbox.dll",
    "amd_fidelityfx_framegeneration_xbox.dll",
];

fn path() -> Option<std::path::PathBuf> {
    std::env::var_os(VARIABLE)
        .filter(|p| !p.is_empty())
        .map(Into::into)
}

/// Whether the redirect is on.
pub fn enabled() -> bool {
    path().is_some()
}

/// Whether the title's `file` is answered by the redirect instead of being
/// mapped from the title's folder.
pub fn replaces(file: &str) -> bool {
    enabled() && DLLS.iter().any(|d| d.eq_ignore_ascii_case(file))
}

// ffxReturnCode_t
const OK: u32 = 0;
const ERROR: u32 = 1;
const ERROR_PARAMETER: u32 = 6;

// Description types (`ffx_api_types.h`, `ffx_api_dx12.h`, `ffx_upscale.h`).
const DESC_OVERRIDE_VERSION: u64 = 0x1;
const CREATE_BACKEND_DX12: u64 = 0x2;
const DISPATCH_UPSCALE: u64 = 0x1_0001;
const DISPATCH_GENERATE_REACTIVE_MASK: u64 = 0x1_0002;

type Create = unsafe extern "system" fn(*mut *mut c_void, *mut u8, *const c_void) -> u32;
type Destroy = unsafe extern "system" fn(*mut *mut c_void, *const c_void) -> u32;
type Header = unsafe extern "system" fn(*mut *mut c_void, *mut u8) -> u32;

struct Api {
    create: Create,
    destroy: Destroy,
    configure: Header,
    query: Header,
    dispatch: Header,
}

/// The PC DLL's functions, loaded on first use.
fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(|| {
        let path = path()?;
        match load(&path) {
            Ok(api) => {
                println!("fidelityfx     {} (host, for the title's upscaler)", path.display());
                Some(api)
            }
            Err(error) => {
                tracing::error!(path = %path.display(), %error, "the PC FidelityFX DLL");
                None
            }
        }
    })
    .as_ref()
}

fn load(path: &std::path::Path) -> Result<Api, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::LibraryLoader::{
        GetProcAddress, LOAD_WITH_ALTERED_SEARCH_PATH, LoadLibraryExW,
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: a NUL-terminated path. The DLL's own folder is searched for what it needs.
    let module = unsafe { LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH) };
    if module.is_null() {
        return Err(format!("LoadLibrary failed: {}", std::io::Error::last_os_error()));
    }
    let export = |name: &'static [u8]| {
        // SAFETY: a loaded module and a NUL-terminated name.
        unsafe { GetProcAddress(module, name.as_ptr()) }
            .map(|f| f as usize)
            .ok_or_else(|| format!("no export {}", String::from_utf8_lossy(&name[..name.len() - 1])))
    };
    // SAFETY: the five exports have these signatures in every FidelityFX API DLL (`ffx_api.h`).
    unsafe {
        Ok(Api {
            create: std::mem::transmute::<usize, Create>(export(b"ffxCreateContext\0")?),
            destroy: std::mem::transmute::<usize, Destroy>(export(b"ffxDestroyContext\0")?),
            configure: std::mem::transmute::<usize, Header>(export(b"ffxConfigure\0")?),
            query: std::mem::transmute::<usize, Header>(export(b"ffxQuery\0")?),
            dispatch: std::mem::transmute::<usize, Header>(export(b"ffxDispatch\0")?),
        })
    }
}

/// Words of title memory changed for one call, put back after it.
#[derive(Default)]
struct Patches(Vec<(*mut usize, usize)>);

impl Patches {
    /// Sets the word at `at` to `new`.
    ///
    /// # Safety
    /// `at` is a writable, aligned word of a description the title passed.
    unsafe fn set(&mut self, at: *mut usize, new: usize) {
        // SAFETY: as the caller says.
        unsafe {
            self.0.push((at, at.read()));
            at.write(new);
        }
    }
}

impl Drop for Patches {
    fn drop(&mut self) {
        for &(at, old) in self.0.iter().rev() {
            // SAFETY: written by `set` on the same call.
            unsafe { at.write(old) };
        }
    }
}

fn note(call: &str, head: *const u8) {
    static SEEN: Mutex<Vec<(String, u64)>> = Mutex::new(Vec::new());
    let mut node = head;
    for _ in 0..16 {
        if node.is_null() {
            break;
        }
        // SAFETY: a title description: type, then the next one.
        let (ty, next) = unsafe { (node.cast::<u64>().read_unaligned(), node.add(8).cast::<*const u8>().read_unaligned()) };
        let fresh = {
            let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
            let key = (call.to_owned(), ty);
            let fresh = !seen.contains(&key);
            if fresh {
                seen.push(key);
            }
            fresh
        };
        if fresh {
            // SAFETY: the first 12 words of a description.
            let words: Vec<u64> = (0..12).map(|i| unsafe { node.cast::<u64>().add(i).read_unaligned() }).collect();
            tracing::info!(
                call,
                ty = format_args!("{ty:#x}"),
                words = ?words.iter().map(|w| format!("{w:#x}")).collect::<Vec<_>>(),
                "FidelityFX description"
            );
        }
        node = next;
    }
}

fn swap_device(patches: &mut Patches, node: *mut u8, from: usize, to: usize, host: *mut c_void) {
    let mut at = from;
    while at + 8 <= to {
        // SAFETY: inside a description the title passed; read unaligned first.
        unsafe {
            let word = node.add(at).cast::<usize>();
            if external::is_title_device(word.read_unaligned() as *const c_void) && word.is_aligned() {
                patches.set(word, host as usize);
            }
        }
        at += 8;
    }
}

extern "system" fn ffxCreateContext(context: *mut *mut c_void, desc: *mut u8, _alloc: *const c_void) -> u32 {
    let Some(api) = api() else { return ERROR };
    let Some(device) = external::host_device() else { return ERROR };
    note("create", desc);
    let mut patches = Patches::default();
    let mut backend = [CREATE_BACKEND_DX12, 0, device as u64];
    let mut has_backend = false;
    let mut last = desc;
    let mut previous: *mut u8 = std::ptr::null_mut();
    let mut node = desc;
    for _ in 0..32 {
        if node.is_null() {
            break;
        }
        // SAFETY: a title description: type, then the next one.
        let (ty, next) = unsafe { (node.cast::<u64>().read_unaligned(), node.add(8).cast::<*mut u8>().read_unaligned()) };
        if ty == CREATE_BACKEND_DX12 {
            has_backend = true;
            swap_device(&mut patches, node, 16, 24, device);
        }
        if ty == DESC_OVERRIDE_VERSION && !previous.is_null() {
            tracing::info!("FidelityFX: the title's version override is not passed to the host DLL");
            // SAFETY: the previous description's next-pointer word.
            unsafe { patches.set(previous.add(8).cast(), next as usize) };
        } else {
            previous = node;
        }
        last = node;
        node = next;
    }
    if !has_backend && !last.is_null() {
        tracing::info!("FidelityFX: no backend description from the title; adding the host device");
        // SAFETY: the last description's next-pointer word; `backend` lives through the call.
        unsafe { patches.set(last.add(8).cast(), backend.as_mut_ptr() as usize) };
    }
    // SAFETY: the PC DLL's export, with a description the title wrote and we patched.
    let code = unsafe { (api.create)(context, desc, std::ptr::null()) };
    drop(patches);
    tracing::info!(code, "ffxCreateContext on the host DLL");
    code
}

extern "system" fn ffxDestroyContext(context: *mut *mut c_void, _alloc: *const c_void) -> u32 {
    let Some(api) = api() else { return ERROR };
    // SAFETY: the PC DLL's export.
    unsafe { (api.destroy)(context, std::ptr::null()) }
}

fn with_host_device(call: &str, context: *mut *mut c_void, desc: *mut u8, f: impl FnOnce(&Api) -> Header) -> u32 {
    let Some(api) = api() else { return ERROR };
    note(call, desc);
    let mut patches = Patches::default();
    if let Some(device) = external::host_device() {
        let mut node = desc;
        for _ in 0..16 {
            if node.is_null() {
                break;
            }
            swap_device(&mut patches, node, 16, 96, device);
            // SAFETY: a title description's next-pointer word.
            node = unsafe { node.add(8).cast::<*mut u8>().read_unaligned() };
        }
    }
    // SAFETY: the PC DLL's export.
    unsafe { f(api)(context, desc) }
}

extern "system" fn ffxConfigure(context: *mut *mut c_void, desc: *mut u8) -> u32 {
    with_host_device("configure", context, desc, |api| api.configure)
}

extern "system" fn ffxQuery(context: *mut *mut c_void, desc: *mut u8) -> u32 {
    with_host_device("query", context, desc, |api| api.query)
}

fn d3d12_state(ffx: u32) -> u32 {
    const UNORDERED_ACCESS: u32 = 1 << 0;
    const COMPUTE_READ: u32 = 1 << 1;
    const PIXEL_READ: u32 = 1 << 2;
    const COPY_SRC: u32 = 1 << 3;
    const COPY_DEST: u32 = 1 << 4;
    const INDIRECT_ARGUMENT: u32 = 1 << 5;
    const RENDER_TARGET: u32 = 1 << 7;
    let mut state = 0;
    for (bit, d3d) in [
        (UNORDERED_ACCESS, 0x8),
        (COMPUTE_READ, 0x40),
        (PIXEL_READ, 0x80),
        (COPY_SRC, 0x800),
        (COPY_DEST, 0x400),
        (INDIRECT_ARGUMENT, 0x200),
        (RENDER_TARGET, 0x4),
    ] {
        if ffx & bit != 0 {
            state |= d3d;
        }
    }
    state
}

const RESOURCE_SIZE: usize = 48;
const RESOURCE_STATE: usize = 40;
const COMMAND_LIST: usize = 16;
const UPSCALE: [(usize, bool); 7] = [
    (24, false),
    (24 + RESOURCE_SIZE, false),
    (24 + 2 * RESOURCE_SIZE, false),
    (24 + 3 * RESOURCE_SIZE, false),
    (24 + 4 * RESOURCE_SIZE, false),
    (24 + 5 * RESOURCE_SIZE, false),
    (24 + 6 * RESOURCE_SIZE, true),
];
const REACTIVE_MASK: [(usize, bool); 3] = [
    (24, false),
    (24 + RESOURCE_SIZE, false),
    (24 + 2 * RESOURCE_SIZE, true),
];

extern "system" fn ffxDispatch(context: *mut *mut c_void, desc: *mut u8) -> u32 {
    let Some(api) = api() else { return ERROR };
    if desc.is_null() {
        return ERROR_PARAMETER;
    }
    note("dispatch", desc);
    // SAFETY: a description: its type first.
    let ty = unsafe { desc.cast::<u64>().read_unaligned() };
    let resources: &[(usize, bool)] = match ty {
        DISPATCH_UPSCALE => &UPSCALE,
        DISPATCH_GENERATE_REACTIVE_MASK => &REACTIVE_MASK,
        _ => {
            // SAFETY: the PC DLL's export; nothing here names console objects.
            return unsafe { (api.dispatch)(context, desc) };
        }
    };
    // SAFETY: the command list's word of a dispatch description.
    let list = unsafe { desc.add(COMMAND_LIST).cast::<usize>().read_unaligned() };
    let mut code = ERROR;
    let mut patches = Patches::default();
    let recorded = external::record(list as *const c_void, |pass| {
        // SAFETY: the words of a dispatch description the title passed.
        unsafe {
            patches.set(desc.add(COMMAND_LIST).cast(), pass.command_list() as usize);
            for &(at, write) in resources {
                let slot = desc.add(at).cast::<usize>();
                let title = slot.read_unaligned();
                if title == 0 {
                    continue;
                }
                let declared = desc.add(at + RESOURCE_STATE).cast::<u32>().read_unaligned();
                match pass.texture(title as *const c_void, d3d12_state(declared), write) {
                    Some(host) => patches.set(slot, host as usize),
                    None => {
                        tracing::warn!(slot = at, resource = title, "FidelityFX dispatch: not a texture of the title");
                        code = ERROR_PARAMETER;
                        return;
                    }
                }
            }
            code = (api.dispatch)(context, desc);
        }
    });
    drop(patches);
    if !recorded {
        tracing::warn!("FidelityFX dispatch: the command list is not one of the title's");
        return ERROR_PARAMETER;
    }
    if code != OK {
        tracing::warn!(code, "ffxDispatch on the host DLL");
    }
    code
}

pub(crate) fn module() -> HleModule {
    HleModule {
        name: "fidelityfx",
        dlls: &DLLS,
        exports: vec![
            crate::hle_export!(ffxCreateContext),
            crate::hle_export!(ffxDestroyContext),
            crate::hle_export!(ffxConfigure),
            crate::hle_export!(ffxQuery),
            crate::hle_export!(ffxDispatch),
        ],
    }
}

pub use streamline_interpose::{Interpose, LOADER_DLL};

/// The title's own FidelityFX with DLSS and frame generation
/// (`victory_gpu::streamline`, `VICTORY_DLSS=1`, `VICTORY_FRAMEGEN=1`): the
/// title's imports of [`LOADER_DLL`] are bound to the five functions here
/// ([`Interpose`]), which call the title's own DLL for everything except an
/// upscale dispatch DLSS takes: DLSS then records the upscaling at that place
/// of the title's command list instead, from the same textures into the same
/// output. With frame generation alone the title's own upscaling runs and its
/// depth and motion vectors are noted for the present. Without either switch,
/// or without Streamline, the imports are bound as before. (Not the
/// `VICTORY_FSR_DLL` redirect above, which replaces the title's DLL with a PC
/// one; the two do not mix.)
///
/// The description layouts are the FidelityFX SDK's API (MIT) as DexNom's fork
/// reads them (DexNom/VIctory, `crates/victory-hle/src/modules/fidelityfx.rs`,
/// GPL-3.0-or-later), from which they are ported.
mod streamline_interpose {
    // Title code calls these through its import table.
    #![allow(clippy::not_unsafe_ptr_arg_deref)]

    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

    use victory_gpu::object::Object;
    use victory_gpu::streamline::{self, Taken, Upscale, plan::Camera};
    use victory_loader::{ImportResolver, Provenance, Resolution};
    use victory_pe::ImportSymbol;


    /// The DLL the title imports the API from (it loads the upscaler itself).
    pub const LOADER_DLL: &str = "amd_fidelityfx_loader_xbox.dll";

    // Return codes.
    const OK: u32 = 0;
    const PARAMETER: u32 = 6;

    // Description types.
    const CREATE_UPSCALE: u64 = 0x1_0000;
    const DISPATCH_UPSCALE: u64 = 0x1_0001;

    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    struct Header {
        kind: u64,
        next: *mut Header,
    }

    /// An upscaler context: flags, largest render and output sizes, a message
    /// callback.
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    struct CreateUpscale {
        header: Header,
        flags: u32,
        max_render: [u32; 2],
        max_output: [u32; 2],
        message: usize,
    }

    /// A resource, its description (type, format, sizes, mips, flags, usage)
    /// and the state it is in.
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    struct Resource {
        resource: *mut c_void,
        description: [u32; 8],
        state: u32,
    }

    /// One upscaling: the command list, then colour, depth, motion vectors,
    /// exposure, reactive mask, transparency mask and output; jitter,
    /// motion-vector scale, sizes, sharpening, frame time (ms), pre-exposure,
    /// reset, camera and flags. (The C `bool`s are read as bytes.)
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    struct DispatchUpscale {
        header: Header,
        list: *mut c_void,
        resources: [Resource; 7],
        jitter: [f32; 2],
        motion_scale: [f32; 2],
        render_size: [u32; 2],
        upscale_size: [u32; 2],
        enable_sharpening: u8,
        sharpness: f32,
        frame_time_delta: f32,
        pre_exposure: f32,
        reset: u8,
        camera_near: f32,
        camera_far: f32,
        camera_fov_angle_vertical: f32,
        view_space_to_meters_factor: f32,
        flags: u32,
    }

    const _: () = assert!(size_of::<CreateUpscale>() == 0x30);
    const _: () = assert!(size_of::<Resource>() == 0x30);
    const _: () = assert!(size_of::<DispatchUpscale>() == 0x1b0);

    type Create = unsafe extern "C" fn(*mut *mut c_void, *mut Header, *const c_void) -> u32;
    type Destroy = unsafe extern "C" fn(*mut *mut c_void, *const c_void) -> u32;
    /// Configure, query and dispatch.
    type Call = unsafe extern "C" fn(*mut *mut c_void, *mut Header) -> u32;

    /// The title's own functions, by [`NAMES`].
    static ORIGINALS: [AtomicUsize; 5] = [const { AtomicUsize::new(0) }; 5];
    const NAMES: [&str; 5] = ["ffxCreateContext", "ffxDestroyContext", "ffxConfigure", "ffxQuery", "ffxDispatch"];

    /// An upscaler context the title made: its creation flags and largest
    /// sizes.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Created {
        flags: u32,
        max_render: [u32; 2],
        max_output: [u32; 2],
        /// Made smaller than asked for the shipping FSR4 (DLSS on): the title's
        /// own upscaling must not run on it at the size asked.
        clamped: bool,
    }

    static CONTEXTS: Mutex<Option<HashMap<usize, Created>>> = Mutex::new(None);

    macro_rules! once {
        ($level:ident, $($arg:tt)*) => {{
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| tracing::$level!($($arg)*));
        }};
    }

    /// Binds the title's imports of [`LOADER_DLL`] to the functions here when
    /// DLSS or frame generation is on, keeping what `inner` binds them to (the
    /// title's own DLL) for them to call; otherwise binds as `inner` does.
    pub struct Interpose<'a> {
        pub inner: &'a dyn ImportResolver,
    }

    impl ImportResolver for Interpose<'_> {
        fn resolve(&self, dll: &str, symbol: &ImportSymbol) -> Resolution {
            let found = self.inner.resolve(dll, symbol);
            if !dll.eq_ignore_ascii_case(LOADER_DLL) || !streamline::serving() {
                return found;
            }
            match (&found, symbol) {
                (&Resolution::Found { address, .. }, ImportSymbol::Name { name, .. }) => match interpose(name, address) {
                    Some(ours) => Resolution::Found { address: ours, provenance: Provenance::Hle },
                    None => found,
                },
                _ => found,
            }
        }
    }

    /// Keeps `original` as the title's `name` and gives ours, if we have one.
    fn interpose(name: &str, original: usize) -> Option<usize> {
        let i = NAMES.iter().position(|n| *n == name)?;
        ORIGINALS[i].store(original, Relaxed);
        let ours = [
            ffxCreateContext as *const () as usize,
            ffxDestroyContext as *const () as usize,
            ffxConfigure as *const () as usize,
            ffxQuery as *const () as usize,
            ffxDispatch as *const () as usize,
        ][i];
        tracing::info!(name, original = format_args!("{original:#x}"), "FidelityFX: the title's import goes through ours (DLSS / frame generation)");
        Some(ours)
    }

    fn original(i: usize) -> Option<*const ()> {
        let address = ORIGINALS[i].load(Relaxed);
        (address != 0).then_some(address as *const ())
    }

    fn contexts<T>(f: impl FnOnce(&mut HashMap<usize, Created>) -> T) -> T {
        let mut contexts = CONTEXTS.lock().unwrap_or_else(|e| e.into_inner());
        f(contexts.get_or_insert_with(HashMap::new))
    }

    /// The largest output the title's shipping FSR4 accepts: it carries only the
    /// fixed-resolution quality shader tables of the Series S (1280x720 in,
    /// 1920x1080 out). Other sizes fail the context's creation ("Shipping FSR4
    /// configuration mismatch: maxUpscaleSize must be 1920x1080", "maxRenderSize
    /// must be 1280x720 for a tight fixed-resolution allocation or 1920x1080 for
    /// the legacy allocation"), and the title, which does not look at the result,
    /// divides by zero at its first upscaling (2026-11-26, 4070 SUPER).
    const SHIPPING_MAX_OUTPUT: [u32; 2] = [1920, 1080];
    const SHIPPING_MAX_RENDER: [u32; 2] = [1280, 720];

    /// The sizes the title's shipping FSR4 accepts for a context, if `render` and
    /// `output` are not those: (render, output).
    fn shipping_sizes(render: [u32; 2], output: [u32; 2]) -> Option<([u32; 2], [u32; 2])> {
        ((render, output) != (SHIPPING_MAX_RENDER, SHIPPING_MAX_OUTPUT)).then_some((SHIPPING_MAX_RENDER, SHIPPING_MAX_OUTPUT))
    }

    extern "C" fn ffxCreateContext(context: *mut *mut c_void, desc: *mut Header, memory: *const c_void) -> u32 {
        let Some(create) = original(0) else { return PARAMETER };
        // SAFETY: the title's own function, with its arguments.
        let call = |desc: *mut Header| unsafe { std::mem::transmute::<*const (), Create>(create)(context, desc, memory) };
        // SAFETY: the title passes a chain of descriptions.
        let upscaler = !desc.is_null() && unsafe { (*desc).kind } == CREATE_UPSCALE;
        if !upscaler {
            return call(desc);
        }
        // SAFETY: an upscaler description, by its type.
        let asked = unsafe { desc.cast::<CreateUpscale>().read() };
        // With DLSS taking the upscalings, the title's own context only has to
        // exist (for its queries and our bookkeeping): it is made at a size its
        // shipping tables accept, and DLSS gets the sizes the title asked for.
        let shipping = streamline::upscaling().then(|| shipping_sizes(asked.max_render, asked.max_output)).flatten();
        let clamped = shipping.is_some();
        let code = if let Some((render, output)) = shipping {
            let mut fitted = asked;
            fitted.max_render = render;
            fitted.max_output = output;
            // SAFETY: the title's description, rewritten for the call and put back.
            unsafe { desc.cast::<CreateUpscale>().write(fitted) };
            let code = call(desc);
            // SAFETY: as above.
            unsafe { desc.cast::<CreateUpscale>().write(asked) };
            tracing::info!(
                asked_render = ?asked.max_render,
                asked_output = ?asked.max_output,
                made_render = ?fitted.max_render,
                made_output = ?fitted.max_output,
                code,
                "FidelityFX: the title's upscaler made at a size its shipping FSR4 accepts; DLSS upscales to the size asked"
            );
            code
        } else {
            call(desc)
        };
        if code != OK || context.is_null() {
            // Frame generation without DLSS leaves the upscaling to the title's
            // own FidelityFX, which is made for the Series S sizes only.
            if code != OK && !streamline::upscaling() {
                tracing::error!(
                    code,
                    max_render = ?asked.max_render,
                    max_output = ?asked.max_output,
                    "FidelityFX: the title's own upscaler was not made at these sizes (its shipping FSR4 takes 1280x720 or 1920x1080 in, 1920x1080 out): with frame generation at another output size, set VICTORY_DLSS=1 as well; the title fails at its first upscaling without an upscaler"
                );
            }
            return code;
        }
        // SAFETY: the context was made.
        let handle = unsafe { *context } as usize;
        let created = Created { flags: asked.flags, max_render: asked.max_render, max_output: asked.max_output, clamped };
        tracing::info!(
            context = format_args!("{handle:#x}"),
            flags = format_args!("{:#x}", created.flags),
            max_render = ?created.max_render,
            max_output = ?created.max_output,
            "FidelityFX: the title made its upscaler"
        );
        contexts(|c| c.insert(handle, created));
        code
    }

    extern "C" fn ffxDestroyContext(context: *mut *mut c_void, memory: *const c_void) -> u32 {
        let Some(destroy) = original(1) else { return PARAMETER };
        if !context.is_null() {
            // SAFETY: the title passes its context.
            let handle = unsafe { *context } as usize;
            if contexts(|c| c.remove(&handle)).is_some() {
                streamline::upscaler_destroyed(handle);
            }
        }
        // SAFETY: the title's own function, with its arguments.
        unsafe { std::mem::transmute::<*const (), Destroy>(destroy)(context, memory) }
    }

    extern "C" fn ffxConfigure(context: *mut *mut c_void, desc: *mut Header) -> u32 {
        let Some(configure) = original(2) else { return PARAMETER };
        // SAFETY: the title's own function, with its arguments.
        unsafe { std::mem::transmute::<*const (), Call>(configure)(context, desc) }
    }

    extern "C" fn ffxQuery(context: *mut *mut c_void, desc: *mut Header) -> u32 {
        let Some(query) = original(3) else { return PARAMETER };
        if !desc.is_null() {
            // SAFETY: the title passes a description.
            let kind = unsafe { (*desc).kind };
            static SEEN: Mutex<Vec<u64>> = Mutex::new(Vec::new());
            let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
            if !seen.contains(&kind) && seen.len() < 32 {
                seen.push(kind);
                tracing::info!(kind = format_args!("{kind:#x}"), "FidelityFX: the title asks a query of this kind (answered by its own DLL)");
            }
        }
        // SAFETY: the title's own function, with its arguments.
        unsafe { std::mem::transmute::<*const (), Call>(query)(context, desc) }
    }

    extern "C" fn ffxDispatch(context: *mut *mut c_void, desc: *mut Header) -> u32 {
        let Some(dispatch) = original(4) else { return PARAMETER };
        // SAFETY: the title's own function, with its arguments.
        let own = || unsafe { std::mem::transmute::<*const (), Call>(dispatch)(context, desc) };
        if context.is_null() || desc.is_null() {
            return own();
        }
        // SAFETY: the title passes a description and its context.
        let (kind, handle) = unsafe { ((*desc).kind, *context as usize) };
        if kind != DISPATCH_UPSCALE {
            return own();
        }
        let Some(created) = contexts(|c| c.get(&handle).copied()) else {
            once!(warn, "FidelityFX: an upscale dispatch on a context we did not see made; the title's own runs");
            return own();
        };
        // SAFETY: an upscaling, by its type.
        let d = unsafe { desc.cast::<DispatchUpscale>().read() };
        first_dispatch(&d, &created);
        let up = upscale(&d, &created, handle);
        match streamline::upscale(&up) {
            Taken::Dlss => OK,
            Taken::Noted => own(),
            Taken::No(why) if created.clamped => {
                // The title's FidelityFX on a context made for 1080p at the
                // output size asked would read past its tables: no upscaling
                // this frame (the output keeps the last one) rather than that.
                refused(why);
                once!(warn, "FidelityFX: DLSS did not take an upscaling on a context made smaller than asked; none ran");
                OK
            }
            Taken::No(why) => {
                streamline::forwarded();
                refused(why);
                own()
            }
        }
    }

    /// Each reason the title's own upscaling ran instead, once.
    fn refused(why: &'static str) {
        static SEEN: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
        let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
        if !seen.contains(&why) {
            seen.push(why);
            tracing::warn!("FidelityFX: the title's own upscaling runs: {why}");
        }
    }

    /// The first upscale dispatch, all of it.
    fn first_dispatch(d: &DispatchUpscale, created: &Created) {
        once!(
            info,
            render = ?d.render_size,
            output = ?d.upscale_size,
            max_render = ?created.max_render,
            max_output = ?created.max_output,
            create_flags = format_args!("{:#x}", created.flags),
            flags = format_args!("{:#x}", d.flags),
            jitter = ?d.jitter,
            motion_vector_scale = ?d.motion_scale,
            near = d.camera_near,
            far = d.camera_far,
            fov = d.camera_fov_angle_vertical,
            frame_ms = d.frame_time_delta,
            pre_exposure = d.pre_exposure,
            reset = d.reset != 0,
            sharpening = d.enable_sharpening != 0,
            resources = ?d.resources.iter().map(|r| (r.resource as usize != 0, r.description[1], r.description[2], r.description[3], r.state)).collect::<Vec<_>>(),
            "FidelityFX: the title's first upscale dispatch (resources: present, format, width, height, state)"
        );
    }

    /// An upscaling in `victory_gpu::streamline`'s terms. The output size is the
    /// context's largest when the dispatch names none (as FidelityFX reads it).
    fn upscale(d: &DispatchUpscale, created: &Created, context: usize) -> Upscale {
        let output = if d.upscale_size == [0, 0] { created.max_output } else { d.upscale_size };
        Upscale {
            context,
            list: d.list.cast::<Object>().cast_const(),
            resources: d.resources.map(|r| (r.resource.cast::<Object>().cast_const(), r.state)),
            camera: Camera {
                jitter: d.jitter,
                motion_scale: d.motion_scale,
                render: d.render_size,
                output,
                near: d.camera_near,
                far: d.camera_far,
                fov_y: d.camera_fov_angle_vertical,
                reset: d.reset != 0,
                flags: created.flags,
            },
            pre_exposure: d.pre_exposure,
            frame_ms: d.frame_time_delta,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::mem::offset_of;
        use std::ptr::null_mut;

        #[test]
        fn descriptions_lay_out_as_the_sdk() {
            assert_eq!(offset_of!(CreateUpscale, flags), 0x10);
            assert_eq!(offset_of!(CreateUpscale, max_output), 0x1c);
            assert_eq!(offset_of!(Resource, state), 0x28);
            assert_eq!(offset_of!(DispatchUpscale, list), 0x10);
            assert_eq!(offset_of!(DispatchUpscale, resources), 0x18);
            assert_eq!(offset_of!(DispatchUpscale, jitter), 0x168);
            assert_eq!(offset_of!(DispatchUpscale, render_size), 0x178);
            assert_eq!(offset_of!(DispatchUpscale, upscale_size), 0x180);
            assert_eq!(offset_of!(DispatchUpscale, enable_sharpening), 0x188);
            assert_eq!(offset_of!(DispatchUpscale, frame_time_delta), 0x190);
            assert_eq!(offset_of!(DispatchUpscale, reset), 0x198);
            assert_eq!(offset_of!(DispatchUpscale, camera_near), 0x19c);
            assert_eq!(offset_of!(DispatchUpscale, flags), 0x1ac);
        }

        fn resource(at: usize, state: u32) -> Resource {
            Resource { resource: at as *mut c_void, description: [1, 10, 2560, 1440, 1, 1, 0, 0], state }
        }

        fn dispatch(output: [u32; 2]) -> DispatchUpscale {
            DispatchUpscale {
                header: Header { kind: DISPATCH_UPSCALE, next: null_mut() },
                list: 0x1000 as *mut c_void,
                resources: [
                    resource(0x10, 0xc),
                    resource(0x20, 0xc),
                    resource(0x30, 0xc),
                    resource(0, 0),
                    resource(0x50, 0xc),
                    resource(0, 0),
                    resource(0x70, 0x2),
                ],
                jitter: [0.25, -0.125],
                motion_scale: [1706.0, 960.0],
                render_size: [1706, 960],
                upscale_size: output,
                enable_sharpening: 0,
                sharpness: 0.0,
                frame_time_delta: 33.3,
                pre_exposure: 1.5,
                reset: 1,
                camera_near: 0.1,
                camera_far: 5000.0,
                camera_fov_angle_vertical: 0.9,
                view_space_to_meters_factor: 1.0,
                flags: 0,
            }
        }

        /// A dispatch reads as the upscaling it names; without an output size
        /// the context's largest is the output.
        #[test]
        fn a_dispatch_reads_as_its_upscaling() {
            let created = Created { flags: 0x29, max_render: [2560, 1440], max_output: [2560, 1440], clamped: false };
            let up = upscale(&dispatch([2560, 1440]), &created, 0xabc);
            assert_eq!(up.context, 0xabc);
            assert_eq!(up.list as usize, 0x1000);
            assert_eq!(up.resources[streamline::COLOUR], (0x10 as *const Object, 0xc));
            assert_eq!(up.resources[streamline::EXPOSURE].0 as usize, 0);
            assert_eq!(up.resources[streamline::OUTPUT], (0x70 as *const Object, 0x2));
            assert_eq!(up.camera.render, [1706, 960]);
            assert_eq!(up.camera.output, [2560, 1440]);
            assert_eq!(up.camera.flags, 0x29);
            assert!(up.camera.reset && up.camera.depth_inverted());
            assert_eq!((up.pre_exposure, up.frame_ms), (1.5, 33.3));
            let up = upscale(&dispatch([0, 0]), &Created { max_output: [1920, 1080], ..created }, 0);
            assert_eq!(up.camera.output, [1920, 1080]);
        }

        /// A context at other sizes than the shipping FSR4's is made at those
        /// (1280x720 in, 1920x1080 out); one at them is left as it is.
        #[test]
        fn contexts_are_made_at_the_shipping_sizes() {
            let shipping = Some(([1280, 720], [1920, 1080]));
            assert_eq!(shipping_sizes([1706, 960], [2560, 1440]), shipping);
            assert_eq!(shipping_sizes([2560, 1440], [2560, 1440]), shipping);
            assert_eq!(shipping_sizes([1280, 720], [2560, 1440]), shipping);
            assert_eq!(shipping_sizes([1280, 720], [1920, 1080]), None);
        }

        struct Fixed;
        impl ImportResolver for Fixed {
            fn resolve(&self, dll: &str, _: &ImportSymbol) -> Resolution {
                if dll.eq_ignore_ascii_case(LOADER_DLL) {
                    Resolution::Found { address: 0x1234, provenance: Provenance::Title }
                } else {
                    Resolution::Missing
                }
            }
        }

        /// Without Streamline the title's imports bind as before; the names we
        /// serve are the API's five.
        #[test]
        fn without_streamline_the_titles_own_binds() {
            let name = |n: &str| ImportSymbol::Name { hint: 0, name: n.into() };
            let interposed = Interpose { inner: &Fixed };
            assert_eq!(
                interposed.resolve("AMD_FIDELITYFX_LOADER_XBOX.DLL", &name("ffxDispatch")),
                Resolution::Found { address: 0x1234, provenance: Provenance::Title }
            );
            assert_eq!(interposed.resolve("other.dll", &name("ffxDispatch")), Resolution::Missing);
            assert_eq!(interpose("ffxGetVersion", 1), None);
            assert!(interpose("ffxQuery", 0x5678).is_some());
            assert_eq!(original(3), Some(0x5678 as *const ()));
            ORIGINALS[3].store(0, Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_map_to_d3d12() {
        assert_eq!(d3d12_state(0), 0);
        assert_eq!(d3d12_state(1), 0x8);
        assert_eq!(d3d12_state((1 << 1) | (1 << 2)), 0xc0);
        assert_eq!(d3d12_state((1 << 3) | (1 << 1)), 0x840);
    }

    #[test]
    fn patches_are_put_back() {
        let mut words = [1usize, 2];
        {
            let mut p = Patches::default();
            // SAFETY: aligned words of a local array.
            unsafe {
                p.set(&mut words[0], 9);
                p.set(&mut words[0], 10);
                p.set(&mut words[1], 7);
            }
            assert_eq!(words, [10, 7]);
        }
        assert_eq!(words, [1, 2]);
    }
}
