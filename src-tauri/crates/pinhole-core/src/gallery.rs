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

use pinhole_catalog::api::{civitai_auth_header, is_preview_url, ModelImage, API_BASE};
use pinhole_catalog::cards::thumbnail_url;
use pinhole_catalog::filters::ContentMode;
use pinhole_catalog::lenient;
use pinhole_catalog::safe::{SafeFilter, LEVEL_BLOCKED, LEVEL_R};
use pinhole_net::NetError;
use pinhole_store::installed::CreatorNotes;
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

/// A version image: the catalog's [`ModelImage`] plus its generation data.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VersionImage {
    #[serde(flatten)]
    pub image: ModelImage,
    /// Generation data (prompt, settings, resources). In memory only.
    #[serde(default, deserialize_with = "lenient::opt_obj")]
    pub meta: Option<Map<String, Value>>,
}

impl VersionImage {
    /// Unrated images of an NSFW model count as NSFW (same rule as the cards).
    fn is_nsfw(&self, model_nsfw: bool) -> bool {
        self.image.is_nsfw()
            || (self.image.nsfw_level.is_none() && self.image.nsfw.is_none() && model_nsfw)
    }
}

/// The details page's gallery. Asked anonymously first; on 401/403 (a
/// sign-in-only version) asked once more with the API key, like install
/// planning. Offline mode: no request, `offline: true`.
pub async fn model_gallery(
    core: &AppCore,
    version_id: u64,
    content: ContentMode,
    model_nsfw: bool,
) -> CoreResult<ModelGallery> {
    if core.offline.get() {
        return Ok(ModelGallery {
            items: Vec::new(),
            hidden_nsfw: 0,
            trained_words: Vec::new(),
            creator_notes: stored_notes(core, version_id, content, model_nsfw),
            offline: true,
        });
    }
    let filters = crate::catalog::filters(core)?;
    let url = format!("{API_BASE}/model-versions/{version_id}");
    let version: VersionImages = match core.http.get_json(&url, &[]).await {
        Err(NetError::Unauthorized(_)) | Err(NetError::Status(401 | 403)) => {
            let key = crate::catalog::api_key(core).await;
            match civitai_auth_header(key.as_deref(), &url) {
                Some((k, v)) => core.http.get_json(&url, &[(k.as_str(), v.as_str())]).await,
                None => Err(NetError::Unauthorized(401)),
            }
        }
        other => other,
    }
    .map_err(crate::catalog::net_error)?;
    let mut g = gallery(
        &version,
        content,
        model_nsfw,
        &filters.safe,
        filters.preview_width,
    );
    // The description is a bonus: if it can't be read, the page shows what
    // was saved at install (or nothing).
    let client = crate::catalog::civitai_client(core).await;
    g.creator_notes =
        match crate::catalog::fetch_version(&core.models.versions, &client, version_id).await {
            Ok(fetched) => {
                let (v, m) = (&fetched.0, &fetched.1);
                let adult = |l: Option<u32>| l.is_some_and(|l| l >= LEVEL_R);
                let is_adult = model_nsfw
                    || adult(v.nsfw_level)
                    || m.as_ref().is_some_and(|m| {
                        m.nsfw || adult(m.nsfw_level) || filters.safe.adult_reason(m).is_some()
                    });
                notes_for(
                    content,
                    model_nsfw,
                    CreatorNotes::from_html(
                        m.as_ref().and_then(|m| m.description.as_deref()),
                        v.description.as_deref(),
                        is_adult,
                    ),
                )
            }
            Err(_) => stored_notes(core, version_id, content, model_nsfw),
        };
    Ok(g)
}

/// The creator's description saved when the model was installed.
fn stored_notes(
    core: &AppCore,
    version_id: u64,
    content: ContentMode,
    model_nsfw: bool,
) -> Option<CreatorNotes> {
    let stored = core
        .installed
        .lock()
        .files
        .iter()
        .filter_map(|f| f.civitai.as_ref())
        .find(|c| c.version_id == version_id)
        .and_then(|c| c.creator_notes.clone());
    notes_for(content, model_nsfw, stored)
}

/// Safe mode hides the description of a model made for adults, like its images.
fn notes_for(
    content: ContentMode,
    model_nsfw: bool,
    notes: Option<CreatorNotes>,
) -> Option<CreatorNotes> {
    let adult = model_nsfw || notes.as_ref().is_some_and(|n| n.adult);
    if content == ContentMode::Safe && adult {
        None
    } else {
        notes
    }
}

/// Links from a creator's description open in the system browser: https only.
/// The UI passes the address of a link it found, so check it here.
pub fn external_link(url: &str) -> CoreResult<String> {
    let bad = || CoreError::invalid("Pinhole only opens https links from descriptions.");
    if url.len() > 2048 {
        return Err(bad());
    }
    let parsed = url::Url::parse(url.trim()).map_err(|_| bad())?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none_or(str::is_empty)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(bad());
    }
    Ok(parsed.to_string())
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
    "Lora hashes",
    "TI hashes",
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
    /// Images hidden because they are made for adults and Safe mode is on.
    pub hidden_nsfw: usize,
    /// LoRA trigger words.
    pub trained_words: Vec<String>,
    /// What the creator wrote about the model (raw HTML: the UI sanitizes it).
    /// `None` when there is none, or Safe mode hides it.
    pub creator_notes: Option<CreatorNotes>,
    pub offline: bool,
}

/// Gallery for a version. Videos and images CivitAI blocked are skipped. With
/// Safe mode on, only images that pass the same rule as card previews
/// ([`SafeFilter::is_safe_preview`]: rated PG, or flagged not NSFW) are shown.
pub fn gallery(
    version: &VersionImages,
    content: ContentMode,
    model_nsfw: bool,
    safe: &SafeFilter,
    thumb_width: u32,
) -> ModelGallery {
    let mut hidden_nsfw = 0;
    let items = version
        .images
        .iter()
        .enumerate()
        .filter(|(_, i)| i.image.nsfw_level.is_none_or(|l| l < LEVEL_BLOCKED))
        .filter_map(|(index, i)| {
            let it = item(index, i, model_nsfw, thumb_width)?;
            if content == ContentMode::Safe && (it.nsfw || !safe.is_safe_preview(&i.image)) {
                hidden_nsfw += 1;
                return None;
            }
            Some(it)
        })
        .collect();
    ModelGallery {
        items,
        hidden_nsfw,
        trained_words: version.trained_words.clone(),
        creator_notes: None,
        offline: false,
    }
}

fn item(index: usize, v: &VersionImage, model_nsfw: bool, thumb_width: u32) -> Option<GalleryItem> {
    let i = &v.image;
    if i.url.trim().is_empty() || i.is_video() || !is_preview_url(&i.url) {
        return None;
    }
    Some(GalleryItem {
        index,
        thumb_url: thumbnail_url(&i.url, thumb_width),
        full_url: original_url(&i.url),
        width: i.width,
        height: i.height,
        nsfw: v.is_nsfw(model_nsfw),
        generation: v.meta.as_ref().and_then(kept_meta),
    })
}

/// Only the keys the UI maps; `None` when there is nothing to apply.
fn kept_meta(meta: &Map<String, Value>) -> Option<Map<String, Value>> {
    let kept: Map<String, Value> = KEPT_META_KEYS
        .iter()
        .filter_map(|k| {
            meta.get(*k)
                .filter(|v| !v.is_null())
                .map(|v| (k.to_string(), v.clone()))
        })
        .collect();
    let useful = kept
        .get("prompt")
        .and_then(Value::as_str)
        .is_some_and(|p| !p.trim().is_empty())
        || kept.contains_key("steps");
    useful.then_some(kept)
}

/// The full-size rendition (`original=true`) of a CivitAI image URL.
/// Other URLs are unchanged.
pub fn original_url(url: &str) -> String {
    if !is_preview_url(url) {
        return url.to_string();
    }
    let (path, rest) = url.split_at(url.find(['?', '#']).unwrap_or(url.len()));
    let is_transform = |s: &str| {
        s.split(',').all(|p| p.contains('='))
            && s.split(',')
                .any(|p| p.starts_with("width=") || p == "original=true")
    };
    let mut segs: Vec<&str> = path.split('/').collect();
    // Skip "https:", "" and the host.
    let Some(pos) = segs
        .iter()
        .skip(3)
        .position(|s| is_transform(s))
        .map(|p| p + 3)
    else {
        return url.to_string();
    };
    segs[pos] = "original=true";
    format!("{}{rest}", segs.join("/"))
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
                  "Model hash": "0928b30687", "negativePrompt": "blurry", "ADetailer model": "face_yolov8n.pt",
                  "Lora hashes": "detail_tweaker: abc1234567"}},
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
        let g = gallery(
            &version(),
            ContentMode::Safe,
            false,
            &SafeFilter::default(),
            450,
        );
        // Image 0 is PG-13: fine with Safe mode off, but not PG, so hidden with it on.
        assert_eq!(g.items.iter().map(|i| i.index).collect::<Vec<_>>(), vec![4]);
        assert_eq!(g.hidden_nsfw, 2);
        assert_eq!(g.trained_words, vec!["analog style"]);
        let all = gallery(
            &version(),
            ContentMode::All,
            false,
            &SafeFilter::default(),
            450,
        );
        assert_eq!(
            all.items.iter().map(|i| i.index).collect::<Vec<_>>(),
            vec![0, 1, 4]
        );
        assert!(all.items[1].nsfw && all.hidden_nsfw == 0);
    }

    #[test]
    fn safe_mode_hides_notes_of_adult_models() {
        let n = CreatorNotes::from_html(Some("<p>hi</p>"), None, false);
        assert!(notes_for(ContentMode::Safe, true, n.clone()).is_none());
        // Saved at install for an adult model: hidden even when the card says nothing.
        let adult = CreatorNotes::from_html(Some("<p>hi</p>"), None, true);
        assert!(notes_for(ContentMode::Safe, false, adult.clone()).is_none());
        assert_eq!(notes_for(ContentMode::All, false, adult.clone()), adult);
        assert_eq!(notes_for(ContentMode::Safe, false, n.clone()), n);
        assert_eq!(notes_for(ContentMode::All, true, n.clone()), n);
    }

    #[test]
    fn external_links_are_https_only() {
        assert!(external_link("https://patreon.com/x?y=1").is_ok());
        for bad in [
            "http://example.com",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://user:pw@example.com",
            "not a url",
        ] {
            assert!(external_link(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn keeps_only_known_generation_keys() {
        let g = gallery(
            &version(),
            ContentMode::All,
            false,
            &SafeFilter::default(),
            450,
        );
        let m = g.items[0].generation.as_ref().unwrap();
        assert_eq!(m.get("steps"), Some(&Value::from(6)));
        assert!(
            m.contains_key("prompt")
                && m.contains_key("Model hash")
                && m.contains_key("Lora hashes")
        );
        assert!(!m.contains_key("ADetailer model") && !m.contains_key("Version"));
        // Only a model name: nothing to apply.
        assert!(g.items[1].generation.is_none());
    }

    #[test]
    fn renditions() {
        let g = gallery(
            &version(),
            ContentMode::All,
            false,
            &SafeFilter::default(),
            450,
        );
        assert_eq!(
            g.items[0].thumb_url,
            thumbnail_url(
                "https://image.civitai.com/xG1/5403/original=true/12221833.jpeg",
                450
            )
        );
        assert!(g.items[0].thumb_url.contains("/width="));
        assert_eq!(
            g.items[0].full_url,
            "https://image.civitai.com/xG1/5403/original=true/12221833.jpeg"
        );
        assert_eq!(
            original_url("https://image.civitai.com/a/u/width=450/2.jpeg"),
            "https://image.civitai.com/a/u/original=true/2.jpeg"
        );
        assert_eq!(
            original_url("https://image.civitai.com/a/u/width=450,optimized=true/2.jpeg"),
            "https://image.civitai.com/a/u/original=true/2.jpeg"
        );
        assert_eq!(
            original_url("https://image.civitai.com/a/u/2.jpeg"),
            "https://image.civitai.com/a/u/2.jpeg"
        );
    }

    #[test]
    fn unrated_images_of_nsfw_models_are_nsfw() {
        let v = VersionImages {
            images: vec![VersionImage {
                image: ModelImage {
                    url: "https://image.civitai.com/a/b/width=450/1.jpeg".into(),
                    ..Default::default()
                },
                meta: None,
            }],
            ..Default::default()
        };
        assert!(
            gallery(&v, ContentMode::Safe, true, &SafeFilter::default(), 450)
                .items
                .is_empty()
        );
        // Unrated images need an explicit "not NSFW" flag in Safe mode, like card previews.
        assert!(
            gallery(&v, ContentMode::Safe, false, &SafeFilter::default(), 450)
                .items
                .is_empty()
        );
        let mut flagged = v.clone();
        flagged.images[0].image.nsfw = Some(false);
        assert_eq!(
            gallery(
                &flagged,
                ContentMode::Safe,
                false,
                &SafeFilter::default(),
                450
            )
            .items
            .len(),
            1
        );
        assert_eq!(
            gallery(&v, ContentMode::All, false, &SafeFilter::default(), 450)
                .items
                .len(),
            1
        );
    }

    #[test]
    fn safe_mode_uses_the_preview_rule_and_blocked_images_never_show() {
        let img = |level: u32| VersionImage {
            image: ModelImage {
                url: format!("https://image.civitai.com/a/b/width=450/{level}.jpeg"),
                nsfw_level: Some(level),
                ..Default::default()
            },
            meta: None,
        };
        let v = VersionImages {
            images: vec![img(1), img(2), img(32)],
            ..Default::default()
        };
        let safe = gallery(&v, ContentMode::Safe, false, &SafeFilter::default(), 450);
        assert_eq!(
            safe.items.iter().map(|i| i.index).collect::<Vec<_>>(),
            vec![0]
        );
        assert_eq!(safe.hidden_nsfw, 1);
        let all = gallery(&v, ContentMode::All, false, &SafeFilter::default(), 450);
        assert_eq!(
            all.items.iter().map(|i| i.index).collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[tokio::test]
    async fn offline_needs_no_network() {
        let (_t, core) = crate::app::tests::test_core(std::sync::Arc::new(
            crate::app::tests::Recorder::default(),
        ));
        core.offline.set(true);
        assert!(
            model_gallery(&core, 1, ContentMode::Safe, false)
                .await
                .unwrap()
                .offline
        );
        assert_eq!(civitai_page(0, None, false).unwrap_err().code, "invalid");
    }

    #[test]
    fn page_urls() {
        assert_eq!(
            civitai_page_url(4201, Some(501240), false),
            "https://civitai.com/models/4201?modelVersionId=501240"
        );
        assert_eq!(
            civitai_page_url(4201, None, true),
            "https://civitai.red/models/4201"
        );
    }
}
