//! Model-card view models for Browse (SPEC §5.4 "Model card").

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use pinhole_registry::vram::{self, Fit, VramNeed};
use pinhole_registry::wiring::HwContext;
use pinhole_registry::Registry;
use pinhole_store::InstalledIndex;

use crate::api::{Model, ModelImage, ModelVersion};
use crate::families::{self, FamilyResolution, OTHER_BASE_MODEL};
use crate::filters::{BrowseQuery, CatalogFilters, ContentMode};
use crate::select;
use crate::view::CatalogCard;

/// What a card needs to know about Pinhole (registry, hardware, installed files).
/// Abstracted so card logic can be tested without a registry.
pub trait CatalogEnv {
    /// Family for a CivitAI base model (+ file hash for known files).
    fn family_for(&self, base_model: &str, sha256: Option<&str>) -> Option<FamilyInfo>;
    /// Whether Pinhole can run models with this CivitAI base model.
    fn is_compatible(&self, base_model: &str) -> bool {
        self.family_for(base_model, None).is_some()
    }
    /// "Needs ~X GB" + badge for a main model file of `family_id`.
    fn vram_for(&self, family_id: &str, main_bytes: u64) -> Option<(VramNeed, Fit)>;
    fn is_installed(&self, version_id: u64, sha256: Option<&str>) -> bool;
}

#[derive(Debug, Clone, PartialEq)]
pub struct FamilyInfo {
    pub id: String,
    pub label: String,
    pub license_note: Option<String>,
}

/// The real environment: registry + effective hardware + installed index.
pub struct RegistryEnv<'a> {
    pub registry: &'a Registry,
    pub hw: HwContext,
    pub installed_versions: HashSet<u64>,
    pub installed_sha: HashSet<String>,
}

impl<'a> RegistryEnv<'a> {
    pub fn new(registry: &'a Registry, hw: HwContext, index: &InstalledIndex) -> Self {
        Self {
            registry,
            hw,
            installed_versions: index.files.iter().filter_map(|f| f.civitai.as_ref().map(|c| c.version_id)).collect(),
            installed_sha: index.files.iter().map(|f| f.sha256.to_ascii_lowercase()).collect(),
        }
    }
}

impl CatalogEnv for RegistryEnv<'_> {
    fn family_for(&self, base_model: &str, sha256: Option<&str>) -> Option<FamilyInfo> {
        let id = match families::resolve_family(self.registry, sha256, Some(base_model), None) {
            FamilyResolution::Resolved(id) => id,
            FamilyResolution::Ambiguous(c) if !base_model.eq_ignore_ascii_case(OTHER_BASE_MODEL) => c.into_iter().next()?,
            _ => return None,
        };
        let f = self.registry.family(&id)?;
        Some(FamilyInfo { id, label: f.label.clone(), license_note: f.license_note.clone() })
    }

    fn vram_for(&self, family_id: &str, main_bytes: u64) -> Option<(VramNeed, Fit)> {
        let f = self.registry.family(family_id)?;
        let need = families::family_need(self.registry, f, &self.hw, main_bytes);
        Some((need, vram::fit(&need, self.hw.vram_gb)))
    }

    fn is_installed(&self, version_id: u64, sha256: Option<&str>) -> bool {
        self.installed_versions.contains(&version_id) || sha256.is_some_and(|h| self.installed_sha.contains(&h.to_ascii_lowercase()))
    }
}

/// Browse-time context.
pub struct CardContext<'a> {
    pub filters: &'a CatalogFilters,
    pub query: &'a BrowseQuery,
    pub now: DateTime<Utc>,
}

/// Build the card for one `/models` item, or `None` if a filter drops it.
pub fn build_card(ctx: &CardContext, env: &dyn CatalogEnv, m: &Model) -> Option<CatalogCard> {
    let f = ctx.filters;
    if !f.keep_model(ctx.query, m) {
        return None;
    }
    let version = f.pick_version(ctx.query, m, |b| env.is_compatible(b), ctx.now)?;
    Some(card_for_version(ctx.filters, ctx.query.content, env, m, version, ctx.now))
}

/// Card for a specific version (also used by install plans and paste lookups).
pub fn card_for_version(
    filters: &CatalogFilters,
    content: ContentMode,
    env: &dyn CatalogEnv,
    m: &Model,
    v: &ModelVersion,
    now: DateTime<Utc>,
) -> CatalogCard {
    let file = select::select_file(&v.files, &filters.allowed_file_formats);
    let sha = file.as_ref().ok().and_then(|f| f.sha256());
    let family = env.family_for(&v.base_model, sha.as_deref());
    let is_lora = filters.is_lora_type(&m.kind);
    let mut blocked_reason = file.as_ref().err().cloned();
    if blocked_reason.is_none() && family.is_none() && !v.base_model.eq_ignore_ascii_case(OTHER_BASE_MODEL) {
        blocked_reason = Some(families::unsupported_message(Some(v.base_model.as_str()).filter(|b| !b.is_empty())));
    }
    if m.is_unavailable() {
        blocked_reason = Some("This model was archived by its creator and can't be downloaded.".into());
    }
    let download_bytes = file.as_ref().ok().map(|f| f.size_bytes()).filter(|b| *b > 0);
    let vram = match (&family, download_bytes) {
        (Some(fam), Some(bytes)) if !is_lora => env.vram_for(&fam.id, bytes),
        _ => None,
    };
    let preview = pick_preview(&v.images, content);
    let commercial_ok = m.allow_commercial_use.allows("Image");
    CatalogCard {
        model_id: m.id,
        version_id: v.id,
        name: m.name.clone(),
        version_name: v.name.clone(),
        kind: m.kind.clone(),
        base_model: v.base_model.clone(),
        family_id: family.as_ref().map(|f| f.id.clone()),
        style_badge: filters.style_badge(&m.tags),
        creator: m.creator.as_ref().and_then(|c| c.username.clone()),
        preview_url: preview.map(|i| thumbnail_url(&i.url, filters.preview_width)),
        preview_is_video: preview.is_some_and(|i| i.is_video()),
        // Unrated images of an NSFW model are treated as NSFW.
        preview_nsfw: preview.is_some_and(|i| i.is_nsfw() || (i.nsfw_level.is_none() && i.nsfw.is_none() && m.nsfw)),
        model_nsfw: m.nsfw,
        thumbs_up_ratio: m.stats.thumbs_up_ratio().or_else(|| v.stats.thumbs_up_ratio()),
        download_count: m.stats.download_count.max(v.stats.download_count),
        download_bytes,
        vram: vram.map(|(n, _)| n),
        fit: vram.map(|(_, f)| f),
        early_access: v.is_early_access(now),
        commercial_ok,
        license_note: family.as_ref().and_then(|f| f.license_note.clone()),
        installed: env.is_installed(v.id, sha.as_deref()),
        blocked_reason,
    }
}

/// First preview image of the version. In Safe mode a safe still image is
/// preferred; otherwise still images beat videos. The UI blurs `previewNsfw`.
pub fn pick_preview(images: &[ModelImage], content: ContentMode) -> Option<&ModelImage> {
    let usable = |i: &&ModelImage| !i.url.trim().is_empty();
    let still = |i: &&ModelImage| !i.is_video();
    let sfw = |i: &&ModelImage| !i.is_nsfw();
    if content == ContentMode::Safe {
        if let Some(i) = images.iter().filter(usable).find(|i| still(i) && sfw(i)) {
            return Some(i);
        }
        if let Some(i) = images.iter().filter(usable).find(sfw) {
            return Some(i);
        }
    }
    images.iter().filter(usable).find(still).or_else(|| images.iter().find(usable))
}

/// Ask CivitAI's image CDN for a small rendition: replace the `width=…` /
/// `original=true` path segment with `width=<w>`. Other URLs are unchanged.
pub fn thumbnail_url(url: &str, width: u32) -> String {
    let Ok(mut parsed) = url::Url::parse(url) else { return url.to_string() };
    if !parsed.host_str().is_some_and(|h| pinhole_net::allow::host_matches(h, "civitai.com")) {
        return url.to_string();
    }
    let segs: Vec<String> = match parsed.path_segments() {
        Some(s) => s.map(str::to_string).collect(),
        None => return url.to_string(),
    };
    let is_transform = |s: &str| s.split(',').all(|p| p.contains('=')) && s.split(',').any(|p| p.starts_with("width=") || p == "original=true");
    let Some(pos) = segs.iter().position(|s| is_transform(s)) else { return url.to_string() };
    let mut new = segs.clone();
    new[pos] = format!("width={width}");
    parsed.set_path(&new.join("/"));
    parsed.to_string()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::api::ModelsPage;
    use crate::filters::tests::{filters, now};
    use crate::filters::{CatalogKind, PriceMode};

    /// Fake environment: a fixed base-model → family table.
    pub(crate) struct FakeEnv {
        pub vram_gb: f32,
        pub installed_versions: Vec<u64>,
    }

    impl CatalogEnv for FakeEnv {
        fn family_for(&self, base_model: &str, _sha: Option<&str>) -> Option<FamilyInfo> {
            let (id, label, lic) = match base_model {
                "SDXL 1.0" => ("sdxl", "SDXL", None),
                "Pony" => ("sdxl_pony", "SDXL · Pony", None),
                "Illustrious" => ("sdxl_illustrious", "SDXL · Illustrious / NoobAI", None),
                "SD 1.5" => ("sd15", "Stable Diffusion 1.5", None),
                "Flux.1 D" => ("flux1_dev", "FLUX.1 dev", Some("Non-commercial license")),
                _ => return None,
            };
            Some(FamilyInfo { id: id.into(), label: label.into(), license_note: lic.map(Into::into) })
        }
        fn vram_for(&self, _family: &str, bytes: u64) -> Option<(VramNeed, Fit)> {
            let gb = bytes as f32 / 1e9 + 2.0;
            let need = VramNeed { gb, min_gb: gb * 0.7, estimate: true };
            let fit = if gb <= self.vram_gb - 1.0 {
                Fit::Fits
            } else if need.min_gb <= self.vram_gb {
                Fit::Tight
            } else {
                Fit::TooBig
            };
            Some((need, fit))
        }
        fn is_installed(&self, version_id: u64, _sha: Option<&str>) -> bool {
            self.installed_versions.contains(&version_id)
        }
    }

    fn page() -> ModelsPage {
        serde_json::from_str(include_str!("../tests/fixtures/models_page.json")).unwrap()
    }

    fn env() -> FakeEnv {
        FakeEnv { vram_gb: 8.0, installed_versions: vec![789646] }
    }

    #[test]
    fn realistic_checkpoint_card() {
        let f = filters();
        let q = BrowseQuery::default();
        let ctx = CardContext { filters: &f, query: &q, now: now() };
        let page = page();
        let c = build_card(&ctx, &env(), &page.items[0]).unwrap();
        assert_eq!(c.model_id, 139562);
        assert_eq!(c.version_id, 789646);
        assert_eq!(c.kind, "Checkpoint");
        assert_eq!(c.family_id.as_deref(), Some("sdxl"));
        assert_eq!(c.style_badge.as_deref(), Some("Realistic"));
        assert_eq!(c.creator.as_deref(), Some("SG_161222"));
        assert_eq!(
            c.preview_url.as_deref(),
            Some("https://image.civitai.com/xG1nkqKTMzGDvpLrqFT7WA/3a4bd3bb-0cf8-4a0e-9e4c-9a3f5b1b0c11/width=450/26940521.jpeg")
        );
        assert!(!c.preview_nsfw);
        assert_eq!(c.thumbs_up_ratio, Some(0.98));
        assert_eq!(c.download_count, 1_234_567);
        assert_eq!(c.download_bytes, Some(6_938_065_160));
        assert_eq!(c.fit, Some(Fit::Tight));
        assert!(c.vram.unwrap().estimate);
        assert!(c.commercial_ok);
        assert!(!c.early_access);
        assert!(c.installed);
        assert_eq!(c.blocked_reason, None);
        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(json["type"], "Checkpoint");
        assert_eq!(json["versionId"], 789646);
        assert_eq!(json["fit"], "tight");
        assert_eq!(json["vram"]["minGb"].as_f64().map(|v| v > 0.0), Some(true));
    }

    #[test]
    fn blocked_and_unsupported_cards() {
        let f = filters();
        let q = BrowseQuery { compatible_only: false, ..Default::default() };
        let ctx = CardContext { filters: &f, query: &q, now: now() };
        let page = page();
        let by = |id: u64| build_card(&ctx, &env(), page.items.iter().find(|m| m.id == id).unwrap()).unwrap();
        assert!(by(5000).blocked_reason.unwrap().contains(".ckpt"));
        assert_eq!(by(900001).blocked_reason.as_deref(), Some("Pinhole can't run SD 3.5 Large models yet."));
        assert!(by(900001).vram.is_none());
        assert!(by(777001).blocked_reason.unwrap().contains("scan"));
        let flux = by(618692);
        assert_eq!(flux.blocked_reason, None);
        assert_eq!(flux.license_note.as_deref(), Some("Non-commercial license"));
        assert!(!flux.commercial_ok);
        assert_eq!(flux.preview_url, None);
        assert_eq!(flux.thumbs_up_ratio, None);
    }

    #[test]
    fn nsfw_and_video_previews() {
        let f = filters();
        let page = page();
        // Include 18+ LoRA: first image is X-rated → flagged.
        let q = BrowseQuery { kind: CatalogKind::StyleAddons, content: ContentMode::Include18Plus, price: PriceMode::Include, ..Default::default() };
        let ctx = CardContext { filters: &f, query: &q, now: now() };
        let c = build_card(&ctx, &env(), &page.items[1]).unwrap();
        assert!(c.preview_nsfw);
        assert!(c.model_nsfw);
        assert!(c.early_access);
        assert_eq!(c.vram, None, "no VRAM figure for LoRAs");
        assert_eq!(c.family_id.as_deref(), Some("sdxl_pony"));
        assert_eq!(c.style_badge.as_deref(), Some("Anime"));

        // Video first → the still image is used.
        let q = BrowseQuery::default();
        let ctx = CardContext { filters: &f, query: &q, now: now() };
        let toon = build_card(&ctx, &env(), page.items.iter().find(|m| m.id == 800002).unwrap()).unwrap();
        assert!(!toon.preview_is_video);
        assert!(toon.preview_url.unwrap().ends_with("/width=450/5.jpeg"));
        assert_eq!(toon.style_badge.as_deref(), Some("3D"));
    }

    #[test]
    fn safe_mode_prefers_safe_preview() {
        let imgs: Vec<ModelImage> = serde_json::from_str(
            r#"[{"url":"https://image.civitai.com/x/a/width=450/1.jpeg","nsfwLevel":8},{"url":"https://image.civitai.com/x/b/width=450/2.jpeg","nsfwLevel":2}]"#,
        )
        .unwrap();
        assert!(pick_preview(&imgs, ContentMode::Safe).unwrap().url.ends_with("2.jpeg"));
        assert!(pick_preview(&imgs, ContentMode::Include18Plus).unwrap().url.ends_with("1.jpeg"));
        assert!(pick_preview(&[], ContentMode::Safe).is_none());
    }

    #[test]
    fn thumbnails() {
        assert_eq!(
            thumbnail_url("https://image.civitai.com/xG1/1c65/original=true/12221824.jpeg", 450),
            "https://image.civitai.com/xG1/1c65/width=450/12221824.jpeg"
        );
        assert_eq!(
            thumbnail_url("https://image.civitai.com/xG1/1c65/anim=false,width=1024/1.jpeg", 320),
            "https://image.civitai.com/xG1/1c65/width=320/1.jpeg"
        );
        assert_eq!(thumbnail_url("https://example.com/width=100/x.jpeg", 450), "https://example.com/width=100/x.jpeg");
        assert_eq!(thumbnail_url("not a url", 450), "not a url");
    }
}
