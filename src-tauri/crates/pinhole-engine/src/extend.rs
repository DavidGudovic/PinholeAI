//! "Extend" (Edit, SPEC §5.2): make a picture wider or taller and let the model
//! draw the new edges. Place the source on a bigger canvas, fill the new space
//! with a blurred copy of the nearest edge (a colour hint), mask the new space
//! plus a thin seam over the old edge, and scale both to the size the engine
//! draws at. Afterwards the redraw is scaled back to the canvas size and the
//! source's own pixels are pasted over it, fading into the redraw across the seam.
//! All in memory.

use image::imageops::{self, FilterType};
use image::{GrayImage, ImageBuffer, Luma, Rgba, RgbaImage};

use crate::image::{decode_rgba, encode_png_rgba, ImageError};

/// Longest canvas side (source pixels).
pub const MAX_SIDE: u32 = 8192;

/// Where the source goes on the new canvas (source pixels; `left`/`top` = the
/// source's top-left corner on the canvas).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub left: u32,
    pub top: u32,
}

/// Engine inputs for the canvas plus what [`ExtendPlan::blend`] needs afterwards.
pub struct ExtendPlan {
    src: RgbaImage,
    pub canvas: Canvas,
    /// Size the engine draws at.
    pub work: (u32, u32),
    /// Canvas scaled to `work` (PNG): the engine's `init_image`.
    pub init_png: Vec<u8>,
    /// White = draw (new space + seam), scaled to `work` (PNG): the engine's `mask_image`.
    pub mask_png: Vec<u8>,
    /// Seam width per side in source pixels (left, top, right, bottom); 0 where nothing was added.
    seam: [u32; 4],
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExtendError {
    /// The canvas doesn't hold the source, or adds nothing.
    #[error("nothing to add")]
    NothingToAdd,
    #[error("canvas too big")]
    TooBig,
    #[error(transparent)]
    Image(#[from] ImageError),
}

impl ExtendPlan {
    /// Plan an extension of `src` (any accepted format) onto `canvas`.
    /// `work_size` turns the canvas size into the size the engine should draw at.
    pub fn new(
        src: &[u8],
        canvas: Canvas,
        work_size: impl FnOnce(u32, u32) -> (u32, u32),
    ) -> Result<Self, ExtendError> {
        let (px, w, h) = decode_rgba(src)?;
        let src = RgbaImage::from_raw(w, h, px).ok_or(ImageError::Corrupt("pixels".into()))?;
        let Canvas {
            width: cw,
            height: ch,
            left,
            top,
        } = canvas;
        let fits = u64::from(left) + u64::from(w) <= u64::from(cw)
            && u64::from(top) + u64::from(h) <= u64::from(ch);
        if !fits || (cw, ch) == (w, h) {
            return Err(ExtendError::NothingToAdd);
        }
        if cw > MAX_SIDE || ch > MAX_SIDE {
            return Err(ExtendError::TooBig);
        }
        let work = work_size(cw, ch);
        let (ww, wh) = work;
        let sx = f64::from(ww) / f64::from(cw);
        let sy = f64::from(wh) / f64::from(ch);

        // The seam: a strip of the old picture next to each new edge is drawn
        // again so the new part joins up. About 1/24 of the work size (at least
        // 16 work pixels = two latent pixels), never more than a third of the source.
        let seam_work = (f64::from(ww.max(wh)) / 24.0).max(16.0);
        let seam_x = ((seam_work / sx).round() as u32).min(w / 3);
        let seam_y = ((seam_work / sy).round() as u32).min(h / 3);
        let right = cw - left - w;
        let bottom = ch - top - h;
        let seam = [
            if left > 0 { seam_x } else { 0 },
            if top > 0 { seam_y } else { 0 },
            if right > 0 { seam_x } else { 0 },
            if bottom > 0 { seam_y } else { 0 },
        ];

        // Init image at work size: the source where it sits, a blurred stretch of
        // its nearest edge pixels in the new space.
        let src_work = (
            ((f64::from(w) * sx).round() as u32).clamp(1, ww),
            ((f64::from(h) * sy).round() as u32).clamp(1, wh),
        );
        let (ox, oy) = (
            ((f64::from(left) * sx).round() as u32).min(ww - src_work.0),
            ((f64::from(top) * sy).round() as u32).min(wh - src_work.1),
        );
        let small = imageops::resize(&src, src_work.0, src_work.1, FilterType::Triangle);
        let stretched = RgbaImage::from_fn(ww, wh, |x, y| {
            let x = x.saturating_sub(ox).min(src_work.0 - 1);
            let y = y.saturating_sub(oy).min(src_work.1 - 1);
            let p = small.get_pixel(x, y).0;
            Rgba([p[0], p[1], p[2], 255])
        });
        let hint = imageops::fast_blur(&stretched, (ww.max(wh) as f32 / 32.0).max(4.0));
        let inside =
            |x: u32, y: u32| x >= ox && y >= oy && x < ox + src_work.0 && y < oy + src_work.1;
        let init = RgbaImage::from_fn(ww, wh, |x, y| {
            if inside(x, y) {
                *stretched.get_pixel(x, y)
            } else {
                *hint.get_pixel(x, y)
            }
        });

        // Mask: white outside the source rectangle shrunk by the seam.
        let inner = |v: u32, lo: u32, len: u32, s_lo: u32, s_hi: u32, scale: f64| {
            let s_lo = (f64::from(s_lo) * scale).round() as u32;
            let s_hi = (f64::from(s_hi) * scale).round() as u32;
            v >= lo + s_lo && v + s_hi < lo + len
        };
        let mask = RgbaImage::from_fn(ww, wh, |x, y| {
            let keep = inner(x, ox, src_work.0, seam[0], seam[2], sx)
                && inner(y, oy, src_work.1, seam[1], seam[3], sy);
            let v = if keep { 0 } else { 255 };
            Rgba([v, v, v, 255])
        });

        Ok(Self {
            init_png: encode_png_rgba(init.as_raw(), ww, wh)?,
            mask_png: encode_png_rgba(mask.as_raw(), ww, wh)?,
            src,
            canvas,
            work,
            seam,
        })
    }

    /// Size of the finished image (the canvas).
    pub fn size(&self) -> (u32, u32) {
        (self.canvas.width, self.canvas.height)
    }

    /// Scale the engine's redraw to the canvas and paste the source over it,
    /// fading into the redraw across the seam. Returns the whole image as PNG.
    pub fn blend(&self, redraw: &[u8]) -> Result<Vec<u8>, ExtendError> {
        let (px, w, h) = decode_rgba(redraw)?;
        let redraw: RgbaImage =
            ImageBuffer::from_raw(w, h, px).ok_or(ImageError::Corrupt("pixels".into()))?;
        let Canvas {
            width: cw,
            height: ch,
            left,
            top,
        } = self.canvas;
        let mut out = imageops::resize(&redraw, cw, ch, FilterType::Lanczos3);
        for p in out.pixels_mut() {
            p.0[3] = 255;
        }
        let (sw, sh) = self.src.dimensions();
        let alpha = seam_alpha(sw, sh, self.seam);
        for (x, y, s) in self.src.enumerate_pixels() {
            let a = u32::from(alpha.get_pixel(x, y).0[0]);
            let o = out.get_pixel_mut(left + x, top + y);
            if a == 255 {
                *o = *s;
                continue;
            }
            for (dst, src) in o.0.iter_mut().zip(s.0).take(3) {
                *dst = ((u32::from(src) * a + u32::from(*dst) * (255 - a) + 127) / 255) as u8;
            }
            o.0[3] = s.0[3];
        }
        Ok(encode_png_rgba(out.as_raw(), cw, ch)?)
    }
}

/// How much of the source to keep per source pixel: 255 inside, a smooth ramp
/// down to 0 at each edge that got new space next to it.
fn seam_alpha(w: u32, h: u32, seam: [u32; 4]) -> GrayImage {
    // 0 at the edge, 1 at `len` pixels in (smoothstep).
    let ramp = |d: u32, len: u32| -> f64 {
        if len == 0 {
            return 1.0;
        }
        let t = (f64::from(d) + 0.5) / f64::from(len);
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    GrayImage::from_fn(w, h, |x, y| {
        let a = ramp(x, seam[0])
            * ramp(y, seam[1])
            * ramp(w - 1 - x, seam[2])
            * ramp(h - 1 - y, seam[3]);
        Luma([(a * 255.0).round() as u8])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let img = RgbaImage::from_fn(w, h, |x, y| Rgba(f(x, y)));
        encode_png_rgba(img.as_raw(), w, h).unwrap()
    }

    #[test]
    fn wider_canvas_masks_the_new_sides_and_a_seam() {
        // 400×400 → 800×400, centred: 200 px added left and right.
        let src = png(400, 400, |_, _| [200, 50, 50, 255]);
        let canvas = Canvas {
            width: 800,
            height: 400,
            left: 200,
            top: 0,
        };
        let mut asked = None;
        let plan = ExtendPlan::new(&src, canvas, |w, h| {
            asked = Some((w, h));
            (w * 2, h * 2)
        })
        .unwrap();
        assert_eq!(asked, Some((800, 400)));
        assert_eq!(plan.work, (1600, 800));
        assert_eq!(plan.size(), (800, 400));
        // Seam: 1600/24 ≈ 67 work px = 33 source px, on the left and right only.
        assert_eq!(plan.seam, [33, 0, 33, 0]);

        let (m, mw, _) = decode_rgba(&plan.mask_png).unwrap();
        let at = |x: u32, y: u32| m[((y * mw + x) * 4) as usize];
        assert_eq!(at(10, 400), 255, "new left side");
        assert_eq!(at(1590, 400), 255, "new right side");
        assert_eq!(at(420, 400), 255, "seam over the old left edge");
        assert_eq!(at(800, 400), 0, "middle of the picture is kept");
        assert_eq!(at(800, 2), 0, "no seam at the top (nothing added there)");

        // The new space gets the edge colour as a hint.
        let (init, iw, _) = decode_rgba(&plan.init_png).unwrap();
        let p = &init[((400 * iw + 10) * 4) as usize..][..3];
        assert!(
            p.iter()
                .zip([200u8, 50, 50])
                .all(|(a, b)| a.abs_diff(b) <= 3),
            "{p:?}"
        );
    }

    #[test]
    fn blend_keeps_the_source_and_fades_across_the_seam() {
        let src = png(300, 200, |x, _| [(x % 200) as u8, 60, 90, 255]);
        let canvas = Canvas {
            width: 300,
            height: 400,
            left: 0,
            top: 200,
        };
        let plan = ExtendPlan::new(&src, canvas, |w, h| (w, h)).unwrap();
        // 400/24 ≈ 17 px along the top, where the new space is.
        assert_eq!(plan.seam, [0, 17, 0, 0]);
        // The engine "draws" everything in green.
        let green = png(300, 400, |_, _| [0, 255, 0, 255]);
        let out = plan.blend(&green).unwrap();
        let (px, ow, oh) = decode_rgba(&out).unwrap();
        assert_eq!((ow, oh), (300, 400));
        let get = |x: u32, y: u32| &px[((y * ow + x) * 4) as usize..][..4];
        assert_eq!(get(10, 50), &[0, 255, 0, 255], "new space: the redraw");
        assert_eq!(get(10, 399), &[10, 60, 90, 255], "old picture: untouched");
        assert_eq!(get(150, 250), &[150, 60, 90, 255]);
        let seam = get(10, 204);
        assert!(seam[1] > 60 && seam[1] < 255, "seam blends: {seam:?}");
        let near_edge = get(10, 201)[1];
        let further_in = get(10, 212)[1];
        assert!(near_edge > further_in, "fades towards the source");
    }

    #[test]
    fn rejects_canvases_that_add_nothing_or_are_too_big() {
        let src = png(64, 64, |_, _| [1, 2, 3, 255]);
        let plan = |c| ExtendPlan::new(&src, c, |w, h| (w, h)).err();
        let c = |width, height, left, top| Canvas {
            width,
            height,
            left,
            top,
        };
        assert_eq!(plan(c(64, 64, 0, 0)), Some(ExtendError::NothingToAdd));
        assert_eq!(plan(c(100, 64, 40, 0)), Some(ExtendError::NothingToAdd));
        assert_eq!(plan(c(100, 60, 0, 0)), Some(ExtendError::NothingToAdd));
        assert_eq!(plan(c(MAX_SIDE + 1, 64, 0, 0)), Some(ExtendError::TooBig));
        assert!(plan(c(100, 64, 36, 0)).is_none());
    }
}
