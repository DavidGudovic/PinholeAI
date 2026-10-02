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
    young: Vec<usize>,
    count: usize,
}

#[derive(Default)]
struct Loaded {
    nudity: Option<Model>,
    tagger: Option<(Model, TagIndex)>,
    faces: Option<Model>,
    age: Option<Model>,
    age_years: Option<Model>,
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
        let any = l.nudity.is_some()
            || l.tagger.is_some()
            || l.faces.is_some()
            || l.age.is_some()
            || l.age_years.is_some();
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
        let mut nudity = self.nudity(&mut l, &img)?;
        // Always: an explicit picture the nudity classifier scores low must still reach
        // the child tags.
        let mut tags = Some(self.tags(&mut l, &img)?);
        // A large picture is also measured in sections: shrunk whole to the classifiers'
        // size, a part of it can be too small to judge. The first section that reads as
        // sexual (or else intimate) stands for the picture, and the child and photo tags
        // are the highest seen anywhere (a child in one section, sexual content in another).
        let secs = sections(img.dimensions());
        if !secs.is_empty() {
            let mut sexual = rules::is_sexual(nudity, tags.as_ref());
            let mut intimate = rules::is_intimate(nudity, tags.as_ref());
            let mut most = tags.unwrap_or_default();
            for (x, y, w, h) in secs {
                let part = image::imageops::crop_imm(&img, x, y, w, h).to_image();
                let n = self.nudity(&mut l, &part)?;
                let t = self.tags(&mut l, &part)?;
                most.minor = most.minor.max(t.minor);
                most.realistic = most.realistic.max(t.realistic);
                most.photorealistic = most.photorealistic.max(t.photorealistic);
                most.young_context = most.young_context.max(t.young_context);
                if sexual {
                    continue;
                }
                if rules::is_sexual(n, Some(&t)) {
                    (nudity, tags, sexual) = (n, Some(t), true);
                } else if !intimate && rules::is_intimate(n, Some(&t)) {
                    (nudity, tags, intimate) = (n, Some(t), true);
                }
            }
            if let Some(t) = tags.as_mut() {
                t.minor = most.minor;
                t.realistic = most.realistic;
                t.photorealistic = most.photorealistic;
                t.young_context = most.young_context;
            }
        }
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

    /// Boxes (x, y, w, h, in image pixels) of the clear faces on a picture that are big
    /// enough to redraw ([`Face::counts`]), largest first. Upright only, no age estimate.
    pub fn face_boxes(&self, png: &[u8]) -> Result<Vec<[f32; 4]>, CheckError> {
        let img = decode(png)?;
        let mut l = self.loaded.lock();
        *self.last_used.lock() = Some(Instant::now());
        let mut found: Vec<_> = self
            .faces(&mut l, &img, true)?
            .into_iter()
            .filter(|f| f.0.counts())
            .collect();
        found.sort_by(|a, b| b.0.side.total_cmp(&a.0.side));
        // The same face can be found by more than one pass: keep the first of overlapping boxes.
        let mut out: Vec<[f32; 4]> = Vec::new();
        for (_, b) in found {
            if !out.iter().any(|o| overlap(o, &b) > 0.5) {
                out.push(b);
            }
        }
        Ok(out)
    }

    /// Only the faces of a picture with their age estimates (measurement tools: the
    /// age half of rule 2 on folders of portraits, without the slower classifiers).
    pub fn face_readings(&self, png: &[u8]) -> Result<Vec<Face>, CheckError> {
        let img = decode(png)?;
        let mut l = self.loaded.lock();
        *self.last_used.lock() = Some(Instant::now());
        self.judged_faces(&mut l, &img)
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

    /// Measure a brought-in picture once: is there a person? What it already shows doesn't
    /// matter (rule 1 applies to every brought-in picture of a person).
    pub fn original(&self, png: &[u8]) -> Result<Original, CheckError> {
        let img = decode(png)?;
        let mut l = self.loaded.lock();
        *self.last_used.lock() = Some(Instant::now());
        Ok(Original {
            has_face: self.original_faces(&mut l, &img)?.is_some(),
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
            young_context: highest(&ix.young),
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
    /// `windows`: a large picture is also searched at its own resolution (upright only; the
    /// turned tries are a fallback and stay quick).
    fn faces(
        &self,
        l: &mut Loaded,
        img: &RgbImage,
        windows: bool,
    ) -> Result<Vec<(Face, [f32; 4])>, CheckError> {
        let m = self.face_model(l)?;
        if windows && img.width().max(img.height()) > WINDOW * 2 {
            let mut all = find_faces(&m, img, 1.0)?;
            all.extend(find_faces(&m, img, 0.5)?);
            all.extend(find_faces_in_windows(&m, img)?);
            return Ok(merge_faces(all));
        }
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
        let upright = self.faces(l, img, true)?;
        let mut pick = (upright, None);
        if !pick.0.iter().any(|f| f.0.judged()) {
            for turn in [rotate90, rotate180, rotate270] {
                let turned = turn(img);
                let found = self.faces(l, &turned, false)?;
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
            let (child, under_20) = self.age_groups(l, on, f.1)?;
            f.0.child_face = Some(child);
            f.0.under_20_face = Some(under_20);
            f.0.age = Some(self.age_years(l, on, f.1)?);
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
        use image::imageops::{rotate180, rotate270, rotate90};
        let m = self.face_model(l)?;
        let keep = original_boxes;
        // The whole picture first (quick); then, for a large picture, its own resolution.
        let mut whole = find_faces(&m, img, 1.0)?;
        whole.extend(find_faces(&m, img, 0.5)?);
        let whole = keep(whole);
        if !whole.is_empty() {
            return Ok(Some(whole));
        }
        if img.width().max(img.height()) > WINDOW {
            let upright = keep(find_faces_in_windows(&m, img)?);
            if !upright.is_empty() {
                return Ok(Some(upright));
            }
        }
        // Turned on its side, upside down, or at a slant (a face is only found upright).
        // Each turn is made only when the one before found nobody.
        let small = shrink(img, 1280);
        let turns: [&dyn Fn() -> RgbImage; 7] = [
            &|| rotate90(&small),
            &|| rotate180(&small),
            &|| rotate270(&small),
            &|| rotate_any(&small, 45.0),
            &|| rotate_any(&small, 135.0),
            &|| rotate_any(&small, 225.0),
            &|| rotate_any(&small, 315.0),
        ];
        for turn in turns {
            let t = turn();
            if !keep(find_faces(&m, &t, 1.0)?).is_empty()
                || !keep(find_faces(&m, &t, 0.5)?).is_empty()
            {
                return Ok(Some(Vec::new()));
            }
        }
        Ok(None)
    }

    /// The age estimate's confidence that a face is a child's (its 0–2 and 3–9 groups
    /// together) and that it is under 20 (0–2, 3–9 and 10–19 together).
    fn age_groups(
        &self,
        l: &mut Loaded,
        img: &RgbImage,
        b: [f32; 4],
    ) -> Result<(f32, f32), CheckError> {
        if l.age.is_none() {
            l.age = Some(self.load(&files::AGE, &[1, 3, 224, 224])?);
        }
        let m = l.age.clone().expect("loaded");
        let crop = face_crop(img, b);
        let out = m.run(tvec!(vit_input(&crop, 224).into()))?;
        // Classes: 0–2, 3–9, 10–19, 20–29, …
        let p = softmax(&flat(&out[0])?);
        let group = |i: usize| p.get(i).copied().unwrap_or(0.0);
        let child = group(0) + group(1);
        Ok((child, child + group(2)))
    }

    /// The face's age in years (MiVOLO v2, face only: its body input gets MiVOLO's own
    /// "no body" image).
    fn age_years(&self, l: &mut Loaded, img: &RgbImage, b: [f32; 4]) -> Result<f32, CheckError> {
        if l.age_years.is_none() {
            l.age_years = Some(self.load_age_years()?);
        }
        let m = l.age_years.clone().expect("loaded");
        let crop = box_crop(img, b);
        let out = m.run(tvec!(mivolo_input(&crop).into()))?;
        let v = flat(&out[0])?;
        v.first()
            .copied()
            .ok_or(CheckError::Damaged(files::AGE_YEARS.label))
    }

    /// MiVOLO uses a step (`Col2Im`) the ONNX runtime doesn't have; it is rewritten into two
    /// that it has (see [`rewrite_col2im`]) before the graph is built.
    fn load_age_years(&self) -> Result<Model, CheckError> {
        use tract_onnx::prelude::Framework;
        init_threads();
        let f = &files::AGE_YEARS;
        let bytes = files::read_verified(&self.dir, f)?;
        let onnx = tract_onnx::onnx()
            .with_ignore_output_shapes(true)
            .with_ignore_value_info(true);
        let mut proto = onnx.proto_model_for_read(&mut bytes.as_slice())?;
        let graph = proto.graph.as_mut().ok_or(CheckError::Damaged(f.label))?;
        if rewrite_col2im(graph).is_none() {
            return Err(CheckError::Damaged(f.label));
        }
        let model = onnx
            .model_for_proto_model(&proto)?
            .with_input_fact(0, f32::fact([1, 6, MIVOLO_SIDE, MIVOLO_SIDE]).into())?
            .into_optimized()?;
        Ok(model.into_runnable()?)
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
    /// A setting, clothing or object that presents someone as under 18 (13 tags), and the
    /// child tags again at a lower score. Read only for a borderline face in a photo.
    pub const YOUNG_CONTEXT: &[u32] = &[
        16509, 268819, 221, 374849, 463399, 3468, 394881, 392008, 4855, 538859, 460404, 379915,
        9831, 128, 2614, 12667,
    ];
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
        young: all(YOUNG_CONTEXT)?,
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
                    under_20_face: None,
                    age: None,
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

/// Side (pixels) of the windows a large picture is searched in at its own resolution: the
/// face finder's input size.
const WINDOW: u32 = 640;

/// Faces in overlapping windows of a picture larger than the finder's input, at its own
/// resolution and at half of it: shrunk whole, a face in a large picture is too small to find.
/// Boxes are in picture pixels (not merged; see `merge_faces`).
fn find_faces_in_windows(m: &Model, img: &RgbImage) -> Result<Vec<(Face, [f32; 4])>, CheckError> {
    let mut all = Vec::new();
    let (w, h) = img.dimensions();
    for win in [WINDOW, WINDOW * 2] {
        if w.max(h) <= win {
            continue;
        }
        let step = win * 3 / 4;
        for y in starts(h, win, step) {
            for x in starts(w, win, step) {
                let tile = image::imageops::crop_imm(img, x, y, win.min(w), win.min(h)).to_image();
                for (f, b) in find_faces(m, &tile, 1.0)? {
                    all.push((f, [b[0] + x as f32, b[1] + y as f32, b[2], b[3]]));
                }
            }
        }
    }
    Ok(all)
}

/// The boxes of faces that count in a brought-in picture, one per face. Filtered before
/// merging: a confident box too small to count must not hide a larger box of the same face.
fn original_boxes(found: Vec<(Face, [f32; 4])>) -> Vec<[f32; 4]> {
    merge_faces(
        found
            .into_iter()
            .filter(|f| f.0.counts_in_original())
            .collect(),
    )
    .into_iter()
    .map(|f| f.1)
    .collect()
}

/// One find per face: overlapping boxes from several passes are merged, the most confident
/// kept.
fn merge_faces(mut all: Vec<(Face, [f32; 4])>) -> Vec<(Face, [f32; 4])> {
    all.sort_by(|a, b| b.0.score.total_cmp(&a.0.score));
    let mut keep: Vec<(Face, [f32; 4])> = Vec::new();
    for f in all {
        if keep.iter().all(|k| iou(&k.1, &f.1) < 0.3) {
            keep.push(f);
        }
    }
    keep
}

/// Window positions along one side: every `step`, the last one flush with the end.
fn starts(len: u32, win: u32, step: u32) -> Vec<u32> {
    if len <= win {
        return vec![0];
    }
    let mut out: Vec<u32> = (0..len - win).step_by(step.max(1) as usize).collect();
    out.push(len - win);
    out
}

/// Sections a large or long result is also measured in (x, y, w, h): none up to 2048 pixels
/// on the long side and 3:1; otherwise overlapping squares of about a third of the long side
/// (at least half the short side, at most all of it), so a part shrunk whole to the
/// classifiers' size can still be judged.
fn sections((w, h): (u32, u32)) -> Vec<(u32, u32, u32, u32)> {
    let long = w.max(h);
    if long <= 2048 && long <= w.min(h) * 3 {
        return Vec::new();
    }
    let side = (long / 3).max(w.min(h) / 2).min(w.min(h)).max(1);
    let step = side * 3 / 4;
    let mut out = Vec::new();
    for y in starts(h, side, step) {
        for x in starts(w, side, step) {
            out.push((x, y, side.min(w), side.min(h)));
        }
    }
    out
}

/// The picture shrunk so its long side is at most `max`.
fn shrink(img: &RgbImage, max: u32) -> RgbImage {
    let (w, h) = img.dimensions();
    if w.max(h) <= max {
        return img.clone();
    }
    let s = max as f32 / w.max(h) as f32;
    image::imageops::resize(
        img,
        ((w as f32 * s).round() as u32).max(1),
        ((h as f32 * s).round() as u32).max(1),
        FilterType::Triangle,
    )
}

/// The picture turned by `degrees` (clockwise) on a black canvas large enough to hold it.
fn rotate_any(img: &RgbImage, degrees: f32) -> RgbImage {
    let (w, h) = (img.width() as f32, img.height() as f32);
    let (sin, cos) = degrees.to_radians().sin_cos();
    let nw = (w * cos.abs() + h * sin.abs()).ceil().max(1.0) as u32;
    let nh = (w * sin.abs() + h * cos.abs()).ceil().max(1.0) as u32;
    let (cx, cy, ncx, ncy) = (w / 2.0, h / 2.0, nw as f32 / 2.0, nh as f32 / 2.0);
    RgbImage::from_fn(nw, nh, |x, y| {
        let (dx, dy) = (x as f32 + 0.5 - ncx, y as f32 + 0.5 - ncy);
        let sx = cos * dx + sin * dy + cx;
        let sy = -sin * dx + cos * dy + cy;
        if sx >= 0.0 && sy >= 0.0 && sx < w && sy < h {
            *img.get_pixel(sx as u32, sy as u32)
        } else {
            image::Rgb([0, 0, 0])
        }
    })
}

/// MiVOLO's input side.
const MIVOLO_SIDE: usize = 384;

/// The face box itself (MiVOLO was trained on face detector boxes), inside the picture.
fn box_crop(img: &RgbImage, b: [f32; 4]) -> RgbImage {
    let x0 = (b[0].max(0.0) as u32).min(img.width().saturating_sub(1));
    let y0 = (b[1].max(0.0) as u32).min(img.height().saturating_sub(1));
    let w = (b[2].max(1.0) as u32).min(img.width() - x0).max(1);
    let h = (b[3].max(1.0) as u32).min(img.height() - y0).max(1);
    image::imageops::crop_imm(img, x0, y0, w, h).to_image()
}

/// MiVOLO input: the face letterboxed (aspect kept, black bars, centred) to 384×384, RGB,
/// ImageNet mean/std; channels 3–5 are the body, here an all-black image normalised the same
/// way (MiVOLO's "no body").
fn mivolo_input(face: &RgbImage) -> Tensor {
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];
    let s = MIVOLO_SIDE as u32;
    let k = s as f32 / face.width().max(face.height()).max(1) as f32;
    let w = ((face.width() as f32 * k).round() as u32).clamp(1, s);
    let h = ((face.height() as f32 * k).round() as u32).clamp(1, s);
    let r = image::imageops::resize(face, w, h, FilterType::Triangle);
    let (ox, oy) = ((s - w) / 2, (s - h) / 2);
    tract_ndarray::Array4::from_shape_fn((1, 6, MIVOLO_SIDE, MIVOLO_SIDE), |(_, c, y, x)| {
        let (x, y) = (x as u32, y as u32);
        let v = if c < 3 && x >= ox && y >= oy && x - ox < w && y - oy < h {
            r.get_pixel(x - ox, y - oy)[c] as f32 / 255.0
        } else {
            0.0
        };
        (v - MEAN[c % 3]) / STD[c % 3]
    })
    .into()
}

/// Rewrite every `Col2Im` (ONNX opset 18; the ONNX runtime has none) into a `Reshape` and a
/// grouped `ConvTranspose` whose kernel is one-hot: each of the k×k input channels of a group
/// lands on its own kernel position, which is exactly what `Col2Im` sums. Needs the input's
/// shape and the image and block shapes as constants (true for the pinned file; anything
/// else returns `None` and the file counts as damaged). Checked against the original graph in
/// `onnxruntime`: same output to 5 digits.
fn rewrite_col2im(g: &mut tract_onnx::pb::GraphProto) -> Option<()> {
    use tract_onnx::pb::{attribute_proto::AttributeType, AttributeProto, NodeProto, TensorProto};
    const INT64: i32 = 7;
    const FLOAT: i32 = 1;
    let ints_of = |g: &tract_onnx::pb::GraphProto, name: &str| -> Option<Vec<i64>> {
        let t = g.initializer.iter().find(|t| t.name == name)?;
        if t.data_type != INT64 {
            return None;
        }
        if !t.int64_data.is_empty() {
            return Some(t.int64_data.clone());
        }
        let raw = &t.raw_data;
        (raw.len() % 8 == 0).then(|| {
            raw.chunks(8)
                .map(|c| i64::from_le_bytes(c.try_into().expect("8 bytes")))
                .collect()
        })
    };
    let attr = |n: &NodeProto, name: &str| -> Option<Vec<i64>> {
        n.attribute
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.ints.clone())
    };
    let ints = |name: &str, v: Vec<i64>| AttributeProto {
        name: name.into(),
        r#type: AttributeType::Ints as i32,
        ints: v,
        ..Default::default()
    };
    let mut nodes = Vec::with_capacity(g.node.len() + 4);
    let mut added = Vec::new();
    let mut rewritten = 0;
    for n in std::mem::take(&mut g.node) {
        if n.op_type != "Col2Im" {
            nodes.push(n);
            continue;
        }
        // The input's shape [N, C·kh·kw, L] from the Reshape that makes it.
        let shape_name = nodes
            .iter()
            .find(|p: &&NodeProto| p.op_type == "Reshape" && p.output.first() == n.input.first())?
            .input
            .get(1)?
            .clone();
        let [batch, ck, cols] = ints_of(g, &shape_name)?[..] else {
            return None;
        };
        let [ih, iw] = ints_of(g, n.input.get(1)?)?[..] else {
            return None;
        };
        let [kh, kw] = ints_of(g, n.input.get(2)?)?[..] else {
            return None;
        };
        let strides = attr(&n, "strides").unwrap_or(vec![1, 1]);
        let pads = attr(&n, "pads").unwrap_or(vec![0, 0, 0, 0]);
        let dil = attr(&n, "dilations").unwrap_or(vec![1, 1]);
        if strides.len() != 2 || pads.len() != 4 || dil != [1, 1] || kh < 1 || kw < 1 {
            return None;
        }
        let (sh, sw) = (strides[0], strides[1]);
        let lh = (ih + pads[0] + pads[2] - kh) / sh + 1;
        let lw = (iw + pads[1] + pads[3] - kw) / sw + 1;
        let k = kh * kw;
        if lh * lw != cols || ck % k != 0 || sh < 1 || sw < 1 {
            return None;
        }
        let groups = ck / k;
        // ConvTranspose: out = (L − 1)·s − pads + k, plus output_padding to reach the image.
        let oph = ih - ((lh - 1) * sh - pads[0] - pads[2] + kh);
        let opw = iw - ((lw - 1) * sw - pads[1] - pads[3] + kw);
        if !(0..sh).contains(&oph) || !(0..sw).contains(&opw) {
            return None;
        }
        let tag = format!("{}_pinhole", n.name);
        let mut w = vec![0f32; (ck * k) as usize];
        for c in 0..ck {
            w[(c * k + c % k) as usize] = 1.0;
        }
        added.push(TensorProto {
            name: format!("{tag}_weight"),
            dims: vec![ck, 1, kh, kw],
            data_type: FLOAT,
            float_data: w,
            ..Default::default()
        });
        added.push(TensorProto {
            name: format!("{tag}_shape"),
            dims: vec![4],
            data_type: INT64,
            int64_data: vec![batch, ck, lh, lw],
            ..Default::default()
        });
        nodes.push(NodeProto {
            name: format!("{tag}_reshape"),
            op_type: "Reshape".into(),
            input: vec![n.input[0].clone(), format!("{tag}_shape")],
            output: vec![format!("{tag}_cols")],
            ..Default::default()
        });
        nodes.push(NodeProto {
            name: format!("{tag}_deconv"),
            op_type: "ConvTranspose".into(),
            input: vec![format!("{tag}_cols"), format!("{tag}_weight")],
            output: n.output.clone(),
            attribute: vec![
                AttributeProto {
                    name: "group".into(),
                    r#type: AttributeType::Int as i32,
                    i: groups,
                    ..Default::default()
                },
                ints("kernel_shape", vec![kh, kw]),
                ints("strides", strides),
                ints("pads", pads),
                ints("output_padding", vec![oph, opw]),
            ],
            ..Default::default()
        });
        rewritten += 1;
    }
    g.node = nodes;
    g.initializer.extend(added);
    (rewritten > 0).then_some(())
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
    fn overlap_is_a_share_of_the_smaller_box() {
        let big = [0.0, 0.0, 100.0, 100.0];
        assert_eq!(overlap(&big, &[10.0, 10.0, 20.0, 20.0]), 1.0);
        assert_eq!(overlap(&big, &[90.0, 0.0, 20.0, 20.0]), 0.5);
        assert_eq!(overlap(&big, &[200.0, 0.0, 20.0, 20.0]), 0.0);
    }

    #[test]
    fn tag_index_reads_the_csv_order() {
        use tag_ids::*;
        let mut ids: Vec<u32> = RATINGS.to_vec();
        ids.extend(MINOR);
        ids.extend(PHOTO_STYLE);
        ids.push(1);
        ids.extend(NUDE);
        ids.extend(UNDERWEAR);
        // The child tags are in both lists; each id is one row.
        let new_young: Vec<u32> = YOUNG_CONTEXT
            .iter()
            .copied()
            .filter(|id| !ids.contains(id))
            .collect();
        ids.extend(&new_young);
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
        let young_end = end + new_young.len();
        let row = |id: &u32| ids.iter().position(|i| i == id).unwrap();
        assert_eq!(ix.young, YOUNG_CONTEXT.iter().map(row).collect::<Vec<_>>());
        assert_eq!(ix.count, young_end);
        // A list without the tags the rules need is rejected.
        assert!(tag_index(b"tag_id,name,category,count\n1,tag,0,1\n").is_none());
    }

    #[test]
    fn windows_cover_the_whole_side() {
        assert_eq!(starts(500, 640, 480), vec![0]);
        assert_eq!(starts(640, 640, 480), vec![0]);
        let s = starts(4000, 640, 480);
        assert_eq!(*s.first().unwrap(), 0);
        assert_eq!(*s.last().unwrap(), 4000 - 640);
        assert!(s.windows(2).all(|p| p[1] - p[0] <= 480), "{s:?}");
        // Sections only for large pictures, inside the picture, covering both ends.
        assert!(sections((2048, 1024)).is_empty());
        assert!(sections((768, 256)).is_empty());
        for dims in [
            (4096, 4096),
            (4096, 256),
            (8192, 6144),
            (3000, 2100),
            (2048, 256),
        ] {
            let secs = sections(dims);
            assert!(!secs.is_empty(), "{dims:?}");
            assert!(secs
                .iter()
                .all(|&(x, y, w, h)| x + w <= dims.0 && y + h <= dims.1));
            assert!(secs.iter().any(|&(x, y, _, _)| x == 0 && y == 0));
            assert!(secs
                .iter()
                .any(|&(x, y, w, h)| x + w == dims.0 && y + h == dims.1));
        }
    }

    #[test]
    fn a_small_confident_box_does_not_hide_a_face_that_counts() {
        let face = |score: f32, side: f32| Face {
            score,
            side,
            ..Default::default()
        };
        let found = vec![
            (face(0.95, 10.0), [100.0, 100.0, 10.0, 10.0]),
            (face(0.7, 16.0), [97.0, 97.0, 16.0, 16.0]),
        ];
        assert_eq!(original_boxes(found), vec![[97.0, 97.0, 16.0, 16.0]]);
    }

    #[test]
    fn rotate_any_keeps_the_picture_and_turns_it() {
        // A white dot right of the centre ends up below it after a quarter turn clockwise.
        let mut img = RgbImage::new(41, 41);
        img.put_pixel(35, 20, image::Rgb([255, 255, 255]));
        let r = rotate_any(&img, 90.0);
        assert_eq!(r.dimensions(), (41, 41));
        let (x, y) = r
            .enumerate_pixels()
            .find(|p| p.2[0] > 0)
            .map(|p| (p.0, p.1))
            .unwrap();
        assert!((19..=21).contains(&x) && (34..=36).contains(&y), "{x},{y}");
        let d = rotate_any(&RgbImage::new(100, 50), 45.0);
        assert!(d.width() >= 106 && d.height() >= 106);
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

/// Overlap of two (x, y, w, h) boxes as a share of the smaller one.
fn overlap(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let w = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
    let h = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
    if w <= 0.0 || h <= 0.0 {
        return 0.0;
    }
    let smaller = (a[2] * a[3]).min(b[2] * b[3]).max(1.0);
    w * h / smaller
}
