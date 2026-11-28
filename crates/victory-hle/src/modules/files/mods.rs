//! Loose-file read-only mod override index (`VICTORY_MODS_DIR`).
//!
//! When `VICTORY_MODS_DIR=<dir>` is set (and exists), files placed inside it
//! matching a relative path on the game package drive (`G:` by default, or the
//! comma-separated letters in `VICTORY_MODS_DRIVES`) transparently override
//! read-only opens (`CreateFile*`, `fopen`/`_wfopen`, `GetFileAttributes*`,
//! and DirectStorage `OpenFile` which routes through `files::CreateFileW`).
//!
//! Write/create/delete opens and save/storage drives bypass overrides so saves
//! and settings always write to real title storage. Zero overhead when unset.
//!
//! Scope note: overrides apply to whole files opened on the virtual drive
//! (loose configs/videos/DLLs or whole replacement `.rpf` archives). Single-entry
//! replacement *inside* packed RPF9 archives is out of scope here.

use std::collections::{HashMap, HashSet};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

struct ModIndex {
    /// Allowed virtual drive letters (upper-case ASCII as u16, default `['G']`).
    drives: Vec<u16>,
    /// Upper-case normalized relative path (leading `\` stripped, `/` -> `\`)
    /// mapped to its absolute NUL-terminated host UTF-16 path.
    files: HashMap<Vec<u16>, Vec<u16>>,
    /// Relative paths already logged once so logs stay clean.
    logged: Mutex<HashSet<Vec<u16>>>,
}

static OVERRIDES_HIT: AtomicU64 = AtomicU64::new(0);

/// Number of read-only opens redirected to `VICTORY_MODS_DIR`.
pub fn override_hits() -> u64 {
    OVERRIDES_HIT.load(Ordering::Relaxed)
}

/// Number of indexed override files (`0` if `VICTORY_MODS_DIR` is unset/empty).
pub fn indexed_count() -> usize {
    index().map_or(0, |m| m.files.len())
}

fn parse_drives(spec: Option<&str>) -> Vec<u16> {
    let Some(spec) = spec else {
        return vec![u16::from(b'G')];
    };
    let mut out: Vec<u16> = spec
        .split([',', ';', ' '])
        .filter_map(|s| s.trim().trim_end_matches(':').bytes().next())
        .filter(u8::is_ascii_alphabetic)
        .map(|b| u16::from(b.to_ascii_uppercase()))
        .collect();
    if out.is_empty() {
        out.push(u16::from(b'G'));
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Normalizes a virtual-drive relative tail into an upper-case `\`-separated
/// key. Returns `None` if empty or if any segment traverses above root (`..`).
fn normalize_tail(tail: &[u16]) -> Option<Vec<u16>> {
    let sep = |c: &u16| *c == u16::from(b'\\') || *c == u16::from(b'/');
    let dot = u16::from(b'.');
    let mut parts: Vec<Vec<u16>> = Vec::new();
    for part in tail.split(sep) {
        match part {
            [] => {}
            [d] if *d == dot => {}
            [d, e] if [*d, *e] == [dot; 2] => {
                // Reject any path escaping the drive root.
                parts.pop()?;
            }
            _ => {
                let up: Vec<u16> = part
                    .iter()
                    .map(|&c| {
                        if (u16::from(b'a')..=u16::from(b'z')).contains(&c) {
                            c - 32
                        } else {
                            c
                        }
                    })
                    .collect();
                parts.push(up);
            }
        }
    }
    if parts.is_empty() {
        return None;
    }
    let mut key = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            key.push(u16::from(b'\\'));
        }
        key.extend_from_slice(part);
    }
    Some(key)
}

fn walk_dir(root: &Path, cur: &Path, out: &mut HashMap<Vec<u16>, Vec<u16>>) {
    let Ok(entries) = std::fs::read_dir(cur) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_dir() {
            walk_dir(root, &path, out);
        } else if ft.is_file()
            && let Ok(rel) = path.strip_prefix(root)
        {
            let raw_rel: Vec<u16> = rel.as_os_str().encode_wide().collect();
            if let Some(key) = normalize_tail(&raw_rel) {
                let abs = std::path::absolute(&path).unwrap_or(path);
                let mut wide: Vec<u16> = abs.as_os_str().encode_wide().collect();
                wide.push(0);
                out.insert(key, wide);
            }
        }
    }
}

fn build_index(root: &Path, drives_env: Option<&str>) -> Option<ModIndex> {
    if !root.is_dir() {
        return None;
    }
    let abs_root = std::path::absolute(root).unwrap_or_else(|_| PathBuf::from(root));
    let mut files = HashMap::new();
    walk_dir(&abs_root, &abs_root, &mut files);
    let drives = parse_drives(drives_env);
    tracing::info!(
        dir = %abs_root.display(),
        files = files.len(),
        "loose-file mod override active (VICTORY_MODS_DIR)"
    );
    if !files.is_empty() {
        println!(
            "mods           {} loose override file(s) indexed from {}",
            files.len(),
            abs_root.display()
        );
    }
    Some(ModIndex {
        drives,
        files,
        logged: Mutex::new(HashSet::new()),
    })
}

fn index() -> Option<&'static ModIndex> {
    static INDEX: OnceLock<Option<ModIndex>> = OnceLock::new();
    INDEX
        .get_or_init(|| {
            let dir = std::env::var_os("VICTORY_MODS_DIR")?;
            if dir.is_empty() {
                return None;
            }
            let drives = std::env::var("VICTORY_MODS_DRIVES").ok();
            build_index(Path::new(&dir), drives.as_deref())
        })
        .as_ref()
}

/// Looks up a read-only override for virtual drive `letter` (upper-case ASCII)
/// and relative `tail`. Returns the NUL-terminated host UTF-16 path if found.
pub(super) fn lookup(letter: u16, tail: &[u16]) -> Option<Vec<u16>> {
    let idx = index()?;
    if !idx.drives.contains(&letter) || idx.files.is_empty() {
        return None;
    }
    let key = normalize_tail(tail)?;
    let hit = idx.files.get(&key)?;
    OVERRIDES_HIT.fetch_add(1, Ordering::Relaxed);
    if idx
        .logged
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.clone())
    {
        tracing::info!(
            drive = %char::from(letter as u8),
            file = %String::from_utf16_lossy(&key),
            "mods: redirecting read-only open to VICTORY_MODS_DIR"
        );
    }
    Some(hit.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16s(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn normalize_rejects_escape_and_folds_case() {
        assert_eq!(normalize_tail(&u16s("/common/Data/Levels.xml")), Some(u16s("COMMON\\DATA\\LEVELS.XML")));
        assert_eq!(normalize_tail(&u16s("\\a\\b\\..\\c.dat")), Some(u16s("A\\C.DAT")));
        assert_eq!(normalize_tail(&u16s("..\\secret.txt")), None);
        assert_eq!(normalize_tail(&u16s("a\\..\\..\\escape.txt")), None);
        assert_eq!(normalize_tail(&u16s("\\")), None);
    }

    #[test]
    fn parse_drives_defaults_to_g() {
        assert_eq!(parse_drives(None), vec![u16::from(b'G')]);
        assert_eq!(parse_drives(Some("g:, t")), vec![u16::from(b'G'), u16::from(b'T')]);
    }

    #[test]
    fn build_index_finds_nested_files() {
        let dir = std::env::temp_dir().join(format!("victory-mods-test-{}", std::process::id()));
        let sub = dir.join("common").join("data");
        std::fs::create_dir_all(&sub).unwrap();
        let file = sub.join("test.xml");
        std::fs::write(&file, b"<test/>").unwrap();

        let idx = build_index(&dir, None).expect("index built");
        assert_eq!(idx.files.len(), 1);
        let key = normalize_tail(&u16s("/Common/DATA/test.xml")).unwrap();
        assert!(idx.files.contains_key(&key));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
