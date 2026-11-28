//! The F1 settings menu, top-left FPS/FG line, and startup frame-gen notice.
//!
//! Press F1 while the game window has focus to open/close:
//! - Up/Down pick a setting or action
//! - Left/Right (or Enter on a setting) cycle its value
//! - Enter on an action runs it (Save & Restart, Save for Next Start, Reset, Close)
//! - Esc or F1 closes
//!
//! While open, `crate::SETTINGS_MENU_OPEN` suppresses keyboard input to the title.
//! Live items (Frame Generation 1X/2X..6X, FPS line, F3 stats panel) apply
//! immediately on the next frame; startup items save to `VICTORY_MENU_FILE`
//! (default `victory-settings.txt` next to the runner) and apply on restart.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_DOWN, VK_ESCAPE, VK_F1, VK_LEFT, VK_RETURN, VK_RIGHT, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use super::font::{colours, glyph};
use super::{Host, HostList, transition};

const GLYPH: (u32, u32) = (5, 7);
const CELL: (u32, u32) = (6, 9);
const COLUMNS: u32 = 62;
const ROWS: usize = 18;
const LINES: u32 = ROWS as u32 + 6;
const REPO: &str = "GITHUB.COM/TOLIMANL/VICTORY-HYDRA";
const RIM: u32 = 4;
const UPLOADS: usize = 4;

struct Item {
    var: &'static str,
    label: &'static str,
    values: &'static [(&'static str, &'static str)],
    live: bool,
}

const ON_OFF: &[(&str, &str)] = &[("1", "ON"), ("0", "OFF")];
const OFF_ON: &[(&str, &str)] = &[("0", "OFF"), ("1", "ON")];

#[rustfmt::skip]
const ITEMS: &[Item] = &[
    Item {
        var: "VICTORY_AMBIENT",
        label: "AMBIENT LIGHT",
        values: &[
            ("0.8", "ESTIMATE 0.8 (GI)"),
            ("1.0", "ESTIMATE 1.0 (GI)"),
            ("0.5", "ESTIMATE 0.5 (GI)"),
            ("constant:0.8", "CONSTANT 0.8"),
            ("off", "OFF"),
        ],
        live: true,
    },
    Item {
        var: "VICTORY_AMBIENT_SPECULAR",
        label: "SPECULAR REFLECTIONS",
        values: &[
            ("1", "ON (1.0)"),
            ("0.6", "ON (0.6)"),
            ("1,0.2", "ON (1.0 / 0.2 SKY)"),
            ("0", "OFF"),
        ],
        live: true,
    },
    Item {
        var: "VICTORY_AMBIENT_SPECULAR_ALPHA",
        label: "SPECULAR ALPHA LAYER",
        values: &[
            ("0", "OPAQUE ONLY"),
            ("1", "OPAQUE AND ALPHA"),
        ],
        live: true,
    },
    Item {
        var: "VICTORY_AMBIENT_ONCE",
        label: "AMBIENT 1-BOUNCE",
        values: &[
            ("0", "OFF (MULTI-BOUNCE)"),
            ("1", "ON (1 BOUNCE)"),
        ],
        live: true,
    },
    Item {
        var: "VICTORY_AMBIENT_AIR",
        label: "AMBIENT FOG CAP",
        values: &[
            ("0", "OFF"),
            ("1.5", "1.5X"),
            ("1.0", "1.0X"),
            ("2.0", "2.0X"),
        ],
        live: true,
    },
    Item {
        var: "VICTORY_AMBIENT_REFLECT",
        label: "GLOSSY REFLECT CAP",
        values: &[
            ("0", "OFF (WITH SPECULAR)"),
            ("0.5", "0.5 CAP"),
            ("1.0", "1.0 CAP"),
        ],
        live: true,
    },
    Item {
        var: "VICTORY_MENU_FRAMEGEN",
        label: "FRAME GENERATION",
        values: &[
            ("0", "OFF (1X)"),
            ("2", "2X"),
            ("3", "3X (RTX 40+)"),
            ("4", "4X (RTX 40+)"),
        ],
        live: false,
    },
    Item {
        var: "VICTORY_DLSS_PRESET",
        label: "DLSS MODEL PRESET",
        values: &[
            ("", "DEFAULT"),
            ("k", "K (DLSS 4)"),
            ("j", "J (DLSS 4)"),
            ("m", "M"),
            ("l", "L"),
        ],
        live: false,
    },
    Item {
        var: "VICTORY_MENU_FPS",
        label: "FRAME RATE CAP",
        values: &[
            ("0", "NONE (UNLIMITED)"),
            ("60", "60 FPS"),
            ("120", "120 FPS"),
            ("30", "30 FPS"),
        ],
        live: true,
    },
    Item {
        var: "VICTORY_FPS_LINE",
        label: "FPS/FG AT TOP-LEFT",
        values: ON_OFF,
        live: true,
    },
    Item {
        var: "VICTORY_PANEL",
        label: "STATS PANEL (F3)",
        values: OFF_ON,
        live: true,
    },
    Item {
        var: "VICTORY_WC_NO_BACK",
        label: "SKIP WC COPY-BACKS",
        values: ON_OFF,
        live: true,
    },
    Item {
        var: "VICTORY_ROOT_BIND_CACHE",
        label: "ROOT BIND CACHE",
        values: ON_OFF,
        live: true,
    },
    Item {
        var: "VICTORY_TEXTURE_NO_WAIT",
        label: "TEXTURE NO WAIT",
        values: ON_OFF,
        live: true,
    },
    Item {
        var: "VICTORY_GPU_DETILE",
        label: "GPU TEXTURE DETILE",
        values: OFF_ON,
        live: true,
    },
    Item {
        var: "VICTORY_PIX",
        label: "PIX/NSIGHT MARKERS",
        values: OFF_ON,
        live: true,
    },
    Item {
        var: "VICTORY_PROF",
        label: "PASS/SHADER PROFILER",
        values: OFF_ON,
        live: true,
    },
    Item {
        var: "VICTORY_LOW_SHADOWS",
        label: "RAGE9 SHADOW MAPS",
        values: &[
            ("0", "STOCK (CONSOLE)"),
            ("1", "LOW 1280PX"),
            ("2", "ULTRA-LOW 1024PX"),
        ],
        live: false,
    },
    Item {
        var: "VICTORY_LOW_VOLUMETRIC",
        label: "RAGE9 VOLUMETRIC MAP",
        values: &[
            ("0", "STOCK (256PX/8M)"),
            ("1", "LOW (128PX/7M)"),
        ],
        live: false,
    },
    Item {
        var: "VICTORY_MENU_DLSS",
        label: "DLSS SUPER RES",
        values: &[
            ("quality", "QUALITY (0.67)"),
            ("balanced", "BALANCED (0.58)"),
            ("performance", "PERFORMANCE (0.50)"),
            ("ultraperformance", "ULTRA PERF (0.33)"),
            ("dlaa", "DLAA (NATIVE)"),
            ("off", "OFF"),
        ],
        live: false,
    },
    Item {
        var: "VICTORY_MENU_FG_TYPE",
        label: "FRAME GEN ENGINE",
        values: &[
            ("auto", "AUTO"),
            ("fsr", "AMD FSR 3 (RTX 20/30)"),
            ("dlss", "NVIDIA DLSS (RTX 40+)"),
        ],
        live: false,
    },
    Item {
        var: "VICTORY_MENU_OUTPUT",
        label: "OUTPUT RESOLUTION",
        values: &[
            ("1280x720", "1280X720 (FAST)"),
            ("1600x900", "1600X900"),
            ("1920x1080", "1920X1080 (FHD)"),
            ("2560x1440", "2560X1440 (QHD)"),
        ],
        live: false,
    },
    Item {
        var: "VICTORY_WATER_LIGHT",
        label: "WATER LIGHT",
        values: &[
            ("0", "OFF (FAST)"),
            ("1", "ON (WAVE32 FIXED)"),
        ],
        live: false,
    },
    Item {
        var: "VICTORY_LEAVE_OUT",
        label: "HEAT HAZE / DEFRAG",
        values: &[
            ("02384a60fda6f2d0,731a39171b568298", "SKIP (FAST)"),
            ("", "RUN ALL (SLOWER)"),
        ],
        live: false,
    },
    Item {
        var: "VICTORY_SKIP_RT_TRACE",
        label: "SKIP DEAD RT TRACE",
        values: ON_OFF,
        live: false,
    },
    Item {
        var: "VICTORY_EARLY_DEPTH",
        label: "EARLY DEPTH TEST",
        values: ON_OFF,
        live: false,
    },
];

const ACTIONS: [&str; 4] = [
    "SAVE AND RESTART NOW",
    "SAVE FOR NEXT START",
    "RESET ALL TO DEFAULTS",
    "CLOSE MENU",
];

fn get(var: &str) -> String {
    std::env::var(var).unwrap_or_default()
}

fn new_console(command_line: &str) -> std::io::Result<()> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        CREATE_NEW_CONSOLE, CreateProcessW, PROCESS_INFORMATION, STARTUPINFOW,
    };
    let mut line: Vec<u16> = command_line.encode_utf16().chain(Some(0)).collect();
    // SAFETY: plain structs, zero is their empty value.
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: `line` is a writable, terminated UTF-16 string; no handles inherited.
    let made = unsafe {
        CreateProcessW(
            std::ptr::null(),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_NEW_CONSOLE,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut process,
        )
    };
    if made == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the two handles CreateProcessW gave, closed once.
    unsafe {
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    Ok(())
}

fn live(item: &Item) -> bool {
    item.live
        || (item.var == "VICTORY_MENU_FRAMEGEN" && crate::streamline::framegen_live())
        || (item.var == "VICTORY_DLSS_PRESET" && crate::streamline::dlss_live())
}

fn fps_line_on() -> &'static AtomicBool {
    static ON: OnceLock<AtomicBool> = OnceLock::new();
    ON.get_or_init(|| AtomicBool::new(std::env::var_os("VICTORY_FPS_LINE").is_none_or(|v| v != "0")))
}

fn fps_line() -> Option<String> {
    if !fps_line_on().load(Ordering::Relaxed) {
        return None;
    }
    if super::panel::shown().load(Ordering::Relaxed) {
        return None;
    }
    let fps = f64::from(super::METER_FPS_TENTHS.load(Ordering::Relaxed)) / 10.0;
    if fps <= 0.0 {
        return None;
    }
    Some(match crate::streamline::shown_per_frame() {
        1 => format!("FPS: {fps:.1}  (F1 MENU)"),
        per => format!("FPS: {fps:.1}  FG {per}X: {:.1}  (F1)", fps * f64::from(per)),
    })
}

fn index_of(item: &Item, value: &str) -> Option<usize> {
    item.values
        .iter()
        .position(|(v, _)| v.eq_ignore_ascii_case(value))
}

fn engine_status_tag(var: &str, value: &str) -> &'static str {
    if value.is_empty() || value == "0" {
        return "";
    }
    let status_var = match var {
        "VICTORY_LOW_SHADOWS" => "VICTORY_ENGINE_STATUS_LOW_SHADOWS",
        "VICTORY_LOW_VOLUMETRIC" => "VICTORY_ENGINE_STATUS_LOW_VOLUMETRIC",
        _ => return "",
    };
    match std::env::var(status_var).as_deref() {
        Ok("Done") => " (OK)",
        Ok("Differs" | "Failed") => " (NOT APPLIED)",
        _ => "",
    }
}

fn shown_value(item: &Item, value: &str) -> String {
    let base = match index_of(item, value) {
        Some(i) => item.values[i].1.to_owned(),
        None if value.is_empty() => "DEFAULT".to_owned(),
        None => value.to_ascii_uppercase(),
    };
    let tag = engine_status_tag(item.var, value);
    if tag.is_empty() { base } else { format!("{base}{tag}") }
}

fn keys() -> &'static Mutex<Vec<u16>> {
    static KEYS: OnceLock<Mutex<Vec<u16>>> = OnceLock::new();
    KEYS.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("settings menu keys".into())
            .spawn(|| {
                const WATCHED: [u16; 7] =
                    [VK_F1, VK_UP, VK_DOWN, VK_LEFT, VK_RIGHT, VK_RETURN, VK_ESCAPE];
                let mut was = [false; WATCHED.len()];
                loop {
                    std::thread::sleep(Duration::from_millis(10));
                    let mut process = 0;
                    // SAFETY: plain queries; room for the answer.
                    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), &mut process) };
                    let ours = process == std::process::id();
                    for (i, &vk) in WATCHED.iter().enumerate() {
                        // SAFETY: a plain query.
                        let down = ours && unsafe { GetAsyncKeyState(i32::from(vk)) } < 0;
                        if down && !was[i] {
                            keys().lock().unwrap_or_else(|e| e.into_inner()).push(vk);
                        }
                        was[i] = down;
                    }
                }
            });
        Mutex::new(Vec::new())
    })
}

const fn box_size(columns: u32, lines: u32, scale: u32) -> (u32, u32) {
    ((columns * CELL.0 + 2 * RIM) * scale, (lines * CELL.1 + 2 * RIM) * scale)
}

const fn size(scale: u32) -> (u32, u32) {
    box_size(COLUMNS, LINES, scale)
}

fn lit(lines: &[String], columns: u32, x: u32, y: u32) -> bool {
    let (Some(x), Some(y)) = (x.checked_sub(RIM), y.checked_sub(RIM)) else {
        return false;
    };
    if x % CELL.0 >= GLYPH.0 || y % CELL.1 >= GLYPH.1 || x / CELL.0 >= columns {
        return false;
    }
    let Some(&c) = lines
        .get((y / CELL.1) as usize)
        .and_then(|l| l.as_bytes().get((x / CELL.0) as usize))
    else {
        return false;
    };
    (glyph(c.to_ascii_uppercase())[(y % CELL.1) as usize] >> (GLYPH.0 - 1 - x % CELL.0)) & 1 != 0
}

fn painted(lines: &[String], columns: u32, rows: u32, scale: u32, colours: [&[u8]; 2], pitch: usize) -> Vec<u8> {
    let (width, height) = box_size(columns, rows, scale);
    let bytes = colours[0].len();
    let mut out = vec![0; pitch * height as usize];
    for y in 0..height / scale {
        let top = pitch * (y * scale) as usize;
        for x in 0..width / scale {
            let colour = colours[usize::from(lit(lines, columns, x, y))];
            for i in 0..scale {
                let at = top + (x * scale + i) as usize * bytes;
                out[at..at + bytes].copy_from_slice(colour);
            }
        }
        for i in 1..scale as usize {
            out.copy_within(top..top + pitch, top + i * pitch);
        }
    }
    out
}

struct Buffers {
    columns: u32,
    rows: u32,
    format: DXGI_FORMAT,
    scale: u32,
    pitch: u32,
    uploads: Vec<(ID3D12Resource, *mut u8)>,
    card: ID3D12Resource,
    turn: usize,
    fresh: bool,
    used: bool,
}

// SAFETY: free-threaded resources, and memory only written under the presenter's lock.
unsafe impl Send for Buffers {}

impl Buffers {
    fn new(host: &Host, columns: u32, rows: u32, format: DXGI_FORMAT, scale: u32) -> Option<Buffers> {
        let (width, height) = box_size(columns, rows, scale);
        let pitch = (width * colours(format)?[0].len() as u32).next_multiple_of(256);
        let bytes = u64::from(pitch) * u64::from(height);
        let uploads = (0..UPLOADS)
            .map(|_| host.upload_buffer(bytes))
            .collect::<Option<Vec<_>>>()?;
        let heap = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_DEFAULT,
            ..Default::default()
        };
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Width: bytes,
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            ..Default::default()
        };
        let mut card: Option<ID3D12Resource> = None;
        // SAFETY: plain creation of a buffer.
        unsafe {
            host.device
                .CreateCommittedResource(
                    &heap,
                    D3D12_HEAP_FLAG_NONE,
                    &desc,
                    D3D12_RESOURCE_STATE_COMMON,
                    None,
                    &mut card,
                )
                .ok()?;
        }
        Some(Buffers {
            columns,
            rows,
            format,
            scale,
            pitch,
            uploads,
            card: card?,
            turn: 0,
            fresh: false,
            used: false,
        })
    }

    fn write(&mut self, lines: &[String]) {
        let Some(colours) = colours(self.format) else {
            return;
        };
        let picture = painted(lines, self.columns, self.rows, self.scale, colours, self.pitch as usize);
        self.turn = (self.turn + 1) % UPLOADS;
        // SAFETY: a mapped upload buffer of the picture's size.
        unsafe {
            std::ptr::copy_nonoverlapping(picture.as_ptr(), self.uploads[self.turn].1, picture.len());
        }
        self.fresh = true;
    }

    fn copy(
        &mut self,
        list: &ID3D12GraphicsCommandList,
        back: &ID3D12Resource,
        at: (u32, u32),
        clip: (u32, u32),
        (width, height): (u32, u32),
    ) {
        if at.0 >= width || at.1 >= height {
            return;
        }
        let w = clip.0.min(width - at.0);
        let h = clip.1.min(height - at.1);
        let full = box_size(self.columns, self.rows, self.scale);
        // SAFETY: an open list and live resources; the box lies within both.
        unsafe {
            if std::mem::take(&mut self.fresh) {
                if std::mem::replace(&mut self.used, true) {
                    list.ResourceBarrier(&[transition(
                        &self.card,
                        D3D12_RESOURCE_STATE_COPY_SOURCE,
                        D3D12_RESOURCE_STATE_COPY_DEST,
                    )]);
                }
                list.CopyResource(&self.card, &self.uploads[self.turn].0);
                list.ResourceBarrier(&[transition(
                    &self.card,
                    D3D12_RESOURCE_STATE_COPY_DEST,
                    D3D12_RESOURCE_STATE_COPY_SOURCE,
                )]);
            }
            let from = D3D12_TEXTURE_COPY_LOCATION {
                pResource: std::mem::transmute_copy(&self.card),
                Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                        Offset: 0,
                        Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                            Format: self.format,
                            Width: full.0,
                            Height: full.1,
                            Depth: 1,
                            RowPitch: self.pitch,
                        },
                    },
                },
            };
            let to = D3D12_TEXTURE_COPY_LOCATION {
                pResource: std::mem::transmute_copy(back),
                Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
            };
            let part = D3D12_BOX {
                left: 0,
                top: 0,
                front: 0,
                right: w,
                bottom: h,
                back: 1,
            };
            list.CopyTextureRegion(&to, at.0, at.1, 0, &from, Some(&part));
        }
    }
}

pub(super) struct Menu {
    values: Vec<String>,
    started: Vec<String>,
    at: usize,
    note: String,
    changed: bool,
    painted: Option<String>,
    buffers: Option<Buffers>,
    failed: bool,
}

impl Menu {
    pub(super) fn new() -> Menu {
        let _ = keys();
        if std::env::var_os("VICTORY_MENU_OPEN").is_some_and(|v| v == "1") {
            crate::SETTINGS_MENU_OPEN.store(true, Ordering::Relaxed);
        }
        let values: Vec<String> = ITEMS.iter().map(|i| get(i.var)).collect();
        Menu {
            started: values.clone(),
            values,
            at: 0,
            note: String::new(),
            changed: true,
            painted: None,
            buffers: None,
            failed: false,
        }
    }

    fn open() -> bool {
        crate::SETTINGS_MENU_OPEN.load(Ordering::Relaxed)
    }

    fn lines(&self) -> Vec<String> {
        let total = ITEMS.len() + ACTIONS.len();
        let first = self.at.saturating_sub(ROWS / 2).min(total.saturating_sub(ROWS));
        let mut out = vec![
            "VICTORY-HYDRA SETTINGS              F1 / ESC CLOSES".to_owned(),
            "UP/DOWN PICK   LEFT/RIGHT CHANGE   ENTER RUNS ACTION".to_owned(),
            String::new(),
        ];
        for row in first..(first + ROWS).min(total) {
            let cursor = if row == self.at { '>' } else { ' ' };
            let line = if let Some(item) = ITEMS.get(row) {
                let value = &self.values[row];
                let shown = shown_value(item, value);
                let mark = if *value != self.started[row] {
                    if live(item) { " *LIVE" } else { " *" }
                } else if live(item) {
                    " LIVE"
                } else {
                    ""
                };
                format!("{cursor} {:<22} < {shown} >{mark}", item.label)
            } else {
                format!("{cursor} {}", ACTIONS[row - ITEMS.len()])
            };
            out.push(line.chars().take(COLUMNS as usize).collect());
        }
        while out.len() < 3 + ROWS {
            out.push(String::new());
        }
        out.push(String::new());
        out.push(if self.note.is_empty() {
            "* = CHANGED (NEEDS RESTART UNLESS MARKED LIVE)".to_owned()
        } else {
            self.note.clone()
        });
        out.push(REPO.to_owned());
        out
    }

    fn change(&mut self, i: usize, step: isize) {
        let item = &ITEMS[i];
        let n = item.values.len() as isize;
        let now = index_of(item, &self.values[i]).map_or(-1, |x| x as isize);
        let next = if now < 0 {
            if step > 0 { 0 } else { n - 1 }
        } else {
            (now + step).rem_euclid(n)
        };
        self.values[i] = item.values[next as usize].0.to_owned();
        if live(item) {
            self.apply_live(i);
            self.note = "APPLIED LIVE (SAVE FOR NEXT START TO KEEP)".to_owned();
        } else {
            self.note.clear();
        }
    }

    fn apply_live(&self, i: usize) {
        let (var, value) = (ITEMS[i].var, &self.values[i]);
        // SAFETY: live variables are read on demand; Windows locks its env block.
        unsafe {
            if value.is_empty() {
                std::env::remove_var(var);
            } else {
                std::env::set_var(var, value);
            }
        }
        let _ = apply_live_var(var, value);
    }

    fn settings_path() -> Option<std::path::PathBuf> {
        if let Some(p) = std::env::var_os("VICTORY_MENU_FILE") {
            return Some(std::path::PathBuf::from(p));
        }
        std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.join("victory-settings.txt")))
    }

    fn save(&mut self) -> bool {
        let Some(path) = Self::settings_path() else {
            self.note = "NO SETTINGS FILE PATH".to_owned();
            return false;
        };
        let mut kept: Vec<(String, String)> = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let l = l.trim();
                if l.is_empty() || l.starts_with('#') {
                    return None;
                }
                l.split_once('=')
                    .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
            })
            .filter(|(k, _)| k.starts_with("VICTORY_"))
            .collect();
        for (i, item) in ITEMS.iter().enumerate() {
            if self.values[i] == self.started[i] {
                continue;
            }
            kept.retain(|(k, _)| k != item.var);
            kept.push((item.var.to_owned(), self.values[i].clone()));
        }
        let mut text = String::from(
            "# Saved by the VIctory-Hydra F1 menu; loaded automatically at startup.\r\n",
        );
        for (k, v) in &kept {
            text.push_str(&format!("{k}={v}\r\n"));
        }
        match std::fs::write(&path, text) {
            Ok(()) => {
                tracing::info!(path = %path.display(), "settings menu: saved");
                self.started = self.values.clone();
                self.note = "SAVED TO VICTORY-SETTINGS.TXT".to_owned();
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, path = %path.display(), "settings menu: not saved");
                self.note = "COULD NOT SAVE SETTINGS FILE".to_owned();
                false
            }
        }
    }

    fn restart(&mut self) {
        let cmdline = if let Some(play) = std::env::var_os("VICTORY_MENU_PLAY") {
            let play = play.to_string_lossy();
            let args = get("VICTORY_MENU_ARGS");
            if play.to_ascii_lowercase().ends_with(".ps1") {
                let escaped = play.replace('\'', "''");
                let ps = format!("Start-Sleep 2; & '{escaped}' {args}");
                format!(
                    "powershell.exe -NoProfile -ExecutionPolicy Bypass -Command \"{}\"",
                    ps.replace('"', "\\\"")
                )
            } else {
                format!("cmd.exe /c \"timeout /t 2 /nobreak >nul && call \"\"{play}\"\" {args}\"")
            }
        } else if let Ok(exe) = std::env::current_exe() {
            let mut joined = format!("\"{}\"", exe.display());
            for a in std::env::args_os().skip(1) {
                let s = a.to_string_lossy();
                if s.contains(' ') {
                    joined.push_str(&format!(" \"{s}\""));
                } else {
                    joined.push_str(&format!(" {s}"));
                }
            }
            format!("cmd.exe /c \"timeout /t 2 /nobreak >nul && {joined}\"")
        } else {
            self.note = "SAVED. RESTART THE LAUNCHER YOURSELF".to_owned();
            return;
        };

        match new_console(&cmdline) {
            Ok(()) => {
                tracing::info!(cmdline, "settings menu: restarting");
                // SAFETY: immediate termination avoids Streamline DLL unload deadlocks.
                unsafe {
                    use windows_sys::Win32::System::Threading::{
                        GetCurrentProcess, TerminateProcess,
                    };
                    TerminateProcess(GetCurrentProcess(), 0);
                }
                std::process::exit(0);
            }
            Err(e) => {
                tracing::warn!(error = %e, "settings menu: restart failed");
                self.note = "SAVED, BUT RESTART FAILED".to_owned();
            }
        }
    }

    fn reset(&mut self) {
        if let Some(path) = Self::settings_path() {
            let _ = std::fs::remove_file(path);
        }
        self.note = "RESET. LAUNCHER DEFAULTS ON NEXT START".to_owned();
    }

    pub(super) fn frame(&mut self) {
        let pressed = std::mem::take(&mut *keys().lock().unwrap_or_else(|e| e.into_inner()));
        if pressed.is_empty() {
            return;
        }
        let total = ITEMS.len() + ACTIONS.len();
        for vk in pressed {
            let open = Self::open();
            if vk == VK_F1 || (open && vk == VK_ESCAPE) {
                crate::SETTINGS_MENU_OPEN.store(!open && vk == VK_F1, Ordering::Relaxed);
                self.note.clear();
                self.changed = true;
                continue;
            }
            if !open {
                continue;
            }
            match vk {
                VK_UP => self.at = (self.at + total - 1) % total,
                VK_DOWN => self.at = (self.at + 1) % total,
                VK_LEFT if self.at < ITEMS.len() => self.change(self.at, -1),
                VK_RIGHT if self.at < ITEMS.len() => self.change(self.at, 1),
                VK_RETURN if self.at < ITEMS.len() => self.change(self.at, 1),
                VK_RETURN => match self.at - ITEMS.len() {
                    0 => {
                        if self.save() {
                            self.restart();
                        }
                    }
                    1 => {
                        self.save();
                    }
                    2 => self.reset(),
                    _ => crate::SETTINGS_MENU_OPEN.store(false, Ordering::Relaxed),
                },
                _ => {}
            }
            self.changed = true;
        }
    }

    pub(super) fn draw(
        &mut self,
        host: &Host,
        list: &HostList,
        back: &ID3D12Resource,
        (width, height): (u32, u32),
        format: DXGI_FORMAT,
    ) {
        if self.failed {
            return;
        }
        let open = Self::open();
        let line = if open { None } else { fps_line() };
        if !open && line.is_none() {
            return;
        }
        #[cfg(feature = "diagnostics")]
        let _ours = crate::replay::record::unrecorded();
        let scale = (height / 540).max(1);
        if self
            .buffers
            .as_ref()
            .is_none_or(|b| (b.format, b.scale) != (format, scale))
        {
            std::mem::forget(self.buffers.take());
            self.buffers = Buffers::new(host, COLUMNS, LINES, format, scale);
            self.failed = self.buffers.is_none();
            self.changed = true;
        }
        if self.painted != line {
            self.painted = line.clone();
            self.changed = true;
        }
        let lines = if self.changed {
            Some(line.as_ref().map_or_else(|| self.lines(), |l| vec![l.clone()]))
        } else {
            None
        };
        let Some(buffers) = &mut self.buffers else {
            return;
        };
        if let Some(lines) = lines {
            self.changed = false;
            buffers.write(&lines);
        }
        let (w, h) = match &line {
            Some(l) => (
                (l.len() as u32 * CELL.0 + 2 * RIM) * scale,
                (CELL.1 + 2 * RIM) * scale,
            ),
            None => size(scale),
        };
        let (w, h) = (w.min(width), h.min(height));
        let at = if line.is_some() {
            (0, 0)
        } else {
            ((width - w) / 2, (height - h) / 2)
        };
        buffers.copy(&list.list, back, at, (w, h), (width, height));
    }
}

const NOTICE_FOR: Duration = Duration::from_secs(20);
const NOTICE_COLUMNS: u32 = 54;

/// Top-center startup notice showing active GPU / Frame Generation engine for 20s.
pub(super) struct Notice {
    lines: Vec<String>,
    columns: u32,
    rows: u32,
    since: Option<Instant>,
    buffers: Option<Buffers>,
    written: bool,
    done: bool,
}

impl Notice {
    pub(super) fn new() -> Notice {
        let lines: Vec<String> = crate::streamline::notice()
            .into_iter()
            .map(|l| l.to_ascii_uppercase())
            .collect();
        let columns = lines
            .iter()
            .map(|l| l.len() as u32)
            .max()
            .unwrap_or(0)
            .min(NOTICE_COLUMNS);
        let rows = lines.len() as u32;
        Notice {
            done: lines.is_empty(),
            lines,
            columns,
            rows,
            since: None,
            buffers: None,
            written: false,
        }
    }

    pub(super) fn draw(
        &mut self,
        host: &Host,
        list: &HostList,
        back: &ID3D12Resource,
        (width, height): (u32, u32),
        format: DXGI_FORMAT,
    ) {
        if self.done {
            return;
        }
        let since = *self.since.get_or_insert_with(Instant::now);
        if since.elapsed() > NOTICE_FOR {
            std::mem::forget(self.buffers.take());
            self.done = true;
            return;
        }
        #[cfg(feature = "diagnostics")]
        let _ours = crate::replay::record::unrecorded();
        let scale = (height / 540).max(1);
        if self
            .buffers
            .as_ref()
            .is_none_or(|b| (b.format, b.scale) != (format, scale))
        {
            std::mem::forget(self.buffers.take());
            self.buffers = Buffers::new(host, self.columns, self.rows, format, scale);
            self.written = false;
            if self.buffers.is_none() {
                self.done = true;
                return;
            }
        }
        let Some(buffers) = &mut self.buffers else {
            return;
        };
        if !std::mem::replace(&mut self.written, true) {
            buffers.write(&self.lines);
        }
        let (w, h) = box_size(self.columns, self.rows, scale);
        let at = (width.saturating_sub(w) / 2, 24 * scale);
        buffers.copy(&list.list, back, at, (w, h), (width, height));
    }
}

/// Applies one live-editable setting without calling `std::env::set_var`
/// (safe for both the F1 menu and the Hydra Control pipe thread).
/// Returns `Err("restart-only")` for startup-only `OnceLock` knobs and
/// `Err("unknown-var")` for unrecognized variables.
pub(crate) fn apply_live_var(var: &str, value: &str) -> Result<(), &'static str> {
    match var {
        "VICTORY_AMBIENT" => crate::ambient::set_live(value),
        "VICTORY_AMBIENT_SPECULAR" => crate::ambient::set_specular_live(value),
        "VICTORY_AMBIENT_SPECULAR_ALPHA" => {
            crate::ambient::set_specular_alpha_live(value == "1")
        }
        "VICTORY_AMBIENT_ONCE" => crate::ambient::set_once_live(value == "1"),
        "VICTORY_AMBIENT_AIR" => crate::ambient::set_air_live(value),
        "VICTORY_AMBIENT_REFLECT" => crate::ambient::set_reflect_live(value),
        "VICTORY_DLSS_PRESET" => crate::streamline::set_dlss_preset_live(value),
        "VICTORY_MENU_FRAMEGEN" => {
            crate::streamline::set_framegen(value.parse().unwrap_or(0));
        }
        "VICTORY_MENU_FPS" => {
            let ms = match value {
                "30" => 33.333,
                "60" => 16.667,
                "120" => 8.333,
                _ => 0.0,
            };
            crate::frame::set_interval_live(ms);
        }
        "VICTORY_FPS_LINE" => {
            fps_line_on().store(value != "0", Ordering::Relaxed);
        }
        "VICTORY_PANEL" => {
            super::panel::shown().store(value == "1", Ordering::Relaxed);
        }
        "VICTORY_ROOT_BIND_CACHE" => {
            crate::draw::set_root_bind_cache_live(value == "1");
        }
        "VICTORY_WC_NO_BACK" => {
            crate::shadow::set_wc_no_back_live(value == "1");
        }
        "VICTORY_TEXTURE_NO_WAIT" => {
            crate::texture::set_no_wait_live(value == "1");
        }
        "VICTORY_GPU_DETILE" => {
            crate::detile::set_live(value == "1");
        }
        "VICTORY_PIX" => {
            crate::pix::set_live(value == "1");
        }
        "VICTORY_PROF" => {
            crate::prof::set_live(value == "1");
        }
        other => {
            if ITEMS.iter().any(|item| item.var == other)
                || matches!(
                    other,
                    "VICTORY_SHADER_OPT"
                        | "VICTORY_LDS_ANALYZE"
                        | "VICTORY_CARD_COMPARE_EVERY"
                        | "VICTORY_PORT_CONSTANTS"
                        | "VICTORY_CARD_TOUCHED"
                        | "VICTORY_LOAD_DLLS"
                        | "VICTORY_MODS_DIR"
                )
            {
                return Err("restart-only");
            }
            return Err("unknown-var");
        }
    }
    Ok(())
}

/// Lists all F1-tracked settings and whether each applies live without restart.
pub(crate) fn list_vars() -> Vec<(&'static str, bool)> {
    ITEMS.iter().map(|item| (item.var, live(item))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_line_fits_and_every_character_has_a_picture() {
        let mut menu = Menu {
            values: ITEMS.iter().map(|i| i.values[0].0.to_owned()).collect(),
            started: Vec::new(),
            at: 0,
            note: String::new(),
            changed: false,
            painted: None,
            buffers: None,
            failed: false,
        };
        menu.started = menu.values.clone();
        for at in 0..ITEMS.len() + ACTIONS.len() {
            menu.at = at;
            for line in menu.lines() {
                assert!(line.len() <= COLUMNS as usize, "{line}");
                for c in line.bytes() {
                    assert_ne!(
                        glyph(c.to_ascii_uppercase()),
                        glyph(b'#'),
                        "{line}: {}",
                        c as char
                    );
                }
            }
        }
    }

    #[test]
    fn values_cycle_both_ways() {
        let mut menu = Menu {
            values: ITEMS.iter().map(|i| i.values[0].0.to_owned()).collect(),
            started: Vec::new(),
            at: 0,
            note: String::new(),
            changed: false,
            painted: None,
            buffers: None,
            failed: false,
        };
        menu.started = menu.values.clone();
        let n = ITEMS[0].values.len();
        for _ in 0..n {
            menu.change(0, 1);
        }
        assert_eq!(menu.values[0], ITEMS[0].values[0].0);
        menu.change(0, -1);
        assert_eq!(menu.values[0], ITEMS[0].values[n - 1].0);
    }
}
