//! Model-card view models for Browse (SPEC §5.4 "Model card").

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use pinhole_registry::vram::{Fit, VramNeed};
use pinhole_registry::wiring::HwContext;
use pinhole_registry::Registry;
use pinhole_store::datadir::ModelKind;
use pinhole_store::InstalledIndex;

use crate::api::{Model, ModelImage, ModelVersion};
use crate::families::{self, FamilyResolution, OTHER_BASE_MODEL};
use crate::filters::{BrowseQuery, CatalogFilters, ContentMode, Hidden};
use crate::safe::SafeFilter;
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
    /// The family's main file is the diffusion model alone (VAE and text encoders come
    /// separately), so a GGUF of it can stand in for a safetensors file.
    pub diffusion_only: bool,
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
            installed_versions: index
                .files
                .iter()
                .filter_map(|f| f.civitai.as_ref().map(|c| c.version_id))
                .collect(),
            installed_sha: index
                .files
                .iter()
                .map(|f| f.sha256.to_ascii_lowercase())
                .collect(),
        }
    }
}

impl CatalogEnv for RegistryEnv<'_> {
    fn family_for(&self, base_model: &str, sha256: Option<&str>) -> Option<FamilyInfo> {
        let id = match families::resolve_family(self.registry, sha256, Some(base_model), None) {
            FamilyResolution::Resolved(id) => id,
            FamilyResolution::Ambiguous(c)
                if !base_model.eq_ignore_ascii_case(OTHER_BASE_MODEL) =>
            {
                c.into_iter().next()?
            }
            _ => return None,
        };
        let f = self.registry.family(&id)?;
        Some(FamilyInfo {
            id,
            label: f.label.clone(),
            license_note: f.license_note.clone(),
            diffusion_only: families::main_model_kind(f) == ModelKind::Diffusion,
        })
    }

    fn vram_for(&self, family_id: &str, main_bytes: u64) -> Option<(VramNeed, Fit)> {
        let f = self.registry.family(family_id)?;
        let need = families::family_need(self.registry, f, &self.hw, main_bytes);
        Some(families::need_and_fit(
            self.registry,
            f,
            &self.hw,
            need,
            main_bytes,
        ))
    }

    fn is_installed(&self, version_id: u64, sha256: Option<&str>) -> bool {
        self.installed_versions.contains(&version_id)
            || sha256.is_some_and(|h| self.installed_sha.contains(&h.to_ascii_lowercase()))
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
    card_or_hidden(ctx, env, m).ok()
}

/// The card for one `/models` item, or why a filter drops it.
pub fn card_or_hidden(
    ctx: &CardContext,
    env: &dyn CatalogEnv,
    m: &Model,
) -> Result<CatalogCard, Hidden> {
    let f = ctx.filters;
    if let Some(hidden) = f.hidden_by(ctx.query, m) {
        return Err(hidden);
    }
    let version = f
        .pick_version(ctx.query, m, |b| env.is_compatible(b), ctx.now)
        .ok_or(Hidden::Other)?;
    let card = card_for_version(ctx.filters, ctx.query.content, env, m, version, ctx.now);
    // Unknown size (LoRAs, no family) is never hidden: they have no figure of their own.
    if ctx.query.runs_on_my_card && card.fit == Some(Fit::TooBig) {
        return Err(Hidden::TooBig);
    }
    Ok(card)
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
    let default_file = select::select_file(&v.files, &filters.allowed_file_formats);
    let family = env.family_for(
        &v.base_model,
        default_file
            .as_ref()
            .ok()
            .and_then(|f| f.sha256())
            .as_deref(),
    );
    let is_lora = filters.is_lora_type(&m.kind);
    // A smaller file of the version when the usual one doesn't fit this card (SPEC §6.2).
    let (file, smaller) = match (&family, is_lora) {
        (Some(fam), false) => {
            match select::select_file_for_machine(
                &v.files,
                &filters.allowed_file_formats,
                fam.diffusion_only,
                None,
                |f| env.vram_for(&fam.id, f.size_bytes()).map(|(_, fit)| fit),
            ) {
                Ok((f, smaller)) => (Ok(f), smaller),
                Err(e) => (Err(e), false),
            }
        }
        _ => (default_file, false),
    };
    let sha = file.as_ref().ok().and_then(|f| f.sha256());
    let mut blocked_reason = file.as_ref().err().cloned();
    if blocked_reason.is_none()
        && family.is_none()
        && !v.base_model.eq_ignore_ascii_case(OTHER_BASE_MODEL)
    {
        blocked_reason = Some(families::unsupported_message(
            Some(v.base_model.as_str()).filter(|b| !b.is_empty()),
        ));
    }
    if m.is_unavailable() {
        blocked_reason =
            Some("This model was archived by its creator and can't be downloaded.".into());
    }
    let download_bytes = file
        .as_ref()
        .ok()
        .map(|f| f.size_bytes())
        .filter(|b| *b > 0);
    let vram = match (&family, download_bytes) {
        (Some(fam), Some(bytes)) if !is_lora => env.vram_for(&fam.id, bytes),
        _ => None,
    };
    let preview = pick_preview(m, v, content, &filters.safe);
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
        preview_url: preview.map(|i| preview_url(i, filters.preview_width)),
        preview_is_video: preview.is_some_and(|i| i.is_video()),
        // Unrated images of an NSFW model are treated as NSFW.
        preview_nsfw: preview
            .is_some_and(|i| i.is_nsfw() || (i.nsfw_level.is_none() && i.nsfw.is_none() && m.nsfw)),
        model_nsfw: m.nsfw,
        thumbs_up_ratio: m
            .stats
            .thumbs_up_ratio()
            .or_else(|| v.stats.thumbs_up_ratio()),
        download_count: m.stats.download_count.max(v.stats.download_count),
        download_bytes,
        vram: vram.map(|(n, _)| n),
        fit: vram.map(|(_, f)| f),
        early_access: v.is_early_access(now),
        commercial_ok,
        license_note: family.as_ref().and_then(|f| f.license_note.clone()),
        installed: env.is_installed(v.id, sha.as_deref()),
        blocked_reason,
        smaller_file: file
            .as_ref()
            .ok()
            .filter(|_| smaller)
            .map(|f| select::precision_label(f)),
    }
}

/// The card's preview image. Safe mode uses only images rated PG (the
/// shipped `max_preview_level`): a still one of the shown version, then of
/// another version, then a video; none → no preview. Other modes: the
/// version's first still image, else its first video, else another version's
/// still image. The UI blurs `previewNsfw` while Safe mode is on.
pub fn pick_preview<'a>(
    m: &'a Model,
    v: &'a ModelVersion,
    content: ContentMode,
    safe: &SafeFilter,
) -> Option<&'a ModelImage> {
    let usable = |i: &&ModelImage| {
        !i.url.trim().is_empty() && i.nsfw_level.is_none_or(|l| l < crate::safe::LEVEL_BLOCKED)
    };
    let still = |i: &&ModelImage| !i.is_video();
    let others = || {
        m.model_versions
            .iter()
            .filter(|o| o.id != v.id)
            .flat_map(|o| o.images.iter())
    };
    if content == ContentMode::Safe {
        let ok = |i: &&ModelImage| usable(i) && safe.is_safe_preview(i);
        return v
            .images
            .iter()
            .filter(ok)
            .find(still)
            .or_else(|| others().filter(ok).find(still))
            .or_else(|| v.images.iter().find(ok));
    }
    v.images
        .iter()
        .filter(usable)
        .find(still)
        .or_else(|| v.images.iter().find(usable))
        .or_else(|| others().filter(usable).find(still))
}

/// Widths CivitAI's image CDN keeps renditions of (`COMMON_IMAGE_WIDTHS` in CivitAI's
/// `src/client-utils/edge-url.ts`). Other widths are snapped up to the next one, as
/// CivitAI's own site does, so card previews are CDN cache hits.
pub const CDN_WIDTHS: [u32; 8] = [96, 320, 450, 512, 800, 1200, 1600, 2200];
/// CivitAI caps requested widths at this.
const CDN_MAX_WIDTH: u32 = 1800;

fn snap_width(width: u32) -> u32 {
    CDN_WIDTHS
        .iter()
        .copied()
        .find(|w| *w >= width)
        .unwrap_or(width)
        .min(CDN_MAX_WIDTH)
}

/// Ask CivitAI's image CDN for a small, compressed rendition: the transform path
/// segment (`original=true`, `width=…`) becomes `width=<w>,optimized=true` — the URL
/// CivitAI's own model cards use (`getEdgeUrl(src, { width: 450 })`). Other URLs are
/// unchanged.
pub fn thumbnail_url(url: &str, width: u32) -> String {
    edge_url(url, width, false)
}

/// The card preview URL for an image: [`thumbnail_url`] for stills; for a video a
/// still frame (`anim=false,transcode=true`, `.jpeg` name, as CivitAI's site asks
/// for when autoplay is off), so a card never downloads a video.
pub fn preview_url(img: &ModelImage, width: u32) -> String {
    edge_url(&img.url, width, img.is_video())
}

fn edge_url(url: &str, width: u32, video_still: bool) -> String {
    let Ok(mut parsed) = url::Url::parse(url) else {
        return url.to_string();
    };
    if !parsed
        .host_str()
        .is_some_and(|h| pinhole_net::allow::host_matches(h, "civitai.com"))
    {
        return url.to_string();
    }
    let mut segs: Vec<String> = match parsed.path_segments() {
        Some(s) => s.map(str::to_string).collect(),
        None => return url.to_string(),
    };
    let is_transform = |s: &str| {
        s.split(',').all(|p| p.contains('='))
            && s.split(',')
                .any(|p| p.starts_with("width=") || p == "original=true")
    };
    let Some(pos) = segs.iter().position(|s| is_transform(s)) else {
        return url.to_string();
    };
    let w = snap_width(width);
    segs[pos] = if video_still {
        format!("anim=false,transcode=true,width={w},optimized=true")
    } else {
        format!("width={w},optimized=true")
    };
    let has_name = pos + 1 < segs.len();
    if let Some(name) = segs.last_mut().filter(|_| video_still && has_name) {
        let stem = name
            .rsplit_once('.')
            .map_or(name.as_str(), |(stem, _)| stem)
            .to_string();
        *name = format!("{stem}.jpeg");
    }
    parsed.set_path(&segs.join("/"));
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
            Some(FamilyInfo {
                id: id.into(),
                label: label.into(),
                license_note: lic.map(Into::into),
                diffusion_only: id.starts_with("flux"),
            })
        }
        fn vram_for(&self, _family: &str, bytes: u64) -> Option<(VramNeed, Fit)> {
            let gb = bytes as f32 / 1e9 + 2.0;
            let need = VramNeed {
                gb,
                min_gb: gb * 0.7,
                estimate: true,
                on_cpu: false,
            };
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
        FakeEnv {
            vram_gb: 8.0,
            installed_versions: vec![789646],
        }
    }

    #[test]
    fn realistic_checkpoint_card() {
        let f = filters();
        let q = BrowseQuery::default();
        let ctx = CardContext {
            filters: &f,
            query: &q,
            now: now(),
        };
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
            Some("https://image.civitai.com/xG1nkqKTMzGDvpLrqFT7WA/3a4bd3bb-0cf8-4a0e-9e4c-9a3f5b1b0c11/width=450,optimized=true/26940521.jpeg")
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
        let q = BrowseQuery {
            compatible_only: false,
            ..Default::default()
        };
        let ctx = CardContext {
            filters: &f,
            query: &q,
            now: now(),
        };
        let page = page();
        let by = |id: u64| {
            build_card(
                &ctx,
                &env(),
                page.items.iter().find(|m| m.id == id).unwrap(),
            )
            .unwrap()
        };
        assert!(by(5000).blocked_reason.unwrap().contains(".ckpt"));
        assert_eq!(
            by(900001).blocked_reason.as_deref(),
            Some("Pinhole can't run SD 3.5 Large models yet.")
        );
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
        // Safe mode off, LoRA: first image is X-rated → flagged.
        let q = BrowseQuery {
            kind: CatalogKind::StyleAddons,
            content: ContentMode::All,
            price: PriceMode::Include,
            ..Default::default()
        };
        let ctx = CardContext {
            filters: &f,
            query: &q,
            now: now(),
        };
        let c = build_card(&ctx, &env(), &page.items[1]).unwrap();
        assert!(c.preview_nsfw);
        assert!(c.model_nsfw);
        assert!(c.early_access);
        assert_eq!(c.vram, None, "no VRAM figure for LoRAs");
        assert_eq!(c.family_id.as_deref(), Some("sdxl_pony"));
        assert_eq!(c.style_badge.as_deref(), Some("Anime"));

        // Video first → the still image is used.
        let q = BrowseQuery::default();
        let ctx = CardContext {
            filters: &f,
            query: &q,
            now: now(),
        };
        let toon = build_card(
            &ctx,
            &env(),
            page.items.iter().find(|m| m.id == 800002).unwrap(),
        )
        .unwrap();
        assert!(!toon.preview_is_video);
        assert!(toon
            .preview_url
            .unwrap()
            .ends_with("/width=450,optimized=true/5.jpeg"));
        assert_eq!(toon.style_badge.as_deref(), Some("3D"));
    }

    #[test]
    fn safe_mode_uses_pg_previews_only() {
        let f = filters();
        let m: Model = serde_json::from_value(serde_json::json!({ "id": 1, "modelVersions": [
            { "id": 11, "images": [
                { "url": "https://image.civitai.com/x/a/original=true/1.jpeg", "nsfwLevel": 8 },
                { "url": "https://image.civitai.com/x/b/original=true/2.jpeg", "nsfwLevel": 2 },
                { "url": "https://image.civitai.com/x/c/original=true/3.mp4", "nsfwLevel": 1 } ] },
            { "id": 12, "images": [
                { "url": "https://image.civitai.com/x/d/original=true/4.jpeg", "nsfwLevel": 1 } ] },
            { "id": 13, "images": [
                { "url": "https://image.civitai.com/x/e/original=true/5.jpeg", "nsfwLevel": 4 } ] } ] }))
        .unwrap();
        let v = |i: usize| &m.model_versions[i];
        let pick = |i: usize, c| {
            pick_preview(&m, v(i), c, &f.safe)
                .map(|img| img.url.rsplit('/').next().unwrap().to_string())
        };
        assert_eq!(
            pick(0, ContentMode::Safe).as_deref(),
            Some("4.jpeg"),
            "a PG still of another version beats a PG video"
        );
        assert_eq!(pick(0, ContentMode::All).as_deref(), Some("1.jpeg"));
        assert_eq!(pick(1, ContentMode::Safe).as_deref(), Some("4.jpeg"));
        assert_eq!(
            pick(2, ContentMode::Safe).as_deref(),
            Some("4.jpeg"),
            "never the R image in Safe"
        );
        assert_eq!(pick(2, ContentMode::All).as_deref(), Some("5.jpeg"));
        let only_video: Model =
            serde_json::from_value(serde_json::json!({ "id": 2, "modelVersions": [{ "id": 21, "images": [{ "url": "https://image.civitai.com/x/v.mp4", "nsfwLevel": 1 }] }] }))
                .unwrap();
        assert!(pick_preview(
            &only_video,
            &only_video.model_versions[0],
            ContentMode::Safe,
            &f.safe
        )
        .unwrap()
        .is_video());
        let none: Model = serde_json::from_value(serde_json::json!({ "id": 3, "modelVersions": [{ "id": 31, "images": [{ "url": "u", "nsfwLevel": 2 }] }] })).unwrap();
        assert!(
            pick_preview(&none, &none.model_versions[0], ContentMode::Safe, &f.safe).is_none(),
            "PG-13 only → no preview"
        );
        let blocked: Model = serde_json::from_value(
            serde_json::json!({ "id": 4, "modelVersions": [{ "id": 41, "images": [
            { "url": "https://image.civitai.com/x/b/original=true/b.jpeg", "nsfwLevel": 32 },
            { "url": "https://image.civitai.com/x/r/original=true/r.jpeg", "nsfwLevel": 4 } ] }] }),
        )
        .unwrap();
        let p = pick_preview(
            &blocked,
            &blocked.model_versions[0],
            ContentMode::All,
            &f.safe,
        )
        .unwrap();
        assert!(
            p.url.ends_with("r.jpeg"),
            "images CivitAI blocked are never previews"
        );
    }

    #[test]
    fn registry_env() {
        use crate::testkit::{hw, index, model, registry, with_civitai};
        let reg = registry();
        let idx = index(vec![with_civitai(
            model(
                "j",
                "sdxl",
                pinhole_store::datadir::ModelKind::Checkpoint,
                "j.safetensors",
            ),
            42,
        )]);
        let env = RegistryEnv::new(&reg, hw(8.0), &idx);
        assert_eq!(env.family_for("SDXL 1.0", None).unwrap().id, "sdxl");
        assert_eq!(env.family_for("Pony", None).unwrap().id, "sdxl_pony");
        assert_eq!(
            env.family_for("NoobAI", None).unwrap().id,
            "sdxl_illustrious"
        );
        assert_eq!(
            env.family_for("Flux.1 D", None)
                .unwrap()
                .license_note
                .as_deref(),
            Some("Non-commercial license")
        );
        assert_eq!(
            env.family_for("Qwen", None).unwrap().id,
            "qwen_image",
            "ambiguous: first candidate for display"
        );
        assert!(
            env.family_for("MiniMax H3", None).is_none(),
            "video + audio only in the pinned engine"
        );
        assert_eq!(env.family_for("SD 3.5 Large", None).unwrap().id, "sd3");
        assert!(env.family_for("Other", None).is_none());
        // Known hash beats the base model.
        let zit = "2407613050b809ffdff18a4ac99af83ea6b95443ecebdf80e064a79c825574a6";
        assert_eq!(
            env.family_for("ZImageTurbo", Some(zit)).unwrap().id,
            "z_image_turbo"
        );
        assert!(env.is_installed(42, None));
        assert!(!env.is_installed(43, None));
        assert!(env.is_installed(1, Some(&idx.files[0].sha256.to_ascii_uppercase())));
        let (need, fit) = env.vram_for("sdxl", 6_938_065_160).unwrap();
        assert_eq!((need.gb, need.min_gb, need.estimate), (10.0, 6.0, false));
        assert_eq!(fit, Fit::Tight);
        let (need, _) = env.vram_for("flux1_dev", 12_000_000_000).unwrap();
        assert!(need.estimate, "no registry figure for FLUX.1 dev files");
    }

    #[test]
    fn thumbnails() {
        // The URL CivitAI's own cards use (verified in CivitAI's edge-url.ts).
        assert_eq!(
            thumbnail_url(
                "https://image.civitai.com/xG1/1c65/original=true/12221824.jpeg",
                450
            ),
            "https://image.civitai.com/xG1/1c65/width=450,optimized=true/12221824.jpeg"
        );
        assert_eq!(
            thumbnail_url(
                "https://image.civitai.com/xG1/1c65/anim=false,width=1024/1.jpeg",
                320
            ),
            "https://image.civitai.com/xG1/1c65/width=320,optimized=true/1.jpeg"
        );
        // Widths snap up to the CDN's sizes (cache hits), capped like CivitAI does.
        assert!(
            thumbnail_url("https://image.civitai.com/x/y/original=true/1.jpeg", 400)
                .contains("/width=450,optimized=true/")
        );
        assert!(
            thumbnail_url("https://image.civitai.com/x/y/original=true/1.jpeg", 2048)
                .contains("/width=1800,optimized=true/")
        );
        assert_eq!(
            thumbnail_url("https://example.com/width=100/x.jpeg", 450),
            "https://example.com/width=100/x.jpeg"
        );
        assert_eq!(
            thumbnail_url("https://image.civitai.com/x/1.jpeg", 450),
            "https://image.civitai.com/x/1.jpeg",
            "no transform segment"
        );
        assert_eq!(thumbnail_url("not a url", 450), "not a url");
    }

    #[test]
    fn video_previews_become_stills() {
        let img = |json: &str| serde_json::from_str::<ModelImage>(json).unwrap();
        let video = img(
            r#"{"url":"https://image.civitai.com/xG1/418f/original=true/141593346.mp4","type":"video","nsfwLevel":1}"#,
        );
        assert_eq!(preview_url(&video, 450), "https://image.civitai.com/xG1/418f/anim=false,transcode=true,width=450,optimized=true/141593346.jpeg");
        let still = img(
            r#"{"url":"https://image.civitai.com/xG1/13f6/original=true/141594075.jpeg","type":"image"}"#,
        );
        assert_eq!(
            preview_url(&still, 450),
            "https://image.civitai.com/xG1/13f6/width=450,optimized=true/141594075.jpeg"
        );
        // No transform segment: left alone (the UI then shows "Video preview").
        let bare = img(r#"{"url":"https://image.civitai.com/x/v.mp4"}"#);
        assert_eq!(preview_url(&bare, 450), "https://image.civitai.com/x/v.mp4");
    }

    #[test]
    fn runs_on_my_card_hides_too_big_models() {
        let f = filters();
        let page = page();
        let small = FakeEnv {
            vram_gb: 4.0,
            installed_versions: vec![],
        };
        let q = BrowseQuery {
            runs_on_my_card: true,
            ..Default::default()
        };
        let ctx = CardContext {
            filters: &f,
            query: &q,
            now: now(),
        };
        // The 6.9 GB SDXL checkpoint needs ~8.9 GB: Too big on a 4 GB card.
        assert!(matches!(
            card_or_hidden(&ctx, &small, &page.items[0]),
            Err(Hidden::TooBig)
        ));
        // Tight still runs: shown.
        assert_eq!(
            card_or_hidden(&ctx, &env(), &page.items[0]).unwrap().fit,
            Some(Fit::Tight)
        );
        let q = BrowseQuery::default();
        let ctx = CardContext {
            filters: &f,
            query: &q,
            now: now(),
        };
        assert_eq!(
            card_or_hidden(&ctx, &small, &page.items[0]).unwrap().fit,
            Some(Fit::TooBig),
            "off: shown with its badge"
        );
    }

    #[test]
    fn smaller_file_is_picked_when_the_usual_one_does_not_fit() {
        let f = filters();
        let q = BrowseQuery::default();
        let ctx = CardContext {
            filters: &f,
            query: &q,
            now: now(),
        };
        let file = |id: u64, name: &str, fp: &str, kb: f64, primary: bool| {
            serde_json::json!({ "id": id, "name": name, "sizeKB": kb, "type": "Model", "primary": primary,
                "pickleScanResult": "Success", "virusScanResult": "Success",
                "metadata": { "format": "SafeTensor", "fp": fp }, "hashes": { "SHA256": format!("{id}").repeat(64) } })
        };
        let m: Model = serde_json::from_value(serde_json::json!({ "id": 7, "name": "Big", "type": "Checkpoint", "modelVersions": [
            { "id": 70, "baseModel": "SDXL 1.0", "files": [ file(1, "big_fp16.safetensors", "fp16", 13_000_000.0, true), file(2, "big_fp8.safetensors", "fp8", 6_500_000.0, false) ] } ] }))
        .unwrap();
        // FakeEnv: need = GB + 2; 12 GB card → fp16 (15.3) too big, fp8 (8.7) Fits.
        let e = FakeEnv {
            vram_gb: 12.0,
            installed_versions: vec![],
        };
        let c = card_or_hidden(&ctx, &e, &m).unwrap();
        assert_eq!(c.download_bytes, Some(6_656_000_000));
        assert_eq!(c.fit, Some(Fit::Fits));
        assert_eq!(c.smaller_file.as_deref(), Some("Compact (FP8)"));
        let e = FakeEnv {
            vram_gb: 24.0,
            installed_versions: vec![],
        };
        let c = card_or_hidden(&ctx, &e, &m).unwrap();
        assert_eq!(c.download_bytes, Some(13_312_000_000));
        assert_eq!(c.smaller_file, None);
    }
}
