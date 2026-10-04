//! In-memory image store (SPEC §4): generated and imported
//! images live here until Save or Reset.
//!
//! Nothing here touches disk except [`save_image`] / [`save_image_as`], which
//! run only on an explicit user click. Saved file names are never derived from
//! prompts: `pinhole_YYYYMMDD_HHMMSS_<seed>.png`.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;
use pinhole_engine::image::{self as img, Kind};
use pinhole_engine::provenance::{self, AiLabel};

use crate::generate::{ImportedImage, Origin, ResultImage, SavedBatch, SavedEntry, SavedImage};
use crate::imagecheck::CheckedPng;
use crate::{AppCore, CoreError, CoreResult};

/// One image held in RAM.
#[derive(Debug, Clone)]
pub struct SessionImage {
    pub id: String,
    /// Always a PNG: generated images with text chunks scrubbed, imported
    /// PNGs scrubbed, imported JPEG/WebP re-encoded (EXIF/XMP dropped).
    pub bytes: Arc<Vec<u8>>,
    pub kind: Kind,
    pub width: u32,
    pub height: u32,
    /// Generation settings (no prompt) for generated / upscaled images.
    pub meta: Option<ResultImage>,
    /// Made in Pinhole or brought in (RELEASE-SPEC §3.1). Held in memory only.
    pub origin: Origin,
    /// The brought-in pictures a generated image was made from, through every step
    /// (the image check compares results with them). Empty for imported images: they
    /// are their own source (see [`SessionImage::sources`]), unless the file is one
    /// Pinhole saved earlier in this session: then it keeps that picture's sources.
    pub made_from: Arc<[Source]>,
    /// A brought-in picture whose file said it was made with AI (IPTC digital source type in
    /// XMP or a C2PA manifest), read before its metadata was dropped. Written back on export;
    /// never used by the image check (anyone can write such a label onto a real photo).
    pub ai_label: Option<AiLabel>,
    /// Made with a model or add-on marked "safe images only" (RELEASE-SPEC §3.2 rule 3), or
    /// from a picture that was. A saved one opened again in the same session keeps it.
    pub safe_images_only: bool,
}

/// A brought-in picture at the start of a chain of edits. Its bytes stay with every
/// image made from it, so discarding the original doesn't lose it for the check.
#[derive(Debug, Clone)]
pub struct Source {
    pub id: String,
    pub bytes: Arc<Vec<u8>>,
    /// See [`SessionImage::ai_label`].
    pub ai_label: Option<AiLabel>,
}

impl SessionImage {
    pub fn parent_id(&self) -> Option<&str> {
        self.meta.as_ref().and_then(|m| m.parent_id.as_deref())
    }

    /// The brought-in pictures this image comes from: itself when it was brought in
    /// (a saved picture opened again: what that picture came from).
    pub fn sources(&self) -> Vec<Source> {
        if self.meta.is_none() && self.origin == Origin::Imported && self.made_from.is_empty() {
            vec![Source {
                id: self.id.clone(),
                bytes: self.bytes.clone(),
                ai_label: self.ai_label,
            }]
        } else if self.meta.is_none() {
            // A saved picture opened again: its own file's label (what Pinhole wrote on it)
            // counts for every picture it came from.
            self.made_from
                .iter()
                .map(|s| Source {
                    ai_label: provenance::strongest(s.ai_label, self.ai_label),
                    ..s.clone()
                })
                .collect()
        } else {
            self.made_from.to_vec()
        }
    }

    /// This image as a fed-in picture for the image check, when it was made from a
    /// brought-in chain: a face can show up in it (enlarged, straightened, sharpened) that
    /// the brought-in picture didn't show clearly. `None` for brought-in pictures and
    /// pictures made from scratch.
    pub fn fed_in_source(&self) -> Option<Source> {
        let from = self.sources();
        (!from.is_empty() && !from.iter().any(|s| s.id == self.id)).then(|| Source {
            id: self.id.clone(),
            bytes: self.bytes.clone(),
            ai_label: None,
        })
    }
}

/// The session store. Cheap clones of image bytes via `Arc`.
#[derive(Default)]
pub struct Session {
    images: RwLock<HashMap<String, SessionImage>>,
    /// Bumped by every [`Session::clear`] (Reset), under the write lock.
    epoch: AtomicU64,
}

impl Session {
    /// Result intake: store a (scrubbed) generated PNG that passed the image check under
    /// `meta.id`, but only while no Reset happened since [`Session::epoch`] returned `epoch` (a
    /// job that finishes after Reset must not bring its images back). Returns whether it was
    /// stored. The only way a made picture gets into the session: it takes nothing but a
    /// [`CheckedPng`].
    pub fn insert_generated(&self, epoch: u64, checked: CheckedPng, meta: ResultImage) -> bool {
        let (png, made_from, safe_images_only) = checked.into_parts();
        let mut images = self.images.write();
        if self.epoch.load(Ordering::SeqCst) != epoch {
            return false;
        }
        let img = SessionImage {
            id: meta.id.clone(),
            bytes: Arc::new(png),
            kind: Kind::Png,
            width: meta.width,
            height: meta.height,
            origin: meta.origin,
            meta: Some(meta),
            made_from,
            ai_label: None,
            safe_images_only,
        };
        images.insert(img.id.clone(), img);
        true
    }

    /// Changes whenever the session is cleared (Reset).
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    /// Brought-in pictures only ([`import_image`]); made pictures go through
    /// [`Session::insert_generated`].
    fn insert_imported(&self, img: SessionImage) {
        debug_assert!(img.meta.is_none() && img.origin == Origin::Imported);
        self.images.write().insert(img.id.clone(), img);
    }

    pub fn get(&self, id: &str) -> Option<SessionImage> {
        self.images.read().get(id).cloned()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.images.read().contains_key(id)
    }

    pub fn remove(&self, id: &str) -> bool {
        self.images.write().remove(id).is_some()
    }

    pub fn clear(&self) {
        let mut images = self.images.write();
        self.epoch.fetch_add(1, Ordering::SeqCst);
        images.clear();
    }

    pub fn len(&self) -> usize {
        self.images.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Total bytes held (for diagnostics / tests).
    pub fn bytes_held(&self) -> usize {
        self.images.read().values().map(|i| i.bytes.len()).sum()
    }
}

// ---------------------------------------------------------------- service fns

/// Put user-provided image bytes (PNG/JPEG/WebP) into the session. Rejects
/// files over 64 MB or 50 megapixels (from the header, before decoding). Every
/// session image is a PNG without metadata: PNG text/eXIf chunks are dropped
/// (they may hold someone's prompt or location), JPEG/WebP are decoded (EXIF
/// orientation applied) and re-encoded, which drops EXIF / XMP (GPS, camera). Only an AI-origin
/// label the file carries is kept (see [`SessionImage::ai_label`]).
pub fn import_image(core: &AppCore, bytes: Vec<u8>) -> CoreResult<ImportedImage> {
    let info = img::sniff(&bytes).map_err(|e| CoreError::invalid(e.to_string()))?;
    let ai_label = provenance::ai_label(&bytes);
    let (made_from, safe_images_only) = core.check.exported_from(&bytes).map_or_else(
        || (Arc::from(Vec::new()), false),
        |e| (e.made_from, e.safe_images_only),
    );
    let (bytes, width, height) = if info.kind == Kind::Png {
        (
            pinhole_engine::png::scrub(&bytes)
                .map_err(|_| CoreError::invalid("The PNG file is damaged."))?,
            info.width,
            info.height,
        )
    } else {
        img::reencode_png(&bytes).map_err(|e| {
            CoreError::invalid("This image couldn't be read. Try saving it as PNG first.")
                .with_details(e.to_string())
        })?
    };
    let id = uuid::Uuid::new_v4().to_string();
    core.session.insert_imported(SessionImage {
        id: id.clone(),
        bytes: Arc::new(bytes),
        kind: Kind::Png,
        width,
        height,
        meta: None,
        origin: Origin::Imported,
        made_from,
        ai_label,
        safe_images_only,
    });
    Ok(ImportedImage { id, width, height })
}

/// Bytes of a session image (PNG for generated images).
pub fn get(core: &AppCore, id: &str) -> CoreResult<Arc<Vec<u8>>> {
    core.session.get(id).map(|i| i.bytes).ok_or_else(missing)
}

pub fn discard(core: &AppCore, id: &str) {
    core.session.remove(id);
    core.check.forget_image(id);
}

/// Reset: drop every image immediately (and the engines' output
/// buffers), and stop sd-server if it ran a job — it keeps finished results
/// for 600 s behind an unauthenticated API (`crate::engine::IDLE_STOP_AFTER`). The
/// next Generate reloads the model. If a job is running, the engine stops
/// right after it.
pub async fn clear(core: &AppCore) {
    core.session.clear();
    core.check.forget();
    core.gen.logs.clear();
    core.describe.logs.clear();
    crate::engine::clear_engine_results(core).await;
}

/// RGBA8 pixels for the clipboard: the export's pixels (watermarked like [`export_png`];
/// a clipboard picture carries no metadata).
pub fn decode_rgba(core: &AppCore, id: &str) -> CoreResult<(Vec<u8>, u32, u32)> {
    let im = core.session.get(id).ok_or_else(missing)?;
    marked_pixels(&im)
}

fn missing() -> CoreError {
    CoreError::not_found("That image isn't in this session anymore.")
}

/// Settings-only metadata (never prompt/negative/style text).
fn settings_text(m: &ResultImage) -> String {
    serde_json::json!({
        "app": "Pinhole",
        "model": m.model_label,
        "modelId": m.model_id,
        "family": m.family_id,
        "seed": m.seed,
        "steps": m.steps,
        "cfg": m.cfg,
        "guidance": m.guidance,
        "sampler": m.sampler,
        "scheduler": m.scheduler,
        "width": m.base_size.map_or(m.width, |s| s.0),
        "height": m.base_size.map_or(m.height, |s| s.1),
        "seamless": m.seamless,
    })
    .to_string()
}

/// Keyword of the optional "settings (no prompt)" text chunk.
const SETTINGS_KEYWORD: &str = "pinhole";
/// The settings chunk is a few hundred bytes; anything bigger isn't ours.
const MAX_SETTINGS_CHUNK: usize = 4096;

/// What a picture saved with "Settings (no prompt)" says about how it was made (see
/// [`settings_text`]). Read back so the user can reuse them; never contains a prompt.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PictureSettings {
    pub model: Option<String>,
    pub model_id: Option<String>,
    pub family: Option<String>,
    pub seed: Option<i64>,
    pub steps: Option<u32>,
    pub cfg: Option<f32>,
    pub guidance: Option<f32>,
    pub sampler: Option<String>,
    pub scheduler: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Made with "Repeats without seams". `None` for pictures saved before it was recorded.
    pub seamless: Option<bool>,
}

fn plain_text(v: &serde_json::Value) -> Option<String> {
    let s = v.as_str()?.trim();
    (!s.is_empty() && s.chars().count() <= 120 && !s.chars().any(char::is_control))
        .then(|| s.to_string())
}

fn number_in(v: &serde_json::Value, lo: f64, hi: f64) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite() && *n >= lo && *n <= hi)
}

/// Read the settings chunk of a PNG Pinhole saved. `None` for any other file (no chunk, not a
/// PNG, not ours, damaged). Only the known fields are taken, each range-checked: a file from
/// somewhere else can carry anything in a chunk with this name. The picture is not kept.
pub fn read_picture_settings(bytes: &[u8]) -> Option<PictureSettings> {
    let chunk = pinhole_engine::png::text_chunks(bytes)
        .into_iter()
        .find(|(kind, data)| {
            kind == "tEXt"
                && data.len() <= MAX_SETTINGS_CHUNK
                && data.starts_with(SETTINGS_KEYWORD.as_bytes())
                && data.get(SETTINGS_KEYWORD.len()) == Some(&0)
        })?
        .1;
    // tEXt is Latin-1; the writer replaced anything else with `?`.
    let text: String = chunk[SETTINGS_KEYWORD.len() + 1..]
        .iter()
        .map(|b| *b as char)
        .collect();
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    if v.get("app")?.as_str()? != "Pinhole" {
        return None;
    }
    let get = |k: &str| v.get(k).unwrap_or(&serde_json::Value::Null);
    // Sizes beyond the Fine-tune range are an upscale's size, not a Create size.
    let size = |k: &str| number_in(get(k), 64.0, 4096.0).map(|n| n as u32);
    let (width, height) = match (size("width"), size("height")) {
        (Some(w), Some(h)) => (Some(w), Some(h)),
        _ => (None, None),
    };
    let out = PictureSettings {
        model: plain_text(get("model")),
        model_id: plain_text(get("modelId")),
        family: plain_text(get("family")),
        seed: number_in(get("seed"), 0.0, 9.0e15).map(|n| n as i64),
        steps: number_in(get("steps"), 1.0, 150.0).map(|n| n as u32),
        cfg: number_in(get("cfg"), 0.0, 30.0).map(|n| n as f32),
        guidance: number_in(get("guidance"), 0.0, 30.0).map(|n| n as f32),
        sampler: plain_text(get("sampler")),
        scheduler: plain_text(get("scheduler")),
        width,
        height,
        seamless: get("seamless").as_bool(),
    };
    (out != PictureSettings::default()).then_some(out)
}

/// IPTC digital source type for an image Pinhole made (RELEASE-SPEC §2).
const SOURCE_GENERATED: &str =
    "http://cv.iptc.org/newscodes/digitalsourcetype/trainedAlgorithmicMedia";
/// ... and for one made from a picture the user brought in.
const SOURCE_COMPOSITE: &str =
    "http://cv.iptc.org/newscodes/digitalsourcetype/compositeWithTrainedAlgorithmicMedia";

/// ... and for a picture the user brought in that was only upscaled.
const SOURCE_ENHANCED: &str =
    "http://cv.iptc.org/newscodes/digitalsourcetype/algorithmicallyEnhanced";

fn label_source(l: AiLabel) -> &'static str {
    match l {
        AiLabel::Generated => SOURCE_GENERATED,
        AiLabel::Composite => SOURCE_COMPOSITE,
        AiLabel::Enhanced => SOURCE_ENHANCED,
    }
}

/// The IPTC digital source type a picture leaves with, or `None` for a brought-in picture that
/// didn't say it was made with AI. A label read from a brought-in file is carried forward and never
/// weakened: on the unchanged picture as it was; on anything made only from pictures labelled as
/// made entirely with AI as "made with AI" (not "edited photo" or "enhanced photo", which would
/// claim a real capture behind it); on an upscale of a picture with AI-made parts as a composite.
pub fn source_type(im: &SessionImage) -> Option<&'static str> {
    let Some(m) = &im.meta else {
        return im.ai_label.map(label_source);
    };
    if im.origin == Origin::Generated {
        return Some(SOURCE_GENERATED);
    }
    let labels: Vec<Option<AiLabel>> = im.sources().iter().map(|s| s.ai_label).collect();
    let all =
        |want: fn(Option<AiLabel>) -> bool| !labels.is_empty() && labels.iter().all(|l| want(*l));
    let ai_parts = labels
        .iter()
        .any(|l| matches!(l, Some(AiLabel::Generated | AiLabel::Composite)));
    Some(if all(|l| l == Some(AiLabel::Generated)) {
        SOURCE_GENERATED
    } else if upscaled_import(m) && !ai_parts {
        SOURCE_ENHANCED
    } else {
        SOURCE_COMPOSITE
    })
}

/// The AI-generated marker (RELEASE-SPEC §2, EU AI Act Art. 50): XMP with only
/// the IPTC digital source type ("made with AI"). No app name,
/// prompt, seed, model, user or machine.
pub fn ai_marker_xmp(source: &str) -> String {
    format!(
        concat!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">"#,
            r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">"#,
            r#"<rdf:Description rdf:about="""#,
            r#" xmlns:Iptc4xmpExt="http://iptc.org/std/Iptc4xmpExt/2008-02-29/""#,
            r#" Iptc4xmpExt:DigitalSourceType="{source}"/>"#,
            r#"</rdf:RDF></x:xmpmeta>"#
        ),
        source = source,
    )
}

/// An upscale (of an upscale…) of a picture the user brought in: no model ever ran on it.
fn upscaled_import(m: &ResultImage) -> bool {
    m.kind == crate::generate::ResultKind::Upscaled && m.model_id.is_empty()
}

/// Decoded pixels of a session image, watermarked when Pinhole made it (RELEASE-SPEC §2).
fn marked_pixels(im: &SessionImage) -> CoreResult<(Vec<u8>, u32, u32)> {
    let (mut rgba, w, h) = img::decode_rgba(&im.bytes).map_err(|e| {
        CoreError::internal("The image in memory is damaged.").with_details(e.to_string())
    })?;
    if im.meta.is_some() {
        pinhole_engine::watermark::embed(&mut rgba, w, h);
    }
    Ok((rgba, w, h))
}

/// Export (RELEASE-SPEC §1 item 4): the one function behind Save, Save as, Save as one sheet
/// and Copy. Images made by Pinhole (generated, edited, upscaled) always get the
/// AI-generated marker (invisible watermark + XMP) (there is no setting for it) plus the optional
/// "settings (no prompt)" chunk. A picture the user added and didn't change
/// leaves as it came in (already scrubbed at import). Several pictures make one sheet (see
/// [`sheet_bytes`]).
pub fn export_png(core: &AppCore, ims: &[SessionImage]) -> CoreResult<Vec<u8>> {
    let bytes = match ims {
        [] => return Err(missing()),
        [im] => export_bytes(core, im)?,
        _ => sheet_bytes(ims)?,
    };
    let sources = ims.iter().flat_map(|im| im.sources()).collect();
    let safe_images_only = ims.iter().any(|im| im.safe_images_only);
    core.check.note_export(&bytes, sources, safe_images_only);
    Ok(bytes)
}

/// Most pictures on one sheet.
pub const SHEET_MAX: usize = 8;

/// "Save as one sheet": the pictures in a grid ([`img::sheet`]), each one watermarked like its
/// own export first. The sheet is watermarked again when it carries a marker, and carries
/// the XMP marker: their shared source type, or "composite" when they differ. No settings chunk
/// (each picture has its own settings).
fn sheet_bytes(ims: &[SessionImage]) -> CoreResult<Vec<u8>> {
    if ims.len() > SHEET_MAX {
        return Err(CoreError::invalid(format!(
            "A sheet takes up to {SHEET_MAX} pictures."
        )));
    }
    let types: Vec<Option<&str>> = ims.iter().map(source_type).collect();
    let source = if types.iter().all(|t| *t == types[0]) {
        types[0]
    } else {
        types
            .iter()
            .any(Option::is_some)
            .then_some(SOURCE_COMPOSITE)
    };
    // One picture decoded at a time: each is placed (and shrunk) before the next is read.
    let sizes: Vec<(u32, u32)> = ims.iter().map(|im| (im.width, im.height)).collect();
    let (mut rgba, w, h) = img::sheet_with(&sizes, ims.iter().map(marked_pixels))?;
    // Resizing can wash out a picture's own watermark (a saved one opened again carries one
    // in its pixels only), so any sheet with a marker gets one of its own.
    if source.is_some() {
        pinhole_engine::watermark::embed(&mut rgba, w, h);
    }
    let png = img::encode_png_rgba(&rgba, w, h).map_err(|e| {
        CoreError::internal("Couldn't prepare the sheet.").with_details(e.to_string())
    })?;
    match source {
        Some(source) => {
            pinhole_engine::png::add_itxt_chunk(&png, "XML:com.adobe.xmp", &ai_marker_xmp(source))
                .map_err(|_| CoreError::internal("Couldn't prepare the sheet."))
        }
        None => Ok(png),
    }
}

/// "Save as one sheet" to a user-chosen path (from the save dialog): 2 to [`SHEET_MAX`] session
/// pictures, in order, as one PNG.
pub fn save_sheet_as(core: &AppCore, ids: &[String], path: &str) -> CoreResult<SavedImage> {
    if ids.len() < 2 {
        return Err(CoreError::invalid(
            "Pick at least two pictures for a sheet.",
        ));
    }
    let ims = ids
        .iter()
        .map(|id| core.session.get(id).ok_or_else(missing))
        .collect::<CoreResult<Vec<_>>>()?;
    let mut path = PathBuf::from(path);
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(CoreError::invalid(
            "Pick a folder and file name to save to.",
        ));
    }
    let png = path
        .extension()
        .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("png"));
    if !png {
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(".png");
        path.set_file_name(name);
        if path.exists() {
            return Err(CoreError::invalid(format!(
                "A file named “{}” is already there. Pick another name.",
                path.file_name().unwrap_or_default().to_string_lossy()
            )));
        }
    }
    let bytes = export_png(core, &ims)?;
    write_file(&path, &bytes)?;
    Ok(SavedImage {
        path: path.to_string_lossy().into_owned(),
    })
}

fn export_bytes(core: &AppCore, im: &SessionImage) -> CoreResult<Vec<u8>> {
    let Some(m) = &im.meta else {
        // A brought-in picture, unchanged: as it came in (already scrubbed), plus the AI label
        // its file carried.
        return match source_type(im) {
            Some(source) => pinhole_engine::png::add_itxt_chunk(
                &im.bytes,
                "XML:com.adobe.xmp",
                &ai_marker_xmp(source),
            )
            .map_err(|_| CoreError::internal("The image in memory is damaged.")),
            None => Ok(im.bytes.as_ref().clone()),
        };
    };
    let damaged = |_| CoreError::internal("The image in memory is damaged.");
    // Layer 1: the invisible pixel watermark (survives screenshots and re-saving). The PNG
    // is re-encoded from the watermarked pixels, which also drops every other chunk.
    let (rgba, w, h) = marked_pixels(im)?;
    let clean = img::encode_png_rgba(&rgba, w, h).map_err(|e| {
        CoreError::internal("Couldn't prepare the image.").with_details(e.to_string())
    })?;
    // Layer 2: the XMP marker.
    let marked = pinhole_engine::png::add_itxt_chunk(
        &clean,
        "XML:com.adobe.xmp",
        &ai_marker_xmp(source_type(im).unwrap_or(SOURCE_GENERATED)),
    )
    .map_err(damaged)?;
    if core.settings.read().saved_metadata == "settings" {
        pinhole_engine::png::add_text_chunk(&marked, "pinhole", &settings_text(m))
            .map_err(|_| CoreError::internal("Couldn't add the settings to the image."))
    } else {
        Ok(marked)
    }
}

/// Save into the Save folder (`Pictures/Pinhole` by default, `Data/outputs/` for a
/// portable copy, or the one picked in Settings) as `pinhole_YYYYMMDD_HHMMSS_<seed>.<ext>`
/// (unique suffix `_2`, `_3`… on collision). Imported images use `import` instead of a seed.
pub fn save_image(core: &AppCore, id: &str) -> CoreResult<SavedImage> {
    // Missing picture first: no folder is created for nothing.
    core.session.get(id).ok_or_else(missing)?;
    save_into(core, id, &crate::app::save_folder_for_write(core)?)
}

/// "Save all": every listed image into a folder the user picked, named like
/// `save_image`. One failure doesn't stop the rest; the caller sees which were saved.
pub fn save_images_to(core: &AppCore, ids: &[String], dir: &str) -> CoreResult<SavedBatch> {
    let dir = PathBuf::from(dir);
    if !dir.is_absolute() || !dir.is_dir() {
        return Err(CoreError::invalid("Pick an existing folder to save to."));
    }
    let mut saved = Vec::new();
    let mut failed = 0;
    let mut first_err = None;
    for id in ids {
        match save_into(core, id, &dir) {
            Ok(s) => saved.push(SavedEntry {
                id: id.clone(),
                path: s.path,
            }),
            Err(e) => {
                failed += 1;
                first_err.get_or_insert(e);
            }
        }
    }
    if saved.is_empty() {
        if let Some(e) = first_err {
            return Err(e);
        }
    }
    Ok(SavedBatch { saved, failed })
}

fn save_into(core: &AppCore, id: &str, dir: &Path) -> CoreResult<SavedImage> {
    let im = core.session.get(id).ok_or_else(missing)?;
    let bytes = export_png(core, std::slice::from_ref(&im))?;
    fs::create_dir_all(dir).map_err(|e| io_err(dir, e))?;
    let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let tag = im
        .meta
        .as_ref()
        .map(|m| m.seed.to_string())
        .unwrap_or_else(|| "import".into());
    let base = format!("pinhole_{stamp}_{tag}");
    let ext = im.kind.ext();
    for n in 1..10_000u32 {
        let name = if n == 1 {
            format!("{base}.{ext}")
        } else {
            format!("{base}_{n}.{ext}")
        };
        let path = dir.join(name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                if let Err(e) = f.write_all(&bytes).and_then(|_| f.sync_all()) {
                    drop(f);
                    let _ = fs::remove_file(&path);
                    return Err(io_err(&path, e));
                }
                return Ok(SavedImage {
                    path: path.to_string_lossy().into_owned(),
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io_err(&path, e)),
        }
    }
    Err(CoreError::new(
        "io",
        "Couldn't find a free file name in that folder.",
    ))
}

/// Save to a user-chosen path (from the save dialog). A missing or wrong
/// extension gets the right one appended.
pub fn save_image_as(core: &AppCore, id: &str, path: &str) -> CoreResult<SavedImage> {
    let im = core.session.get(id).ok_or_else(missing)?;
    let mut path = PathBuf::from(path);
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(CoreError::invalid(
            "Pick a folder and file name to save to.",
        ));
    }
    let ext_ok = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|e| match im.kind {
            Kind::Png => e == "png",
            Kind::Jpeg => e == "jpg" || e == "jpeg",
            Kind::Webp => e == "webp",
        });
    if !ext_ok {
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(format!(".{}", im.kind.ext()));
        path.set_file_name(name);
        // The save dialog only asked about the name as typed, not this one.
        if path.exists() {
            return Err(CoreError::invalid(format!(
                "A file named “{}” is already there. Pick another name.",
                path.file_name().unwrap_or_default().to_string_lossy()
            )));
        }
    }
    let bytes = export_png(core, std::slice::from_ref(&im))?;
    write_file(&path, &bytes)?;
    Ok(SavedImage {
        path: path.to_string_lossy().into_owned(),
    })
}

fn write_file(path: &Path, bytes: &[u8]) -> CoreResult<()> {
    let dir = path
        .parent()
        .ok_or_else(|| CoreError::invalid("Pick a folder to save to."))?;
    let tmp = dir.join(format!(".pinhole-save-{}.tmp", uuid::Uuid::new_v4()));
    let res = (|| -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        // Replaces an existing file in one step on every platform (MoveFileExW with
        // MOVEFILE_REPLACE_EXISTING on Windows): a failed save leaves the old file as it was.
        fs::rename(&tmp, path)
    })();
    if let Err(e) = res {
        let _ = fs::remove_file(&tmp);
        return Err(io_err(path, e));
    }
    Ok(())
}

fn io_err(path: &Path, e: std::io::Error) -> CoreError {
    CoreError::new(
        "io",
        format!(
            "Couldn't save to {}. Check the folder exists and you can write to it.",
            path.display()
        ),
    )
    .with_details(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_over_a_file_replaces_it_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        fs::write(&path, b"old").unwrap();
        write_file(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    fn meta(id: &str) -> ResultImage {
        ResultImage {
            id: id.into(),
            kind: crate::generate::ResultKind::Generated,
            width: 4,
            height: 4,
            seed: 1234,
            model_id: "m".into(),
            model_label: "Model".into(),
            family_id: "sdxl".into(),
            steps: 20,
            cfg: 6.0,
            guidance: None,
            sampler: Some("euler".into()),
            scheduler: None,
            parent_id: Some("p".into()),
            origin: Origin::Generated,
            upscaler: None,
            seamless: false,
            base_size: None,
        }
    }

    #[test]
    fn save_all_writes_every_image_to_the_chosen_folder() {
        let (tmp, core) =
            crate::app::tests::test_core(Arc::new(crate::app::tests::Recorder::default()));
        let png = img::encode_png_rgba(&[9; 64], 4, 4).unwrap();
        let epoch = core.session.epoch();
        assert!(core.session.insert_generated(
            epoch,
            CheckedPng::unchecked_for_tests(png.clone()),
            meta("a")
        ));
        assert!(core.session.insert_generated(
            epoch,
            CheckedPng::unchecked_for_tests(png),
            meta("b")
        ));
        let dir = tmp.path().join("picked");
        fs::create_dir_all(&dir).unwrap();
        let ids = vec!["a".to_string(), "b".to_string(), "gone".to_string()];
        let batch = save_images_to(&core, &ids, dir.to_str().unwrap()).unwrap();
        assert_eq!((batch.saved.len(), batch.failed), (2, 1));
        assert_eq!(batch.saved[0].id, "a");
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names.len(),
            2,
            "same seed and second still get unique names: {names:?}"
        );
        assert!(names
            .iter()
            .all(|n| n.starts_with("pinhole_") && n.ends_with(".png")));
        // Nothing saved, or a folder that isn't there: an error, not a silent no-op.
        assert!(save_images_to(&core, &["gone".to_string()], dir.to_str().unwrap()).is_err());
        assert!(save_images_to(&core, &ids, tmp.path().join("missing").to_str().unwrap()).is_err());
    }

    #[test]
    fn store_roundtrip_and_clear() {
        let s = Session::default();
        let png = img::encode_png_rgba(&[9; 64], 4, 4).unwrap();
        assert!(s.insert_generated(
            s.epoch(),
            CheckedPng::unchecked_for_tests(png.clone()),
            meta("a")
        ));
        let got = s.get("a").unwrap();
        assert_eq!(got.bytes.as_slice(), png.as_slice());
        assert_eq!(got.parent_id(), Some("p"));
        assert_eq!(s.len(), 1);
        assert!(s.remove("a"));
        assert!(s.get("a").is_none());
        assert!(s.insert_generated(s.epoch(), CheckedPng::unchecked_for_tests(png), meta("b")));
        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn results_from_before_a_reset_are_dropped() {
        let s = Session::default();
        let png = img::encode_png_rgba(&[9; 64], 4, 4).unwrap();
        let epoch = s.epoch();
        assert!(s.insert_generated(
            epoch,
            CheckedPng::unchecked_for_tests(png.clone()),
            meta("a")
        ));
        s.clear();
        assert!(
            !s.insert_generated(
                epoch,
                CheckedPng::unchecked_for_tests(png.clone()),
                meta("b")
            ),
            "Reset happened since the job started"
        );
        assert!(s.is_empty());
        assert!(s.insert_generated(s.epoch(), CheckedPng::unchecked_for_tests(png), meta("c")));
    }

    #[test]
    fn settings_text_has_no_prompt_fields() {
        let t = settings_text(&meta("x"));
        let v: serde_json::Value = serde_json::from_str(&t).unwrap();
        for k in ["prompt", "negative", "negativePrompt", "style"] {
            assert!(v.get(k).is_none(), "{k}");
        }
        assert_eq!(v["seed"], 1234);
    }

    fn saved_with_settings(m: &ResultImage) -> Vec<u8> {
        let png = img::encode_png_rgba(&[255, 0, 0, 255].repeat(4), 2, 2).unwrap();
        pinhole_engine::png::add_text_chunk(&png, "pinhole", &settings_text(m)).unwrap()
    }

    #[test]
    fn settings_read_back_from_a_saved_picture() {
        let mut m = meta("x");
        m.sampler = Some("euler_a".into());
        m.scheduler = None;
        (m.width, m.height) = (832, 1216);
        let s = read_picture_settings(&saved_with_settings(&m)).expect("settings");
        assert_eq!(s.seed, Some(1234));
        assert_eq!(s.model_id.as_deref(), Some(m.model_id.as_str()));
        assert_eq!(s.model.as_deref(), Some(m.model_label.as_str()));
        assert_eq!(s.sampler.as_deref(), Some("euler_a"));
        assert_eq!(s.scheduler, None);
        assert_eq!((s.width, s.height), (Some(m.width), Some(m.height)));

        // After hires fix or an upscale: the size it was made at, not the final size.
        (m.width, m.height) = (2048, 2048);
        m.base_size = Some((1024, 1024));
        let s = read_picture_settings(&saved_with_settings(&m)).expect("settings");
        assert_eq!((s.width, s.height), (Some(1024), Some(1024)));
        assert_eq!(s.seamless, Some(false));

        // "Repeats without seams" is recorded, so reusing the settings turns it back on.
        m.seamless = true;
        let s = read_picture_settings(&saved_with_settings(&m)).expect("settings");
        assert_eq!(s.seamless, Some(true));
        // Pictures saved before it was recorded don't say.
        let png = img::encode_png_rgba(&[255, 0, 0, 255].repeat(4), 2, 2).unwrap();
        let older =
            pinhole_engine::png::add_text_chunk(&png, "pinhole", r#"{"app":"Pinhole","seed":5}"#)
                .unwrap();
        assert_eq!(read_picture_settings(&older).unwrap().seamless, None);
    }

    #[test]
    fn save_as_never_replaces_a_file_under_an_added_extension() {
        let (tmp, core) =
            crate::app::tests::test_core(Arc::new(crate::app::tests::Recorder::default()));
        let png = img::encode_png_rgba(&[9; 64], 4, 4).unwrap();
        let epoch = core.session.epoch();
        assert!(core.session.insert_generated(
            epoch,
            CheckedPng::unchecked_for_tests(png),
            meta("a")
        ));
        let existing = tmp.path().join("cat.png");
        fs::write(&existing, b"earlier picture").unwrap();
        let typed = tmp.path().join("cat");
        let err = save_image_as(&core, "a", typed.to_str().unwrap()).unwrap_err();
        assert_eq!(err.code, "invalid");
        assert_eq!(fs::read(&existing).unwrap(), b"earlier picture");
        // The name as picked (the dialog asked about it) is replaced.
        save_image_as(&core, "a", existing.to_str().unwrap()).unwrap();
        assert_ne!(fs::read(&existing).unwrap(), b"earlier picture");
    }

    #[test]
    fn a_picture_without_the_chunk_or_not_ours_gives_nothing() {
        let plain = img::encode_png_rgba(&[0, 0, 0, 255].repeat(4), 2, 2).unwrap();
        assert!(read_picture_settings(&plain).is_none());
        assert!(read_picture_settings(b"not a png").is_none());
        let other =
            pinhole_engine::png::add_text_chunk(&plain, "pinhole", r#"{"app":"Else","seed":1}"#)
                .unwrap();
        assert!(read_picture_settings(&other).is_none());
        let junk = pinhole_engine::png::add_text_chunk(&plain, "pinhole", "{oops").unwrap();
        assert!(read_picture_settings(&junk).is_none());
        // A chunk with another name (like A1111's "parameters", which holds prompts) is never read.
        let params = pinhole_engine::png::add_text_chunk(
            &plain,
            "parameters",
            r#"{"app":"Pinhole","seed":1}"#,
        )
        .unwrap();
        assert!(read_picture_settings(&params).is_none());
    }

    #[test]
    fn out_of_range_values_are_dropped_and_extra_fields_ignored() {
        let plain = img::encode_png_rgba(&[0, 0, 0, 255].repeat(4), 2, 2).unwrap();
        let body = serde_json::json!({
            "app": "Pinhole", "seed": 7, "steps": 100000, "cfg": -3, "width": 9000, "height": 512,
            "sampler": "bad\nname", "model": "Good model", "prompt": "SECRET words",
        })
        .to_string();
        let png = pinhole_engine::png::add_text_chunk(&plain, "pinhole", &body).unwrap();
        let s = read_picture_settings(&png).unwrap();
        assert_eq!(s.seed, Some(7));
        assert_eq!(
            (s.steps, s.cfg, s.width, s.height),
            (None, None, None, None)
        );
        assert_eq!(s.sampler, None);
        assert_eq!(s.model.as_deref(), Some("Good model"));
        assert!(!format!("{s:?}").contains("SECRET"));
    }

    #[test]
    fn ai_marker_says_made_with_ai_and_nothing_else() {
        let gen = ai_marker_xmp(SOURCE_GENERATED);
        assert!(
            gen.contains("digitalsourcetype/trainedAlgorithmicMedia\""),
            "{gen}"
        );
        assert!(!gen.contains("Pinhole") && !gen.contains(env!("CARGO_PKG_VERSION")));
        let mut m = meta("a");
        assert!(!upscaled_import(&m));
        m.kind = crate::generate::ResultKind::Upscaled;
        assert!(!upscaled_import(&m), "upscale of a generated picture");
        m.model_id.clear();
        assert!(upscaled_import(&m));
    }

    fn source(id: &str, ai_label: Option<AiLabel>) -> Source {
        Source {
            id: id.into(),
            bytes: Arc::new(Vec::new()),
            ai_label,
        }
    }

    fn image(meta: Option<ResultImage>, origin: Origin, from: Vec<Source>) -> SessionImage {
        SessionImage {
            id: "x".into(),
            bytes: Arc::new(Vec::new()),
            kind: Kind::Png,
            width: 4,
            height: 4,
            meta,
            origin,
            made_from: Arc::from(from),
            ai_label: None,
            safe_images_only: false,
        }
    }

    #[test]
    fn source_type_of_made_pictures() {
        let edit = || Some(meta("e"));
        let upscale = || {
            let mut m = meta("u");
            m.kind = crate::generate::ResultKind::Upscaled;
            m.model_id.clear();
            Some(m)
        };
        let photo = || source("p", None);
        let ai = || source("a", Some(AiLabel::Generated));
        let mixed = || source("c", Some(AiLabel::Composite));
        let t = |m, from| source_type(&image(m, Origin::Imported, from));
        assert_eq!(
            source_type(&image(edit(), Origin::Generated, vec![])),
            Some(SOURCE_GENERATED)
        );
        // From a photo, as before.
        assert_eq!(t(edit(), vec![photo()]), Some(SOURCE_COMPOSITE));
        assert_eq!(t(upscale(), vec![photo()]), Some(SOURCE_ENHANCED));
        // From pictures that said they were made with AI: made with AI, not an edited photo.
        assert_eq!(t(edit(), vec![ai()]), Some(SOURCE_GENERATED));
        assert_eq!(t(upscale(), vec![ai(), ai()]), Some(SOURCE_GENERATED));
        // Never weaker than the input said.
        assert_eq!(t(upscale(), vec![mixed()]), Some(SOURCE_COMPOSITE));
        assert_eq!(t(upscale(), vec![ai(), mixed()]), Some(SOURCE_COMPOSITE));
        let enhanced = || source("h", Some(AiLabel::Enhanced));
        assert_eq!(t(upscale(), vec![enhanced()]), Some(SOURCE_ENHANCED));
        assert_eq!(t(edit(), vec![enhanced()]), Some(SOURCE_COMPOSITE));
        // A photo in the mix: it is a composite with a real capture.
        assert_eq!(t(edit(), vec![ai(), photo()]), Some(SOURCE_COMPOSITE));
        assert_eq!(t(upscale(), vec![ai(), photo()]), Some(SOURCE_COMPOSITE));
    }

    #[test]
    fn source_type_of_unchanged_brought_in_pictures() {
        let mut im = image(None, Origin::Imported, vec![]);
        assert_eq!(source_type(&im), None, "a photo stays as it came in");
        im.ai_label = Some(AiLabel::Generated);
        assert_eq!(source_type(&im), Some(SOURCE_GENERATED));
        assert_eq!(im.sources()[0].ai_label, Some(AiLabel::Generated));
        im.ai_label = Some(AiLabel::Composite);
        assert_eq!(source_type(&im), Some(SOURCE_COMPOSITE));
        im.ai_label = Some(AiLabel::Enhanced);
        assert_eq!(source_type(&im), Some(SOURCE_ENHANCED));
    }

    /// Regression: a saved edit of a photo opened again in the same session keeps the photo as
    /// its source; an upscale of it used to leave as an "enhanced photo", losing the label its
    /// own file carried.
    #[test]
    fn a_saved_picture_opened_again_keeps_its_own_label() {
        let mut reopened = image(None, Origin::Imported, vec![source("p", None)]);
        reopened.ai_label = Some(AiLabel::Composite);
        let from = reopened.sources();
        assert_eq!(from[0].id, "p");
        assert_eq!(from[0].ai_label, Some(AiLabel::Composite));
        let mut m = meta("u");
        m.kind = crate::generate::ResultKind::Upscaled;
        m.model_id.clear();
        let upscaled = image(Some(m), Origin::Imported, from);
        assert_eq!(source_type(&upscaled), Some(SOURCE_COMPOSITE));
    }

    /// A PNG another tool marked as made with AI, with private data next to the label.
    fn labelled_png(code: &str) -> Vec<u8> {
        let png = img::encode_png_rgba(&[9; 64], 4, 4).unwrap();
        let xmp = format!(
            r#"<x:xmpmeta><rdf:Description Iptc4xmpExt:DigitalSourceType="http://cv.iptc.org/newscodes/digitalsourcetype/{code}" exif:GPSLatitude="42,26.1N"/></x:xmpmeta>"#
        );
        let png = pinhole_engine::png::add_itxt_chunk(&png, "XML:com.adobe.xmp", &xmp).unwrap();
        pinhole_engine::png::add_text_chunk(&png, "parameters", "PINHOLE_SENTINEL_7f3a").unwrap()
    }

    fn exported_xmp(bytes: &[u8]) -> Vec<String> {
        pinhole_engine::png::text_chunks(bytes)
            .into_iter()
            .map(|(k, v)| format!("{k}:{}", String::from_utf8_lossy(&v)))
            .collect()
    }

    #[test]
    fn an_ai_label_on_a_brought_in_picture_is_kept_and_nothing_else() {
        let (_tmp, core) =
            crate::app::tests::test_core(Arc::new(crate::app::tests::Recorder::default()));
        let id = import_image(&core, labelled_png("trainedAlgorithmicMedia"))
            .unwrap()
            .id;
        let im = core.session.get(&id).unwrap();
        assert_eq!(im.ai_label, Some(AiLabel::Generated));
        // The label is never trusted as a sign the picture isn't a real photo (RELEASE-SPEC §3.1):
        // it stays a brought-in picture, checked as one.
        assert_eq!(im.origin, Origin::Imported);
        assert_eq!(im.sources().len(), 1);
        let chunks = exported_xmp(&export_png(&core, std::slice::from_ref(&im)).unwrap());
        assert_eq!(chunks.len(), 1, "{chunks:?}");
        assert!(chunks[0].contains("digitalsourcetype/trainedAlgorithmicMedia\""));
        assert!(!chunks[0].contains("GPS") && !chunks[0].contains("SENTINEL"));
        // The label is the same one Pinhole writes, so it survives being opened again.
        let again = export_png(&core, std::slice::from_ref(&im)).unwrap();
        let id2 = import_image(&core, again).unwrap().id;
        assert_eq!(
            core.session.get(&id2).unwrap().ai_label,
            Some(AiLabel::Generated)
        );
    }

    #[test]
    fn a_picture_without_an_ai_label_leaves_as_it_came_in() {
        let (_tmp, core) =
            crate::app::tests::test_core(Arc::new(crate::app::tests::Recorder::default()));
        for code in ["digitalCapture", "compositeSynthetic"] {
            let id = import_image(&core, labelled_png(code)).unwrap().id;
            let im = core.session.get(&id).unwrap();
            assert_eq!(im.ai_label, None, "{code}");
            assert!(
                exported_xmp(&export_png(&core, std::slice::from_ref(&im)).unwrap()).is_empty()
            );
        }
    }

    #[test]
    fn a_jpeg_ai_label_is_kept() {
        let (_tmp, core) =
            crate::app::tests::test_core(Arc::new(crate::app::tests::Recorder::default()));
        // A phone-style JPEG (EXIF with private text) that also says it was edited with AI.
        let mut jpeg = img::jpeg_with_exif(8, 8, "PINHOLE_SENTINEL_7f3a");
        let xmp = b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta Iptc4xmpExt:DigitalSourceType=\"http://cv.iptc.org/newscodes/digitalsourcetype/compositeWithTrainedAlgorithmicMedia\"/>";
        let mut app1 = vec![0xff, 0xe1];
        app1.extend_from_slice(&((xmp.len() + 2) as u16).to_be_bytes());
        app1.extend_from_slice(xmp);
        jpeg.splice(2..2, app1);
        let id = import_image(&core, jpeg).unwrap().id;
        let im = core.session.get(&id).unwrap();
        assert_eq!(im.ai_label, Some(AiLabel::Composite));
        let chunks = exported_xmp(&export_png(&core, std::slice::from_ref(&im)).unwrap());
        assert_eq!(chunks.len(), 1, "{chunks:?}");
        assert!(chunks[0].contains("compositeWithTrainedAlgorithmicMedia"));
        assert!(!chunks[0].contains("SENTINEL"));
    }

    /// A made picture of `w`×`h` with some texture (the watermark needs detail to sit in).
    fn made(id: &str, w: u32, h: u32) -> SessionImage {
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                let v = (96 + (x * 7 + y * 13) % 64) as u8;
                [v, v / 2 + 40, 255 - v, 255]
            })
            .collect();
        let mut im = image(Some(meta(id)), Origin::Generated, vec![]);
        im.id = id.into();
        im.bytes = Arc::new(img::encode_png_rgba(&rgba, w, h).unwrap());
        (im.width, im.height) = (w, h);
        im
    }

    #[test]
    fn a_sheet_is_one_marked_picture_without_settings() {
        let (_tmp, core) =
            crate::app::tests::test_core(Arc::new(crate::app::tests::Recorder::default()));
        core.settings.write().saved_metadata = "settings".into();
        let ims: Vec<_> = (0..4).map(|i| made(&format!("s{i}"), 320, 256)).collect();
        let bytes = export_png(&core, &ims).unwrap();
        let chunks = exported_xmp(&bytes);
        assert_eq!(chunks.len(), 1, "only the marker: {chunks:?}");
        assert!(chunks[0].contains("digitalsourcetype/trainedAlgorithmicMedia\""));
        let (rgba, w, h) = img::decode_rgba(&bytes).unwrap();
        assert_eq!((w, h), (2 * 320 + 3 * 8, 2 * 256 + 3 * 8));
        assert!(pinhole_engine::watermark::is_marked(&rgba, w, h));
        // One picture cut out of the sheet still carries its own mark.
        let tile: Vec<u8> = (8..8 + 256)
            .flat_map(|y| {
                let start = ((y * w + 8) * 4) as usize;
                rgba[start..start + 320 * 4].to_vec()
            })
            .collect();
        assert!(pinhole_engine::watermark::is_marked(&tile, 320, 256));
    }

    #[test]
    fn a_sheet_has_the_pixels_of_its_marked_pictures_placed_together() {
        let ims = [made("a", 120, 80), made("b", 60, 100), made("c", 120, 100)];
        let bytes = sheet_bytes(&ims).unwrap();
        // All pictures decoded first, then placed.
        let tiles = ims.iter().map(|im| marked_pixels(im).unwrap()).collect();
        let (mut want, w, h) = img::sheet(tiles);
        pinhole_engine::watermark::embed(&mut want, w, h);
        assert_eq!(img::decode_rgba(&bytes).unwrap(), (want, w, h));
    }

    #[test]
    fn a_sheet_of_a_photo_and_a_made_picture_is_a_composite() {
        let (_tmp, core) =
            crate::app::tests::test_core(Arc::new(crate::app::tests::Recorder::default()));
        let photo_png = made("p", 200, 200).bytes.as_ref().clone();
        let photo = core
            .session
            .get(&import_image(&core, photo_png).unwrap().id)
            .unwrap();
        let bytes = export_png(&core, &[photo.clone(), made("m", 200, 200)]).unwrap();
        let chunks = exported_xmp(&bytes);
        assert!(
            chunks[0].contains("compositeWithTrainedAlgorithmicMedia"),
            "{chunks:?}"
        );
        // Photos only: no marker, like the photos themselves.
        let bytes = export_png(&core, &[photo.clone(), photo]).unwrap();
        assert!(exported_xmp(&bytes).is_empty());
    }

    #[test]
    fn save_sheet_as_writes_a_png_and_checks_its_input() {
        let (tmp, core) =
            crate::app::tests::test_core(Arc::new(crate::app::tests::Recorder::default()));
        let path = tmp.path().join("sheet");
        let one = vec!["missing".to_string()];
        assert_eq!(
            save_sheet_as(&core, &one, path.to_str().unwrap())
                .unwrap_err()
                .code,
            "invalid"
        );
        let two = vec!["missing".to_string(), "gone".to_string()];
        assert_eq!(
            save_sheet_as(&core, &two, path.to_str().unwrap())
                .unwrap_err()
                .code,
            "not_found"
        );
        let ids: Vec<String> = (0..2)
            .map(|_| {
                import_image(&core, made("p", 120, 90).bytes.as_ref().clone())
                    .unwrap()
                    .id
            })
            .collect();
        let saved = save_sheet_as(&core, &ids, path.to_str().unwrap()).unwrap();
        assert!(saved.path.ends_with("sheet.png"));
        let (_, w, h) = img::decode_rgba(&std::fs::read(&saved.path).unwrap()).unwrap();
        assert_eq!((w, h), (2 * 120 + 3 * 8, 90 + 2 * 8));
        let nine: Vec<String> = std::iter::repeat_n(ids[0].clone(), 9).collect();
        assert!(save_sheet_as(&core, &nine, tmp.path().join("x.png").to_str().unwrap()).is_err());
    }
}
