//! In-memory image store (CLAUDE.md privacy rule 3): generated and imported
//! images live here until Save or Reset. OWNER: engine agent.
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

use crate::generate::{ImportedImage, Origin, ResultImage, SavedBatch, SavedEntry, SavedImage};
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
}

/// A brought-in picture at the start of a chain of edits. Its bytes stay with every
/// image made from it, so discarding the original doesn't lose it for the check.
#[derive(Debug, Clone)]
pub struct Source {
    pub id: String,
    pub bytes: Arc<Vec<u8>>,
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
            }]
        } else {
            self.made_from.to_vec()
        }
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
    /// Result intake: store a (scrubbed) generated PNG under `meta.id`, but
    /// only while no Reset happened since [`Session::epoch`] returned `epoch` (a job that finishes after
    /// Reset must not bring its images back). Returns whether it was stored.
    pub fn insert_generated_since(&self, epoch: u64, png: Vec<u8>, meta: ResultImage) -> bool {
        self.insert_generated_from(epoch, png, meta, Arc::from(Vec::new()))
    }

    /// [`Session::insert_generated_since`] for an image made from `made_from`.
    pub fn insert_generated_from(
        &self,
        epoch: u64,
        png: Vec<u8>,
        meta: ResultImage,
        made_from: Arc<[Source]>,
    ) -> bool {
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
        };
        images.insert(img.id.clone(), img);
        true
    }

    /// Changes whenever the session is cleared (Reset).
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    pub fn insert(&self, img: SessionImage) {
        self.images.write().insert(img.id.clone(), img);
    }

    pub fn get(&self, id: &str) -> Option<SessionImage> {
        self.images.read().get(id).cloned()
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
/// orientation applied) and re-encoded, which drops EXIF / XMP (GPS, camera).
pub fn import_image(core: &AppCore, bytes: Vec<u8>) -> CoreResult<ImportedImage> {
    let info = img::sniff(&bytes).map_err(|e| CoreError::invalid(e.to_string()))?;
    let made_from = core
        .check
        .exported_from(&bytes)
        .unwrap_or_else(|| Arc::from(Vec::new()));
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
    core.session.insert(SessionImage {
        id: id.clone(),
        bytes: Arc::new(bytes),
        kind: Kind::Png,
        width,
        height,
        meta: None,
        origin: Origin::Imported,
        made_from,
    });
    Ok(ImportedImage { id, width, height })
}

/// Bytes of a session image (PNG for generated images).
pub fn get(core: &AppCore, id: &str) -> CoreResult<Arc<Vec<u8>>> {
    core.session.get(id).map(|i| i.bytes).ok_or_else(missing)
}

pub fn discard(core: &AppCore, id: &str) {
    core.session.remove(id);
}

/// Reset: drop every image immediately (and the engines' output
/// buffers), and stop sd-server if it ran a job — it keeps finished results
/// for 600 s behind an unauthenticated API (`generate::IDLE_STOP_AFTER`). The
/// next Generate reloads the model. If a job is running, the engine stops
/// right after it.
pub async fn clear(core: &AppCore) {
    core.session.clear();
    core.check.forget();
    core.gen.logs.clear();
    core.describe.logs.clear();
    crate::generate::clear_engine_results(core).await;
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
        "width": m.width,
        "height": m.height,
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

/// The AI-generated marker (RELEASE-SPEC §2, EU AI Act Art. 50): XMP with only
/// the IPTC digital source type ("made with AI"). No app name (David, 2026-09-30),
/// prompt, seed, model, user or machine.
pub fn ai_marker_xmp(origin: Origin, upscaled_import: bool) -> String {
    let source = match origin {
        Origin::Generated => SOURCE_GENERATED,
        Origin::Imported if upscaled_import => SOURCE_ENHANCED,
        Origin::Imported => SOURCE_COMPOSITE,
    };
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

/// Export (RELEASE-SPEC §1 item 4): the one function behind Save, Save as and
/// Copy. Images made by Pinhole (generated, edited, upscaled) always get the
/// AI-generated marker (invisible watermark + XMP) (there is no setting for it) plus the optional
/// "settings (no prompt)" chunk. A picture the user added and didn't change
/// leaves as it came in (already scrubbed at import).
pub fn export_png(core: &AppCore, im: &SessionImage) -> CoreResult<Vec<u8>> {
    let bytes = export_bytes(core, im)?;
    core.check.note_export(&bytes, im.sources());
    Ok(bytes)
}

fn export_bytes(core: &AppCore, im: &SessionImage) -> CoreResult<Vec<u8>> {
    let Some(m) = &im.meta else {
        return Ok(im.bytes.as_ref().clone());
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
        &ai_marker_xmp(im.origin, upscaled_import(m)),
    )
    .map_err(damaged)?;
    if core.settings.read().saved_metadata == "settings" {
        pinhole_engine::png::add_text_chunk(&marked, "pinhole", &settings_text(m))
            .map_err(|_| CoreError::internal("Couldn't add the settings to the image."))
    } else {
        Ok(marked)
    }
}

/// Save into `Data/outputs/pinhole_YYYYMMDD_HHMMSS_<seed>.<ext>` (unique suffix
/// `_2`, `_3`… on collision). Imported images use `import` instead of a seed.
pub fn save_image(core: &AppCore, id: &str) -> CoreResult<SavedImage> {
    save_into(core, id, &core.data.outputs())
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
    let bytes = export_png(core, &im)?;
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
    }
    let bytes = export_png(core, &im)?;
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
        if path.exists() {
            // Windows rename doesn't replace; the user already confirmed overwrite.
            fs::remove_file(path)?;
        }
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
        }
    }

    #[test]
    fn save_all_writes_every_image_to_the_chosen_folder() {
        let (tmp, core) =
            crate::app::tests::test_core(Arc::new(crate::app::tests::Recorder::default()));
        let png = img::encode_png_rgba(&[9; 64], 4, 4).unwrap();
        let epoch = core.session.epoch();
        assert!(core
            .session
            .insert_generated_since(epoch, png.clone(), meta("a")));
        assert!(core.session.insert_generated_since(epoch, png, meta("b")));
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
        assert!(s.insert_generated_since(s.epoch(), png.clone(), meta("a")));
        let got = s.get("a").unwrap();
        assert_eq!(got.bytes.as_slice(), png.as_slice());
        assert_eq!(got.parent_id(), Some("p"));
        assert_eq!(s.len(), 1);
        assert!(s.remove("a"));
        assert!(s.get("a").is_none());
        assert!(s.insert_generated_since(s.epoch(), png, meta("b")));
        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn results_from_before_a_reset_are_dropped() {
        let s = Session::default();
        let png = img::encode_png_rgba(&[9; 64], 4, 4).unwrap();
        let epoch = s.epoch();
        assert!(s.insert_generated_since(epoch, png.clone(), meta("a")));
        s.clear();
        assert!(
            !s.insert_generated_since(epoch, png.clone(), meta("b")),
            "Reset happened since the job started"
        );
        assert!(s.is_empty());
        assert!(s.insert_generated_since(s.epoch(), png, meta("c")));
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
        let gen = ai_marker_xmp(Origin::Generated, false);
        assert!(
            gen.contains("digitalsourcetype/trainedAlgorithmicMedia\""),
            "{gen}"
        );
        assert!(!gen.contains("Pinhole") && !gen.contains(env!("CARGO_PKG_VERSION")));
        assert!(
            ai_marker_xmp(Origin::Imported, false).contains("compositeWithTrainedAlgorithmicMedia")
        );
        assert!(ai_marker_xmp(Origin::Imported, true).contains("algorithmicallyEnhanced"));
        let mut m = meta("a");
        assert!(!upscaled_import(&m));
        m.kind = crate::generate::ResultKind::Upscaled;
        assert!(!upscaled_import(&m), "upscale of a generated picture");
        m.model_id.clear();
        assert!(upscaled_import(&m));
    }
}
