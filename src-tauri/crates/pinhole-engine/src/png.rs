//! PNG chunk surgery without re-encoding (SPEC §4).
//!
//! [`scrub`] keeps only an allow-list of chunks needed to display the image
//! (IHDR, PLTE, IDAT, IEND + colour/transparency/physical-size ancillaries) and
//! drops everything else — in particular `tEXt`/`zTXt`/`iTXt` (where
//! sd-server's `embed_image_metadata` puts the prompt), `eXIf`, `tIME` and any
//! private chunk. Pixel data is copied byte-for-byte, so the image still decodes.
//!
//! [`add_text_chunk`] inserts one `tEXt` chunk (the optional "settings, no
//! prompt" metadata) before the first `IDAT`.

pub const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

/// Chunks kept by [`scrub`]. Everything else is dropped.
const KEEP: &[&[u8; 4]] = &[
    b"IHDR", b"PLTE", b"IDAT", b"IEND", b"tRNS", b"gAMA", b"cHRM", b"sRGB", b"iCCP", b"sBIT",
    b"bKGD", b"pHYs", b"cICP",
];

/// Maximum accepted chunk length (PNG spec: 2^31 - 1).
const MAX_CHUNK: u32 = 0x7fff_ffff;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PngError {
    #[error("not a PNG file")]
    NotPng,
    #[error("truncated or malformed PNG")]
    Malformed,
    #[error("text chunk keyword must be 1–79 Latin-1 characters")]
    BadKeyword,
}

/// One raw chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk<'a> {
    pub kind: [u8; 4],
    pub data: &'a [u8],
}

impl Chunk<'_> {
    pub fn kind_str(&self) -> String {
        String::from_utf8_lossy(&self.kind).into_owned()
    }
}

pub fn is_png(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && bytes[..8] == SIGNATURE
}

/// Split a PNG into chunks (CRC not verified: we only re-emit known-safe chunks
/// with freshly computed CRCs for anything we create).
pub fn chunks(bytes: &[u8]) -> Result<Vec<Chunk<'_>>, PngError> {
    if !is_png(bytes) {
        return Err(PngError::NotPng);
    }
    let mut out = Vec::new();
    let mut pos = 8usize;
    let mut seen_iend = false;
    while pos < bytes.len() {
        if pos + 8 > bytes.len() {
            return Err(PngError::Malformed);
        }
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap());
        if len > MAX_CHUNK {
            return Err(PngError::Malformed);
        }
        let kind: [u8; 4] = bytes[pos + 4..pos + 8].try_into().unwrap();
        if !kind.iter().all(|b| b.is_ascii_alphabetic()) {
            return Err(PngError::Malformed);
        }
        let data_start = pos + 8;
        let data_end = data_start
            .checked_add(len as usize)
            .ok_or(PngError::Malformed)?;
        let crc_end = data_end.checked_add(4).ok_or(PngError::Malformed)?;
        if crc_end > bytes.len() {
            return Err(PngError::Malformed);
        }
        out.push(Chunk {
            kind,
            data: &bytes[data_start..data_end],
        });
        pos = crc_end;
        if &kind == b"IEND" {
            seen_iend = true;
            break;
        }
    }
    if !seen_iend || out.first().map(|c| &c.kind) != Some(b"IHDR") {
        return Err(PngError::Malformed);
    }
    Ok(out)
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut h = crc32fast::Hasher::new();
    h.update(kind);
    h.update(data);
    out.extend_from_slice(&h.finalize().to_be_bytes());
}

/// Rebuild the PNG keeping only allow-listed chunks (drops all text/metadata).
pub fn scrub(bytes: &[u8]) -> Result<Vec<u8>, PngError> {
    let chunks = chunks(bytes)?;
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&SIGNATURE);
    for c in &chunks {
        if KEEP.iter().any(|k| **k == c.kind) {
            write_chunk(&mut out, &c.kind, c.data);
        }
    }
    Ok(out)
}

/// Insert one `tEXt` chunk (`keyword\0text`, Latin-1) before the first IDAT.
/// Non-Latin-1 characters in `text` are replaced by `?`.
pub fn add_text_chunk(bytes: &[u8], keyword: &str, text: &str) -> Result<Vec<u8>, PngError> {
    if keyword.is_empty()
        || keyword.chars().count() > 79
        || keyword.chars().any(|c| (c as u32) < 32 || (c as u32) > 255)
    {
        return Err(PngError::BadKeyword);
    }
    let mut data: Vec<u8> = keyword.chars().map(|c| c as u32 as u8).collect();
    data.push(0);
    data.extend(text.chars().map(|c| {
        if (c as u32) <= 255 && c != '\0' {
            c as u32 as u8
        } else {
            b'?'
        }
    }));
    insert_chunk(bytes, b"tEXt", &data)
}

/// Insert one uncompressed `iTXt` chunk (`keyword\0` + flags + empty language
/// and translated keyword + UTF-8 text) before the first IDAT. Used for XMP
/// (`XML:com.adobe.xmp`).
pub fn add_itxt_chunk(bytes: &[u8], keyword: &str, text: &str) -> Result<Vec<u8>, PngError> {
    if keyword.is_empty() || keyword.len() > 79 || !keyword.bytes().all(|b| (32..127).contains(&b))
    {
        return Err(PngError::BadKeyword);
    }
    let mut data = keyword.as_bytes().to_vec();
    // NUL, compression flag 0, method 0, empty language tag, empty translated keyword.
    data.extend_from_slice(&[0, 0, 0, 0, 0]);
    data.extend_from_slice(text.replace('\0', "").as_bytes());
    insert_chunk(bytes, b"iTXt", &data)
}

/// Insert an arbitrary chunk before the first IDAT (or IEND). Used by
/// [`add_text_chunk`] and by the test mocks to simulate engine metadata.
pub fn insert_chunk(bytes: &[u8], kind: &[u8; 4], data: &[u8]) -> Result<Vec<u8>, PngError> {
    let chunks = chunks(bytes)?;
    let mut out = Vec::with_capacity(bytes.len() + data.len() + 12);
    out.extend_from_slice(&SIGNATURE);
    let mut inserted = false;
    for c in &chunks {
        if !inserted && (&c.kind == b"IDAT" || &c.kind == b"IEND") {
            write_chunk(&mut out, kind, data);
            inserted = true;
        }
        write_chunk(&mut out, &c.kind, c.data);
    }
    Ok(out)
}

/// `(width, height)` from IHDR.
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if !is_png(bytes) || bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((w, h))
}

/// `(chunk type, payload)` of every text-like chunk (tests / privacy scans).
pub fn text_chunks(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    chunks(bytes)
        .map(|cs| {
            cs.into_iter()
                .filter(|c| matches!(&c.kind, b"tEXt" | b"zTXt" | b"iTXt" | b"eXIf" | b"tIME"))
                .map(|c| (c.kind_str(), c.data.to_vec()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tiny RGBA PNG with tEXt/iTXt/zTXt/tIME/private chunks, like sd-server's default output.
    fn dirty_png(prompt: &str) -> Vec<u8> {
        let clean = crate::image::encode_png_rgba(&[255, 0, 0, 255].repeat(4 * 3), 4, 3).unwrap();
        let cs = chunks(&clean).unwrap();
        let mut out = SIGNATURE.to_vec();
        for c in &cs {
            if &c.kind == b"IDAT" {
                write_chunk(
                    &mut out,
                    b"tEXt",
                    format!("parameters\0{prompt}").as_bytes(),
                );
                write_chunk(
                    &mut out,
                    b"iTXt",
                    format!("prompt\0\0\0\0\0{prompt}").as_bytes(),
                );
                write_chunk(
                    &mut out,
                    b"zTXt",
                    b"comment\0\0x\x9c\x03\x00\x00\x00\x00\x01",
                );
                write_chunk(&mut out, b"tIME", &[7, 234, 9, 28, 12, 0, 0]);
                write_chunk(&mut out, b"prVt", prompt.as_bytes());
            }
            write_chunk(&mut out, &c.kind, c.data);
        }
        // Trailing text chunk after IDAT too.
        let iend = out.len() - 12;
        let mut tail = Vec::new();
        write_chunk(&mut tail, b"tEXt", format!("late\0{prompt}").as_bytes());
        out.splice(iend..iend, tail);
        out
    }

    #[test]
    fn add_itxt_chunk_writes_an_uncompressed_itxt() {
        let clean = crate::image::encode_png_rgba(&[0, 0, 255, 255].repeat(4), 2, 2).unwrap();
        let with = add_itxt_chunk(&clean, "XML:com.adobe.xmp", "<x>é</x>").unwrap();
        let t = text_chunks(&with);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].0, "iTXt");
        let mut want = b"XML:com.adobe.xmp\0\0\0\0\0".to_vec();
        want.extend_from_slice("<x>é</x>".as_bytes());
        assert_eq!(t[0].1, want);
        assert!(crate::image::decode_rgba(&with).is_ok());
        assert_eq!(add_itxt_chunk(&clean, "", "x"), Err(PngError::BadKeyword));
    }

    #[test]
    fn scrub_removes_every_text_chunk_and_keeps_pixels() {
        let prompt = "PINHOLE_SENTINEL_7f3a";
        let dirty = dirty_png(prompt);
        assert!(dirty.windows(prompt.len()).any(|w| w == prompt.as_bytes()));
        assert_eq!(text_chunks(&dirty).len(), 5);

        let clean = scrub(&dirty).unwrap();
        assert!(
            !clean.windows(prompt.len()).any(|w| w == prompt.as_bytes()),
            "prompt bytes gone"
        );
        assert!(text_chunks(&clean).is_empty());
        let kinds: Vec<String> = chunks(&clean)
            .unwrap()
            .iter()
            .map(|c| c.kind_str())
            .collect();
        assert_eq!(kinds.first().map(String::as_str), Some("IHDR"));
        assert_eq!(kinds.last().map(String::as_str), Some("IEND"));
        assert!(!kinds.iter().any(|k| k == "prVt" || k == "tIME"));

        // Still a valid image with the same pixels.
        let (rgba, w, h) = crate::image::decode_rgba(&clean).unwrap();
        assert_eq!((w, h), (4, 3));
        assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
        assert_eq!(dimensions(&clean), Some((4, 3)));
    }

    #[test]
    fn add_text_chunk_inserts_before_idat_and_decodes() {
        let clean = crate::image::encode_png_rgba(&[0, 0, 255, 255].repeat(4), 2, 2).unwrap();
        let with = add_text_chunk(&clean, "pinhole", "{\"seed\":42}").unwrap();
        let t = text_chunks(&with);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].0, "tEXt");
        assert_eq!(t[0].1, b"pinhole\0{\"seed\":42}");
        let kinds: Vec<String> = chunks(&with)
            .unwrap()
            .iter()
            .map(|c| c.kind_str())
            .collect();
        let text_pos = kinds.iter().position(|k| k == "tEXt").unwrap();
        let idat_pos = kinds.iter().position(|k| k == "IDAT").unwrap();
        assert!(text_pos < idat_pos);
        assert!(crate::image::decode_rgba(&with).is_ok());
        assert_eq!(add_text_chunk(&clean, "", "x"), Err(PngError::BadKeyword));
    }

    #[test]
    fn rejects_garbage_and_truncation() {
        assert_eq!(scrub(b"hello"), Err(PngError::NotPng));
        let clean = crate::image::encode_png_rgba(&[0; 16], 2, 2).unwrap();
        assert_eq!(scrub(&clean[..clean.len() - 6]), Err(PngError::Malformed));
        let mut huge = clean.clone();
        huge[8..12].copy_from_slice(&0xffff_ffffu32.to_be_bytes());
        assert_eq!(scrub(&huge), Err(PngError::Malformed));
    }
}
