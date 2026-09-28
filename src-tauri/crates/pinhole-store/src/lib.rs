//! Everything Pinhole writes to disk lives here (SPEC §3). Nothing in this crate
//! may ever accept or store prompt / negative-prompt text — the only user text
//! stored is a Style the user explicitly saves (`Data/styles/`).
//!
//! OWNER: store agent.

pub mod datadir;
pub mod installed;
pub mod keychain;
pub mod presets;
pub mod settings;
pub mod styles;

mod files;

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub use datadir::DataDir;
pub use installed::{InstalledFile, InstalledIndex};
pub use settings::Settings;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("disk error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid file {path}: {msg}")]
    Parse { path: String, msg: String },
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("keychain: {0}")]
    Keychain(String),
}

impl StoreError {
    /// Stable `CoreError` code for this error (`io`, `not_found`, `invalid`).
    pub fn code(&self) -> &'static str {
        match self {
            StoreError::Io(_) | StoreError::Parse { .. } | StoreError::Keychain(_) => "io",
            StoreError::NotFound(_) => "not_found",
            StoreError::Invalid(_) => "invalid",
        }
    }

    /// Plain-language message for the UI (says what to do next where it can).
    /// `Invalid` and `Keychain` messages are already written for users.
    pub fn user_message(&self) -> String {
        match self {
            StoreError::Io(e) => format!("Could not read or write Pinhole's Data folder: {e}"),
            StoreError::Parse { path, msg } => format!("A file in Pinhole's Data folder is damaged ({path}): {msg}"),
            StoreError::NotFound(what) => format!("{what} was not found. It may have been deleted."),
            StoreError::Invalid(msg) | StoreError::Keychain(msg) => msg.clone(),
        }
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `bytes` to `path` atomically (temp file in the same dir + fsync + rename).
///
/// Creates the parent directory if needed. Readers see either the old or the new
/// contents, never a partial file. On Windows `std::fs::rename` uses
/// `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`, so an existing file is replaced; a
/// short retry covers the case where another process (e.g. a virus scanner)
/// briefly holds the target open.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let file_name = path
        .file_name()
        .ok_or_else(|| StoreError::Invalid(format!("not a file path: {}", path.display())))?;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    fs::create_dir_all(&parent)?;

    let (tmp_path, file) = create_temp_sibling(&parent, &file_name.to_string_lossy())?;
    let result = finish_atomic_write(file, bytes, &tmp_path, path);
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    } else {
        sync_dir(&parent);
    }
    result.map_err(StoreError::from)
}

fn create_temp_sibling(dir: &Path, file_name: &str) -> std::io::Result<(PathBuf, File)> {
    let pid = std::process::id();
    let mut last_err = None;
    for _ in 0..64 {
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!(".{file_name}.{pid}.{n}.tmp"));
        match OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(f) => return Ok((tmp, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last_err = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("could not create a temporary file")))
}

fn finish_atomic_write(mut file: File, bytes: &[u8], tmp: &Path, dest: &Path) -> std::io::Result<()> {
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    rename_replacing(tmp, dest)
}

#[cfg(windows)]
fn rename_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempt = 0u64;
    loop {
        match fs::rename(from, to) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && attempt < 5 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(20 * attempt));
            }
            other => return other,
        }
    }
}

#[cfg(not(windows))]
fn rename_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}

/// Persist the rename itself (best effort; directories can't be opened on Windows).
#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

/// Maximum slug length in characters.
const SLUG_MAX_CHARS: usize = 64;

/// File-name-safe slug ("Film photo" → "film-photo"); never derived from prompts.
///
/// Lower-cases, keeps letters and digits (any script), turns every other run of
/// characters into a single `-`, drops apostrophes, trims to 64 characters.
/// Never empty (`"untitled"`), and never a reserved Windows device name.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut chars = 0usize;
    let mut pending_dash = false;
    'outer: for c in name.chars() {
        if matches!(c, '\'' | '\u{2019}') {
            continue;
        }
        for lc in c.to_lowercase() {
            if lc.is_alphanumeric() {
                if pending_dash && !out.is_empty() {
                    if chars + 2 > SLUG_MAX_CHARS {
                        break 'outer;
                    }
                    out.push('-');
                    chars += 1;
                }
                pending_dash = false;
                if chars + 1 > SLUG_MAX_CHARS {
                    break 'outer;
                }
                out.push(lc);
                chars += 1;
            } else {
                pending_dash = true;
            }
        }
    }
    if out.is_empty() {
        return "untitled".into();
    }
    if is_windows_reserved(&out) {
        out.push_str("-1");
    }
    out
}

fn is_windows_reserved(slug: &str) -> bool {
    matches!(slug, "con" | "prn" | "aux" | "nul")
        || ((slug.starts_with("com") || slug.starts_with("lpt"))
            && slug.len() == 4
            && slug.as_bytes()[3].is_ascii_digit())
}

/// True when `id` is a slug as produced by [`slugify`] (safe to use as a file stem:
/// no separators, no dots, no traversal).
pub fn is_valid_slug(id: &str) -> bool {
    !id.is_empty() && slugify(id) == id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Film photo"), "film-photo");
        assert_eq!(slugify("  Studio product shot on white!  "), "studio-product-shot-on-white");
        assert_eq!(slugify("Anime -- cel // shading"), "anime-cel-shading");
        assert_eq!(slugify("Mike's style"), "mikes-style");
        assert_eq!(slugify("35mm"), "35mm");
        assert_eq!(slugify(""), "untitled");
        assert_eq!(slugify("!!!"), "untitled");
        assert_eq!(slugify("../../etc/passwd"), "etc-passwd");
        assert_eq!(slugify("C:\\Windows\\x.yaml"), "c-windows-x-yaml");
        assert_eq!(slugify("Ölgemälde Stil"), "ölgemälde-stil");
        assert_eq!(slugify("水彩画"), "水彩画");
    }

    #[test]
    fn slugify_reserved_and_length() {
        assert_eq!(slugify("CON"), "con-1");
        assert_eq!(slugify("lpt1"), "lpt1-1");
        assert_eq!(slugify("console"), "console");
        let long = "a".repeat(100);
        assert_eq!(slugify(&long).chars().count(), SLUG_MAX_CHARS);
        let words = "word ".repeat(40);
        let s = slugify(&words);
        assert!(s.chars().count() <= SLUG_MAX_CHARS);
        assert!(!s.ends_with('-'));
    }

    #[test]
    fn valid_slug() {
        assert!(is_valid_slug("film-photo"));
        assert!(is_valid_slug("film-photo-2"));
        assert!(!is_valid_slug(""));
        assert!(!is_valid_slug("../x"));
        assert!(!is_valid_slug("a/b"));
        assert!(!is_valid_slug("Film"));
        assert!(!is_valid_slug("x.yaml"));
        assert!(!is_valid_slug("builtin:film-photo"));
        assert!(!is_valid_slug("con"));
    }

    #[test]
    fn write_atomic_creates_and_replaces() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested").join("deeper").join("file.txt");
        write_atomic(&path, b"one").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"one");
        write_atomic(&path, b"two, longer").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two, longer");
        // No temp files left behind.
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["file.txt".to_string()]);
    }

    #[test]
    fn write_atomic_rejects_non_file_path() {
        assert!(matches!(write_atomic(Path::new("/"), b"x"), Err(StoreError::Invalid(_))));
    }

    #[test]
    fn store_error_messages() {
        let e = StoreError::Invalid("Built-in styles can't be deleted.".into());
        assert_eq!(e.code(), "invalid");
        assert_eq!(e.user_message(), "Built-in styles can't be deleted.");
        assert_eq!(StoreError::NotFound("Style".into()).code(), "not_found");
        assert_eq!(StoreError::Keychain("k".into()).user_message(), "k");
    }
}
