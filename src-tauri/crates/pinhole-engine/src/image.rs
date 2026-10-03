//! Image helpers on in-memory buffers: format sniffing + header dimensions for
//! imports, RGBA decode (clipboard), PNG encode, 2×2 box downsample (2× upscale
//! = ESRGAN 4× then halve). Never touches disk.

use std::io::Cursor;

use image::metadata::Orientation;
use image::{
    DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Limits,
};

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
    let (width, height) = reader
        .into_dimensions()
        .map_err(|e| ImageError::Corrupt(short(&e.to_string())))?;
    if width == 0 || height == 0 {
        return Err(ImageError::Corrupt("zero size".into()));
    }
    if u64::from(width) * u64::from(height) > MAX_IMPORT_PIXELS {
        return Err(ImageError::TooManyPixels);
    }
    Ok(ImageInfo {
        kind,
        width,
        height,
    })
}

/// Decode any accepted format to RGBA8.
pub fn decode_rgba(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), ImageError> {
    let kind = kind_of(bytes).ok_or(ImageError::Unsupported)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), kind.format());
    reader.limits(limits());
    let img = reader
        .decode()
        .map_err(|e| ImageError::Corrupt(short(&e.to_string())))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Ok((rgba.into_raw(), w, h))
}

/// Decode an accepted image (PNG/JPEG/WebP) and re-encode it as a plain PNG:
/// pixels only, with the EXIF orientation applied first, so EXIF, XMP, ICC and
/// comment metadata are all dropped. Returns the PNG and its (oriented) size.
pub fn reencode_png(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), ImageError> {
    let kind = kind_of(bytes).ok_or(ImageError::Unsupported)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), kind.format());
    reader.limits(limits());
    let corrupt = |e: image::ImageError| ImageError::Corrupt(short(&e.to_string()));
    let mut decoder = reader.into_decoder().map_err(corrupt)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(corrupt)?;
    img.apply_orientation(orientation);
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let png = encode_png_rgba(rgba.as_raw(), w, h)?;
    Ok((png, w, h))
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
        let rgb: Vec<u8> = rgba
            .chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        enc.write_image(&rgb, width, height, ExtendedColorType::Rgb8)
    } else {
        enc.write_image(rgba, width, height, ExtendedColorType::Rgba8)
    };
    res.map_err(|e| ImageError::Corrupt(short(&e.to_string())))?;
    Ok(out)
}

/// Test helper: a `w`×`h` grey JPEG with an APP1 EXIF segment (Orientation 6 =
/// rotate 90° clockwise, plus `text` as ImageDescription) — like a phone photo
/// carrying GPS / camera details.
#[cfg(any(test, feature = "test-util"))]
pub fn jpeg_with_exif(w: u32, h: u32, text: &str) -> Vec<u8> {
    let mut plain = Vec::new();
    let rgb = vec![128u8; (w * h * 3) as usize];
    image::codecs::jpeg::JpegEncoder::new(&mut plain)
        .write_image(&rgb, w, h, ExtendedColorType::Rgb8)
        .expect("jpeg");
    // TIFF (little endian): IFD0 with ImageDescription (0x010e) and Orientation (0x0112).
    let text = format!("{text}\0");
    let mut tiff = b"II*\0".to_vec();
    tiff.extend_from_slice(&8u32.to_le_bytes());
    tiff.extend_from_slice(&2u16.to_le_bytes());
    let text_off: u32 = 8 + 2 + 2 * 12 + 4;
    tiff.extend_from_slice(&0x010eu16.to_le_bytes());
    tiff.extend_from_slice(&2u16.to_le_bytes());
    tiff.extend_from_slice(&(text.len() as u32).to_le_bytes());
    tiff.extend_from_slice(&text_off.to_le_bytes());
    tiff.extend_from_slice(&0x0112u16.to_le_bytes());
    tiff.extend_from_slice(&3u16.to_le_bytes());
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&[6, 0, 0, 0]);
    tiff.extend_from_slice(&0u32.to_le_bytes());
    tiff.extend_from_slice(text.as_bytes());
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend_from_slice(&tiff);
    let mut out = plain[..2].to_vec();
    out.extend_from_slice(&[0xff, 0xe1]);
    out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&app1);
    out.extend_from_slice(&plain[2..]);
    out
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
                let sum = px(2 * x, 2 * y)
                    + px(2 * x + 1, 2 * y)
                    + px(2 * x, 2 * y + 1)
                    + px(2 * x + 1, 2 * y + 1);
                out[(y * nw as usize + x) * 4 + c] = ((sum + 2) / 4) as u8;
            }
        }
    }
    (out, nw, nh)
}

/// Largest side of a sheet ("Save as one sheet"); bigger sheets are scaled down to fit.
pub const SHEET_MAX_SIDE: u32 = 8192;

/// Columns for a sheet of `n` pictures: one row up to 3, then rows of 2, 3 or 4.
pub fn sheet_columns(n: usize) -> usize {
    match n {
        0..=3 => n.max(1),
        4 => 2,
        5 | 6 => 3,
        _ => 4,
    }
}

/// Pictures (RGBA8, row-major) side by side in one picture: a grid of equal cells the size of the
/// largest picture, each picture fitted and centred in its cell, small white gaps between them
/// and around the edge, no text. Scaled down so neither side is over [`SHEET_MAX_SIDE`].
pub fn sheet(tiles: &[(Vec<u8>, u32, u32)]) -> (Vec<u8>, u32, u32) {
    use image::imageops::{self, FilterType};
    let n = tiles.len().max(1);
    let cols = sheet_columns(n) as u32;
    let rows = n.div_ceil(cols as usize) as u32;
    let cell_w = tiles.iter().map(|t| t.1).max().unwrap_or(1).max(1);
    let cell_h = tiles.iter().map(|t| t.2).max().unwrap_or(1).max(1);
    let gap = (cell_w.min(cell_h) / 50).max(8);
    let full_w = cols * cell_w + (cols + 1) * gap;
    let full_h = rows * cell_h + (rows + 1) * gap;
    let scale = (SHEET_MAX_SIDE as f64 / full_w.max(full_h) as f64).min(1.0);
    let px = |v: u32| ((v as f64 * scale).round() as u32).max(1);
    let (cw, ch, g) = (px(cell_w), px(cell_h), px(gap));
    let (w, h) = (cols * cw + (cols + 1) * g, rows * ch + (rows + 1) * g);
    let mut out = image::RgbaImage::from_pixel(w, h, image::Rgba([255, 255, 255, 255]));
    for (i, (rgba, tw, th)) in tiles.iter().enumerate() {
        let Some(tile) = image::RgbaImage::from_raw(*tw, *th, rgba.clone()) else {
            continue;
        };
        // Fit inside the cell, keeping the shape.
        let fit = (cw as f64 / *tw as f64).min(ch as f64 / *th as f64);
        let (fw, fh) = (
            ((*tw as f64 * fit).round() as u32).clamp(1, cw),
            ((*th as f64 * fit).round() as u32).clamp(1, ch),
        );
        let tile = if (fw, fh) == (*tw, *th) {
            tile
        } else {
            imageops::resize(&tile, fw, fh, FilterType::Lanczos3)
        };
        let (c, r) = (i as u32 % cols, i as u32 / cols);
        let x = g + c * (cw + g) + (cw - fw) / 2;
        let y = g + r * (ch + g) + (ch - fh) / 2;
        imageops::overlay(&mut out, &tile, i64::from(x), i64::from(y));
    }
    (out.into_raw(), w, h)
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
        image::codecs::jpeg::JpegEncoder::new(&mut out)
            .write_image(&rgb, w, h, ExtendedColorType::Rgb8)
            .unwrap();
        out
    }

    #[test]
    fn sniffs_png_jpeg_and_rejects_others() {
        let png = encode_png_rgba(&[1, 2, 3, 255].repeat(6 * 5), 6, 5).unwrap();
        assert_eq!(
            sniff(&png).unwrap(),
            ImageInfo {
                kind: Kind::Png,
                width: 6,
                height: 5
            }
        );
        let j = jpeg(7, 3);
        assert_eq!(
            sniff(&j).unwrap(),
            ImageInfo {
                kind: Kind::Jpeg,
                width: 7,
                height: 3
            }
        );
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
    fn reencode_drops_exif_and_applies_orientation() {
        let j = jpeg_with_exif(8, 4, "GPS 52.52N PINHOLE_EXIF_SECRET");
        assert!(j.windows(19).any(|w| w == b"PINHOLE_EXIF_SECRET"));
        assert_eq!(
            sniff(&j).unwrap(),
            ImageInfo {
                kind: Kind::Jpeg,
                width: 8,
                height: 4
            }
        );
        let (png, w, h) = reencode_png(&j).unwrap();
        assert_eq!((w, h), (4, 8), "rotated by the EXIF orientation");
        assert_eq!(kind_of(&png), Some(Kind::Png));
        assert!(!png.windows(19).any(|w| w == b"PINHOLE_EXIF_SECRET"));
        let kinds: Vec<String> = crate::png::chunks(&png)
            .unwrap()
            .iter()
            .map(|c| c.kind_str())
            .collect();
        assert!(
            kinds
                .iter()
                .all(|k| ["IHDR", "IDAT", "IEND"].contains(&k.as_str())),
            "{kinds:?}"
        );
        assert_eq!(
            sniff(&png).unwrap(),
            ImageInfo {
                kind: Kind::Png,
                width: 4,
                height: 8
            }
        );
        assert_eq!(reencode_png(b"nope").unwrap_err(), ImageError::Unsupported);
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

    #[test]
    fn sheet_puts_pictures_in_a_grid_with_gaps() {
        let red = ([200u8, 0, 0, 255].repeat(100 * 60), 100, 60);
        let tiles = [red.clone(), red.clone(), red.clone(), red];
        let (px, w, h) = sheet(&tiles);
        // 2×2 cells of 100×60 with 8 px gaps.
        assert_eq!((w, h), (2 * 100 + 3 * 8, 2 * 60 + 3 * 8));
        let at = |x: u32, y: u32| &px[((y * w + x) * 4) as usize..((y * w + x) * 4 + 4) as usize];
        assert_eq!(at(0, 0), [255, 255, 255, 255]);
        assert_eq!(at(8, 8), [200, 0, 0, 255]);
        assert_eq!(at(8 + 100, 8), [255, 255, 255, 255]);
        assert_eq!(at(w - 9, h - 9), [200, 0, 0, 255]);
    }

    #[test]
    fn sheet_columns_by_count() {
        let cols: Vec<usize> = (2..=8).map(sheet_columns).collect();
        assert_eq!(cols, [2, 3, 2, 3, 3, 4, 4]);
    }

    #[test]
    fn sheet_fits_other_shapes_and_stays_under_the_limit() {
        let wide = ([0u8, 0, 200, 255].repeat(200 * 100), 200, 100);
        let tall = ([0u8, 200, 0, 255].repeat(100 * 200), 100, 200);
        let (_, w, h) = sheet(&[wide.clone(), tall]);
        // Cells take the largest width and height.
        assert_eq!((w, h), (2 * 200 + 3 * 8, 200 + 2 * 8));
        let big = (vec![9u8; 4000 * 3000 * 4], 4000, 3000);
        let (px, w, h) = sheet(&[big.clone(), big.clone(), big]);
        assert!(w <= SHEET_MAX_SIDE && h <= SHEET_MAX_SIDE);
        assert_eq!(px.len() as u64, u64::from(w) * u64::from(h) * 4);
    }
}
