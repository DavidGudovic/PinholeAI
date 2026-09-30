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

use crate::generate::{ImportedImage, Origin, ResultImage, SavedImage};
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
}

impl SessionImage {
    pub fn parent_id(&self) -> Option<&str> {
        self.meta.as_ref().and_then(|m| m.parent_id.as_deref())
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
    core.gen.logs.clear();
    core.describe.logs.clear();
    crate::generate::clear_engine_results(core).await;
}

/// RGBA8 pixels for the clipboard, from [`export_png`] (a clipboard picture
/// carries no metadata; the pixel watermark will, RELEASE-SPEC §2).
pub fn decode_rgba(core: &AppCore, id: &str) -> CoreResult<(Vec<u8>, u32, u32)> {
    let im = core.session.get(id).ok_or_else(missing)?;
    let bytes = export_png(core, &im)?;
    img::decode_rgba(&bytes).map_err(|e| {
        CoreError::invalid("This image couldn't be decoded.").with_details(e.to_string())
    })
}

fn missing() -> CoreError {
    CoreError::not_found("That image isn't in this session anymore.")
}

/// Settings-only metadata (never prompt/negative/style text).
fn settings_text(m: &ResultImage) -> String {
    serde_json::json!({
        "app": "Pinhole",
        "model": m.model_label,
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

/// IPTC digital source type for an image Pinhole made (RELEASE-SPEC §2).
const SOURCE_GENERATED: &str =
    "http://cv.iptc.org/newscodes/digitalsourcetype/trainedAlgorithmicMedia";
/// ... and for one made from a picture the user brought in.
const SOURCE_COMPOSITE: &str =
    "http://cv.iptc.org/newscodes/digitalsourcetype/compositeWithTrainedAlgorithmicMedia";

/// The AI-generated marker (RELEASE-SPEC §2, EU AI Act Art. 50): XMP with the
/// IPTC digital source type and the app name + version. Nothing else: no prompt,
/// seed, model, user or machine.
pub fn ai_marker_xmp(origin: Origin) -> String {
    let source = match origin {
        Origin::Generated => SOURCE_GENERATED,
        Origin::Imported => SOURCE_COMPOSITE,
    };
    format!(
        concat!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">"#,
            r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">"#,
            r#"<rdf:Description rdf:about="""#,
            r#" xmlns:Iptc4xmpExt="http://iptc.org/std/Iptc4xmpExt/2008-02-29/""#,
            r#" xmlns:xmp="http://ns.adobe.com/xap/1.0/""#,
            r#" Iptc4xmpExt:DigitalSourceType="{source}""#,
            r#" xmp:CreatorTool="Pinhole {version}"/>"#,
            r#"</rdf:RDF></x:xmpmeta>"#
        ),
        source = source,
        version = env!("CARGO_PKG_VERSION"),
    )
}

/// Export (RELEASE-SPEC §1 item 4): the one function behind Save, Save as and
/// Copy. Images made by Pinhole (generated, edited, upscaled) always get the
/// AI-generated marker (there is no setting for it) plus the optional
/// "settings (no prompt)" chunk. A picture the user added and didn't change
/// leaves as it came in (already scrubbed at import).
pub fn export_png(core: &AppCore, im: &SessionImage) -> CoreResult<Vec<u8>> {
    let Some(m) = &im.meta else {
        return Ok(im.bytes.as_ref().clone());
    };
    let damaged = |_| CoreError::internal("The image in memory is damaged.");
    let clean = pinhole_engine::png::scrub(&im.bytes).map_err(damaged)?;
    let marked =
        pinhole_engine::png::add_itxt_chunk(&clean, "XML:com.adobe.xmp", &ai_marker_xmp(im.origin))
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
    let im = core.session.get(id).ok_or_else(missing)?;
    let bytes = export_png(core, &im)?;
    let dir = core.data.outputs();
    fs::create_dir_all(&dir).map_err(|e| io_err(&dir, e))?;
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
        "Couldn't find a free file name in the outputs folder.",
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
}
