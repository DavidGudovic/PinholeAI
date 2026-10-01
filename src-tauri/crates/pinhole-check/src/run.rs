//! Loading the check's models and measuring one image. Models load on first use
//! (after their files pass the SHA-256 check) and are dropped by
//! [`Checker::unload_if_idle`], so the ~1 GB they take is only held while in use.
use std::path::PathBuf;
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

use image::{imageops::FilterType, RgbImage};
use parking_lot::Mutex;
use tract_onnx::prelude::*;

use crate::files::{self, CheckFile};
use crate::rules::{self, Face, Original, Readings, Tags};
use crate::CheckError;

type Model = Arc<TypedRunnableModel>;

/// Indices of the tags the rules read, in the tagger's output.
#[derive(Debug, Clone)]
struct TagIndex {
    general: usize,
    sensitive: usize,
    questionable: usize,
    explicit: usize,
    minor: Vec<usize>,
    realistic: usize,
    photorealistic: usize,
    nude: Vec<usize>,
    underwear: Vec<usize>,
    count: usize,
}

#[derive(Default)]
struct Loaded {
    nudity: Option<Model>,
    tagger: Option<(Model, TagIndex)>,
    faces: Option<Model>,
    age: Option<Model>,
}

/// The check for one Data folder. All methods block: call them from
/// `spawn_blocking`. Calls are serialized (one image at a time).
pub struct Checker {
    dir: PathBuf,
    loaded: Mutex<Loaded>,
    last_used: Mutex<Option<Instant>>,
}

impl Checker {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            loaded: Mutex::new(Loaded::default()),
            last_used: Mutex::new(None),
        }
    }

    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// Files absent or of the wrong size.
    pub fn missing_files(&self) -> Vec<CheckFile> {
        files::missing(&self.dir)
    }

    /// Load the model every result needs, so the first check doesn't wait for it.
    pub fn preload(&self) -> Result<(), CheckError> {
        let mut l = self.loaded.lock();
        *self.last_used.lock() = Some(Instant::now());
        self.nudity_model(&mut l)?;
        self.tagger(&mut l).map(|_| ())
    }

    /// Drop the models when unused for `idle`. Returns whether anything was dropped.
    pub fn unload_if_idle(&self, idle: Duration) -> bool {
        let Some(mut l) = self.loaded.try_lock() else {
            return false; // checking right now
        };
        let stale = self.last_used.lock().is_some_and(|t| t.elapsed() >= idle);
        let any = l.nudity.is_some() || l.tagger.is_some() || l.faces.is_some() || l.age.is_some();
        if stale && any {
            *l = Loaded::default();
            true
        } else {
            false
        }
    }

    pub fn unload(&self) {
        *self.loaded.lock() = Loaded::default();
    }

    /// Measure one result. Later steps only run when the earlier ones call for them.
    pub fn readings(&self, png: &[u8]) -> Result<Readings, CheckError> {
        let img = decode(png)?;
        let mut l = self.loaded.lock();
        *self.last_used.lock() = Some(Instant::now());
        let nudity = self.nudity(&mut l, &img)?;
        // Always: an explicit picture the nudity classifier scores low must still reach
        // the child tags.
        let tags = Some(self.tags(&mut l, &img)?);
        let faces = if rules::needs_faces(nudity, tags.as_ref()) {
            Some(self.judged_faces(&mut l, &img)?)
        } else {
            None
        };
        Ok(Readings {
            nudity,
            tags,
            faces,
        })
    }

    /// Every step on one picture, whether or not the rules need it (dev builds' readings
    /// view: shows each half of a rule on ordinary pictures).
    pub fn full_readings(&self, png: &[u8]) -> Result<Readings, CheckError> {
        let img = decode(png)?;
        let mut l = self.loaded.lock();
        *self.last_used.lock() = Some(Instant::now());
        let nudity = self.nudity(&mut l, &img)?;
        let tags = Some(self.tags(&mut l, &img)?);
        Ok(Readings {
            nudity,
            tags,
            faces: Some(self.judged_faces(&mut l, &img)?),
        })
    }

    /// Measure a brought-in picture once: is there a person, and was it intimate already?
    pub fn original(&self, png: &[u8]) -> Result<Original, CheckError> {
        let img = decode(png)?;
        let mut l = self.loaded.lock();
        *self.last_used.lock() = Some(Instant::now());
        let Some(people) = self.original_faces(&mut l, &img)? else {
            return Ok(Original::default());
        };
        let nudity = self.nudity(&mut l, &img)?;
        let tags = Some(self.tags(&mut l, &img)?);
        let mut intimate = rules::is_intimate(nudity, tags.as_ref());
        // Already intimate only counts per person: a picture that is intimate somewhere
        // (a collage) doesn't exempt a person in it who isn't.
        if intimate {
            let mut each = Vec::with_capacity(people.len());
            for b in people.iter().take(rules::MAX_PEOPLE_MEASURED) {
                let region = body_region(&img, *b);
                let n = self.nudity(&mut l, &region)?;
                let t = self.tags(&mut l, &region)?;
                each.push(rules::is_intimate(n, Some(&t)));
            }
            intimate = rules::already_intimate(people.len(), &each);
        }
        Ok(Original {
            has_face: true,
            intimate,
        })
    }

    // ------------------------------------------------------------ models

    fn load(&self, f: &CheckFile, shape: &[usize]) -> Result<Model, CheckError> {
        self.load_checked(f, shape, &[])
    }

    /// Load, and when `outputs` is given, require the graph's outputs in that order.
    fn load_checked(
        &self,
        f: &CheckFile,
        shape: &[usize],
        outputs: &[&str],
    ) -> Result<Model, CheckError> {
        init_threads();
        let bytes = files::read_verified(&self.dir, f)?;
        let model = tract_onnx::onnx()
            .with_ignore_output_shapes(true)
            .with_ignore_value_info(true)
            .model_for_read(&mut bytes.as_slice())?
            .with_input_fact(0, f32::fact(shape).into())?
            .into_optimized()?;
        if !outputs.is_empty() {
            let names: Vec<Option<&str>> = model
                .output_outlets()?
                .iter()
                .map(|o| model.outlet_label(*o))
                .collect();
            let want: Vec<Option<&str>> = outputs.iter().map(|n| Some(*n)).collect();
            if names != want {
                return Err(CheckError::Damaged(f.label));
            }
        }
        Ok(model.into_runnable()?)
    }

    fn nudity_model(&self, l: &mut Loaded) -> Result<Model, CheckError> {
        if l.nudity.is_none() {
            l.nudity = Some(self.load(&files::NUDITY, &[1, 3, 384, 384])?);
        }
        Ok(l.nudity.clone().expect("loaded"))
    }

    fn nudity(&self, l: &mut Loaded, img: &RgbImage) -> Result<f32, CheckError> {
        let m = self.nudity_model(l)?;
        let out = m.run(tvec!(vit_input(img, 384).into()))?;
        // Labels: 0 = sfw, 1 = nsfw.
        Ok(softmax(&flat(&out[0])?).get(1).copied().unwrap_or(1.0))
    }

    fn tagger(&self, l: &mut Loaded) -> Result<(Model, TagIndex), CheckError> {
        if l.tagger.is_none() {
            let csv = files::read_verified(&self.dir, &files::TAGGER_TAGS)?;
            let index = tag_index(&csv).ok_or(CheckError::Damaged(files::TAGGER_TAGS.label))?;
            let model = self.load(&files::TAGGER, &[1, 448, 448, 3])?;
            l.tagger = Some((model, index));
        }
        Ok(l.tagger.clone().expect("loaded"))
    }

    fn tags(&self, l: &mut Loaded, img: &RgbImage) -> Result<Tags, CheckError> {
        let (m, ix) = self.tagger(l)?;
        let out = m.run(tvec!(tagger_input(img).into()))?;
        // Already probabilities (sigmoid inside the model).
        let v = flat(&out[0])?;
        if v.len() != ix.count {
            return Err(CheckError::Damaged(files::TAGGER.label));
        }
        let highest = |at: &[usize]| at.iter().map(|&i| v[i]).fold(0.0, f32::max);
        Ok(Tags {
            general: v[ix.general],
            sensitive: v[ix.sensitive],
            questionable: v[ix.questionable],
            explicit: v[ix.explicit],
            minor: highest(&ix.minor),
            realistic: v[ix.realistic],
            photorealistic: v[ix.photorealistic],
            nude: highest(&ix.nude),
            underwear: highest(&ix.underwear),
        })
    }

    fn face_model(&self, l: &mut Loaded) -> Result<Model, CheckError> {
        if l.faces.is_none() {
            l.faces = Some(self.load_checked(
                &files::FACES,
                &[1, 3, 640, 640],
                &[
                    "cls_8", "cls_16", "cls_32", "obj_8", "obj_16", "obj_32", "bbox_8", "bbox_16",
                    "bbox_32", "kps_8", "kps_16", "kps_32",
                ],
            )?);
        }
        Ok(l.faces.clone().expect("loaded"))
    }

    /// Faces with their box (x, y, w, h) in image pixels.
    fn faces(&self, l: &mut Loaded, img: &RgbImage) -> Result<Vec<(Face, [f32; 4])>, CheckError> {
        let m = self.face_model(l)?;
        let mut found = find_faces(&m, img, 1.0)?;
        if found.iter().any(|f| f.0.counts()) {
            return Ok(found);
        }
        // A face filling the whole picture (a close-up) is found more easily with a
        // border around it. Small faces from the first pass are kept (a sexual photo with
        // a face too small to judge is blocked).
        found.extend(find_faces(&m, img, 0.5)?);
        Ok(found)
    }

    /// Faces on a result, each big enough one with its age estimate. When none is found
    /// upright, the picture is also tried turned 90/180/270° (someone lying down).
    fn judged_faces(&self, l: &mut Loaded, img: &RgbImage) -> Result<Vec<Face>, CheckError> {
        use image::imageops::{rotate180, rotate270, rotate90};
        let upright = self.faces(l, img)?;
        let mut pick = (upright, None);
        if !pick.0.iter().any(|f| f.0.judged()) {
            for turn in [rotate90, rotate180, rotate270] {
                let turned = turn(img);
                let found = self.faces(l, &turned)?;
                if found.iter().any(|f| f.0.judged()) {
                    // Small faces seen upright still count (fail closed).
                    let mut both = found;
                    both.extend(pick.0.into_iter().filter(|f| f.0.too_small_to_judge()));
                    pick = (both, Some(turned));
                    break;
                }
            }
        }
        let (mut found, turned) = pick;
        let on = turned.as_ref().unwrap_or(img);
        for f in found.iter_mut().filter(|f| f.0.judged()) {
            f.0.child_face = Some(self.child_face(l, on, f.1)?);
        }
        Ok(found.into_iter().map(|f| f.0).collect())
    }

    /// Whether a brought-in picture shows a person. Searched harder than a result: a small
    /// face counts (Edit and Upscale enlarge it), the picture is also tried turned on its
    /// side and upside down, and a large picture is also searched in closer sections.
    /// `None` when there's no person; otherwise the upright face boxes found (empty when a
    /// face was only found with the picture turned).
    fn original_faces(
        &self,
        l: &mut Loaded,
        img: &RgbImage,
    ) -> Result<Option<Vec<[f32; 4]>>, CheckError> {
        use image::imageops::{crop_imm, rotate180, rotate270, rotate90};
        let m = self.face_model(l)?;
        let boxes = |img: &RgbImage, fill: f32| -> Result<Vec<[f32; 4]>, CheckError> {
            Ok(find_faces(&m, img, fill)?
                .into_iter()
                .filter(|f| f.0.counts_in_original())
                .map(|f| f.1)
                .collect())
        };
        for fill in [1.0, 0.5] {
            let found = boxes(img, fill)?;
            if !found.is_empty() {
                return Ok(Some(found));
            }
        }
        for turn in [rotate90, rotate180, rotate270] {
            if !boxes(&turn(img), 1.0)?.is_empty() {
                return Ok(Some(Vec::new()));
            }
        }
        let (w, h) = img.dimensions();
        if w.max(h) > 1280 {
            // 3 × 3 overlapping sections, each half the picture's size.
            let (tw, th) = (w / 2, h / 2);
            let mut found = Vec::new();
            for i in 0..3 {
                for j in 0..3 {
                    let (x0, y0) = (i * tw / 2, j * th / 2);
                    let tile = crop_imm(img, x0, y0, tw, th).to_image();
                    for b in boxes(&tile, 1.0)? {
                        let b = [b[0] + x0 as f32, b[1] + y0 as f32, b[2], b[3]];
                        if found.iter().all(|k| iou(k, &b) < 0.3) {
                            found.push(b);
                        }
                    }
                }
            }
            if !found.is_empty() {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    /// Confidence that a face is a child's: the age estimate's 0–2 and 3–9 groups together.
    fn child_face(&self, l: &mut Loaded, img: &RgbImage, b: [f32; 4]) -> Result<f32, CheckError> {
        if l.age.is_none() {
            l.age = Some(self.load(&files::AGE, &[1, 3, 224, 224])?);
        }
        let m = l.age.clone().expect("loaded");
        let crop = face_crop(img, b);
        let out = m.run(tvec!(vit_input(&crop, 224).into()))?;
        // Classes: 0–2, 3–9, 10–19, 20–29, …
        let p = softmax(&flat(&out[0])?);
        Ok(p.first().copied().unwrap_or(0.0) + p.get(1).copied().unwrap_or(0.0))
    }
}

/// The matrix code runs on several cores (set once per process).
fn init_threads() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let n = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .clamp(1, 8);
        tract_linalg::multithread::set_default_executor(
            tract_linalg::multithread::Executor::multithread(n),
        );
    });
}

pub fn decode(png: &[u8]) -> Result<RgbImage, CheckError> {
    image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map(|i| i.to_rgb8())
        .map_err(|_| CheckError::Image)
}

/// A model's output as numbers. Anything not a finite number is an error, so the picture is
/// dropped (fails closed) instead of every threshold reading as "not met".
fn flat(t: &TValue) -> Result<Vec<f32>, CheckError> {
    let v: Vec<f32> = t.to_plain_array_view::<f32>()?.iter().copied().collect();
    finite(v)
}

fn finite(v: Vec<f32>) -> Result<Vec<f32>, CheckError> {
    if v.iter().all(|x| x.is_finite()) {
        Ok(v)
    } else {
        Err(CheckError::Run(
            "a check model returned an invalid number".into(),
        ))
    }
}

fn softmax(v: &[f32]) -> Vec<f32> {
    let m = v.iter().copied().fold(f32::MIN, f32::max);
    let e: Vec<f32> = v.iter().map(|x| (x - m).exp()).collect();
    let s: f32 = e.iter().sum();
    e.iter().map(|x| x / s).collect()
}

/// Hugging Face ViT preprocessing: bilinear resize to s×s, (x/255 − 0.5)/0.5, NCHW.
fn vit_input(img: &RgbImage, s: u32) -> Tensor {
    let r = image::imageops::resize(img, s, s, FilterType::Triangle);
    tract_ndarray::Array4::from_shape_fn((1, 3, s as usize, s as usize), |(_, c, y, x)| {
        (r.get_pixel(x as u32, y as u32)[c] as f32 / 255.0 - 0.5) / 0.5
    })
    .into()
}

/// WD tagger: pad to a white square, bicubic to 448, BGR 0..255, NHWC.
fn tagger_input(img: &RgbImage) -> Tensor {
    let side = img.width().max(img.height()).max(1);
    let mut sq = RgbImage::from_pixel(side, side, image::Rgb([255, 255, 255]));
    image::imageops::overlay(
        &mut sq,
        img,
        ((side - img.width()) / 2) as i64,
        ((side - img.height()) / 2) as i64,
    );
    let r = image::imageops::resize(&sq, 448, 448, FilterType::CatmullRom);
    tract_ndarray::Array4::from_shape_fn((1, 448, 448, 3), |(_, y, x, c)| {
        r.get_pixel(x as u32, y as u32)[2 - c] as f32
    })
    .into()
}

/// The tagger tags each signal reads, by `tag_id` (first column of the pinned
/// `selected_tags.csv`, whose SHA-256 is checked, so the ids can't drift).
mod tag_ids {
    /// The rating rows: general, sensitive, questionable, explicit.
    pub const RATINGS: [u32; 4] = [9999999, 9999998, 9999997, 9999996];
    /// A drawn character tagged as a child (3 tags).
    pub const MINOR: &[u32] = &[128, 2614, 12667];
    /// Photo style: realistic, photorealistic.
    pub const PHOTO_STYLE: [u32; 2] = [376102, 462982];
    /// Nudity and sexual content (20 tags), including implied nudity (covered or censored).
    pub const NUDE: &[u32] = &[
        2365, 822149, 3593, 4528, 8889, 533356, 510254, 4378, 4190, 2357, 390591, 12552, 2217,
        488169, 390314, 522720, 7834, 484631, 421107, 409364,
    ];
    /// Underwear, lingerie and see-through clothing (6 tags; swimwear isn't one of them).
    pub const UNDERWEAR: &[u32] = &[464906, 391, 3796, 319, 451371, 547073];
}

/// Rows of `selected_tags.csv` (`tag_id,name,category,count`) in output order.
fn tag_index(csv: &[u8]) -> Option<TagIndex> {
    let text = std::str::from_utf8(csv).ok()?;
    let ids: Vec<Option<u32>> = text
        .lines()
        .skip(1)
        .filter(|l| !l.is_empty())
        .map(|l| l.split(',').next().and_then(|id| id.parse().ok()))
        .collect();
    let at = |id: u32| ids.iter().position(|t| *t == Some(id));
    let all = |list: &[u32]| list.iter().map(|&id| at(id)).collect::<Option<Vec<_>>>();
    use tag_ids::*;
    Some(TagIndex {
        general: at(RATINGS[0])?,
        sensitive: at(RATINGS[1])?,
        questionable: at(RATINGS[2])?,
        explicit: at(RATINGS[3])?,
        minor: all(MINOR)?,
        realistic: at(PHOTO_STYLE[0])?,
        photorealistic: at(PHOTO_STYLE[1])?,
        nude: all(NUDE)?,
        underwear: all(UNDERWEAR)?,
        count: ids.len(),
    })
}

/// YuNet (OpenCV face detector, 2023mar): the image scaled to `fill` of a 640×640
/// input, centred on black, BGR 0..255, NCHW. Outputs cls/obj/bbox/kps per stride
/// 8, 16, 32. Boxes come back in image pixels.
fn find_faces(m: &Model, img: &RgbImage, fill: f32) -> Result<Vec<(Face, [f32; 4])>, CheckError> {
    const S: u32 = 640;
    let scale = S as f32 * fill / img.width().max(img.height()).max(1) as f32;
    let w = ((img.width() as f32 * scale).round() as u32).clamp(1, S);
    let h = ((img.height() as f32 * scale).round() as u32).clamp(1, S);
    let (ox, oy) = ((S - w) / 2, (S - h) / 2);
    let r = image::imageops::resize(img, w, h, FilterType::Triangle);
    let t: Tensor =
        tract_ndarray::Array4::from_shape_fn((1, 3, S as usize, S as usize), |(_, c, y, x)| {
            let (x, y) = (x as u32, y as u32);
            if x >= ox && y >= oy && x - ox < w && y - oy < h {
                r.get_pixel(x - ox, y - oy)[2 - c] as f32
            } else {
                0.0
            }
        })
        .into();
    let out = m.run(tvec!(t.into()))?;
    if out.len() < 9 {
        return Err(CheckError::Damaged(files::FACES.label));
    }
    let mut found: Vec<(f32, [f32; 4])> = Vec::new();
    for (i, stride) in [8usize, 16, 32].into_iter().enumerate() {
        let cls = out[i].to_plain_array_view::<f32>()?;
        let obj = out[3 + i].to_plain_array_view::<f32>()?;
        let bbox = out[6 + i].to_plain_array_view::<f32>()?;
        let cols = S as usize / stride;
        let n = cls.shape().get(1).copied().unwrap_or(0);
        if obj.shape().get(1) != Some(&n) || bbox.shape().get(1) != Some(&n) {
            return Err(CheckError::Damaged(files::FACES.label));
        }
        let s = stride as f32;
        for k in 0..n {
            let raw = [
                cls[[0, k, 0]],
                obj[[0, k, 0]],
                bbox[[0, k, 0]],
                bbox[[0, k, 1]],
                bbox[[0, k, 2]],
                bbox[[0, k, 3]],
            ];
            if raw.iter().any(|x| !x.is_finite()) {
                return Err(CheckError::Run(
                    "a check model returned an invalid number".into(),
                ));
            }
            let score = (cls[[0, k, 0]].clamp(0.0, 1.0) * obj[[0, k, 0]].clamp(0.0, 1.0)).sqrt();
            if score < 0.5 {
                continue;
            }
            let (row, col) = ((k / cols) as f32, (k % cols) as f32);
            let cx = (col + bbox[[0, k, 0]]) * s;
            let cy = (row + bbox[[0, k, 1]]) * s;
            let bw = bbox[[0, k, 2]].exp() * s;
            let bh = bbox[[0, k, 3]].exp() * s;
            found.push((
                score,
                [
                    (cx - bw / 2.0 - ox as f32) / scale,
                    (cy - bh / 2.0 - oy as f32) / scale,
                    bw / scale,
                    bh / scale,
                ],
            ));
        }
    }
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut keep: Vec<(f32, [f32; 4])> = Vec::new();
    for f in found {
        if keep.iter().all(|k| iou(&k.1, &f.1) < 0.3) {
            keep.push(f);
        }
    }
    Ok(keep
        .into_iter()
        .map(|(score, b)| {
            (
                Face {
                    score,
                    side: b[2].min(b[3]),
                    child_face: None,
                },
                b,
            )
        })
        .collect())
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let x1 = a[0].max(b[0]);
    let y1 = a[1].max(b[1]);
    let x2 = (a[0] + a[2]).min(b[0] + b[2]);
    let y2 = (a[1] + a[3]).min(b[1] + b[3]);
    let inter = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    let union = a[2] * a[3] + b[2] * b[3] - inter;
    if union > 0.0 {
        inter / union
    } else {
        0.0
    }
}

/// The part of the picture that shows the person a face box belongs to: three face widths
/// wide, from just above the face to about six face heights below it, kept inside the image.
fn body_region(img: &RgbImage, b: [f32; 4]) -> RgbImage {
    let (x, y, w, h) = body_box(img.dimensions(), b);
    image::imageops::crop_imm(img, x, y, w, h).to_image()
}

fn body_box((iw, ih): (u32, u32), b: [f32; 4]) -> (u32, u32, u32, u32) {
    let cx = b[0] + b[2] / 2.0;
    let x0 = (cx - b[2] * 1.5).max(0.0);
    let x1 = (cx + b[2] * 1.5).min(iw as f32);
    let y0 = (b[1] - b[3] * 0.5).max(0.0);
    let y1 = (b[1] + b[3] * 6.5).min(ih as f32);
    let x = (x0 as u32).min(iw.saturating_sub(1));
    let y = (y0 as u32).min(ih.saturating_sub(1));
    let w = ((x1 - x0) as u32).clamp(1, iw - x);
    let h = ((y1 - y0) as u32).clamp(1, ih - y);
    (x, y, w, h)
}

/// Square crop 1.5× the face box, kept inside the image.
fn face_crop(img: &RgbImage, b: [f32; 4]) -> RgbImage {
    let side = b[2].max(b[3]) * 1.5;
    let (cx, cy) = (b[0] + b[2] / 2.0, b[1] + b[3] / 2.0);
    let x0 = ((cx - side / 2.0).max(0.0) as u32).min(img.width().saturating_sub(1));
    let y0 = ((cy - side / 2.0).max(0.0) as u32).min(img.height().saturating_sub(1));
    let s = (side as u32)
        .min(img.width() - x0)
        .min(img.height() - y0)
        .max(1);
    image::imageops::crop_imm(img, x0, y0, s, s).to_image()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_index_reads_the_csv_order() {
        use tag_ids::*;
        let mut ids: Vec<u32> = RATINGS.to_vec();
        ids.extend(MINOR);
        ids.extend(PHOTO_STYLE);
        ids.push(1);
        ids.extend(NUDE);
        ids.extend(UNDERWEAR);
        let csv: String = std::iter::once("tag_id,name,category,count\n".to_string())
            .chain(ids.iter().map(|id| format!("{id},tag,0,1\n")))
            .collect();
        let ix = tag_index(csv.as_bytes()).unwrap();
        assert_eq!(
            (ix.general, ix.explicit, ix.minor.clone(), ix.photorealistic),
            (0, 3, vec![4, 5, 6], 8)
        );
        let nude_end = 10 + NUDE.len();
        assert_eq!(ix.nude, (10..nude_end).collect::<Vec<_>>());
        let end = nude_end + UNDERWEAR.len();
        assert_eq!(ix.underwear, (nude_end..end).collect::<Vec<_>>());
        assert_eq!(ix.count, end);
        // A list without the tags the rules need is rejected.
        assert!(tag_index(b"tag_id,name,category,count\n1,tag,0,1\n").is_none());
    }

    #[test]
    fn body_box_covers_the_person_and_stays_inside() {
        // A face at (100, 50), 40 × 50: the region runs from above it to well below.
        let (x, y, w, h) = body_box((1000, 1000), [100.0, 50.0, 40.0, 50.0]);
        assert_eq!((x, y), (60, 25));
        assert_eq!((w, h), (120, 350));
        // Near the edges it is cut to the picture.
        let (x, y, w, h) = body_box((200, 200), [180.0, 150.0, 40.0, 50.0]);
        assert!(x + w <= 200 && y + h <= 200 && w >= 1 && h >= 1);
        let (x, y, w, h) = body_box((10, 10), [-50.0, -50.0, 5.0, 5.0]);
        assert!(x < 10 && y < 10 && x + w <= 10 && y + h <= 10);
    }

    #[test]
    fn face_crop_stays_inside_the_image() {
        let img = RgbImage::new(100, 80);
        for b in [
            [-20.0, -20.0, 50.0, 50.0],
            [90.0, 70.0, 40.0, 40.0],
            [0.0, 0.0, 0.0, 0.0],
        ] {
            let c = face_crop(&img, b);
            assert!(c.width() >= 1 && c.width() <= 100 && c.height() <= 80);
        }
    }

    #[test]
    fn a_missing_file_stops_the_check() {
        let dir = tempfile::tempdir().unwrap();
        let c = Checker::new(dir.path().to_path_buf());
        assert_eq!(c.missing_files().len(), files::FILES.len());
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(RgbImage::new(8, 8))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert!(matches!(c.readings(&png), Err(CheckError::Missing(_))));
        assert!(matches!(c.original(&png), Err(CheckError::Missing(_))));
        assert!(matches!(c.readings(b"not a png"), Err(CheckError::Image)));
    }
}
