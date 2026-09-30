//! Invisible "made with AI" watermark (RELEASE-SPEC §2, the second marking layer next to the
//! XMP marker). It says one thing only, "made with AI": a fixed pattern that is the same for
//! every copy of Pinhole, with no ID for the install, user, machine or picture (David,
//! 2026-09-30: no app name either).
//!
//! How: the picture's brightness is shrunk to a 256×256 grid, and a fixed ±1 pattern is added
//! to five low/mid-frequency DCT coefficients of every 8×8 block of that grid (spread
//! spectrum). The change is made smooth by scaling it back up to the picture's size, so it
//! lives in coarse, low-contrast ripples of 1–3 levels (PSNR ~52 dB) that survive JPEG, resizing (the grid
//! is relative to the picture, not to pixels) and screenshots. Crops and heavy filters remove
//! it; that's accepted (the law asks for marking, not for an unremovable mark).
//!
//! [`detect`] correlates the same coefficients with the pattern: unmarked pictures score
//! about N(0, 1); marked ones score far above [`DETECT_THRESHOLD`].

/// Side of the working grid.
const GRID: usize = 256;
const BLOCK: usize = 8;
/// DCT coefficients (row frequency, column frequency) that carry the pattern.
const COEFFS: [(usize, usize); 5] = [(1, 2), (2, 1), (2, 2), (1, 3), (3, 1)];
/// Seed of the fixed pattern. The same for everyone: the watermark carries no ID.
const KEY: u64 = 0x4d41_4445_5749_5448; // "MADEWITH"
/// Pattern strength in DCT units before masking.
const STRENGTH: f32 = 4.0;
/// Pictures smaller than this on either side aren't marked (and can't be detected).
pub const MIN_SIDE: u32 = 64;
/// Score above which a picture counts as marked (unmarked pictures: ~N(0, 1)).
pub const DETECT_THRESHOLD: f32 = 6.0;

/// Row-major `GRID`×`GRID` values. (image's resize clamps float pixels to 0–1, so the
/// resampling here is our own: an area average down, bilinear up.)
type Grid = Vec<f32>;

/// The fixed ±1 pattern, one value per carried coefficient (xorshift64 from [`KEY`]).
fn pattern() -> Vec<f32> {
    let n = (GRID / BLOCK) * (GRID / BLOCK) * COEFFS.len();
    let mut s = KEY;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            if s >> 63 == 1 {
                1.0
            } else {
                -1.0
            }
        })
        .collect()
}

/// Orthonormal 8-point DCT-II basis: `basis[k][x]`.
fn basis() -> [[f32; BLOCK]; BLOCK] {
    let mut b = [[0.0f32; BLOCK]; BLOCK];
    for (k, row) in b.iter_mut().enumerate() {
        let a = if k == 0 {
            (1.0 / BLOCK as f32).sqrt()
        } else {
            (2.0 / BLOCK as f32).sqrt()
        };
        for (x, v) in row.iter_mut().enumerate() {
            *v = a
                * (std::f32::consts::PI * (2 * x + 1) as f32 * k as f32 / (2 * BLOCK) as f32).cos();
        }
    }
    b
}

/// Brightness (0–255) of the picture, shrunk to the grid by area averaging.
fn luma_grid(rgba: &[u8], width: u32, height: u32) -> Grid {
    let (w, h) = (width as usize, height as usize);
    let mut sum = vec![0.0f32; GRID * GRID];
    let mut count = vec![0u32; GRID * GRID];
    for y in 0..h {
        let gy = y * GRID / h;
        for x in 0..w {
            let gx = x * GRID / w;
            let p = &rgba[(y * w + x) * 4..][..3];
            sum[gy * GRID + gx] += 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
            count[gy * GRID + gx] += 1;
        }
    }
    sum.iter()
        .zip(&count)
        .map(|(s, c)| if *c > 0 { s / *c as f32 } else { 0.0 })
        .collect()
}

/// Bilinear sample of the grid at picture pixel (x, y) of a `width`×`height` picture.
fn upsample(grid: &[f32], width: usize, height: usize, x: usize, y: usize) -> f32 {
    let gx = ((x as f32 + 0.5) * GRID as f32 / width as f32 - 0.5).clamp(0.0, (GRID - 1) as f32);
    let gy = ((y as f32 + 0.5) * GRID as f32 / height as f32 - 0.5).clamp(0.0, (GRID - 1) as f32);
    let (x0, y0) = (gx.floor() as usize, gy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(GRID - 1), (y0 + 1).min(GRID - 1));
    let (fx, fy) = (gx - x0 as f32, gy - y0 as f32);
    let at = |x: usize, y: usize| grid[y * GRID + x];
    (at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx) * (1.0 - fy)
        + (at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx) * fy
}

fn usable(rgba: &[u8], width: u32, height: u32) -> bool {
    width >= MIN_SIDE
        && height >= MIN_SIDE
        && rgba.len() as u64 == u64::from(width) * u64::from(height) * 4
}

/// Add the watermark to RGBA8 pixels in place. Pictures under [`MIN_SIDE`] are left alone.
pub fn embed(rgba: &mut [u8], width: u32, height: u32) {
    if !usable(rgba, width, height) {
        return;
    }
    let grid = luma_grid(rgba, width, height);
    let b = basis();
    let pat = pattern();
    let blocks = GRID / BLOCK;
    let mut residual = vec![0.0f32; GRID * GRID];
    for by in 0..blocks {
        for bx in 0..blocks {
            // Busier blocks hide a stronger change; flat ones (sky, walls) get a weaker one.
            let (mut sum, mut sq) = (0.0f32, 0.0f32);
            for y in 0..BLOCK {
                for x in 0..BLOCK {
                    let v = grid[(by * BLOCK + y) * GRID + bx * BLOCK + x];
                    sum += v;
                    sq += v * v;
                }
            }
            let n = (BLOCK * BLOCK) as f32;
            let sd = (sq / n - (sum / n).powi(2)).max(0.0).sqrt();
            let amp = STRENGTH * (sd / 8.0).clamp(0.6, 1.6);
            for (i, &(u, v)) in COEFFS.iter().enumerate() {
                let d = amp * pat[(by * blocks + bx) * COEFFS.len() + i];
                for y in 0..BLOCK {
                    for x in 0..BLOCK {
                        residual[(by * BLOCK + y) * GRID + bx * BLOCK + x] += d * b[u][y] * b[v][x];
                    }
                }
            }
        }
    }
    let (w, h) = (width as usize, height as usize);
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let r = upsample(&residual, w, h, i % w, i / w);
        for c in &mut px[..3] {
            *c = (*c as f32 + r).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// How strongly RGBA8 pixels carry the watermark: about N(0, 1) for unmarked pictures,
/// above [`DETECT_THRESHOLD`] for marked ones. 0 for pictures under [`MIN_SIDE`].
pub fn detect(rgba: &[u8], width: u32, height: u32) -> f32 {
    if !usable(rgba, width, height) {
        return 0.0;
    }
    let grid = luma_grid(rgba, width, height);
    let b = basis();
    let pat = pattern();
    let blocks = GRID / BLOCK;
    let (mut dot, mut energy) = (0.0f64, 0.0f64);
    for by in 0..blocks {
        for bx in 0..blocks {
            for (i, &(u, v)) in COEFFS.iter().enumerate() {
                let mut c = 0.0f32;
                for y in 0..BLOCK {
                    for x in 0..BLOCK {
                        c += b[u][y] * b[v][x] * grid[(by * BLOCK + y) * GRID + bx * BLOCK + x];
                    }
                }
                dot += f64::from(c * pat[(by * blocks + bx) * COEFFS.len() + i]);
                energy += f64::from(c * c);
            }
        }
    }
    if energy <= f64::EPSILON {
        return 0.0;
    }
    (dot / energy.sqrt()) as f32
}

/// Whether RGBA8 pixels carry the watermark.
pub fn is_marked(rgba: &[u8], width: u32, height: u32) -> bool {
    detect(rgba, width, height) > DETECT_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A photo-like test picture: gradients, a few shapes and fine noise (seeded).
    fn picture(w: u32, h: u32, seed: u64) -> Vec<u8> {
        let mut s = seed | 1;
        let mut rnd = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s % 1000) as f32 / 1000.0
        };
        let (cx, cy, r) = (rnd() * w as f32, rnd() * h as f32, 0.2 + rnd() * 0.3);
        let tint = [rnd(), rnd(), rnd()];
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
                let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt() / w as f32;
                let disc = if d < r { 60.0 } else { 0.0 };
                let wave = 25.0 * (fx * 23.0 + fy * 7.0).sin() * (fy * 11.0).cos();
                for t in tint {
                    let v = 40.0 + 120.0 * fy * t + 60.0 * fx + disc + wave + 12.0 * (rnd() - 0.5);
                    out.push(v.clamp(0.0, 255.0) as u8);
                }
                out.push(255);
            }
        }
        out
    }

    fn jpeg(rgba: &[u8], w: u32, h: u32, quality: u8) -> Vec<u8> {
        let rgb: Vec<u8> = rgba
            .chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
            .encode(&rgb, w, h, image::ExtendedColorType::Rgb8)
            .unwrap();
        let back = image::load_from_memory(&out).unwrap().to_rgba8();
        back.into_raw()
    }

    fn half(rgba: &[u8], w: u32, h: u32) -> (Vec<u8>, u32, u32) {
        let img = image::RgbaImage::from_raw(w, h, rgba.to_vec()).unwrap();
        let small =
            image::imageops::resize(&img, w / 2, h / 2, image::imageops::FilterType::Lanczos3);
        (small.into_raw(), w / 2, h / 2)
    }

    fn psnr(a: &[u8], b: &[u8]) -> f64 {
        let mse = a
            .iter()
            .zip(b)
            .map(|(x, y)| (f64::from(*x) - f64::from(*y)).powi(2))
            .sum::<f64>()
            / a.len() as f64;
        10.0 * (255.0f64.powi(2) / mse.max(1e-9)).log10()
    }

    #[test]
    fn marked_pictures_are_detected_after_jpeg_and_downscale() {
        for (seed, (w, h)) in [(1u64, (768u32, 512u32)), (2, (512, 512)), (3, (640, 960))] {
            let clean = picture(w, h, seed);
            let mut marked = clean.clone();
            embed(&mut marked, w, h);
            assert!(
                psnr(&clean, &marked) > 40.0,
                "invisible: {}",
                psnr(&clean, &marked)
            );
            assert!(is_marked(&marked, w, h), "{}", detect(&marked, w, h));
            // JPEG quality 85, then half size (RELEASE-SPEC §2 test).
            let j = jpeg(&marked, w, h, 85);
            assert!(is_marked(&j, w, h), "jpeg: {}", detect(&j, w, h));
            let (s, sw, sh) = half(&j, w, h);
            assert!(is_marked(&s, sw, sh), "jpeg + half: {}", detect(&s, sw, sh));
            // Unmarked stays unmarked, after the same steps too.
            assert!(!is_marked(&clean, w, h), "{}", detect(&clean, w, h));
            let (s, sw, sh) = half(&jpeg(&clean, w, h, 85), w, h);
            assert!(!is_marked(&s, sw, sh), "{}", detect(&s, sw, sh));
        }
    }

    #[test]
    fn unmarked_pictures_score_low() {
        for seed in 10..40u64 {
            let p = picture(320, 240, seed);
            assert!(
                detect(&p, 320, 240).abs() < 4.0,
                "seed {seed}: {}",
                detect(&p, 320, 240)
            );
        }
    }

    #[test]
    fn flat_and_tiny_pictures() {
        // A flat grey picture still carries it (the weakest masking) and stays invisible.
        let clean = [128u8, 128, 128, 255].repeat(512 * 512);
        let mut marked = clean.clone();
        embed(&mut marked, 512, 512);
        assert!(is_marked(&marked, 512, 512));
        let max = clean
            .iter()
            .zip(&marked)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(max <= 4, "max change {max}");
        // Too small: untouched, and never "detected".
        let tiny = [10u8, 20, 30, 255].repeat(32 * 32);
        let mut t = tiny.clone();
        embed(&mut t, 32, 32);
        assert_eq!(t, tiny);
        assert_eq!(detect(&t, 32, 32), 0.0);
        // Alpha is never touched.
        assert!(marked.chunks_exact(4).all(|p| p[3] == 255));
    }
}
