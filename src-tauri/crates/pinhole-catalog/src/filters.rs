//! Plain-language Browse filters (SPEC §5.4) from `config/catalog-filters.yaml`:
//! the query parameters sent to `GET /api/v1/models` and the client-side rules
//! (Safe mode via [`crate::safe`], free / early access, Look tags, the Tags
//! multi-select, brand commercial use).

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::api::{Model, ModelVersion};
use crate::safe::SafeFilter;
use crate::view::{CatalogFilterOptions, KeyLabel, KeyedLabel, TagOption};

// ------------------------------------------------------------------ query from the UI

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CatalogKind {
    #[default]
    Models,
    StyleAddons,
}

/// Safe mode: `safe` = On (hides models made for adults), `all` = Off.
/// The old keys (`include_18plus`, `only_18plus`) still read as Off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ContentMode {
    #[default]
    #[serde(rename = "safe")]
    Safe,
    #[serde(rename = "all", alias = "include_18plus", alias = "only_18plus")]
    All,
}

impl ContentMode {
    pub fn key(self) -> &'static str {
        match self {
            ContentMode::Safe => "safe",
            ContentMode::All => "all",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceMode {
    #[default]
    Free,
    Include,
    PaidOnly,
}

impl PriceMode {
    pub fn key(self) -> &'static str {
        match self {
            PriceMode::Free => "free",
            PriceMode::Include => "include",
            PriceMode::PaidOnly => "paid_only",
        }
    }
}

/// `BrowseQuery` in types.ts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BrowseQuery {
    pub kind: CatalogKind,
    /// Look key (`realistic` | `anime` | `illustration` | `three_d` | `brand`) or null.
    pub look: Option<String>,
    /// Tag keys (`catalog-filters.yaml → tags`); a model must match every one.
    pub tags: Vec<String>,
    pub content: ContentMode,
    pub price: PriceMode,
    /// `Highest Rated` | `Most Downloaded` | `Newest`
    pub sort: String,
    /// `Week` | `Month` | `Year` | `AllTime`
    pub period: String,
    pub commercial_only: bool,
    pub compatible_only: bool,
    /// "Runs on my card": hide models that are Too big for this machine (SPEC §5.4).
    pub runs_on_my_card: bool,
    pub query: String,
    pub cursor: Option<String>,
}

impl Default for BrowseQuery {
    fn default() -> Self {
        Self {
            kind: CatalogKind::Models,
            look: None,
            tags: Vec::new(),
            content: ContentMode::Safe,
            price: PriceMode::Free,
            sort: "Most Downloaded".into(),
            period: "AllTime".into(),
            commercial_only: false,
            compatible_only: true,
            runs_on_my_card: false,
            query: String::new(),
            cursor: None,
        }
    }
}

// ------------------------------------------------------------------ YAML

#[derive(Debug, thiserror::Error)]
pub enum FiltersError {
    #[error("could not read catalog filters: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid catalog-filters.yaml: {0}")]
    Yaml(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct KindSpec {
    pub label: String,
    pub api_types: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Kinds {
    pub models: KindSpec,
    pub style_addons: KindSpec,
}

impl Default for Kinds {
    fn default() -> Self {
        Self {
            models: KindSpec { label: "Models".into(), api_types: vec!["Checkpoint".into()] },
            style_addons: KindSpec { label: "Style add-ons".into(), api_types: vec!["LORA".into()] },
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AlsoRequire {
    #[serde(rename = "allowCommercialUse_includes", default)]
    pub allow_commercial_use_includes: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct LookYaml {
    label: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    also_require: Option<AlsoRequire>,
}

#[derive(Debug, Clone)]
pub struct Look {
    pub key: String,
    pub label: String,
    /// Lowercased.
    pub tags: Vec<String>,
    pub also_require: AlsoRequire,
}

impl Look {
    pub fn matches_tags(&self, tags: &[String]) -> bool {
        tags.iter().any(|t| {
            let t = t.trim().to_ascii_lowercase();
            self.tags.contains(&t)
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagRule {
    /// Tags, name words and base models below.
    #[default]
    Match,
    /// The models Safe mode hides (see [`crate::safe`]). Only useful with Safe mode off.
    MadeForAdults,
}

#[derive(Debug, Clone, Deserialize)]
struct TagYaml {
    label: String,
    #[serde(default)]
    rule: TagRule,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    name_words: Vec<String>,
    #[serde(default)]
    base_models: Vec<String>,
}

/// One entry of the Tags multi-select.
#[derive(Debug, Clone)]
pub struct TagFilter {
    pub key: String,
    pub label: String,
    pub rule: TagRule,
    /// Lowercased.
    pub tags: Vec<String>,
    /// Lowercased words (phrases split into words), see [`crate::safe::name_words`].
    pub name_words: Vec<Vec<String>>,
    pub base_models: Vec<String>,
}

impl TagFilter {
    fn matches(&self, m: &Model, safe: &SafeFilter) -> bool {
        match self.rule {
            TagRule::MadeForAdults => safe.adult_reason(m).is_some(),
            TagRule::Match => {
                m.tags.iter().any(|t| self.tags.contains(&t.trim().to_ascii_lowercase()))
                    || {
                        let words = crate::safe::name_words(&m.name);
                        self.name_words.iter().any(|needle| words.windows(needle.len()).any(|w| w == needle.as_slice()))
                    }
                    || m.model_versions.iter().any(|v| self.base_models.iter().any(|b| b.eq_ignore_ascii_case(v.base_model.trim())))
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct StyleBadge {
    pub look: String,
    pub badge: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Labelled {
    pub label: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PriceSection {
    pub free: Labelled,
    pub include: Labelled,
    pub paid_only: Labelled,
    #[serde(default)]
    pub default: PriceMode,
}

/// One Content mode. What it keeps is fixed in code ([`CatalogFilters::keep_model`]):
/// the YAML only picks the label and the `nsfw` query parameter.
#[derive(Debug, Clone, Deserialize)]
pub struct ContentOption {
    pub label: String,
    #[serde(default)]
    pub api_nsfw: Option<bool>,
}

/// Why a `/models` item has no card (Browse shows the counts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hidden {
    /// Safe mode hid a model made for adults.
    Content,
    /// Kind, Look, commercial use, price, compatibility or archived.
    Other,
    /// "Runs on my card": too big for this graphics card / computer.
    TooBig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContentSection {
    pub safe: ContentOption,
    pub all: ContentOption,
    #[serde(default)]
    pub default: ContentMode,
    #[serde(default)]
    pub confirm_once_per_session: bool,
    #[serde(default)]
    pub blur_nsfw_previews_when_safe: bool,
}

impl ContentSection {
    pub fn option(&self, mode: ContentMode) -> &ContentOption {
        match mode {
            ContentMode::Safe => &self.safe,
            ContentMode::All => &self.all,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiOption {
    pub label: String,
    pub api: String,
}

#[derive(Debug, Clone, Deserialize)]
struct FiltersYaml {
    #[serde(default)]
    kinds: Kinds,
    #[serde(default = "default_lora_types")]
    lora_types: Vec<String>,
    #[serde(default)]
    looks: serde_yaml::Mapping,
    #[serde(default)]
    style_badges: Vec<StyleBadge>,
    #[serde(default)]
    tags: serde_yaml::Mapping,
    price: PriceSection,
    content: ContentSection,
    sort: Vec<ApiOption>,
    period: Vec<ApiOption>,
    #[serde(default = "default_formats")]
    allowed_file_formats: Vec<String>,
    #[serde(default)]
    safe_filter: SafeFilter,
    #[serde(default)]
    default_sort: Option<String>,
    #[serde(default)]
    default_period: Option<String>,
    #[serde(default = "default_page_size")]
    page_size: u32,
    #[serde(default)]
    api_limit: Option<u32>,
    #[serde(default = "default_extra")]
    max_extra_requests: u32,
    #[serde(default = "default_cache_minutes")]
    cache_minutes: u32,
    #[serde(default = "default_cache_pages")]
    cache_pages: u32,
    #[serde(default = "default_preview_width")]
    preview_width: u32,
}

fn default_lora_types() -> Vec<String> {
    vec!["LORA".into(), "LoCon".into(), "DoRA".into()]
}
fn default_formats() -> Vec<String> {
    vec!["SafeTensor".into(), "GGUF".into()]
}
fn default_page_size() -> u32 {
    24
}
fn default_extra() -> u32 {
    5
}
fn default_preview_width() -> u32 {
    450
}
fn default_cache_minutes() -> u32 {
    5
}
fn default_cache_pages() -> u32 {
    12
}

/// Loaded `catalog-filters.yaml`.
#[derive(Debug, Clone)]
pub struct CatalogFilters {
    pub kinds: Kinds,
    pub lora_types: Vec<String>,
    /// In YAML order.
    pub looks: Vec<Look>,
    pub style_badges: Vec<StyleBadge>,
    /// Tags multi-select, in YAML order.
    pub tags: Vec<TagFilter>,
    pub price: PriceSection,
    pub content: ContentSection,
    /// Safe mode rules (normalized, see [`SafeFilter::normalized`]).
    pub safe: SafeFilter,
    pub sort: Vec<ApiOption>,
    pub period: Vec<ApiOption>,
    /// Opening sort / time (`api` values; always one of `sort` / `period`).
    pub default_sort: String,
    pub default_period: String,
    pub allowed_file_formats: Vec<String>,
    /// Always true in practice; the YAML can't turn scan checks off (CLAUDE.md).
    pub require_scans_success: bool,
    /// Cards per Browse page.
    pub page_size: u32,
    /// `limit` per CivitAI request (≥ page_size, ≤ 100).
    pub api_limit: u32,
    pub max_extra_requests: u32,
    /// RAM cache of CivitAI answers: lifetime and size (0 pages = off).
    pub cache_ttl: std::time::Duration,
    pub cache_pages: usize,
    pub preview_width: u32,
}

impl CatalogFilters {
    pub fn load(path: &Path) -> Result<Self, FiltersError> {
        Self::from_yaml(&std::fs::read_to_string(path)?)
    }

    pub fn from_yaml(yaml: &str) -> Result<Self, FiltersError> {
        let raw: FiltersYaml = serde_yaml::from_str(yaml).map_err(|e| FiltersError::Yaml(e.to_string()))?;
        let mut looks = Vec::new();
        for (k, v) in raw.looks {
            let key = k.as_str().ok_or_else(|| FiltersError::Yaml("look keys must be strings".into()))?.to_string();
            let l: LookYaml = serde_yaml::from_value(v).map_err(|e| FiltersError::Yaml(format!("look `{key}`: {e}")))?;
            looks.push(Look {
                key,
                label: l.label,
                tags: l.tags.iter().map(|t| t.trim().to_ascii_lowercase()).collect(),
                also_require: l.also_require.unwrap_or_default(),
            });
        }
        let mut tags = Vec::new();
        for (k, v) in raw.tags {
            let key = k.as_str().ok_or_else(|| FiltersError::Yaml("tag keys must be strings".into()))?.to_string();
            let t: TagYaml = serde_yaml::from_value(v).map_err(|e| FiltersError::Yaml(format!("tag `{key}`: {e}")))?;
            tags.push(TagFilter {
                key,
                label: t.label,
                rule: t.rule,
                tags: t.tags.iter().map(|t| t.trim().to_ascii_lowercase()).collect(),
                name_words: t.name_words.iter().map(|w| crate::safe::name_words(w)).filter(|w| !w.is_empty()).collect(),
                base_models: t.base_models.iter().map(|b| b.trim().to_string()).collect(),
            });
        }
        if raw.sort.is_empty() || raw.period.is_empty() {
            return Err(FiltersError::Yaml("sort and period need at least one entry".into()));
        }
        let pick = |list: &[ApiOption], wanted: Option<&str>, fallback: &str| -> String {
            wanted
                .and_then(|w| list.iter().find(|o| o.api == w))
                .or_else(|| list.iter().find(|o| o.api == fallback))
                .unwrap_or(&list[0])
                .api
                .clone()
        };
        let default_sort = pick(&raw.sort, raw.default_sort.as_deref(), &raw.sort[0].api);
        let default_period = pick(&raw.period, raw.default_period.as_deref(), "AllTime");
        let page_size = raw.page_size.clamp(1, 100);
        // Security: the YAML may narrow the allowed formats, never widen them.
        let allowed_file_formats: Vec<String> = raw
            .allowed_file_formats
            .into_iter()
            .filter(|f| crate::select::HARD_ALLOWED_FORMATS.iter().any(|a| a.eq_ignore_ascii_case(f)))
            .collect();
        Ok(Self {
            kinds: raw.kinds,
            lora_types: raw.lora_types,
            looks,
            style_badges: raw.style_badges,
            tags,
            price: raw.price,
            content: raw.content,
            safe: raw.safe_filter.normalized(),
            sort: raw.sort,
            period: raw.period,
            default_sort,
            default_period,
            allowed_file_formats,
            require_scans_success: true,
            page_size,
            api_limit: raw.api_limit.unwrap_or(page_size).clamp(page_size, 100),
            max_extra_requests: raw.max_extra_requests.min(20),
            cache_ttl: std::time::Duration::from_secs(u64::from(raw.cache_minutes.min(60)) * 60),
            cache_pages: raw.cache_pages.min(64) as usize,
            preview_width: raw.preview_width.clamp(64, 2048),
        })
    }

    pub fn look(&self, key: &str) -> Option<&Look> {
        self.looks.iter().find(|l| l.key == key)
    }

    /// Friendly badge for a model's tags (`style_badges`, first match wins).
    pub fn style_badge(&self, tags: &[String]) -> Option<String> {
        self.style_badges
            .iter()
            .find(|b| self.look(&b.look).is_some_and(|l| l.matches_tags(tags)))
            .map(|b| b.badge.clone())
    }

    pub fn tag(&self, key: &str) -> Option<&TagFilter> {
        self.tags.iter().find(|t| t.key == key)
    }

    pub fn is_lora_type(&self, civitai_type: &str) -> bool {
        self.lora_types.iter().any(|t| t.eq_ignore_ascii_case(civitai_type))
    }

    pub fn kind_spec(&self, kind: CatalogKind) -> &KindSpec {
        match kind {
            CatalogKind::Models => &self.kinds.models,
            CatalogKind::StyleAddons => &self.kinds.style_addons,
        }
    }

    /// `catalog_filters` command payload.
    pub fn options(&self) -> CatalogFilterOptions {
        CatalogFilterOptions {
            looks: self.looks.iter().map(|l| KeyLabel { key: l.key.clone(), label: l.label.clone() }).collect(),
            sorts: self.sort.iter().map(|s| KeyedLabel { label: s.label.clone(), api: s.api.clone() }).collect(),
            periods: self.period.iter().map(|s| KeyedLabel { label: s.label.clone(), api: s.api.clone() }).collect(),
            tags: self
                .tags
                .iter()
                .map(|t| TagOption { key: t.key.clone(), label: t.label.clone(), needs_safe_mode_off: t.rule == TagRule::MadeForAdults })
                .collect(),
            content: [ContentMode::Safe, ContentMode::All]
                .into_iter()
                .map(|m| KeyLabel { key: m.key().into(), label: self.content.option(m).label.clone() })
                .collect(),
            price: [
                (PriceMode::Free, &self.price.free),
                (PriceMode::Include, &self.price.include),
                (PriceMode::PaidOnly, &self.price.paid_only),
            ]
            .into_iter()
            .map(|(m, l)| KeyLabel { key: m.key().into(), label: l.label.clone() })
            .collect(),
            default_content: self.content.default,
            default_price: self.price.default,
            default_sort: self.default_sort.clone(),
            default_period: self.default_period.clone(),
        }
    }

    /// Query parameters for one `GET /api/v1/models` page (SPEC §5.4 table).
    /// Repeated keys for arrays (`types`, `baseModels`). Never uses `page`
    /// (CivitAI answers 429 when page × limit > 1000): paging is by `cursor`.
    pub fn query_params(&self, q: &BrowseQuery, base_models: &[String], cursor: Option<&str>) -> Vec<(String, String)> {
        let mut p: Vec<(String, String)> = Vec::new();
        let mut push = |k: &str, v: &str| p.push((k.to_string(), v.to_string()));
        push("limit", &self.api_limit.to_string());
        for t in &self.kind_spec(q.kind).api_types {
            push("types", t);
        }
        let sort = self.sort.iter().find(|s| s.api == q.sort).map_or(self.default_sort.as_str(), |s| s.api.as_str());
        push("sort", sort);
        let period = self.period.iter().find(|s| s.api == q.period).map_or(self.default_period.as_str(), |s| s.api.as_str());
        push("period", period);
        if let Some(nsfw) = self.content.option(q.content).api_nsfw {
            push("nsfw", if nsfw { "true" } else { "false" });
        }
        let text = q.query.trim();
        if !text.is_empty() {
            let text: String = text.chars().take(200).collect();
            push("query", &text);
        }
        if self.commercial_required(q).is_some() {
            push("allowCommercialUse", "Image");
        }
        if q.compatible_only {
            for b in base_models {
                push("baseModels", b);
            }
        }
        if let Some(c) = cursor.map(str::trim).filter(|c| !c.is_empty()) {
            push("cursor", c);
        }
        p
    }

    /// Commercial-use permission every result must have (`Image`), from the
    /// "OK for client work" toggle or the Look's `also_require`.
    pub fn commercial_required(&self, q: &BrowseQuery) -> Option<String> {
        if q.commercial_only {
            return Some("Image".into());
        }
        let look = q.look.as_deref().and_then(|k| self.look(k))?;
        look.also_require.allow_commercial_use_includes.clone()
    }

    /// Model-level client-side rules: kind, Safe mode (see [`crate::safe`]),
    /// Look tags (+ brand `also_require`), the Tags multi-select, commercial
    /// use, archived models.
    pub fn keep_model(&self, q: &BrowseQuery, m: &Model) -> bool {
        self.hidden_by(q, m).is_none()
    }

    /// Why [`CatalogFilters::keep_model`] drops a model (`None` = kept).
    pub fn hidden_by(&self, q: &BrowseQuery, m: &Model) -> Option<Hidden> {
        if m.is_unavailable() {
            return Some(Hidden::Other);
        }
        let kind = self.kind_spec(q.kind);
        if !m.kind.is_empty() && !kind.api_types.iter().any(|t| t.eq_ignore_ascii_case(&m.kind)) {
            return Some(Hidden::Other);
        }
        if q.content == ContentMode::Safe && self.safe.adult_reason(m).is_some() {
            return Some(Hidden::Content);
        }
        if let Some(key) = q.look.as_deref() {
            match self.look(key) {
                Some(look) if look.matches_tags(&m.tags) => {}
                Some(_) => return Some(Hidden::Other),
                None => {} // unknown look key: ignore the filter
            }
        }
        for key in &q.tags {
            match self.tag(key) {
                Some(tag) if tag.matches(m, &self.safe) => {}
                Some(_) => return Some(Hidden::Other),
                None => {} // unknown tag key: ignore it
            }
        }
        if let Some(what) = self.commercial_required(q) {
            if !m.allow_commercial_use.allows(&what) {
                return Some(Hidden::Other);
            }
        }
        None
    }

    /// The version a card shows, after compatibility and price rules:
    /// newest version (newest *compatible* one when "Works with Pinhole" is on);
    /// Free drops the model when that version is in early access, Early access
    /// only keeps only those.
    pub fn pick_version<'a>(
        &self,
        q: &BrowseQuery,
        m: &'a Model,
        is_compatible: impl Fn(&str) -> bool,
        now: DateTime<Utc>,
    ) -> Option<&'a ModelVersion> {
        let v = m.model_versions.iter().find(|v| !q.compatible_only || is_compatible(&v.base_model))?;
        let early = v.is_early_access(now);
        match q.price {
            PriceMode::Free if early => None,
            PriceMode::PaidOnly if !early => None,
            _ => Some(v),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::api::ModelsPage;

    pub(crate) fn filters() -> CatalogFilters {
        CatalogFilters::from_yaml(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../config/catalog-filters.yaml"))).unwrap()
    }

    pub(crate) fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-28T12:00:00Z").unwrap().with_timezone(&Utc)
    }

    fn page() -> ModelsPage {
        serde_json::from_str(include_str!("../tests/fixtures/models_page.json")).unwrap()
    }

    fn get<'a>(p: &'a [(String, String)], k: &str) -> Vec<&'a str> {
        p.iter().filter(|(pk, _)| pk == k).map(|(_, v)| v.as_str()).collect()
    }

    #[test]
    fn loads_shipped_yaml() {
        let f = filters();
        assert_eq!(f.looks.iter().map(|l| l.key.as_str()).collect::<Vec<_>>(), ["realistic", "anime", "illustration", "three_d", "brand"]);
        assert_eq!(f.page_size, 24);
        assert_eq!(f.api_limit, 50);
        assert_eq!(f.max_extra_requests, 5);
        assert_eq!(f.cache_ttl, std::time::Duration::from_secs(300));
        assert_eq!(f.cache_pages, 12);
        assert_eq!(f.allowed_file_formats, ["SafeTensor", "GGUF"]);
        assert_eq!((f.default_sort.as_str(), f.default_period.as_str()), ("Most Downloaded", "AllTime"));
        let o = f.options();
        assert_eq!(o.default_content, ContentMode::Safe);
        assert_eq!(o.default_price, PriceMode::Free);
        assert_eq!(o.content.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(), ["safe", "all"]);
        assert_eq!(o.tags.first().map(|t| t.key.as_str()), Some("edit"));
        let nsfw = o.tags.iter().find(|t| t.key == "nsfw").unwrap();
        assert!(nsfw.needs_safe_mode_off);
        assert_eq!(o.tags.iter().filter(|t| t.needs_safe_mode_off).count(), 1);
        assert_eq!(o.price[2].key, "paid_only");
        assert_eq!(o.sorts[0].api, "Highest Rated");
        let json = serde_json::to_value(&o).unwrap();
        assert_eq!(json["defaultContent"], "safe");
        assert_eq!(json["defaultPrice"], "free");
        assert_eq!(json["defaultSort"], "Most Downloaded");
        assert_eq!(json["defaultPeriod"], "AllTime");
        assert_eq!(json["looks"][3]["key"], "three_d");
    }

    #[test]
    fn yaml_cannot_widen_formats_or_safe_previews() {
        let shipped = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../config/catalog-filters.yaml"));
        let y = shipped.replace("allowed_file_formats: [SafeTensor, GGUF]", "allowed_file_formats: [SafeTensor, PickleTensor]");
        assert_eq!(CatalogFilters::from_yaml(&y).unwrap().allowed_file_formats, ["SafeTensor"]);
        let y = shipped.replace("max_preview_level: 1", "max_preview_level: 16");
        assert_eq!(CatalogFilters::from_yaml(&y).unwrap().safe.max_preview_level, crate::safe::LEVEL_PG13);
        // Unknown defaults fall back to configured options; limits are clamped.
        let y = shipped
            .replace("default_sort: \"Most Downloaded\"", "default_sort: \"Most Buzz\"")
            .replace("default_period: AllTime", "default_period: Decade")
            .replace("api_limit: 50", "api_limit: 500");
        let f = CatalogFilters::from_yaml(&y).unwrap();
        assert_eq!((f.default_sort.as_str(), f.default_period.as_str(), f.api_limit), ("Highest Rated", "AllTime", 100));
        let y = shipped.replace("api_limit: 50", "api_limit: 3");
        assert_eq!(CatalogFilters::from_yaml(&y).unwrap().api_limit, 24, "never below page_size");
    }

    #[test]
    fn browse_query_json_matches_types_ts() {
        let q: BrowseQuery = serde_json::from_str(
            r#"{"kind":"styleAddons","look":"anime","tags":["edit","nsfw"],"content":"all","price":"paid_only","sort":"Newest","period":"Week","commercialOnly":true,"compatibleOnly":false,"query":"neon","cursor":"2|5"}"#,
        )
        .unwrap();
        assert_eq!(q.kind, CatalogKind::StyleAddons);
        assert_eq!(q.content, ContentMode::All);
        assert_eq!(q.tags, ["edit", "nsfw"]);
        assert_eq!(q.price, PriceMode::PaidOnly);
        assert!(q.commercial_only && !q.compatible_only);
        for old in ["include_18plus", "only_18plus"] {
            let q: BrowseQuery = serde_json::from_str(&format!(r#"{{"content":"{old}"}}"#)).unwrap();
            assert_eq!(q.content, ContentMode::All, "old 18+ keys read as Safe mode off");
        }
        let q: BrowseQuery = serde_json::from_str("{}").unwrap();
        assert!(q.tags.is_empty());
        assert!(q.compatible_only, "defaults to Works with Pinhole");
    }

    #[test]
    fn query_params_defaults() {
        let f = filters();
        let bases = vec!["SDXL 1.0".to_string(), "Pony".to_string()];
        let p = f.query_params(&BrowseQuery::default(), &bases, None);
        assert_eq!(get(&p, "limit"), ["50"]);
        assert_eq!(get(&p, "types"), ["Checkpoint"]);
        assert_eq!(get(&p, "sort"), ["Most Downloaded"]);
        assert_eq!(get(&p, "period"), ["AllTime"]);
        assert_eq!(get(&p, "nsfw"), ["true"], "Safe mode needs every image rating (filtered client-side)");
        assert_eq!(get(&p, "baseModels"), ["SDXL 1.0", "Pony"]);
        assert!(get(&p, "query").is_empty());
        assert!(get(&p, "cursor").is_empty());
        assert!(get(&p, "allowCommercialUse").is_empty());
        assert!(get(&p, "page").is_empty(), "never page-based paging");
    }

    #[test]
    fn query_params_every_filter() {
        let f = filters();
        let bases = vec!["SD 1.5".to_string()];
        let mut q = BrowseQuery { kind: CatalogKind::StyleAddons, ..Default::default() };
        assert_eq!(get(&f.query_params(&q, &bases, None), "types"), ["LORA"]);

        q.content = ContentMode::All;
        assert_eq!(get(&f.query_params(&q, &bases, None), "nsfw"), ["true"]);
        q.tags = vec!["edit".into(), "nsfw".into()];
        assert_eq!(get(&f.query_params(&q, &bases, None), "nsfw"), ["true"]);
        assert!(get(&f.query_params(&q, &bases, None), "tag").is_empty(), "tags filter client-side");
        q.tags.clear();

        for (sort, period) in [("Most Downloaded", "Week"), ("Newest", "Month"), ("Highest Rated", "Year")] {
            q.sort = sort.into();
            q.period = period.into();
            let p = f.query_params(&q, &bases, None);
            assert_eq!(get(&p, "sort"), [sort]);
            assert_eq!(get(&p, "period"), [period]);
        }
        q.sort = "Most Buzz".into();
        q.period = "Decade".into();
        let p = f.query_params(&q, &bases, None);
        assert_eq!(get(&p, "sort"), ["Most Downloaded"], "unknown sort → the default");
        assert_eq!(get(&p, "period"), ["AllTime"]);
        q.content = ContentMode::Safe;
        assert_eq!(get(&f.query_params(&q, &bases, None), "nsfw"), ["true"], "same pages in every content mode");

        q.commercial_only = true;
        assert_eq!(get(&f.query_params(&q, &bases, None), "allowCommercialUse"), ["Image"]);
        q.commercial_only = false;
        q.look = Some("brand".into());
        assert_eq!(get(&f.query_params(&q, &bases, None), "allowCommercialUse"), ["Image"], "brand look requires Image");
        q.look = Some("anime".into());
        assert!(get(&f.query_params(&q, &bases, None), "allowCommercialUse").is_empty());

        q.compatible_only = false;
        assert!(get(&f.query_params(&q, &bases, None), "baseModels").is_empty());

        q.query = "  neon cat  ".into();
        assert_eq!(get(&f.query_params(&q, &bases, Some("2|17")), "query"), ["neon cat"]);
        assert_eq!(get(&f.query_params(&q, &bases, Some("2|17")), "cursor"), ["2|17"]);
    }

    #[test]
    fn content_client_filters() {
        let f = filters();
        let page = page();
        let nsfw_lora = &page.items[1];
        let sfw_ckpt = &page.items[0];
        let nsfw_tag = vec!["nsfw".to_string()];
        let q = BrowseQuery { kind: CatalogKind::StyleAddons, content: ContentMode::All, tags: nsfw_tag.clone(), ..Default::default() };
        assert!(f.keep_model(&q, nsfw_lora));
        let q = BrowseQuery { content: ContentMode::All, tags: nsfw_tag.clone(), ..Default::default() };
        assert!(!f.keep_model(&q, sfw_ckpt), "the NSFW tag finds only what Safe mode hides");
        assert_eq!(f.hidden_by(&q, sfw_ckpt), Some(Hidden::Other));
        let q = BrowseQuery { kind: CatalogKind::StyleAddons, content: ContentMode::Safe, tags: nsfw_tag, ..Default::default() };
        assert_eq!(f.hidden_by(&q, nsfw_lora), Some(Hidden::Content), "Safe mode on: the NSFW tag finds nothing");
        let q = BrowseQuery { kind: CatalogKind::StyleAddons, content: ContentMode::Safe, ..Default::default() };
        assert!(!f.keep_model(&q, nsfw_lora), "safe mode drops nsfw models");
        assert_eq!(f.hidden_by(&q, nsfw_lora), Some(Hidden::Content));
        let q = BrowseQuery { content: ContentMode::Safe, ..Default::default() };
        assert!(f.keep_model(&q, sfw_ckpt), "RealVisXL: one suggestive tag, mostly PG images");
        let q = BrowseQuery { kind: CatalogKind::StyleAddons, content: ContentMode::All, ..Default::default() };
        assert!(f.keep_model(&q, nsfw_lora));
        let q = BrowseQuery { content: ContentMode::All, ..Default::default() };
        assert!(f.keep_model(&q, sfw_ckpt));
    }

    #[test]
    fn safe_mode_and_nsfw_tag_split_the_live_sample() {
        let f = filters();
        let page = crate::safe::tests::live("month");
        let count = |content, tags: &[&str]| {
            let q = BrowseQuery { content, tags: tags.iter().map(|t| t.to_string()).collect(), ..Default::default() };
            page.items.iter().filter(|m| f.keep_model(&q, m)).count()
        };
        let (safe, adult, all) = (count(ContentMode::Safe, &[]), count(ContentMode::All, &["nsfw"]), count(ContentMode::All, &[]));
        assert_eq!(safe + adult, all);
        assert_eq!(all, page.items.len());
        assert!(safe < all / 2, "most of this month's top-rated checkpoints are made for adults ({safe}/{all})");
    }

    #[test]
    fn kind_filter_is_enforced_client_side() {
        let f = filters();
        let page = page();
        assert!(!f.keep_model(&BrowseQuery::default(), &page.items[1]), "LoRA in the Models view");
        assert!(f.keep_model(&BrowseQuery::default(), &page.items[0]));
    }

    #[test]
    fn price_filters() {
        let f = filters();
        let page = page();
        let lora = &page.items[1]; // newest version in early access, older one free
        let toon = page.items.iter().find(|m| m.id == 800002).unwrap(); // EA ended
        let yes = |_: &str| true;
        let free = BrowseQuery { price: PriceMode::Free, ..Default::default() };
        assert!(f.pick_version(&free, lora, yes, now()).is_none(), "Free drops models whose latest version is early access");
        assert_eq!(f.pick_version(&free, toon, yes, now()).unwrap().id, 800102, "EA that already ended counts as free");
        let include = BrowseQuery { price: PriceMode::Include, ..Default::default() };
        assert_eq!(f.pick_version(&include, lora, yes, now()).unwrap().id, 902001);
        let paid = BrowseQuery { price: PriceMode::PaidOnly, ..Default::default() };
        assert_eq!(f.pick_version(&paid, lora, yes, now()).unwrap().id, 902001);
        assert!(f.pick_version(&paid, toon, yes, now()).is_none(), "Early access only drops free models");
    }

    #[test]
    fn compatible_version_choice() {
        let f = filters();
        let page = page();
        let sd35 = page.items.iter().find(|m| m.id == 900001).unwrap();
        let compat = |b: &str| b != "SD 3.5 Large";
        let q = BrowseQuery::default();
        assert!(f.pick_version(&q, sd35, compat, now()).is_none());
        let q = BrowseQuery { compatible_only: false, ..Default::default() };
        assert_eq!(f.pick_version(&q, sd35, compat, now()).unwrap().id, 900101);
    }

    #[test]
    fn look_and_brand_filters() {
        let f = filters();
        let page = page();
        let by_id = |id: u64| page.items.iter().find(|m| m.id == id).unwrap();
        let q = |look: &str| BrowseQuery { look: Some(look.into()), ..Default::default() };
        assert!(f.keep_model(&q("realistic"), by_id(139562)));
        assert!(!f.keep_model(&q("anime"), by_id(139562)));
        assert!(f.keep_model(&q("anime"), by_id(777001)), "tag `2d`");
        assert!(f.keep_model(&q("three_d"), by_id(800002)));
        assert!(f.keep_model(&q("illustration"), by_id(618692)));
        // Brand: product tags AND commercial Image.
        assert!(f.keep_model(&q("brand"), by_id(800002)), "product tag + {{Image,Sell}}");
        assert!(!f.keep_model(&q("brand"), by_id(618692)), "product tag but no commercial use");
        assert!(!f.keep_model(&q("brand"), by_id(139562)), "commercial but no product tag");
        // Unknown look keys are ignored rather than hiding everything.
        assert!(f.keep_model(&q("nope"), by_id(139562)));
    }

    #[test]
    fn commercial_filter() {
        let f = filters();
        let page = page();
        let q = BrowseQuery { commercial_only: true, ..Default::default() };
        assert!(f.keep_model(&q, &page.items[0]));
        assert!(!f.keep_model(&q, page.items.iter().find(|m| m.id == 5000).unwrap()), "\"None\"");
        assert!(!f.keep_model(&q, page.items.iter().find(|m| m.id == 618692).unwrap()), "[]");
    }

    fn model(name: &str, tags: &[&str], base_model: &str) -> Model {
        serde_json::from_value(serde_json::json!({
            "id": 1, "name": name, "type": "Checkpoint", "tags": tags,
            "modelVersions": [{ "id": 2, "name": "v1", "baseModel": base_model, "files": [], "images": [] }],
        }))
        .unwrap()
    }

    #[test]
    fn edit_tag_matches_tags_name_words_and_base_models() {
        // Names and tags from live /api/v1/models answers (2026-09-28).
        let f = filters();
        let edit = |m: &Model| f.keep_model(&BrowseQuery { tags: vec!["edit".into()], compatible_only: false, ..Default::default() }, m);
        assert!(edit(&model("Qwen Image Edit 2511 GGUF", &["base model"], "Qwen")), "name word");
        assert!(edit(&model("Qwen-Image-Edit-MeiTu", &["base model"], "Qwen")), "hyphenated name");
        assert!(edit(&model("God Save The Qwen", &["base model", "qwen", "image edit"], "Qwen")), "tag");
        assert!(edit(&model("Flux Kontext NF4", &["base model"], "Flux.1 Kontext")), "base model");
        assert!(edit(&model("Nunchaku Flux 4-Bit", &["base model"], "Flux.1 Kontext")), "base model only");
        assert!(!edit(&model("Moonmix - Anime Edition", &["style"], "SD 1.5")), "Edition is not Edit");
        assert!(!edit(&model("Qwen-Image", &["base model", "qwen", "qwen-image"], "Qwen")));
    }

    #[test]
    fn picked_tags_must_all_match_and_unknown_tags_are_ignored() {
        let f = filters();
        let m = model("Colossus Project KONTEXT", &["fantasy", "portrait", "landscape"], "Flux.1 Kontext");
        let q = |tags: &[&str]| BrowseQuery { tags: tags.iter().map(|t| t.to_string()).collect(), compatible_only: false, ..Default::default() };
        assert!(f.keep_model(&q(&["edit", "fantasy", "portraits", "landscapes"]), &m));
        assert!(!f.keep_model(&q(&["edit", "scifi"]), &m));
        assert_eq!(f.hidden_by(&q(&["scifi"]), &m), Some(Hidden::Other));
        assert!(f.keep_model(&q(&["nope"]), &m), "unknown tag keys don't hide everything");
    }

    #[test]
    fn style_badges_first_match_wins() {
        let f = filters();
        let t = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(f.style_badge(&t(&["Anime", "photorealistic"])).as_deref(), Some("Anime"));
        assert_eq!(f.style_badge(&t(&["photography"])).as_deref(), Some("Realistic"));
        assert_eq!(f.style_badge(&t(&["3d", "cartoon"])).as_deref(), Some("3D"));
        assert_eq!(f.style_badge(&t(&["digital art"])).as_deref(), Some("Illustration"));
        assert_eq!(f.style_badge(&t(&["base model"])), None);
    }
}
