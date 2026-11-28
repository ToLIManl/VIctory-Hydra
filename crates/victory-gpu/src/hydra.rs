//! Hydra Control — unified local control plane bridging VIctory (host GPU,
//! live settings, shader profiler, RAGE 9 verified globals) and RAGE 9's
//! script thread (`hydra_bridge.asi` over `shvi_core.dll` ABI 1.7) via a
//! single local Windows Named Pipe (`\\.\pipe\victory_hydra`, JSON lines).
//!
//! Security & thread-safety invariants:
//! - Pipe uses `PIPE_REJECT_REMOTE_CLIENTS`, `FILE_FLAG_FIRST_PIPE_INSTANCE`,
//!   and an explicit current-user-only DACL (`D:P(A;;GA;;;<user SID>)`).
//! - Never calls `std::env::set_var` from the pipe thread; live knobs go
//!   exclusively through `host::menu::apply_live_var`. Startup-only `OnceLock`
//!   knobs honestly return `{"ok":false,"err":"restart-only"}`.
//! - Never acquires rendering hot locks (`CARD_WORK`, `QUEUE_ORDER`, `LEAN`).
//! - Title memory reads use `ReadProcessMemory(GetCurrentProcess(), ...)` on
//!   bounds-checked RVAs so invalid pages return an error instead of faulting.
//! - Game-plane commands (`game.*`) are forwarded via `HydraGameApi::call`,
//!   which enqueues onto RAGE 9's script thread with a bounded timeout.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Version of [`HydraGameApi`] C ABI contract.
pub const HYDRA_GAME_API_VERSION: u32 = 1;

/// C ABI table registered by `hydra_bridge.asi` via `victory_hydra_register_game`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HydraGameApi {
    pub struct_size: u32,
    pub version: u32,
    /// Executes one JSON command on the game's script thread (blocking the pipe
    /// worker thread up to `timeout_ms`, never the render thread). Returns `0`
    /// on success and writes UTF-8 JSON into `out_buf[..*out_len]`.
    pub call: unsafe extern "C" fn(
        cmd_json: *const u8,
        cmd_len: usize,
        out_buf: *mut u8,
        out_cap: usize,
        out_len: *mut usize,
        timeout_ms: u32,
    ) -> i32,
}

// SAFETY: `HydraGameApi` contains only POD fields and a thread-safe C callback pointer.
unsafe impl Send for HydraGameApi {}
unsafe impl Sync for HydraGameApi {}

static GAME_API: Mutex<Option<HydraGameApi>> = Mutex::new(None);
static TITLE_BASE: AtomicUsize = AtomicUsize::new(0);
static TITLE_SIZE: AtomicUsize = AtomicUsize::new(0);
static STARTED: AtomicBool = AtomicBool::new(false);
static START_TIME: OnceLock<Instant> = OnceLock::new();

/// Verified Xbox `game_scarlett_master.exe` (`1.0.2.1`) RAGE 9 data RVAs
/// (confirmed via `victory-loader/src/engine.rs` and `frame_rate.rs`).
const RVA_FW_TIMER_SIM_RATE: usize = 0x0684_9844;
const RVA_FW_TIMER_STEP_CAP: usize = 0x0684_9848;
const RVA_SHADOW_INV_SCALE: usize = 0x084d_dbc0;
const RVA_VOLUMETRIC_SIZE: usize = 0x0855_1628;
const RVA_VOLUMETRIC_MIPS: usize = 0x0855_162c;

/// Registers the game-plane callback table from `hydra_bridge.asi`.
/// Returns `0` on success, negative error code on null/size/version mismatch.
///
/// # Safety
/// If non-null, `api` must point to a valid [`HydraGameApi`] struct for the
/// duration of this call.
pub unsafe fn register_game_api(api: *const HydraGameApi) -> i32 {
    if api.is_null() {
        return -1;
    }
    // SAFETY: caller guarantees `api` is valid for reading `HydraGameApi`.
    let copy = unsafe { *api };
    if (copy.struct_size as usize) < size_of::<HydraGameApi>()
        || copy.version != HYDRA_GAME_API_VERSION
    {
        return -2;
    }
    let mut slot = GAME_API.lock().unwrap_or_else(|e| e.into_inner());
    *slot = Some(copy);
    tracing::info!(
        version = copy.version,
        "hydra: game plane registered (hydra_bridge.asi connected)"
    );
    0
}

/// Starts the Hydra Control local Named Pipe server (`\\.\pipe\victory_hydra`)
/// once per process. Disabled when `VICTORY_HYDRA=0`.
pub fn start(title_base: usize, title_size: usize) {
    TITLE_BASE.store(title_base, Ordering::Release);
    TITLE_SIZE.store(title_size, Ordering::Release);
    let _ = START_TIME.get_or_init(Instant::now);

    if std::env::var_os("VICTORY_HYDRA").is_some_and(|v| v == "0") {
        tracing::info!("hydra: disabled by VICTORY_HYDRA=0");
        return;
    }
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }

    let pipe_name = std::env::var("VICTORY_HYDRA_PIPE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| r"\\.\pipe\victory_hydra".to_owned());

    let spawn = std::thread::Builder::new()
        .name("victory-hydra".to_owned())
        .spawn(move || serve_pipe(&pipe_name));
    if let Err(e) = spawn {
        STARTED.store(false, Ordering::Release);
        tracing::warn!(error = %e, "hydra: failed to spawn pipe thread");
    }
}

/// Safely reads `T` from `title_base + rva` inside our own process using
/// `ReadProcessMemory` so unmapped/protected addresses return `None` instead
/// of throwing an access violation.
fn read_title_pod<T: Copy + Default>(rva: usize) -> Option<T> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn ReadProcessMemory(
            hprocess: *mut c_void,
            lpbaseaddress: *const c_void,
            lpbuffer: *mut c_void,
            nsize: usize,
            lpnumberofbytesread: *mut usize,
        ) -> i32;
    }
    let base = TITLE_BASE.load(Ordering::Acquire);
    let size = TITLE_SIZE.load(Ordering::Acquire);
    if base == 0 || rva.checked_add(size_of::<T>())? > size {
        return None;
    }
    let mut val = T::default();
    let mut read = 0usize;
    // SAFETY: `val` is a valid writable stack buffer of `size_of::<T>()` bytes.
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            (base + rva) as *const c_void,
            (&mut val as *mut T).cast::<c_void>(),
            size_of::<T>(),
            &mut read,
        )
    };
    if ok != 0 && read == size_of::<T>() {
        Some(val)
    } else {
        None
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Extracts a top-level string field `"key":"value"` from a flat JSON object.
fn json_get_str(line: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let mut pos = 0;
    while let Some(idx) = line[pos..].find(&needle) {
        let after_key = pos + idx + needle.len();
        let rest = line[after_key..].trim_start();
        if let Some(after_colon) = rest.strip_prefix(':') {
            let val = after_colon.trim_start();
            if let Some(mut s) = val.strip_prefix('"') {
                let mut out = String::new();
                let mut escaped = false;
                while let Some(ch) = s.chars().next() {
                    s = &s[ch.len_utf8()..];
                    if escaped {
                        match ch {
                            '"' => out.push('"'),
                            '\\' => out.push('\\'),
                            'n' => out.push('\n'),
                            'r' => out.push('\r'),
                            't' => out.push('\t'),
                            other => out.push(other),
                        }
                        escaped = false;
                    } else if ch == '\\' {
                        escaped = true;
                    } else if ch == '"' {
                        return Some(out);
                    } else {
                        out.push(ch);
                    }
                }
                return None;
            }
        }
        pos = after_key;
    }
    None
}

/// Extracts a top-level unsigned integer field `"key":123` from a flat JSON object.
fn json_get_u64(line: &str, key: &str) -> Option<u64> {
    let needle = format!("\"{key}\"");
    let mut pos = 0;
    while let Some(idx) = line[pos..].find(&needle) {
        let after_key = pos + idx + needle.len();
        let rest = line[after_key..].trim_start();
        if let Some(after_colon) = rest.strip_prefix(':') {
            let val = after_colon.trim_start();
            let digits: String = val.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                return digits.parse::<u64>().ok();
            }
        }
        pos = after_key;
    }
    None
}

/// Handles one JSON-line request and returns one JSON-line response (without trailing `\n`).
pub(crate) fn handle_command(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return r#"{"ok":false,"err":"empty"}"#.to_owned();
    }
    let Some(cmd) = json_get_str(trimmed, "cmd") else {
        return r#"{"ok":false,"err":"missing cmd"}"#.to_owned();
    };

    if cmd.starts_with("game.") {
        return forward_to_game(trimmed);
    }

    match cmd.as_str() {
        "ping" => {
            let pid = std::process::id();
            let uptime_s = START_TIME
                .get()
                .map_or(0.0, |t| t.elapsed().as_secs_f64());
            let base = TITLE_BASE.load(Ordering::Acquire);
            let size = TITLE_SIZE.load(Ordering::Acquire);
            let game_plane = GAME_API
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some();
            format!(
                r#"{{"ok":true,"pid":{pid},"uptime_s":{uptime_s:.2},"title_base":"0x{base:x}","title_size":{size},"game_plane":{game_plane},"prof_on":{},"pix_on":{},"detile_on":{}}}"#,
                crate::prof::on(),
                crate::pix::on(),
                crate::detile::on()
            )
        }
        "stats" => {
            let prof_draws = crate::prof::total_draws();
            let (sh_hits, sh_misses) = crate::shader_cache::counts();
            let card_passes = crate::host::card_passes_snapshot();
            let lean_stats = crate::host::lean_stats_snapshot();
            let game_plane = GAME_API
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some();
            format!(
                r#"{{"ok":true,"prof_on":{},"prof_draws":{prof_draws},"pix_on":{},"detile_on":{},"shader_cache_hits":{sh_hits},"shader_cache_misses":{sh_misses},"card_passes":{card_passes:?},"lean_stats":{lean_stats:?},"game_plane":{game_plane}}}"#,
                crate::prof::on(),
                crate::pix::on(),
                crate::detile::on()
            )
        }
        "prof.top" => {
            let limit = json_get_u64(trimmed, "limit").unwrap_or(10).clamp(1, 20) as usize;
            let rows = crate::prof::top_snapshot(limit);
            let mut arr = String::from("[");
            for (i, r) in rows.iter().enumerate() {
                if i > 0 {
                    arr.push(',');
                }
                arr.push_str(&format!(
                    r#"{{"kind":"{}","hash":"{:016x}","us_per_frame":{:.1},"calls_per_frame":{:.1}}}"#,
                    r.kind, r.hash, r.us_per_frame, r.calls_per_frame
                ));
            }
            arr.push(']');
            format!(
                r#"{{"ok":true,"prof_on":{},"rows":{arr}}}"#,
                crate::prof::on()
            )
        }
        "settings.list" => {
            let mut arr = String::from("[");
            for (i, (var, live)) in crate::host::menu::list_vars().into_iter().enumerate() {
                if i > 0 {
                    arr.push(',');
                }
                arr.push_str(&format!(
                    r#"{{"var":{},"live":{live}}}"#,
                    json_escape(var)
                ));
            }
            arr.push(']');
            format!(r#"{{"ok":true,"vars":{arr}}}"#)
        }
        "settings.set" => {
            let Some(var) = json_get_str(trimmed, "var") else {
                return r#"{"ok":false,"err":"missing var"}"#.to_owned();
            };
            let value = json_get_str(trimmed, "value").unwrap_or_default();
            match crate::host::menu::apply_live_var(&var, &value) {
                Ok(()) => format!(
                    r#"{{"ok":true,"applied":"live","var":{},"value":{}}}"#,
                    json_escape(&var),
                    json_escape(&value)
                ),
                Err(reason) => format!(
                    r#"{{"ok":false,"err":{},"var":{}}}"#,
                    json_escape(reason),
                    json_escape(&var)
                ),
            }
        }
        "engine.status" => {
            let low_shadows = std::env::var("VICTORY_ENGINE_STATUS_LOW_SHADOWS")
                .unwrap_or_else(|_| "Unset".to_owned());
            let low_vol = std::env::var("VICTORY_ENGINE_STATUS_LOW_VOLUMETRIC")
                .unwrap_or_else(|_| "Unset".to_owned());
            let sim_rate = read_title_pod::<f32>(RVA_FW_TIMER_SIM_RATE);
            let step_cap = read_title_pod::<f32>(RVA_FW_TIMER_STEP_CAP);
            let shadow_inv = read_title_pod::<f32>(RVA_SHADOW_INV_SCALE);
            let vol_size = read_title_pod::<u32>(RVA_VOLUMETRIC_SIZE);
            let vol_mips = read_title_pod::<u32>(RVA_VOLUMETRIC_MIPS);
            let opt_f32 = |v: Option<f32>| match v {
                Some(x) if x.is_finite() => format!("{x:.6}"),
                _ => "null".to_owned(),
            };
            let opt_u32 = |v: Option<u32>| match v {
                Some(x) => x.to_string(),
                None => "null".to_owned(),
            };
            let shadow_res = shadow_inv
                .filter(|&x| x.is_finite() && x > 0.0001)
                .map(|x| (1024.0 / x).round() as u32);
            format!(
                r#"{{"ok":true,"low_shadows_status":{},"low_volumetric_status":{},"fw_timer_sim_rate":{},"fw_timer_step_cap":{},"shadow_inv_scale":{},"shadow_res_est":{},"volumetric_size":{},"volumetric_mips":{}}}"#,
                json_escape(&low_shadows),
                json_escape(&low_vol),
                opt_f32(sim_rate),
                opt_f32(step_cap),
                opt_f32(shadow_inv),
                opt_u32(shadow_res),
                opt_u32(vol_size),
                opt_u32(vol_mips),
            )
        }
        _ => format!(r#"{{"ok":false,"err":"unknown cmd","cmd":{}}}"#, json_escape(&cmd)),
    }
}

fn forward_to_game(json_line: &str) -> String {
    let api = *GAME_API.lock().unwrap_or_else(|e| e.into_inner());
    let Some(api) = api else {
        return r#"{"ok":false,"err":"no game plane (load shvi_core.dll + hydra_bridge.asi)"}"#
            .to_owned();
    };
    let timeout_ms = json_get_u64(json_line, "timeout_ms")
        .unwrap_or(2500)
        .clamp(100, 15_000) as u32;
    let mut out_buf = vec![0u8; 32 * 1024];
    let mut out_len = 0usize;
    // SAFETY: `api.call` honors the registered `HydraGameApi` C ABI contract.
    let rc = unsafe {
        (api.call)(
            json_line.as_ptr(),
            json_line.len(),
            out_buf.as_mut_ptr(),
            out_buf.len(),
            &mut out_len,
            timeout_ms,
        )
    };
    let used = out_len.min(out_buf.len());
    if used > 0 {
        let s = String::from_utf8_lossy(&out_buf[..used]).trim().to_owned();
        if !s.is_empty() {
            return s;
        }
    }
    if rc == 0 {
        r#"{"ok":true}"#.to_owned()
    } else {
        format!(r#"{{"ok":false,"err":"game plane rc={rc}"}}"#)
    }
}

/// Builds a current-user-only `SECURITY_DESCRIPTOR` (`D:P(A;;GA;;;<SID>)`)
/// so other local accounts and remote clients cannot connect to the pipe.
fn build_current_user_sd() -> Option<*mut c_void> {
    #[repr(C)]
    struct SidAndAttributes {
        sid: *mut c_void,
        attributes: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn CloseHandle(h: *mut c_void) -> i32;
        fn LocalFree(hmem: *mut c_void) -> *mut c_void;
    }
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
        fn GetTokenInformation(
            token: *mut c_void,
            class: u32,
            info: *mut c_void,
            len: u32,
            ret_len: *mut u32,
        ) -> i32;
        fn ConvertSidToStringSidW(sid: *mut c_void, string_sid: *mut *mut u16) -> i32;
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            string_sd: *const u16,
            revision: u32,
            sd: *mut *mut c_void,
            sd_size: *mut u32,
        ) -> i32;
    }
    const TOKEN_QUERY: u32 = 0x0008;
    const TOKEN_USER: u32 = 1;
    const SDDL_REVISION_1: u32 = 1;

    let mut token: *mut c_void = std::ptr::null_mut();
    // SAFETY: valid Win32 handle/pointer calls with checked return codes and cleanup.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let mut need = 0u32;
        GetTokenInformation(token, TOKEN_USER, std::ptr::null_mut(), 0, &mut need);
        if need == 0 {
            CloseHandle(token);
            return None;
        }
        let mut buf = vec![0u8; need as usize];
        let ok = GetTokenInformation(
            token,
            TOKEN_USER,
            buf.as_mut_ptr().cast::<c_void>(),
            need,
            &mut need,
        );
        CloseHandle(token);
        if ok == 0 {
            return None;
        }
        let user = &*(buf.as_ptr().cast::<SidAndAttributes>());
        let mut sid_w: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.sid, &mut sid_w) == 0 || sid_w.is_null() {
            return None;
        }
        let mut len = 0usize;
        while *sid_w.add(len) != 0 {
            len += 1;
        }
        let sid_str = String::from_utf16_lossy(std::slice::from_raw_parts(sid_w, len));
        LocalFree(sid_w.cast::<c_void>());

        let sddl = format!("D:P(A;;GA;;;{sid_str})");
        let sddl_w: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut sd: *mut c_void = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl_w.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            std::ptr::null_mut(),
        ) == 0
            || sd.is_null()
        {
            return None;
        }
        Some(sd)
    }
}

fn serve_pipe(pipe_name: &str) {
    use std::os::windows::ffi::OsStrExt;

    #[repr(C)]
    struct SecurityAttributes {
        length: u32,
        sd: *mut c_void,
        inherit: i32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateNamedPipeW(
            name: *const u16,
            open_mode: u32,
            pipe_mode: u32,
            max_instances: u32,
            out_buf: u32,
            in_buf: u32,
            timeout: u32,
            sa: *const SecurityAttributes,
        ) -> *mut c_void;
        fn ConnectNamedPipe(pipe: *mut c_void, overlapped: *mut c_void) -> i32;
        fn DisconnectNamedPipe(pipe: *mut c_void) -> i32;
        fn ReadFile(
            h: *mut c_void,
            buf: *mut c_void,
            len: u32,
            read: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn WriteFile(
            h: *mut c_void,
            buf: *const c_void,
            len: u32,
            written: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn FlushFileBuffers(h: *mut c_void) -> i32;
        fn CloseHandle(h: *mut c_void) -> i32;
        fn GetLastError() -> u32;
        fn LocalFree(hmem: *mut c_void) -> *mut c_void;
    }

    const INVALID_HANDLE: *mut c_void = -1isize as *mut c_void;
    const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
    const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
    const PIPE_TYPE_BYTE: u32 = 0x0000_0000;
    const PIPE_READMODE_BYTE: u32 = 0x0000_0000;
    const PIPE_WAIT: u32 = 0x0000_0000;
    const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x0000_0008;
    const ERROR_PIPE_CONNECTED: u32 = 535;

    let Some(sd) = build_current_user_sd() else {
        tracing::warn!("hydra: could not build current-user pipe DACL; server disabled");
        return;
    };
    let sa = SecurityAttributes {
        length: size_of::<SecurityAttributes>() as u32,
        sd,
        inherit: 0,
    };
    let name_w: Vec<u16> = std::ffi::OsStr::new(pipe_name)
        .encode_wide()
        .chain(Some(0))
        .collect();

    // Create once with FILE_FLAG_FIRST_PIPE_INSTANCE to prevent squatting, then
    // accept sequential clients on the same handle (DisconnectNamedPipe between clients).
    // SAFETY: valid NUL-terminated pipe name and initialized `SECURITY_ATTRIBUTES`.
    let pipe = unsafe {
        CreateNamedPipeW(
            name_w.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            65536,
            65536,
            0,
            &sa,
        )
    };
    // SAFETY: `sd` was allocated by `ConvertStringSecurityDescriptorToSecurityDescriptorW`.
    unsafe {
        LocalFree(sd);
    }
    if pipe == INVALID_HANDLE || pipe.is_null() {
        tracing::warn!(
            pipe = pipe_name,
            error = %std::io::Error::last_os_error(),
            "hydra: CreateNamedPipeW failed"
        );
        return;
    }
    tracing::info!(pipe = pipe_name, "hydra: control pipe listening (current-user DACL)");

    let mut chunk = [0u8; 4096];
    loop {
        // SAFETY: valid named pipe handle owned solely by this thread.
        let connected = unsafe {
            ConnectNamedPipe(pipe, std::ptr::null_mut()) != 0
                || GetLastError() == ERROR_PIPE_CONNECTED
        };
        if !connected {
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }
        let mut pending = Vec::<u8>::with_capacity(4096);
        loop {
            let mut got = 0u32;
            // SAFETY: valid pipe handle and buffer.
            let ok = unsafe {
                ReadFile(
                    pipe,
                    chunk.as_mut_ptr().cast::<c_void>(),
                    chunk.len() as u32,
                    &mut got,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 || got == 0 {
                break;
            }
            pending.extend_from_slice(&chunk[..got as usize]);
            if pending.len() > 256 * 1024 {
                break;
            }
            while let Some(nl) = pending.iter().position(|&b| b == b'\n') {
                let raw: Vec<u8> = pending.drain(..=nl).collect();
                let line = String::from_utf8_lossy(&raw);
                let mut reply = handle_command(&line);
                reply.push('\n');
                let mut written = 0u32;
                // SAFETY: valid pipe handle and reply slice.
                let w_ok = unsafe {
                    WriteFile(
                        pipe,
                        reply.as_ptr().cast::<c_void>(),
                        reply.len() as u32,
                        &mut written,
                        std::ptr::null_mut(),
                    )
                };
                if w_ok == 0 {
                    break;
                }
            }
        }
        // SAFETY: flush & disconnect client cleanly so the next client can connect.
        unsafe {
            FlushFileBuffers(pipe);
            DisconnectNamedPipe(pipe);
        }
    }
    #[allow(unreachable_code)]
    unsafe {
        CloseHandle(pipe);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_field_extractors_handle_escapes_and_ints() {
        let s = r#"{"cmd":"settings.set","var":"VICTORY_AMBIENT","value":"0.8","limit":15}"#;
        assert_eq!(json_get_str(s, "cmd").as_deref(), Some("settings.set"));
        assert_eq!(json_get_str(s, "var").as_deref(), Some("VICTORY_AMBIENT"));
        assert_eq!(json_get_str(s, "value").as_deref(), Some("0.8"));
        assert_eq!(json_get_u64(s, "limit"), Some(15));
    }

    #[test]
    fn handle_command_reports_ping_and_rejects_restart_only_vars() {
        let ping = handle_command(r#"{"cmd":"ping"}"#);
        assert!(ping.contains(r#""ok":true"#), "{ping}");
        let restart = handle_command(
            r#"{"cmd":"settings.set","var":"VICTORY_LOW_SHADOWS","value":"1"}"#,
        );
        assert!(restart.contains("restart-only"), "{restart}");
    }

    #[test]
    fn game_plane_forwarding_uses_registered_callback() {
        unsafe extern "C" fn mock_call(
            _cmd: *const u8,
            _len: usize,
            out_buf: *mut u8,
            out_cap: usize,
            out_len: *mut usize,
            _timeout_ms: u32,
        ) -> i32 {
            let msg = br#"{"ok":true,"pong":"mock"}"#;
            let n = msg.len().min(out_cap);
            unsafe {
                std::ptr::copy_nonoverlapping(msg.as_ptr(), out_buf, n);
                *out_len = n;
            }
            0
        }
        let api = HydraGameApi {
            struct_size: size_of::<HydraGameApi>() as u32,
            version: HYDRA_GAME_API_VERSION,
            call: mock_call,
        };
        // SAFETY: `api` is a valid stack struct for the duration of this call.
        assert_eq!(unsafe { register_game_api(&api) }, 0);
        let resp = handle_command(r#"{"cmd":"game.ping"}"#);
        assert!(resp.contains(r#""pong":"mock""#), "{resp}");
        *GAME_API.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}
