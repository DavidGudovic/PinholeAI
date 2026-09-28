//! Image helpers on in-memory buffers: format sniffing + header dimensions for
//! imports, RGBA decode (clipboard), PNG encode, 2×2 box downsample (2× upscale
//! = ESRGAN 4× then halve). Never touches disk.

use std::io::Cursor;

use image::{ExtendedColorType, ImageEncoder, ImageFormat, ImageReader, Limits};

/// Import limits (SPEC: reject > 50 MP or > 64 MB).
pub const MAX_IMPORT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_IMPORT_PIXELS: u64 = 50_000_000;
/// Hard cap for anything we decode (engine output included).
const MAX_DECODE_SIDE: u32 = 16_384;
const MAX_DECODE_ALLOC: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Png,
    Jpeg,
    Webp,
}

impl Kind {
    pub fn mime(self) -> &'static str {
        match self {
            Kind::Png => "image/png",
            Kind::Jpeg => "image/jpeg",
            Kind::Webp => "image/webp",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            Kind::Png => "png",
            Kind::Jpeg => "jpg",
            Kind::Webp => "webp",
        }
    }
    fn format(self) -> ImageFormat {
        match self {
            Kind::Png => ImageFormat::Png,
            Kind::Jpeg => ImageFormat::Jpeg,
            Kind::Webp => ImageFormat::WebP,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageInfo {
    pub kind: Kind,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ImageError {
    #[error("That file isn't a PNG, JPEG or WebP image.")]
    Unsupported,
    #[error("That image is too large (over 64 MB).")]
    TooManyBytes,
    #[error("That image is too large (over 50 megapixels).")]
    TooManyPixels,
    #[error("The image file is damaged: {0}")]
    Corrupt(String),
}

/// Format from magic bytes (only formats Pinhole accepts).
pub fn kind_of(bytes: &[u8]) -> Option<Kind> {
    if crate::png::is_png(bytes) {
        Some(Kind::Png)
    } else if bytes.len() >= 3 && bytes[..3] == [0xff, 0xd8, 0xff] {
        Some(Kind::Jpeg)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(Kind::Webp)
    } else {
        None
    }
}

fn limits() -> Limits {
    let mut l = Limits::default();
    l.max_image_width = Some(MAX_DECODE_SIDE);
    l.max_image_height = Some(MAX_DECODE_SIDE);
    l.max_alloc = Some(MAX_DECODE_ALLOC);
    l
}

/// Sniff format and read dimensions from the header only (no pixel decode).
/// Enforces the import limits.
pub fn sniff(bytes: &[u8]) -> Result<ImageInfo, ImageError> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(ImageError::TooManyBytes);
    }
    let kind = kind_of(bytes).ok_or(ImageError::Unsupported)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), kind.format());
    reader.limits(limits());
    let (width, height) = reader.into_dimensions().map_err(|e| ImageError::Corrupt(short(&e.to_string())))?;
    if width == 0 || height == 0 {
        return Err(ImageError::Corrupt("zero size".into()));
    }
    if u64::from(width) * u64::from(height) > MAX_IMPORT_PIXELS {
        return Err(ImageError::TooManyPixels);
    }
    Ok(ImageInfo { kind, width, height })
}

/// Decode any accepted format to RGBA8.
pub fn decode_rgba(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), ImageError> {
    let kind = kind_of(bytes).ok_or(ImageError::Unsupported)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), kind.format());
    reader.limits(limits());
    let img = reader.decode().map_err(|e| ImageError::Corrupt(short(&e.to_string())))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Ok((rgba.into_raw(), w, h))
}

/// Encode RGBA8 as PNG (no metadata chunks). Uses RGB when fully opaque.
pub fn encode_png_rgba(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, ImageError> {
    if rgba.len() as u64 != u64::from(width) * u64::from(height) * 4 {
        return Err(ImageError::Corrupt("pixel buffer size".into()));
    }
    let mut out = Vec::new();
    let enc = image::codecs::png::PngEncoder::new(&mut out);
    let opaque = rgba.chunks_exact(4).all(|p| p[3] == 255);
    let res = if opaque {
        let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        enc.write_image(&rgb, width, height, ExtendedColorType::Rgb8)
    } else {
        enc.write_image(rgba, width, height, ExtendedColorType::Rgba8)
    };
    res.map_err(|e| ImageError::Corrupt(short(&e.to_string())))?;
    Ok(out)
}

/// Halve both sides by averaging 2×2 blocks (odd trailing row/column dropped).
pub fn downscale_2x_box(rgba: &[u8], width: u32, height: u32) -> (Vec<u8>, u32, u32) {
    let (nw, nh) = ((width / 2).max(1), (height / 2).max(1));
    let (w, h) = (width as usize, height as usize);
    let mut out = vec![0u8; nw as usize * nh as usize * 4];
    for y in 0..nh as usize {
        for x in 0..nw as usize {
            for c in 0..4 {
                let px = |xx: usize, yy: usize| -> u32 {
                    let (xx, yy) = (xx.min(w - 1), yy.min(h - 1));
                    u32::from(rgba[(yy * w + xx) * 4 + c])
                };
                let sum = px(2 * x, 2 * y) + px(2 * x + 1, 2 * y) + px(2 * x, 2 * y + 1) + px(2 * x + 1, 2 * y + 1);
                out[(y * nw as usize + x) * 4 + c] = ((sum + 2) / 4) as u8;
            }
        }
    }
    (out, nw, nh)
}

fn short(s: &str) -> String {
    s.chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let rgb = vec![128u8; (w * h * 3) as usize];
        image::codecs::jpeg::JpegEncoder::new(&mut out).write_image(&rgb, w, h, ExtendedColorType::Rgb8).unwrap();
        out
    }

    #[test]
    fn sniffs_png_jpeg_and_rejects_others() {
        let png = encode_png_rgba(&[1, 2, 3, 255].repeat(6 * 5), 6, 5).unwrap();
        assert_eq!(sniff(&png).unwrap(), ImageInfo { kind: Kind::Png, width: 6, height: 5 });
        let j = jpeg(7, 3);
        assert_eq!(sniff(&j).unwrap(), ImageInfo { kind: Kind::Jpeg, width: 7, height: 3 });
        assert_eq!(sniff(b"GIF89a....").unwrap_err(), ImageError::Unsupported);
        assert!(matches!(sniff(&png[..20]), Err(ImageError::Corrupt(_))));
    }

    #[test]
    fn rejects_too_many_pixels_from_header_only() {
        // Forge an IHDR claiming 10000×10000 (100 MP) — rejected without decoding.
        let mut png = encode_png_rgba(&[0; 4], 1, 1).unwrap();
        png[16..20].copy_from_slice(&10_000u32.to_be_bytes());
        png[20..24].copy_from_slice(&10_000u32.to_be_bytes());
        // Fix the IHDR CRC so the header parses.
        let mut h = crc32fast::Hasher::new();
        h.update(&png[12..29]);
        let crc = h.finalize().to_be_bytes();
        png[29..33].copy_from_slice(&crc);
        assert_eq!(sniff(&png).unwrap_err(), ImageError::TooManyPixels);
        let big = vec![0u8; MAX_IMPORT_BYTES + 1];
        assert_eq!(sniff(&big).unwrap_err(), ImageError::TooManyBytes);
    }

    #[test]
    fn decode_and_downscale() {
        let mut rgba = Vec::new();
        for i in 0..16u8 {
            rgba.extend_from_slice(&[i * 10, 0, 0, 255]);
        }
        let png = encode_png_rgba(&rgba, 4, 4).unwrap();
        let (back, w, h) = decode_rgba(&png).unwrap();
        assert_eq!((w, h), (4, 4));
        assert_eq!(back, rgba);
        let (small, sw, sh) = downscale_2x_box(&back, w, h);
        assert_eq!((sw, sh), (2, 2));
        // top-left block: pixels 0,1,4,5 → (0+10+40+50)/4 = 25
        assert_eq!(small[0], 25);
        assert_eq!(small[3], 255);
        let (_, jw, jh) = decode_rgba(&jpeg(9, 4)).unwrap();
        assert_eq!((jw, jh), (9, 4));
    }
}
