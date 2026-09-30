//! "Fix details" (Edit, SPEC §5.2): redraw a small painted spot at the model's
//! native size, then blend it back. Crop a padded box around the mask, scale it
//! up, inpaint only that box (the caller runs the engine), scale the result back
//! down and paste it over the original with a feathered edge. All in memory.

use image::imageops::{self, FilterType};
use image::{GrayImage, ImageBuffer, Luma, Rgba, RgbaImage};

use crate::image::{decode_rgba, encode_png_rgba, ImageError};

/// Mask pixels brighter than this count as painted.
const PAINTED: u8 = 128;
/// Smallest box side (source pixels) so the model sees some surroundings.
const MIN_SIDE: u32 = 128;
/// The box is at most this much wider than tall (or taller than wide).
const MAX_ASPECT: f64 = 2.0;

/// Box in source pixels (`x`, `y` = top left).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Engine inputs for the crop plus what [`DetailPlan::blend`] needs afterwards.
pub struct DetailPlan {
    src: RgbaImage,
    pub crop: Rect,
    /// Blend weight per crop pixel (0 = keep the original, 255 = take the redraw).
    alpha: GrayImage,
    /// Size the engine draws at.
    pub work: (u32, u32),
    /// Crop scaled to `work` (PNG): the engine's `init_image`.
    pub init_png: Vec<u8>,
    /// Mask for the crop scaled to `work` (PNG, white = redraw): the engine's `mask_image`.
    pub mask_png: Vec<u8>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DetailError {
    #[error("nothing painted")]
    NothingPainted,
    #[error(transparent)]
    Image(#[from] ImageError),
}

impl DetailPlan {
    /// Plan a fix from the source image and its mask (any accepted format; the
    /// mask is scaled to the source when their sizes differ). `work_size` turns
    /// the crop's size into the size the engine should draw at.
    pub fn new(
        src: &[u8],
        mask: &[u8],
        work_size: impl FnOnce(u32, u32) -> (u32, u32),
    ) -> Result<Self, DetailError> {
        let (px, w, h) = decode_rgba(src)?;
        let src = RgbaImage::from_raw(w, h, px).ok_or(ImageError::Corrupt("pixels".into()))?;
        let (mpx, mw, mh) = decode_rgba(mask)?;
        let m = RgbaImage::from_raw(mw, mh, mpx).ok_or(ImageError::Corrupt("mask".into()))?;
        let mut mask = GrayImage::from_fn(mw, mh, |x, y| {
            let p = m.get_pixel(x, y).0;
            // White = change; a transparent pixel is unpainted.
            let lum = (u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])) / 3;
            Luma([if lum * u32::from(p[3]) / 255 >= u32::from(PAINTED) {
                255
            } else {
                0
            }])
        });
        if (mw, mh) != (w, h) {
            mask = imageops::resize(&mask, w, h, FilterType::Nearest);
        }
        let painted = painted_box(&mask).ok_or(DetailError::NothingPainted)?;
        let crop = crop_box(painted, w, h);
        let work = work_size(crop.w, crop.h);

        let src_crop = imageops::crop_imm(&src, crop.x, crop.y, crop.w, crop.h).to_image();
        let mask_crop = imageops::crop_imm(&mask, crop.x, crop.y, crop.w, crop.h).to_image();
        let init = imageops::resize(&src_crop, work.0, work.1, FilterType::Lanczos3);
        let work_mask = imageops::resize(&mask_crop, work.0, work.1, FilterType::Nearest);
        let mask_rgba = RgbaImage::from_fn(work.0, work.1, |x, y| {
            let v = work_mask.get_pixel(x, y).0[0];
            Rgba([v, v, v, 255])
        });

        // Feather: blur the painted area, then double it, so everything painted
        // is fully replaced and the redraw fades out just outside the stroke.
        let sigma = feather_sigma(painted);
        let blurred = imageops::fast_blur(&mask_crop, sigma);
        let alpha = GrayImage::from_fn(crop.w, crop.h, |x, y| {
            let m = u16::from(mask_crop.get_pixel(x, y).0[0]);
            let b = u16::from(blurred.get_pixel(x, y).0[0]);
            Luma([(b * 2).max(m).min(255) as u8])
        });

        Ok(Self {
            init_png: encode_png_rgba(init.as_raw(), work.0, work.1)?,
            mask_png: encode_png_rgba(mask_rgba.as_raw(), work.0, work.1)?,
            src,
            crop,
            alpha,
            work,
        })
    }

    /// Size of the finished image (the source's).
    pub fn size(&self) -> (u32, u32) {
        self.src.dimensions()
    }

    /// Scale the engine's redraw of the crop back down and paste it over the
    /// source with the feathered mask. Returns the whole image as PNG.
    pub fn blend(&self, redraw: &[u8]) -> Result<Vec<u8>, DetailError> {
        let (px, w, h) = decode_rgba(redraw)?;
        let redraw: RgbaImage =
            ImageBuffer::from_raw(w, h, px).ok_or(ImageError::Corrupt("pixels".into()))?;
        let back = imageops::resize(&redraw, self.crop.w, self.crop.h, FilterType::Lanczos3);
        let mut out = self.src.clone();
        for (x, y, a) in self.alpha.enumerate_pixels() {
            let a = u32::from(a.0[0]);
            if a == 0 {
                continue;
            }
            let o = out.get_pixel_mut(self.crop.x + x, self.crop.y + y);
            let r = back.get_pixel(x, y).0;
            // Colour only: alpha stays the source's.
            for (dst, src) in o.0.iter_mut().zip(r).take(3) {
                *dst = ((u32::from(src) * a + u32::from(*dst) * (255 - a) + 127) / 255) as u8;
            }
        }
        let (w, h) = out.dimensions();
        Ok(encode_png_rgba(out.as_raw(), w, h)?)
    }
}

/// Bounding box of the painted pixels.
fn painted_box(mask: &GrayImage) -> Option<Rect> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, p) in mask.enumerate_pixels() {
        if p.0[0] >= PAINTED {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    (x0 != u32::MAX).then(|| Rect {
        x: x0,
        y: y0,
        w: x1 - x0 + 1,
        h: y1 - y0 + 1,
    })
}

/// Edge softness (source pixels) for a painted box.
fn feather_sigma(painted: Rect) -> f32 {
    (painted.w.max(painted.h) as f32 / 40.0).clamp(2.0, 12.0)
}

/// The painted box plus room around it (a quarter of its longer side, at least
/// 32 px, and at least [`MIN_SIDE`] overall), not more than 2:1, inside the image.
fn crop_box(painted: Rect, img_w: u32, img_h: u32) -> Rect {
    let long = painted.w.max(painted.h) as f64;
    let pad = (long * 0.25).max(32.0);
    let mut w = (painted.w as f64 + 2.0 * pad).max(f64::from(MIN_SIDE));
    let mut h = (painted.h as f64 + 2.0 * pad).max(f64::from(MIN_SIDE));
    w = w.max(h / MAX_ASPECT);
    h = h.max(w / MAX_ASPECT);
    let w = (w.round() as u32).min(img_w);
    let h = (h.round() as u32).min(img_h);
    // Centre on the painted box, then shift back inside the image.
    let place = |start: u32, len: u32, size: u32, full: u32| -> u32 {
        let centre = f64::from(start) + f64::from(len) / 2.0;
        let x = (centre - f64::from(size) / 2.0).round().max(0.0) as u32;
        x.min(full - size)
    };
    Rect {
        x: place(painted.x, painted.w, w, img_w),
        y: place(painted.y, painted.h, h, img_h),
        w,
        h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let img = RgbaImage::from_fn(w, h, |x, y| Rgba(f(x, y)));
        encode_png_rgba(img.as_raw(), w, h).unwrap()
    }

    fn mask_png(w: u32, h: u32, r: Rect) -> Vec<u8> {
        png(w, h, |x, y| {
            let on = x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h;
            if on {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 255]
            }
        })
    }

    #[test]
    fn crop_box_pads_centres_and_stays_inside() {
        let r = crop_box(
            Rect {
                x: 400,
                y: 300,
                w: 80,
                h: 100,
            },
            1000,
            800,
        );
        // pad = max(25, 32) = 32 → 144×164, centred on (440, 350).
        assert_eq!(
            r,
            Rect {
                x: 368,
                y: 268,
                w: 144,
                h: 164
            }
        );
        // Near the corner: shifted back inside.
        let r = crop_box(
            Rect {
                x: 0,
                y: 0,
                w: 20,
                h: 20,
            },
            1000,
            800,
        );
        assert_eq!((r.x, r.y, r.w, r.h), (0, 0, 128, 128));
        // Thin stroke: not more than 2:1.
        let r = crop_box(
            Rect {
                x: 100,
                y: 400,
                w: 600,
                h: 4,
            },
            1000,
            800,
        );
        assert_eq!(r.w, 900);
        assert_eq!(r.h, 450);
        // Bigger than the image: the whole image.
        let r = crop_box(
            Rect {
                x: 0,
                y: 0,
                w: 300,
                h: 200,
            },
            300,
            200,
        );
        assert_eq!(
            r,
            Rect {
                x: 0,
                y: 0,
                w: 300,
                h: 200
            }
        );
    }

    #[test]
    fn plan_scales_the_crop_up_and_blend_only_changes_the_painted_spot() {
        let (w, h) = (640, 480);
        let src = png(w, h, |x, _| [(x % 256) as u8, 40, 90, 255]);
        let spot = Rect {
            x: 300,
            y: 200,
            w: 40,
            h: 40,
        };
        let mut asked = None;
        let plan = DetailPlan::new(&src, &mask_png(w, h, spot), |cw, ch| {
            asked = Some((cw, ch));
            (cw * 4, ch * 4)
        })
        .unwrap();
        let crop = plan.crop;
        assert_eq!(asked, Some((crop.w, crop.h)));
        assert_eq!(
            crop,
            Rect {
                x: 256,
                y: 156,
                w: 128,
                h: 128
            }
        );
        assert_eq!(plan.work, (512, 512));
        assert_eq!(crate::image::sniff(&plan.init_png).unwrap().width, 512);
        let (m, mw, _) = decode_rgba(&plan.mask_png).unwrap();
        // Painted spot at (44..84) in the crop → (176..336) in the work mask.
        let at = |x: u32, y: u32| m[((y * mw + x) * 4) as usize];
        assert_eq!(at(256, 256), 255);
        assert_eq!(at(20, 20), 0);

        // The engine "redraws" the crop in pure red.
        let red = png(512, 512, |_, _| [255, 0, 0, 255]);
        let out = plan.blend(&red).unwrap();
        let (px, ow, oh) = decode_rgba(&out).unwrap();
        assert_eq!((ow, oh), (w, h));
        let get = |x: u32, y: u32| &px[((y * w + x) * 4) as usize..][..4];
        assert_eq!(get(320, 220), &[255, 0, 0, 255], "painted: replaced");
        assert_eq!(
            get(10, 10),
            &[10, 40, 90, 255],
            "outside the crop: untouched"
        );
        assert_eq!(get(260, 160), &[4, 40, 90, 255], "crop edge: untouched");
        let edge = get(342, 220);
        assert!(
            edge[0] > 90 && edge[0] < 255,
            "feathered just outside: {edge:?}"
        );
    }

    #[test]
    fn empty_mask_is_nothing_painted() {
        let src = png(64, 64, |_, _| [1, 2, 3, 255]);
        let mask = png(64, 64, |_, _| [0, 0, 0, 255]);
        assert_eq!(
            DetailPlan::new(&src, &mask, |w, h| (w, h)).err(),
            Some(DetailError::NothingPainted)
        );
    }
}
