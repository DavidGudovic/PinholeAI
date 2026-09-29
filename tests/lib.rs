//! Shared helpers for workspace integration tests (see tests/tests/*.rs):
//! repo paths, a connection-counting TCP listener, an independent PNG chunk
//! reader (incl. zTXt/iTXt decompression) and a recursive sentinel scanner.
//!
//! These helpers deliberately do not reuse Pinhole's own PNG / file code: the
//! privacy tests must not trust the code they are checking.

use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub mod smoke;

/// Repository root (the workspace).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tests/ has a parent")
        .to_path_buf()
}

/// Shipped `config/` folder (models.yaml, engine.yaml, styles/, presets/).
pub fn config_dir() -> PathBuf {
    repo_root().join("config")
}

/// A unique suffix so sentinels never collide with leftovers of other runs/tests.
pub fn unique_suffix() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

// ---------------------------------------------------------------- connection counter

/// A TCP listener on 127.0.0.1 that counts accepted connections. If something
/// does connect it answers `200 {}` so the client fails an assertion instead of
/// hanging.
pub struct CountingListener {
    addr: SocketAddr,
    accepted: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl CountingListener {
    pub async fn start() -> Self {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind 127.0.0.1:0");
        let addr = listener.local_addr().expect("local addr");
        let accepted = Arc::new(AtomicUsize::new(0));
        let counter = accepted.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        sock.read(&mut buf),
                    )
                    .await;
                    let _ = sock
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                        .await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Self {
            addr,
            accepted,
            task,
        }
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// `http://127.0.0.1:<port><path>`
    pub fn http_url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.addr.port(), path)
    }

    /// `https://127.0.0.1:<port><path>`
    pub fn https_url(&self, path: &str) -> String {
        format!("https://127.0.0.1:{}{}", self.addr.port(), path)
    }

    pub fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }
}

impl Drop for CountingListener {
    fn drop(&mut self) {
        self.task.abort();
    }
}

// ---------------------------------------------------------------- PNG

pub const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

#[derive(Debug, Clone)]
pub struct PngChunk {
    pub kind: String,
    pub data: Vec<u8>,
}

/// Parse PNG chunks (no CRC check needed for scanning). `None` if not a PNG.
pub fn png_chunks(bytes: &[u8]) -> Option<Vec<PngChunk>> {
    if bytes.len() < 8 || bytes[..8] != PNG_SIGNATURE {
        return None;
    }
    let mut out = Vec::new();
    let mut i = 8usize;
    while i + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[i..i + 4].try_into().ok()?) as usize;
        let kind = String::from_utf8_lossy(&bytes[i + 4..i + 8]).to_string();
        let start = i + 8;
        let end = start.checked_add(len)?;
        if end + 4 > bytes.len() {
            break;
        }
        out.push(PngChunk {
            kind: kind.clone(),
            data: bytes[start..end].to_vec(),
        });
        i = end + 4;
        if kind == "IEND" {
            break;
        }
    }
    Some(out)
}

/// `(width, height)` from IHDR.
pub fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let chunks = png_chunks(bytes)?;
    let ihdr = chunks.iter().find(|c| c.kind == "IHDR")?;
    if ihdr.data.len() < 8 {
        return None;
    }
    Some((
        u32::from_be_bytes(ihdr.data[0..4].try_into().ok()?),
        u32::from_be_bytes(ihdr.data[4..8].try_into().ok()?),
    ))
}

fn inflate(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let _ = flate2::read::ZlibDecoder::new(data)
        .take(64 * 1024 * 1024)
        .read_to_end(&mut out);
    out
}

/// Every text chunk (tEXt / zTXt / iTXt) as `(chunk type, keyword, decoded text bytes)`.
/// zTXt and compressed iTXt are decompressed.
pub fn png_text_chunks(bytes: &[u8]) -> Vec<(String, String, Vec<u8>)> {
    let mut out = Vec::new();
    for c in png_chunks(bytes).unwrap_or_default() {
        let nul = c.data.iter().position(|&b| b == 0);
        match (c.kind.as_str(), nul) {
            ("tEXt", Some(n)) => out.push((
                c.kind.clone(),
                latin1(&c.data[..n]),
                c.data[n + 1..].to_vec(),
            )),
            ("zTXt", Some(n)) if c.data.len() > n + 2 => out.push((
                c.kind.clone(),
                latin1(&c.data[..n]),
                inflate(&c.data[n + 2..]),
            )),
            ("iTXt", Some(n)) if c.data.len() > n + 3 => {
                let compressed = c.data[n + 1] == 1;
                // skip language tag and translated keyword (two NUL-terminated fields)
                let rest = &c.data[n + 3..];
                let mut pos = 0;
                for _ in 0..2 {
                    match rest[pos..].iter().position(|&b| b == 0) {
                        Some(z) => pos += z + 1,
                        None => {
                            pos = rest.len();
                            break;
                        }
                    }
                }
                let text = &rest[pos.min(rest.len())..];
                out.push((
                    c.kind.clone(),
                    latin1(&c.data[..n]),
                    if compressed {
                        inflate(text)
                    } else {
                        text.to_vec()
                    },
                ));
            }
            ("tEXt" | "zTXt" | "iTXt", _) => {
                out.push((c.kind.clone(), String::new(), c.data.clone()))
            }
            _ => {}
        }
    }
    out
}

fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

// ---------------------------------------------------------------- sentinel scan

/// A place where a needle was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub needle: String,
    pub path: PathBuf,
    /// `file name`, `contents`, `contents (UTF-16LE)`, `PNG zTXt …`
    pub location: String,
}

/// Files larger than this are only checked by name (engine binaries, model weights).
pub const MAX_SCAN_BYTES: u64 = 512 * 1024 * 1024;

/// Recursively search `root` (file and folder names, raw bytes as UTF-8 and
/// UTF-16LE, and decompressed PNG text chunks) for every needle. Symlinks are
/// not followed. Files that can't be read are reported as hits with
/// location `unreadable` so a test can't pass by accident.
pub fn scan_for(root: &Path, needles: &[&str]) -> Vec<Hit> {
    let mut hits = Vec::new();
    scan_path(root, needles, &mut hits);
    hits
}

fn scan_path(path: &Path, needles: &[&str], hits: &mut Vec<Hit>) {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return,
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    for n in needles {
        if name.contains(n) {
            hits.push(Hit {
                needle: n.to_string(),
                path: path.to_path_buf(),
                location: "file name".into(),
            });
        }
    }
    if meta.file_type().is_symlink() {
        return;
    }
    if meta.is_dir() {
        if let Ok(rd) = std::fs::read_dir(path) {
            let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
            entries.sort();
            for e in entries {
                scan_path(&e, needles, hits);
            }
        }
        return;
    }
    if !meta.is_file() || meta.len() > MAX_SCAN_BYTES {
        return;
    }
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            // Windows may hold transient locks on temp files owned by other processes,
            // and other processes may delete their temp files while we scan.
            if !matches!(
                e.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound
            ) {
                hits.push(Hit {
                    needle: String::new(),
                    path: path.to_path_buf(),
                    location: format!("unreadable: {e}"),
                });
            }
            return;
        }
    };
    scan_bytes(path, &bytes, needles, hits);
}

/// Search one file's bytes (and PNG text chunks) for the needles.
pub fn scan_bytes(path: &Path, bytes: &[u8], needles: &[&str], hits: &mut Vec<Hit>) {
    for n in needles {
        if contains(bytes, n.as_bytes()) {
            hits.push(Hit {
                needle: n.to_string(),
                path: path.to_path_buf(),
                location: "contents".into(),
            });
        }
        let utf16: Vec<u8> = n.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        if contains(bytes, &utf16) {
            hits.push(Hit {
                needle: n.to_string(),
                path: path.to_path_buf(),
                location: "contents (UTF-16LE)".into(),
            });
        }
    }
    for (kind, keyword, text) in png_text_chunks(bytes) {
        for n in needles {
            if contains(&text, n.as_bytes()) || keyword.contains(n) {
                hits.push(Hit {
                    needle: n.to_string(),
                    path: path.to_path_buf(),
                    location: format!("PNG {kind} chunk `{keyword}`"),
                });
            }
        }
    }
}

pub fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && hay.len() >= needle.len()
        && hay.windows(needle.len()).any(|w| w == needle)
}

/// Entries of the OS temp dir whose name contains `pinhole` (case-insensitive).
pub fn pinhole_temp_entries() -> Vec<PathBuf> {
    let tmp = std::env::temp_dir();
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&tmp) {
        for e in rd.flatten() {
            if e.file_name()
                .to_string_lossy()
                .to_lowercase()
                .contains("pinhole")
            {
                out.push(e.path());
            }
        }
    }
    out.sort();
    out
}

/// Pretty list for assertion messages.
pub fn describe_hits(hits: &[Hit]) -> String {
    hits.iter()
        .map(|h| {
            format!(
                "  - `{}` in {} ({})",
                h.needle,
                h.path.display(),
                h.location
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_with(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut out = PNG_SIGNATURE.to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&7u32.to_be_bytes());
        ihdr.extend_from_slice(&5u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut all: Vec<(&[u8; 4], Vec<u8>)> = vec![(b"IHDR", ihdr)];
        all.extend(chunks.iter().cloned());
        all.push((b"IEND", vec![]));
        for (k, d) in all {
            out.extend_from_slice(&(d.len() as u32).to_be_bytes());
            out.extend_from_slice(k);
            out.extend_from_slice(&d);
            out.extend_from_slice(&[0, 0, 0, 0]); // CRC not checked by the scanner
        }
        out
    }

    fn zlib(data: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn finds_sentinels_in_compressed_png_chunks() {
        let mut ztxt = b"parameters\0\0".to_vec();
        ztxt.extend(zlib(b"a photo of SECRET_ZTXT"));
        let mut itxt = b"Description\0\x01\0en\0\0".to_vec();
        itxt.extend(zlib(b"SECRET_ITXT"));
        let png = png_with(&[
            (b"tEXt", b"prompt\0SECRET_TEXT".to_vec()),
            (b"zTXt", ztxt),
            (b"iTXt", itxt),
        ]);
        assert_eq!(png_dimensions(&png), Some((7, 5)));
        let mut hits = Vec::new();
        scan_bytes(
            Path::new("x.png"),
            &png,
            &["SECRET_TEXT", "SECRET_ZTXT", "SECRET_ITXT", "ABSENT"],
            &mut hits,
        );
        let found: Vec<_> = hits
            .iter()
            .map(|h| (h.needle.as_str(), h.location.as_str()))
            .collect();
        assert!(found.contains(&("SECRET_TEXT", "contents")), "{found:?}");
        assert!(
            found
                .iter()
                .any(|(n, l)| *n == "SECRET_ZTXT" && l.starts_with("PNG zTXt")),
            "{found:?}"
        );
        assert!(
            found
                .iter()
                .any(|(n, l)| *n == "SECRET_ITXT" && l.starts_with("PNG iTXt")),
            "{found:?}"
        );
        // compressed text is invisible to a raw byte search — the chunk decoder must catch it
        assert!(!found.contains(&("SECRET_ZTXT", "contents")));
        assert!(!hits.iter().any(|h| h.needle == "ABSENT"));
    }

    #[test]
    fn finds_names_and_utf16_and_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        std::fs::write(dir.path().join("a/b/NAME_SECRET.txt"), b"x").unwrap();
        let utf16: Vec<u8> = "WIDE_SECRET"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        std::fs::write(dir.path().join("a/w.bin"), utf16).unwrap();
        let hits = scan_for(dir.path(), &["NAME_SECRET", "WIDE_SECRET"]);
        assert!(
            hits.iter()
                .any(|h| h.needle == "NAME_SECRET" && h.location == "file name"),
            "{hits:?}"
        );
        assert!(
            hits.iter()
                .any(|h| h.needle == "WIDE_SECRET" && h.location == "contents (UTF-16LE)"),
            "{hits:?}"
        );
    }
}
