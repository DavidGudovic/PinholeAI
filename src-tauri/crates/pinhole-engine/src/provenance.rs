//! AI-origin labels that other tools put in a picture (RELEASE-SPEC §2, §11 "never strip other
//! tools' AI markers"). Importing a picture drops all of its metadata (location, camera, prompts);
//! this reads, before that, the one fact worth keeping: whether the file says it was made with
//! AI. Only that label is kept, in memory, and written back as the IPTC digital source type when
//! the picture or anything made from it is saved. Nothing is signed: a carried label says what
//! the file claimed, not that the claim was verified.
//!
//! Read from the IPTC digital source type, wherever a file carries it as text: XMP (PNG `iTXt`,
//! compressed or not; JPEG APP1; WebP `XMP `) and C2PA manifests (PNG `caBX`, JPEG APP11 JUMBF,
//! WebP `C2PA`), whose actions name the same IPTC codes.

use std::io::Read;

/// What an imported file says about how it was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiLabel {
    /// Made entirely with AI (`trainedAlgorithmicMedia`, `algorithmicMedia`).
    Generated,
    /// A capture combined with AI-made parts (`compositeWithTrainedAlgorithmicMedia`).
    Composite,
    /// A capture enhanced with AI (`algorithmicallyEnhanced`).
    Enhanced,
}

impl AiLabel {
    fn rank(self) -> u8 {
        match self {
            AiLabel::Enhanced => 1,
            AiLabel::Composite => 2,
            AiLabel::Generated => 3,
        }
    }
}

const PREFIX: &[u8] = b"digitalsourcetype/";
/// Codes after [`PREFIX`]. Order matters only for readability: matching checks the whole code
/// up to a delimiter, so `trainedAlgorithmicMedia` never matches inside the composite code.
const CODES: &[(&[u8], AiLabel)] = &[
    (b"trainedAlgorithmicMedia", AiLabel::Generated),
    (b"algorithmicMedia", AiLabel::Generated),
    (b"compositeWithTrainedAlgorithmicMedia", AiLabel::Composite),
    (b"algorithmicallyEnhanced", AiLabel::Enhanced),
];
/// Most text inflated from one file's compressed PNG chunks, all chunks together, and the most
/// compressed chunks looked at (XMP is a few KB; this bounds a file built to inflate forever).
const INFLATE_BUDGET: usize = 16 * 1024 * 1024;
const MAX_COMPRESSED_CHUNKS: usize = 16;

/// The AI label `bytes` (a PNG, JPEG or WebP file) carries, if any. The strongest one wins when a
/// file names several (made with AI over composite over enhanced).
pub fn ai_label(bytes: &[u8]) -> Option<AiLabel> {
    let mut found = scan(bytes);
    if found != Some(AiLabel::Generated) && crate::png::is_png(bytes) {
        let mut budget = INFLATE_BUDGET;
        let compressed = crate::png::chunks(bytes)
            .unwrap_or_default()
            .into_iter()
            .filter(|c| is_compressed_text(&c.kind, c.data))
            .take(MAX_COMPRESSED_CHUNKS);
        for c in compressed {
            if budget == 0 || found == Some(AiLabel::Generated) {
                break;
            }
            if let Some(text) = inflated_text(&c.kind, c.data, budget) {
                budget -= text.len();
                found = strongest(found, scan(&text));
            }
        }
    }
    found
}

/// The stronger of two labels.
pub fn strongest(a: Option<AiLabel>, b: Option<AiLabel>) -> Option<AiLabel> {
    match (a, b) {
        (Some(x), Some(y)) => Some(if y.rank() > x.rank() { y } else { x }),
        (a, b) => a.or(b),
    }
}

fn is_compressed_text(kind: &[u8; 4], data: &[u8]) -> bool {
    match kind {
        b"zTXt" => true,
        b"iTXt" => data
            .iter()
            .position(|&b| b == 0)
            .is_some_and(|n| data.get(n + 1) == Some(&1)),
        _ => false,
    }
}

/// Every IPTC AI code in `bytes`, as plain text.
fn scan(bytes: &[u8]) -> Option<AiLabel> {
    let mut found = None;
    let mut rest = bytes;
    while let Some(i) = find(rest, PREFIX) {
        rest = &rest[i + PREFIX.len()..];
        let code_len = rest
            .iter()
            .position(|b| !b.is_ascii_alphanumeric())
            .unwrap_or(rest.len());
        let code = &rest[..code_len];
        if let Some((_, label)) = CODES.iter().find(|(c, _)| *c == code) {
            found = strongest(found, Some(*label));
        }
    }
    found
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Text of a compressed PNG `zTXt` or `iTXt` chunk (uncompressed ones are already plain bytes),
/// at most `limit` bytes of it.
fn inflated_text(kind: &[u8; 4], data: &[u8], limit: usize) -> Option<Vec<u8>> {
    let nul = data.iter().position(|&b| b == 0)?;
    let compressed = match kind {
        b"zTXt" => data.get(nul + 2..)?,
        b"iTXt" => {
            if data.get(nul + 1) != Some(&1) {
                return None;
            }
            let after = data.get(nul + 3..)?;
            // Language tag and translated keyword, each NUL-terminated.
            let l = after.iter().position(|&b| b == 0)?;
            let t = after[l + 1..].iter().position(|&b| b == 0)?;
            after.get(l + 1 + t + 1..)?
        }
        _ => return None,
    };
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(compressed)
        .take(limit as u64)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const GEN: &str = "http://cv.iptc.org/newscodes/digitalsourcetype/trainedAlgorithmicMedia";
    const COMP: &str =
        "http://cv.iptc.org/newscodes/digitalsourcetype/compositeWithTrainedAlgorithmicMedia";

    fn png() -> Vec<u8> {
        crate::image::encode_png_rgba(&[10, 20, 30, 255].repeat(4), 2, 2).unwrap()
    }
    fn xmp(code: &str) -> String {
        format!(
            r#"<x:xmpmeta><rdf:Description Iptc4xmpExt:DigitalSourceType="{code}"/></x:xmpmeta>"#
        )
    }
    fn zlib(text: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(text).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn xmp_in_a_png() {
        let p = crate::png::add_itxt_chunk(&png(), "XML:com.adobe.xmp", &xmp(GEN)).unwrap();
        assert_eq!(ai_label(&p), Some(AiLabel::Generated));
        let p = crate::png::add_itxt_chunk(&png(), "XML:com.adobe.xmp", &xmp(COMP)).unwrap();
        assert_eq!(ai_label(&p), Some(AiLabel::Composite));
    }

    #[test]
    fn compressed_xmp_in_a_png() {
        let mut itxt = b"XML:com.adobe.xmp\0\x01\x00\0\0".to_vec();
        itxt.extend(zlib(xmp(GEN).as_bytes()));
        let p = crate::png::insert_chunk(&png(), b"iTXt", &itxt).unwrap();
        assert_eq!(scan(&p), None, "only readable inflated");
        assert_eq!(ai_label(&p), Some(AiLabel::Generated));
        let mut ztxt = b"Raw profile\0\x00".to_vec();
        ztxt.extend(zlib(xmp(COMP).as_bytes()));
        let p = crate::png::insert_chunk(&png(), b"zTXt", &ztxt).unwrap();
        assert_eq!(ai_label(&p), Some(AiLabel::Composite));
    }

    #[test]
    fn c2pa_manifest_actions() {
        // A C2PA manifest store holds CBOR; its actions carry the IPTC code as a text string.
        let mut cbor = b"jumb\0\0\0\x20jumdc2pa\0c2pa.actions\x78\x48".to_vec();
        cbor.extend_from_slice(GEN.as_bytes());
        cbor.push(0xa1);
        let p = crate::png::insert_chunk(&png(), b"caBX", &cbor).unwrap();
        assert_eq!(ai_label(&p), Some(AiLabel::Generated));
        // A camera's signed capture (C2PA, no AI code) is not labelled.
        let p = crate::png::insert_chunk(&png(), b"caBX", b"jumb\0jumdc2pa\0c2pa.actions").unwrap();
        assert_eq!(ai_label(&p), None);
    }

    #[test]
    fn jpeg_and_webp_bytes_are_scanned_as_they_are() {
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1, 0x01, 0x00];
        jpeg.extend_from_slice(b"http://ns.adobe.com/xap/1.0/\0");
        jpeg.extend_from_slice(xmp(GEN).as_bytes());
        jpeg.extend_from_slice(&[0xff, 0xd9]);
        assert_eq!(ai_label(&jpeg), Some(AiLabel::Generated));
        let mut webp = b"RIFF\0\0\0\0WEBPXMP \0\0\0\0".to_vec();
        webp.extend_from_slice(xmp(COMP).as_bytes());
        assert_eq!(ai_label(&webp), Some(AiLabel::Composite));
    }

    #[test]
    fn other_source_types_and_plain_pictures_have_no_label() {
        assert_eq!(ai_label(&png()), None);
        for code in [
            "digitalCapture",
            "compositeSynthetic",
            "trainedAlgorithmicMediaX",
        ] {
            let x = xmp(&format!(
                "http://cv.iptc.org/newscodes/digitalsourcetype/{code}"
            ));
            let p = crate::png::add_itxt_chunk(&png(), "XML:com.adobe.xmp", &x).unwrap();
            assert_eq!(ai_label(&p), None, "{code}");
        }
        // A truncated code at the end of the file.
        assert_eq!(ai_label(b"digitalsourcetype/trainedAlgorithmic"), None);
    }

    #[test]
    fn the_strongest_label_wins() {
        let both = format!("{} {}", xmp(COMP), xmp(GEN));
        assert_eq!(ai_label(both.as_bytes()), Some(AiLabel::Generated));
        let enhanced =
            xmp("http://cv.iptc.org/newscodes/digitalsourcetype/algorithmicallyEnhanced");
        assert_eq!(ai_label(enhanced.as_bytes()), Some(AiLabel::Enhanced));
        let both = format!("{enhanced} {}", xmp(COMP));
        assert_eq!(ai_label(both.as_bytes()), Some(AiLabel::Composite));
        assert_eq!(
            strongest(Some(AiLabel::Composite), Some(AiLabel::Enhanced)),
            Some(AiLabel::Composite)
        );
        assert_eq!(
            strongest(None, Some(AiLabel::Enhanced)),
            Some(AiLabel::Enhanced)
        );
    }

    #[test]
    fn compressed_text_is_inflated_within_one_budget() {
        // Many chunks that each inflate to a lot of zeros: reading stops at the budget, and
        // a label after it isn't looked for.
        let bomb = zlib(&vec![0u8; 4 * 1024 * 1024]);
        let mut p = png();
        for _ in 0..MAX_COMPRESSED_CHUNKS {
            let mut z = b"k\0\x00".to_vec();
            z.extend_from_slice(&bomb);
            p = crate::png::insert_chunk(&p, b"zTXt", &z).unwrap();
        }
        let mut late = b"Raw profile\0\x00".to_vec();
        late.extend(zlib(xmp(GEN).as_bytes()));
        let p = crate::png::insert_chunk(&p, b"zTXt", &late).unwrap();
        let start = std::time::Instant::now();
        assert_eq!(ai_label(&p), None);
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
        // One chunk is cut at the limit.
        let one = crate::png::chunks(&p)
            .unwrap()
            .into_iter()
            .find(|c| &c.kind == b"zTXt")
            .unwrap();
        assert_eq!(
            inflated_text(&one.kind, one.data, 1000).unwrap().len(),
            1000
        );
    }

    #[test]
    fn a_damaged_compressed_chunk_is_ignored() {
        let p = crate::png::insert_chunk(&png(), b"zTXt", b"k\0\x00not zlib").unwrap();
        assert_eq!(ai_label(&p), None);
    }
}
