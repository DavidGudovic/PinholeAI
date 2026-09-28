//! Model details page (SPEC §5.4 "Model details"): a version's CivitAI preview
//! images with their generation data, and the link to its CivitAI page.
//!
//! The images come from `GET /model-versions/{id}`: it is the only endpoint
//! that still carries generation data (`/models` and `/images` send
//! `meta: null`; checked live 2026-09-28).
//!
//! Kept apart from the catalog modules on purpose: it reads the version with
//! its own small types, so the Browse code can change without touching this.
//!
//! PRIVACY: the generation data (someone else's prompt and settings) is only
//! passed through to the UI in memory, where the user can copy it into Create.
//! It is never logged or written anywhere.

use pinhole_catalog::api::{is_preview_url, API_BASE, NSFW_LEVEL_MIN};
use pinhole_catalog::cards::thumbnail_url;
use pinhole_catalog::filters::ContentMode;
use pinhole_catalog::lenient;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{AppCore, CoreError, CoreResult};

/// The parts of `GET /model-versions/{id}` the details page needs.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VersionImages {
    #[serde(deserialize_with = "lenient::strings")]
    pub trained_words: Vec<String>,
    #[serde(deserialize_with = "lenient::vec")]
    pub images: Vec<VersionImage>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VersionImage {
    #[serde(deserialize_with = "lenient::string")]
    pub url: String,
    #[serde(deserialize_with = "lenient::opt_u64")]
    pub width: Option<u64>,
    #[serde(deserialize_with = "lenient::opt_u64")]
    pub height: Option<u64>,
    #[serde(deserialize_with = "lenient::nsfw_level")]
    pub nsfw_level: Option<u32>,
    /// Older responses: bool or a level string.
    #[serde(deserialize_with = "lenient::nsfw_level_or_bool")]
    pub nsfw: Option<bool>,
    #[serde(rename = "type", deserialize_with = "lenient::opt_string")]
    pub kind: Option<String>,
    /// Generation data (prompt, settings, resources). In memory only.
    #[serde(deserialize_with = "lenient::opt_obj")]
    pub meta: Option<Map<String, Value>>,
}

impl VersionImage {
    fn is_video(&self) -> bool {
        if self.kind.as_deref().is_some_and(|k| k.eq_ignore_ascii_case("video")) {
            return true;
        }
        let path = self.url.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
        path.ends_with(".mp4") || path.ends_with(".webm") || path.ends_with(".mov")
    }

    fn is_nsfw(&self, model_nsfw: bool) -> bool {
        match (self.nsfw_level, self.nsfw) {
            (Some(level), _) => level >= NSFW_LEVEL_MIN,
            (None, Some(flag)) => flag,
            // Unrated images of an NSFW model count as NSFW (same rule as the cards).
            (None, None) => model_nsfw,
        }
    }
}

/// The details page's gallery. Anonymous, like browsing. Offline mode: no
/// request, `offline: true`.
pub async fn model_gallery(core: &AppCore, version_id: u64, content: ContentMode, model_nsfw: bool) -> CoreResult<ModelGallery> {
    if core.offline.get() {
        return Ok(ModelGallery { items: Vec::new(), hidden_nsfw: 0, trained_words: Vec::new(), offline: true });
    }
    let filters = crate::catalog::filters(core)?;
    let url = format!("{API_BASE}/model-versions/{version_id}");
    let version: VersionImages = core.http.get_json(&url, &[]).await.map_err(crate::catalog::net_error)?;
    Ok(gallery(&version, content, model_nsfw, filters.preview_width))
}

/// `https://civitai.com/models/…` (civitai.red for NSFW models), for the
/// system browser. Built here so the UI can't open arbitrary URLs.
pub fn civitai_page(model_id: u64, version_id: Option<u64>, nsfw: bool) -> CoreResult<String> {
    if model_id == 0 {
        return Err(CoreError::invalid("That model has no CivitAI page."));
    }
    Ok(civitai_page_url(model_id, version_id, nsfw))
}

/// Generation-data keys the UI understands (CivitAI mixes camelCase and
/// A1111-style keys). Everything else (ADetailer, workflow JSON, …) is dropped.
const KEPT_META_KEYS: &[&str] = &[
    "prompt",
    "negativePrompt",
    "steps",
    "sampler",
    "scheduler",
    "Schedule type",
    "cfgScale",
    "guidance",
    "Distilled CFG Scale",
    "seed",
    "Size",
    "clipSkip",
    "Clip skip",
    "Model",
    "Model hash",
    "hashes",
    "resources",
    "civitaiResources",
    "Denoising strength",
    "Hires upscale",
    "Hires steps",
    "Hires upscaler",
    "VAE",
    "VAE hash",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryItem {
    /// Position in the version's image list (CivitAI sends no image id here).
    pub index: usize,
    /// Small rendition for the grid (fetch through `fetch_preview`).
    pub thumb_url: String,
    /// Full-size image for "Edit this" (fetch through `fetch_preview`).
    pub full_url: String,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub nsfw: bool,
    /// Generation data (prompt, settings, resources), or `None` when the
    /// creator didn't share it. In memory only.
    pub generation: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelGallery {
    pub items: Vec<GalleryItem>,
    /// Images hidden because they are 18+ and 18+ content is off.
    pub hidden_nsfw: usize,
    /// LoRA trigger words.
    pub trained_words: Vec<String>,
    pub offline: bool,
}

/// Gallery for a version. Videos are skipped (they can't be edited), and so
/// are 18+ images in Safe mode.
pub fn gallery(version: &VersionImages, content: ContentMode, model_nsfw: bool, thumb_width: u32) -> ModelGallery {
    let mut hidden_nsfw = 0;
    let items = version
        .images
        .iter()
        .enumerate()
        .filter_map(|(index, i)| {
            let it = item(index, i, model_nsfw, thumb_width)?;
            if it.nsfw && content == ContentMode::Safe {
                hidden_nsfw += 1;
                return None;
            }
            Some(it)
        })
        .collect();
    ModelGallery { items, hidden_nsfw, trained_words: version.trained_words.clone(), offline: false }
}

fn item(index: usize, i: &VersionImage, model_nsfw: bool, thumb_width: u32) -> Option<GalleryItem> {
    if i.url.trim().is_empty() || i.is_video() || !is_preview_url(&i.url) {
        return None;
    }
    Some(GalleryItem {
        index,
        thumb_url: thumbnail_url(&i.url, thumb_width),
        full_url: original_url(&i.url),
        width: i.width,
        height: i.height,
        nsfw: i.is_nsfw(model_nsfw),
        generation: i.meta.as_ref().and_then(kept_meta),
    })
}

/// Only the keys the UI maps; `None` when there is nothing to apply.
fn kept_meta(meta: &Map<String, Value>) -> Option<Map<String, Value>> {
    let kept: Map<String, Value> =
        KEPT_META_KEYS.iter().filter_map(|k| meta.get(*k).filter(|v| !v.is_null()).map(|v| (k.to_string(), v.clone()))).collect();
    let useful = kept.get("prompt").and_then(Value::as_str).is_some_and(|p| !p.trim().is_empty()) || kept.contains_key("steps");
    useful.then_some(kept)
}

/// The full-size rendition (`original=true`) of a CivitAI image URL.
pub fn original_url(url: &str) -> String {
    let marked = thumbnail_url(url, 1);
    if marked == url {
        return url.to_string();
    }
    marked.replacen("/width=1/", "/original=true/", 1)
}

/// The model's page on CivitAI. NSFW models live on civitai.red, everything
/// else on civitai.com. Opened in the system browser, never in the WebView.
pub fn civitai_page_url(model_id: u64, version_id: Option<u64>, nsfw: bool) -> String {
    let host = if nsfw { "civitai.red" } else { "civitai.com" };
    match version_id.filter(|v| *v > 0) {
        Some(v) => format!("https://{host}/models/{model_id}?modelVersionId={v}"),
        None => format!("https://{host}/models/{model_id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shape of a live `/model-versions/{id}` response (2026-09-28), trimmed.
    const VERSION: &str = r#"{
      "id": 501240, "modelId": 4201, "name": "V5.1 Hyper (VAE)", "baseModel": "SD 1.5 Hyper",
      "trainedWords": ["analog style"],
      "images": [
        {"id": null, "url": "https://image.civitai.com/xG1/5403/original=true/12221833.jpeg", "width": 512, "height": 768,
         "nsfwLevel": 2, "type": "image", "hasMeta": true,
         "meta": {"VAE": "vae-ft-mse.safetensors", "Size": "512x768", "seed": 3277121308, "Model": "RVHYPO", "steps": 6,
                  "hashes": {"model": "0928b30687"}, "prompt": "a lighthouse at dusk", "Version": "v1.7.0",
                  "sampler": "DPM++ SDE Karras", "cfgScale": 1.5, "resources": [{"hash": "0928b30687", "name": "RVHYPO", "type": "model"}],
                  "Model hash": "0928b30687", "negativePrompt": "blurry", "ADetailer model": "face_yolov8n.pt"}},
        {"id": null, "url": "https://image.civitai.com/xG1/1c65/original=true/12221824.jpeg", "nsfwLevel": 8, "type": "image", "meta": null},
        {"id": null, "url": "https://image.civitai.com/xG1/9eb1/original=true/9eb1.mp4", "nsfwLevel": 1, "type": "video"},
        {"id": null, "url": "https://evil.example/4.jpeg", "nsfwLevel": 1, "type": "image"},
        {"id": null, "url": "https://image.civitai.com/xG1/9d8e/original=true/12221845.jpeg", "nsfwLevel": 1, "meta": {"Model": "x"}}
      ]
    }"#;

    fn version() -> VersionImages {
        serde_json::from_str(VERSION).unwrap()
    }

    #[test]
    fn safe_mode_hides_nsfw_and_skips_videos_and_foreign_hosts() {
        let g = gallery(&version(), ContentMode::Safe, false, 450);
        assert_eq!(g.items.iter().map(|i| i.index).collect::<Vec<_>>(), vec![0, 4]);
        assert_eq!(g.hidden_nsfw, 1);
        assert_eq!(g.trained_words, vec!["analog style"]);
        let all = gallery(&version(), ContentMode::Include18Plus, false, 450);
        assert_eq!(all.items.iter().map(|i| i.index).collect::<Vec<_>>(), vec![0, 1, 4]);
        assert!(all.items[1].nsfw && all.hidden_nsfw == 0);
    }

    #[test]
    fn keeps_only_known_generation_keys() {
        let g = gallery(&version(), ContentMode::Safe, false, 450);
        let m = g.items[0].generation.as_ref().unwrap();
        assert_eq!(m.get("steps"), Some(&Value::from(6)));
        assert!(m.contains_key("prompt") && m.contains_key("Model hash"));
        assert!(!m.contains_key("ADetailer model") && !m.contains_key("Version"));
        // Only a model name: nothing to apply.
        assert!(g.items[1].generation.is_none());
    }

    #[test]
    fn renditions() {
        let g = gallery(&version(), ContentMode::Safe, false, 450);
        assert_eq!(g.items[0].thumb_url, "https://image.civitai.com/xG1/5403/width=450/12221833.jpeg");
        assert_eq!(g.items[0].full_url, "https://image.civitai.com/xG1/5403/original=true/12221833.jpeg");
        assert_eq!(original_url("https://image.civitai.com/a/u/width=450/2.jpeg"), "https://image.civitai.com/a/u/original=true/2.jpeg");
        assert_eq!(original_url("https://image.civitai.com/a/u/2.jpeg"), "https://image.civitai.com/a/u/2.jpeg");
    }

    #[test]
    fn unrated_images_of_nsfw_models_are_nsfw() {
        let v = VersionImages {
            images: vec![VersionImage { url: "https://image.civitai.com/a/b/width=450/1.jpeg".into(), ..Default::default() }],
            ..Default::default()
        };
        assert!(gallery(&v, ContentMode::Safe, true, 450).items.is_empty());
        assert_eq!(gallery(&v, ContentMode::Safe, false, 450).items.len(), 1);
    }

    #[tokio::test]
    async fn offline_needs_no_network() {
        let (_t, core) = crate::app::tests::test_core(std::sync::Arc::new(crate::app::tests::Recorder::default()));
        core.offline.set(true);
        assert!(model_gallery(&core, 1, ContentMode::Safe, false).await.unwrap().offline);
        assert_eq!(civitai_page(0, None, false).unwrap_err().code, "invalid");
    }

    #[test]
    fn page_urls() {
        assert_eq!(civitai_page_url(4201, Some(501240), false), "https://civitai.com/models/4201?modelVersionId=501240");
        assert_eq!(civitai_page_url(4201, None, true), "https://civitai.red/models/4201");
    }
}
