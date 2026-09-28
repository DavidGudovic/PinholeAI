//! Local file helpers for "Add a file I already have" and installs: extension
//! checks, safe file names, copy-while-hashing, and hash matching for pasted
//! CivitAI resources (AutoV2 = first 10 hex chars of the SHA-256).

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Only these can be added from disk (never pickle formats).
pub const LOCAL_EXTENSIONS: &[&str] = &["safetensors", "gguf"];

/// Lowercase extension if it's one Pinhole accepts.
pub fn allowed_extension(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    LOCAL_EXTENSIONS.iter().copied().find(|e| *e == ext)
}

/// File-system-safe file name (keeps letters, digits, `._-()+ `), no path
/// parts, no leading dots, bounded length. Keeps the (validated) extension.
pub fn sanitize_file_name(name: &str, ext: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = match base.rsplit_once('.') {
        Some((s, e)) if e.eq_ignore_ascii_case(ext) => s,
        _ => base,
    };
    let mut clean: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "._-()+ ".contains(c) { c } else { '_' })
        .collect::<String>()
        .trim_matches(|c: char| c == '.' || c == ' ')
        .to_string();
    if clean.is_empty() {
        clean = "model".into();
    }
    if clean.len() > 150 {
        clean.truncate(150);
    }
    // Windows reserved device names.
    let upper = clean.to_ascii_uppercase();
    if ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
        || ((upper.starts_with("COM") || upper.starts_with("LPT")) && upper.len() == 4 && upper.as_bytes()[3].is_ascii_digit())
    {
        clean.insert(0, '_');
    }
    format!("{clean}.{ext}")
}

/// `dir/name`, or `dir/stem-2.ext`, `-3`… while `taken(path)` is true.
pub fn unique_path(dir: &Path, file_name: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let first = dir.join(file_name);
    if !taken(&first) {
        return first;
    }
    let (stem, ext) = file_name.rsplit_once('.').unwrap_or((file_name, ""));
    for i in 2..10_000 {
        let candidate = if ext.is_empty() { dir.join(format!("{stem}-{i}")) } else { dir.join(format!("{stem}-{i}.{ext}")) };
        if !taken(&candidate) {
            return candidate;
        }
    }
    first
}

const CHUNK: usize = 8 * 1024 * 1024;

/// Streaming SHA-256 + size of a file. Blocking.
pub fn hash_file(path: &Path) -> io::Result<(String, u64)> {
    let mut f = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((hex::encode(hasher.finalize()), total))
}

/// Copy `src` to `dest` through `dest.part` while hashing (one read pass),
/// then rename. The source is never modified. Blocking.
pub fn copy_and_hash(src: &Path, dest: &Path) -> io::Result<(String, u64)> {
    let part = part_path(dest);
    let result = (|| {
        let mut input = File::open(src)?;
        let mut out = File::create(&part)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; CHUNK];
        let mut total = 0u64;
        loop {
            let n = input.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n])?;
            total += n as u64;
        }
        out.sync_all()?;
        drop(out);
        std::fs::rename(&part, dest)?;
        Ok((hex::encode(hasher.finalize()), total))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

pub fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// Free bytes on the file system holding `dir` (walks up to an existing parent).
pub fn free_space(dir: &Path) -> u64 {
    let mut p = dir;
    loop {
        if p.exists() {
            return fs2::available_space(p).unwrap_or(0);
        }
        match p.parent() {
            Some(parent) => p = parent,
            None => return 0,
        }
    }
}

/// AutoV2 hash (CivitAI / A1111 "Model hash"): first 10 hex chars of SHA-256.
pub fn autov2(sha256: &str) -> String {
    sha256.trim().chars().take(10).collect::<String>().to_ascii_uppercase()
}

/// Does a pasted hash identify the file with this full SHA-256?
/// Accepts the full SHA-256 (64 hex) or AutoV2 (10 hex), case-insensitive.
/// Other lengths (AutoV1, AutoV3/`SHA256_12`-style LoRA hashes) can't be
/// matched locally — look them up on CivitAI instead.
pub fn hash_matches(sha256: &str, pasted: &str) -> bool {
    let p = pasted.trim();
    let s = sha256.trim();
    if !p.bytes().all(|b| b.is_ascii_hexdigit()) || s.len() != 64 {
        return false;
    }
    match p.len() {
        64 => s.eq_ignore_ascii_case(p),
        10 => s[..10].eq_ignore_ascii_case(p),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "6a35a7855770ae9820a3c931d4964c3817b6d9e3c6f9c4dabb5b3a94e5643b80";

    #[test]
    fn autov2_matching() {
        assert_eq!(autov2(SHA), "6A35A78557");
        assert!(hash_matches(SHA, "6A35A78557"));
        assert!(hash_matches(SHA, "6a35a78557"));
        assert!(hash_matches(SHA, &SHA.to_ascii_uppercase()));
        assert!(!hash_matches(SHA, "6A35A78558"));
        assert!(!hash_matches(SHA, "6A35A7855770"), "12-char LoRA hashes aren't file-hash prefixes");
        assert!(!hash_matches(SHA, "zz35a78557"));
        assert!(!hash_matches("short", "6A35A78557"));
        assert!(!hash_matches(SHA, ""));
    }

    #[test]
    fn extensions() {
        assert_eq!(allowed_extension(Path::new("/x/Model.SafeTensors")), Some("safetensors"));
        assert_eq!(allowed_extension(Path::new("m.gguf")), Some("gguf"));
        assert_eq!(allowed_extension(Path::new("m.ckpt")), None);
        assert_eq!(allowed_extension(Path::new("m.pt")), None);
        assert_eq!(allowed_extension(Path::new("safetensors")), None);
    }

    #[test]
    fn file_names() {
        assert_eq!(sanitize_file_name("../../etc/passwd.safetensors", "safetensors"), "passwd.safetensors");
        assert_eq!(sanitize_file_name("C:\\Users\\me\\My Model (v2).SAFETENSORS", "safetensors"), "My Model (v2).safetensors");
        assert_eq!(sanitize_file_name("..hidden", "gguf"), "hidden.gguf");
        assert_eq!(sanitize_file_name("a:b*c?.gguf", "gguf"), "a_b_c_.gguf");
        assert_eq!(sanitize_file_name("", "gguf"), "model.gguf");
        assert_eq!(sanitize_file_name("CON", "gguf"), "_CON.gguf");
        assert_eq!(sanitize_file_name("✨ JANKU ✨.safetensors", "safetensors"), "_ JANKU _.safetensors");
    }

    #[test]
    fn unique_paths() {
        let dir = Path::new("/d");
        let taken = |p: &Path| p == Path::new("/d/m.gguf") || p == Path::new("/d/m-2.gguf");
        assert_eq!(unique_path(dir, "m.gguf", taken), PathBuf::from("/d/m-3.gguf"));
        assert_eq!(unique_path(dir, "n.gguf", taken), PathBuf::from("/d/n.gguf"));
    }

    #[test]
    fn copy_hashes_and_keeps_source() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.safetensors");
        std::fs::write(&src, b"hello world").unwrap();
        let dest = dir.path().join("out").join("dst.safetensors");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let (sha, size) = copy_and_hash(&src, &dest).unwrap();
        assert_eq!(sha, "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9");
        assert_eq!(size, 11);
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello world");
        assert!(src.exists(), "the user's file is never moved");
        assert!(!part_path(&dest).exists());
        assert_eq!(hash_file(&dest).unwrap(), (sha, 11));
        assert!(free_space(&dir.path().join("does/not/exist")) > 0);
    }
}
