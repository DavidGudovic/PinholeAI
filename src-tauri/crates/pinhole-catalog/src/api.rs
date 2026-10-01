//! CivitAI public REST API (v1): response types and a thin client over
//! [`pinhole_net::HttpClient`] (the only internet client).
//!
//! Endpoints used:
//! * `GET /api/v1/models` — browse/search (cursor paging; text searches page by number; `page` × `limit`
//!   > 1000 answers 429);
//! * `GET /api/v1/models/{id}`;
//! * `GET /api/v1/model-versions/{id}`;
//! * `GET /api/v1/model-versions/by-hash/{hash}` (SHA-256, AutoV2, AutoV3, …).
//!
//! All types are lenient (see [`crate::lenient`]): unknown fields are ignored,
//! nulls become defaults, and malformed list elements are skipped.
//!
//! PRIVACY: the API key is only sent where it is needed (downloads, the
//! download-permission probe, and as a retry when metadata answers 401/403),
//! only as an `Authorization: Bearer` header, and never logged or stored.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use pinhole_net::{HttpClient, NetError};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::lenient;

pub const API_BASE: &str = "https://civitai.com/api/v1";

// ------------------------------------------------------------------ types

/// `GET /api/v1/models` response.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ModelsPage {
    #[serde(deserialize_with = "lenient::vec")]
    pub items: Vec<Model>,
    #[serde(deserialize_with = "lenient::obj")]
    pub metadata: PageMetadata,
}

impl ModelsPage {
    /// Cursor for the next page: `metadata.nextCursor`, or the `cursor` query
    /// parameter of `metadata.nextPage`. Text searches are answered page by
    /// page (`nextPage` has `page=N` and no `cursor`): that comes back as
    /// `page:N`, which [`PAGE_CURSOR_PREFIX`] tells the request builder to send as `page`.
    pub fn next_cursor(&self) -> Option<String> {
        if let Some(c) = self
            .metadata
            .next_cursor
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            return Some(c.to_string());
        }
        let next = self.metadata.next_page.as_deref()?;
        let url = url::Url::parse(next).ok()?;
        let param = |name: &str| {
            url.query_pairs()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.into_owned())
                .filter(|v| !v.trim().is_empty())
        };
        if let Some(c) = param("cursor") {
            return Some(c);
        }
        // CivitAI answers 429 once `page` × `limit` passes 1000: end the list there.
        let page: u32 = param("page")?.trim().parse().ok()?;
        let limit: u32 = param("limit")
            .and_then(|l| l.trim().parse().ok())
            .unwrap_or(100);
        (page.saturating_mul(limit) <= 1000).then(|| format!("{PAGE_CURSOR_PREFIX}{page}"))
    }
}

/// Marks a cursor that is really a page number (CivitAI pages text searches by
/// `page`, not `cursor`).
pub const PAGE_CURSOR_PREFIX: &str = "page:";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PageMetadata {
    #[serde(deserialize_with = "lenient::opt_string")]
    pub next_cursor: Option<String>,
    #[serde(deserialize_with = "lenient::opt_string")]
    pub next_page: Option<String>,
    #[serde(deserialize_with = "lenient::opt_u64")]
    pub total_items: Option<u64>,
}

/// A model (`/models` item or `/models/{id}`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Model {
    #[serde(deserialize_with = "lenient::u64")]
    pub id: u64,
    #[serde(deserialize_with = "lenient::string")]
    pub name: String,
    /// `Checkpoint`, `LORA`, `LoCon`, `DoRA`, `TextualInversion`, `VAE`, …
    #[serde(rename = "type", deserialize_with = "lenient::string")]
    pub kind: String,
    #[serde(deserialize_with = "lenient::bool")]
    pub nsfw: bool,
    #[serde(deserialize_with = "lenient::nsfw_level")]
    pub nsfw_level: Option<u32>,
    /// CivitAI: depicts a real person.
    #[serde(deserialize_with = "lenient::bool")]
    pub poi: bool,
    /// CivitAI: depicts someone under 18 (usually a child character).
    #[serde(deserialize_with = "lenient::bool")]
    pub minor: bool,
    /// The creator asks for no adult content with this model.
    #[serde(deserialize_with = "lenient::bool")]
    pub sfw_only: bool,
    /// `Archived` / `TakenDown` → no downloadable files.
    #[serde(deserialize_with = "lenient::opt_string")]
    pub mode: Option<String>,
    #[serde(deserialize_with = "lenient::opt_string")]
    pub availability: Option<String>,
    pub allow_commercial_use: CommercialUse,
    #[serde(deserialize_with = "lenient::strings")]
    pub tags: Vec<String>,
    /// The creator's description of the model (HTML).
    #[serde(deserialize_with = "lenient::opt_string")]
    pub description: Option<String>,
    /// Every version's base model (newer responses), e.g. `["SD 1.5 Hyper", "SD 1.5"]`.
    #[serde(deserialize_with = "lenient::strings")]
    pub base_models: Vec<String>,
    /// Some version is behind paid access right now (newer responses).
    #[serde(deserialize_with = "lenient::bool")]
    pub has_active_paid_access: bool,
    #[serde(deserialize_with = "lenient::opt_obj")]
    pub creator: Option<Creator>,
    #[serde(deserialize_with = "lenient::obj")]
    pub stats: Stats,
    /// Newest first.
    #[serde(deserialize_with = "lenient::vec")]
    pub model_versions: Vec<ModelVersion>,
}

impl Model {
    /// Model is archived / taken down (files are gone).
    pub fn is_unavailable(&self) -> bool {
        matches!(
            self.mode.as_deref().map(str::to_ascii_lowercase).as_deref(),
            Some("archived") | Some("takendown")
        )
    }

    /// CivitAI marks it as showing a real person or someone under 18 (RELEASE-SPEC §5).
    pub fn is_person_or_minor(&self) -> bool {
        self.poi || self.minor
    }
}

/// A version (and its model, when fetched) CivitAI marks as a real person or a minor.
pub fn version_is_person_or_minor(version: &ModelVersion, model: Option<&Model>) -> bool {
    model.is_some_and(Model::is_person_or_minor)
        || version.model.as_ref().is_some_and(|m| m.poi || m.minor)
}

/// Whether the image check treats a version as "safe images only": the model's own flag, or
/// `true` when the model's details couldn't be fetched (fail closed).
pub fn sfw_only_of(version: &ModelVersion, model: Option<&Model>) -> bool {
    model.map_or(version.model_id > 0, |m| m.sfw_only)
}

/// Why a model CivitAI marks `poi` / `minor` can't be installed (RELEASE-SPEC §5, Level 1).
pub const PERSON_OR_MINOR_REASON: &str =
    "Pinhole doesn't install models that CivitAI marks as showing a real person or someone under 18.";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Creator {
    #[serde(deserialize_with = "lenient::opt_string")]
    pub username: Option<String>,
}

/// Model or version stats. Counts are lenient (numbers or strings).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Stats {
    #[serde(deserialize_with = "lenient::u64")]
    pub download_count: u64,
    #[serde(deserialize_with = "lenient::u64")]
    pub thumbs_up_count: u64,
    #[serde(deserialize_with = "lenient::u64")]
    pub thumbs_down_count: u64,
    #[serde(deserialize_with = "lenient::u64")]
    pub rating_count: u64,
    #[serde(deserialize_with = "lenient::f64")]
    pub rating: f64,
}

impl Stats {
    /// Thumbs-up share, `None` without votes.
    pub fn thumbs_up_ratio(&self) -> Option<f64> {
        let total = self.thumbs_up_count + self.thumbs_down_count;
        (total > 0).then(|| self.thumbs_up_count as f64 / total as f64)
    }
}

/// A model version (`modelVersions[]` item, `/model-versions/{id}`, `/by-hash`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ModelVersion {
    #[serde(deserialize_with = "lenient::u64")]
    pub id: u64,
    /// Present on `/model-versions/*` responses.
    #[serde(deserialize_with = "lenient::u64")]
    pub model_id: u64,
    #[serde(deserialize_with = "lenient::string")]
    pub name: String,
    /// e.g. `SDXL 1.0`, `Pony`, `Illustrious`, `Flux.1 D`, `SD 1.5`.
    #[serde(deserialize_with = "lenient::string")]
    pub base_model: String,
    #[serde(deserialize_with = "lenient::opt_string")]
    pub base_model_type: Option<String>,
    /// `Public` | `EarlyAccess` | `Unsearchable` | `Private` (VERIFY live).
    #[serde(deserialize_with = "lenient::opt_string")]
    pub availability: Option<String>,
    #[serde(deserialize_with = "lenient::opt_string")]
    pub status: Option<String>,
    #[serde(deserialize_with = "lenient::opt_string")]
    pub published_at: Option<String>,
    #[serde(deserialize_with = "lenient::opt_string")]
    pub created_at: Option<String>,
    /// RFC 3339; set while the version is in early access (VERIFY live).
    #[serde(deserialize_with = "lenient::opt_string")]
    pub early_access_ends_at: Option<String>,
    /// Older name of the same idea.
    #[serde(deserialize_with = "lenient::opt_string")]
    pub early_access_deadline: Option<String>,
    /// Older responses: early-access length in days after `publishedAt`.
    #[serde(deserialize_with = "lenient::opt_u64")]
    pub early_access_time_frame: Option<u64>,
    /// Newer responses: `{ timeframe, chargeForDownload, downloadPrice, … }`.
    #[serde(deserialize_with = "lenient::opt_obj")]
    pub early_access_config: Option<Value>,
    /// Newer responses: `null` for public versions, an object while paid.
    #[serde(deserialize_with = "lenient::opt_obj")]
    pub paid_access: Option<Value>,
    /// Some versions require a signed-in user to download (unverified field).
    #[serde(deserialize_with = "lenient::bool")]
    pub require_auth: bool,
    #[serde(deserialize_with = "lenient::strings")]
    pub trained_words: Vec<String>,
    /// The creator's notes for this version (HTML).
    #[serde(deserialize_with = "lenient::opt_string")]
    pub description: Option<String>,
    #[serde(deserialize_with = "lenient::nsfw_level")]
    pub nsfw_level: Option<u32>,
    #[serde(deserialize_with = "lenient::obj")]
    pub stats: Stats,
    #[serde(deserialize_with = "lenient::vec")]
    pub files: Vec<ModelFile>,
    #[serde(deserialize_with = "lenient::vec")]
    pub images: Vec<ModelImage>,
    #[serde(deserialize_with = "lenient::opt_string")]
    pub download_url: Option<String>,
    /// Present on `/model-versions/*` responses: `{ name, type, nsfw, poi }`.
    #[serde(deserialize_with = "lenient::opt_obj")]
    pub model: Option<VersionModel>,
}

impl ModelVersion {
    /// Early access (paid) right now. Checks, in order: `availability ==
    /// "EarlyAccess"`, a non-null `paidAccess`, a future `earlyAccessEndsAt` /
    /// `earlyAccessDeadline`, and the older `earlyAccessTimeFrame` (days after
    /// `publishedAt`). `earlyAccessConfig` alone is not decisive: it can stay
    /// set after early access ended.
    pub fn is_early_access(&self, now: DateTime<Utc>) -> bool {
        if self
            .availability
            .as_deref()
            .is_some_and(|a| a.eq_ignore_ascii_case("EarlyAccess"))
        {
            return true;
        }
        if self
            .paid_access
            .as_ref()
            .is_some_and(|p| !matches!(p, Value::Null | Value::Bool(false)))
        {
            return true;
        }
        for end in [&self.early_access_ends_at, &self.early_access_deadline] {
            if parse_time(end.as_deref()).is_some_and(|t| t > now) {
                return true;
            }
        }
        if let (Some(days), Some(published)) = (
            self.early_access_time_frame,
            parse_time(self.published_at.as_deref()),
        ) {
            if days > 0 && published + chrono::Duration::days(days.min(3650) as i64) > now {
                return true;
            }
        }
        false
    }
}

fn parse_time(s: Option<&str>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s?.trim())
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct VersionModel {
    #[serde(deserialize_with = "lenient::string")]
    pub name: String,
    #[serde(rename = "type", deserialize_with = "lenient::string")]
    pub kind: String,
    #[serde(deserialize_with = "lenient::bool")]
    pub nsfw: bool,
    #[serde(deserialize_with = "lenient::bool")]
    pub poi: bool,
    #[serde(deserialize_with = "lenient::bool")]
    pub minor: bool,
    #[serde(deserialize_with = "lenient::opt_string")]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ModelFile {
    #[serde(deserialize_with = "lenient::u64")]
    pub id: u64,
    #[serde(deserialize_with = "lenient::string")]
    pub name: String,
    /// Kilobytes (KiB), a float.
    #[serde(rename = "sizeKB", deserialize_with = "lenient::f64")]
    pub size_kb: f64,
    /// `Model` | `Pruned Model` | `VAE` | `Config` | `Training Data` | `Negative` | `Archive` …
    #[serde(rename = "type", deserialize_with = "lenient::string")]
    pub kind: String,
    /// `Pending` | `Success` | `Danger` | `Error`
    #[serde(deserialize_with = "lenient::string")]
    pub pickle_scan_result: String,
    #[serde(deserialize_with = "lenient::string")]
    pub virus_scan_result: String,
    #[serde(deserialize_with = "lenient::obj")]
    pub metadata: FileMetadata,
    /// `SHA256`, `AutoV1`, `AutoV2`, `AutoV3`, `CRC32`, `BLAKE3` → hex (uppercase).
    #[serde(deserialize_with = "lenient::string_map")]
    pub hashes: BTreeMap<String, String>,
    #[serde(deserialize_with = "lenient::bool")]
    pub primary: bool,
    #[serde(deserialize_with = "lenient::string")]
    pub download_url: String,
}

impl ModelFile {
    /// File size in bytes (`sizeKB` × 1024).
    pub fn size_bytes(&self) -> u64 {
        if self.size_kb.is_finite() && self.size_kb > 0.0 {
            (self.size_kb * 1024.0).round() as u64
        } else {
            0
        }
    }

    /// Hash by (case-insensitive) algorithm name.
    pub fn hash(&self, algo: &str) -> Option<&str> {
        self.hashes
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(algo))
            .map(|(_, v)| v.as_str())
    }

    /// Lowercase SHA-256, when CivitAI reports a well-formed one.
    pub fn sha256(&self) -> Option<String> {
        let h = self.hash("SHA256")?.trim().to_ascii_lowercase();
        (h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())).then_some(h)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FileMetadata {
    /// `SafeTensor` | `PickleTensor` | `GGUF` | `Diffusers` | `Core ML` | `ONNX` | `Other`
    #[serde(deserialize_with = "lenient::opt_string")]
    pub format: Option<String>,
    /// `full` | `pruned`
    #[serde(deserialize_with = "lenient::opt_string")]
    pub size: Option<String>,
    /// `fp16` | `fp32` | `bf16` | `fp8` | `nf4` …
    #[serde(deserialize_with = "lenient::opt_string")]
    pub fp: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ModelImage {
    #[serde(deserialize_with = "lenient::u64")]
    pub id: u64,
    #[serde(deserialize_with = "lenient::string")]
    pub url: String,
    #[serde(deserialize_with = "lenient::nsfw_level")]
    pub nsfw_level: Option<u32>,
    /// Older responses: bool or `None`/`Soft`/`Mature`/`X`.
    #[serde(deserialize_with = "lenient::nsfw_level_or_bool")]
    pub nsfw: Option<bool>,
    /// `image` | `video`
    #[serde(rename = "type", deserialize_with = "lenient::opt_string")]
    pub kind: Option<String>,
    #[serde(deserialize_with = "lenient::opt_u64")]
    pub width: Option<u64>,
    #[serde(deserialize_with = "lenient::opt_u64")]
    pub height: Option<u64>,
}

/// CivitAI's own split: PG and PG-13 are safe, R (4) and above are NSFW.
pub const NSFW_LEVEL_MIN: u32 = 4;

impl ModelImage {
    pub fn is_video(&self) -> bool {
        if self
            .kind
            .as_deref()
            .is_some_and(|k| k.eq_ignore_ascii_case("video"))
        {
            return true;
        }
        let path = self
            .url
            .split(['?', '#'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        path.ends_with(".mp4") || path.ends_with(".webm") || path.ends_with(".mov")
    }

    pub fn is_nsfw(&self) -> bool {
        match (self.nsfw_level, self.nsfw) {
            (Some(level), _) => level >= NSFW_LEVEL_MIN,
            (None, Some(flag)) => flag,
            (None, None) => false,
        }
    }
}

/// `allowCommercialUse`: newer responses send an array (`["Image","RentCivit","Rent","Sell"]`),
/// older ones a single hierarchical string (`None` < `Image` < `Rent` < `Sell`),
/// and some a Postgres-style set string (`"{Image,Sell}"`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CommercialUse(pub Vec<String>);

impl CommercialUse {
    pub fn allows(&self, what: &str) -> bool {
        self.0.iter().any(|v| v.eq_ignore_ascii_case(what))
    }
}

impl<'de> Deserialize<'de> for CommercialUse {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        fn clean(items: impl IntoIterator<Item = String>) -> Vec<String> {
            items
                .into_iter()
                .map(|s| s.trim().trim_matches('"').to_string())
                .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("none"))
                .collect()
        }
        Ok(CommercialUse(match Value::deserialize(d)? {
            Value::Array(items) => clean(items.iter().filter_map(lenient::value_to_string)),
            Value::String(s) => {
                let t = s.trim();
                if t.starts_with('{') || t.contains(',') {
                    clean(
                        t.trim_matches(|c| c == '{' || c == '}')
                            .split(',')
                            .map(str::to_string),
                    )
                } else {
                    // Legacy single value: each level implies the ones below it.
                    match t.to_ascii_lowercase().as_str() {
                        "sell" => clean(["Image", "RentCivit", "Rent", "Sell"].map(String::from)),
                        "rent" => clean(["Image", "RentCivit", "Rent"].map(String::from)),
                        "rentcivit" => clean(["Image", "RentCivit"].map(String::from)),
                        _ => clean([t.to_string()]),
                    }
                }
            }
            _ => Vec::new(),
        }))
    }
}

// ------------------------------------------------------------------ client

/// CivitAI client. Cheap to build per request (clones the shared `HttpClient`).
#[derive(Clone)]
pub struct CivitaiClient {
    http: HttpClient,
    api_key: Option<String>,
    base: String,
}

impl CivitaiClient {
    pub fn new(http: HttpClient, api_key: Option<String>) -> Self {
        Self::with_base(http, api_key, API_BASE)
    }

    /// Custom API base (tests: a mock server on 127.0.0.1).
    pub fn with_base(http: HttpClient, api_key: Option<String>, base: impl Into<String>) -> Self {
        let api_key = api_key
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty());
        Self {
            http,
            api_key,
            base: base.into().trim_end_matches('/').to_string(),
        }
    }

    pub fn has_key(&self) -> bool {
        self.api_key.is_some()
    }

    /// `Authorization` header for `url`, only for civitai.com hosts.
    pub fn auth_header_for(&self, url: &str) -> Option<(String, String)> {
        civitai_auth_header(self.api_key.as_deref(), url)
    }

    /// The key (for download specs). Never log it.
    pub fn api_key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }

    /// `GET /models?...` URL. Spaces are sent as `%20` (not `+`).
    pub fn search_url(&self, params: &[(String, String)]) -> String {
        let mut url = format!("{}/models", self.base);
        if !params.is_empty() {
            let q: Vec<String> = params
                .iter()
                .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
                .collect();
            url.push('?');
            url.push_str(&q.join("&"));
        }
        url
    }

    async fn get<T: DeserializeOwned>(&self, url: &str, with_key: bool) -> Result<T, NetError> {
        let auth = if with_key {
            self.auth_header_for(url)
        } else {
            None
        };
        let headers: Vec<(&str, &str)> =
            auth.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        self.http.get_json(url, &headers).await
    }

    /// Metadata GET without the key; retried once with the key on 401/403.
    async fn get_metadata<T: DeserializeOwned>(&self, url: &str) -> Result<T, NetError> {
        match self.get(url, false).await {
            Err(NetError::Unauthorized(_))
            | Err(NetError::Status(401))
            | Err(NetError::Status(403))
                if self.has_key() =>
            {
                self.get(url, true).await
            }
            Err(NetError::Status(s @ (401 | 403))) => Err(NetError::Unauthorized(s)),
            other => other,
        }
    }

    /// One page of `GET /models` (see `filters::query_params`). Browsing never
    /// sends the API key.
    pub async fn search(&self, params: &[(String, String)]) -> Result<ModelsPage, NetError> {
        self.get(&self.search_url(params), false).await
    }

    pub async fn model(&self, id: u64) -> Result<Model, NetError> {
        self.get_metadata(&format!("{}/models/{id}", self.base))
            .await
    }

    pub async fn model_version(&self, id: u64) -> Result<ModelVersion, NetError> {
        self.get_metadata(&format!("{}/model-versions/{id}", self.base))
            .await
    }

    /// Look up a version by file hash (SHA-256, AutoV2, AutoV3, BLAKE3, CRC32).
    /// `Ok(None)` when CivitAI doesn't know it (404) or the hash is malformed.
    pub async fn by_hash(&self, hash: &str) -> Result<Option<ModelVersion>, NetError> {
        let h = hash.trim();
        if !(8..=64).contains(&h.len()) || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(None);
        }
        match self
            .get_metadata(&format!(
                "{}/model-versions/by-hash/{}",
                self.base,
                h.to_ascii_uppercase()
            ))
            .await
        {
            Ok(v) => Ok(Some(v)),
            Err(NetError::Status(404)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Ask the download URL for one byte to learn whether it needs an API key.
    /// Returns the HTTP status (401/403 = key needed or access not bought).
    pub async fn probe_download(&self, url: &str) -> Result<u16, NetError> {
        let auth = self.auth_header_for(url);
        let mut headers: Vec<(&str, &str)> = vec![("Range", "bytes=0-0")];
        if let Some((k, v)) = auth.as_ref() {
            headers.push((k.as_str(), v.as_str()));
        }
        match self.http.get_bytes(url, &headers, 64 * 1024).await {
            // TooLarge: the server ignored Range and started sending the file.
            Ok(_) | Err(NetError::TooLarge) => Ok(200),
            Err(NetError::Unauthorized(code)) | Err(NetError::Status(code)) => Ok(code),
            Err(e) => Err(e),
        }
    }
}

/// `Authorization: Bearer <key>` for https://civitai.com (and subdomains)
/// URLs only — never sent to CDNs, Hugging Face or anything else.
pub fn civitai_auth_header(api_key: Option<&str>, url: &str) -> Option<(String, String)> {
    let key = api_key.map(str::trim).filter(|k| !k.is_empty())?;
    let parsed = url::Url::parse(url).ok()?;
    if parsed.scheme() != "https" {
        return None;
    }
    let host = parsed.host_str()?;
    pinhole_net::allow::host_matches(host, "civitai.com")
        .then(|| ("Authorization".to_string(), format!("Bearer {key}")))
}

/// Preview images may only come from `https://<*.>civitai.com/...` (no
/// credentials, no custom port). Redirects are re-checked by the HTTP client.
pub fn is_preview_url(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else {
        return false;
    };
    u.scheme() == "https"
        && u.username().is_empty()
        && u.password().is_none()
        && u.port().is_none()
        && u.host_str()
            .is_some_and(|h| pinhole_net::allow::host_matches(h, "civitai.com"))
}

/// Percent-encode a query component (RFC 3986 unreserved kept, space → `%20`).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn page_fixture() -> ModelsPage {
        serde_json::from_str(include_str!("../tests/fixtures/models_page.json")).unwrap()
    }

    /// Regression: a failed model fetch used to install a "safe images only" model without
    /// the flag; a hand-added file matched by hash never got it.
    #[test]
    fn sfw_only_fails_closed_without_the_model() {
        let v = ModelVersion {
            model_id: 7,
            ..Default::default()
        };
        assert!(sfw_only_of(&v, None));
        assert!(!sfw_only_of(&v, Some(&Model::default())));
        let flagged = Model {
            sfw_only: true,
            ..Default::default()
        };
        assert!(sfw_only_of(&v, Some(&flagged)));
        assert!(
            !sfw_only_of(&ModelVersion::default(), None),
            "no model to fetch"
        );
    }

    #[test]
    fn parses_models_page_fixture() {
        let page = page_fixture();
        assert_eq!(
            page.items.len(),
            7,
            "malformed item must be skipped, others kept"
        );
        assert_eq!(page.next_cursor().as_deref(), Some("2|1718000000000"));

        let rv = &page.items[0];
        assert_eq!(rv.id, 139562);
        assert_eq!(rv.base_models, vec!["SDXL 1.0"]);
        assert_eq!(rv.kind, "Checkpoint");
        assert!(!rv.nsfw);
        assert!(rv.allow_commercial_use.allows("Image"));
        assert!(rv.tags.iter().any(|t| t == "photorealistic"));
        assert_eq!(
            rv.creator.as_ref().and_then(|c| c.username.as_deref()),
            Some("SG_161222")
        );
        assert_eq!(rv.stats.thumbs_up_ratio(), Some(0.98));
        let v = &rv.model_versions[0];
        assert_eq!(v.base_model, "SDXL 1.0");
        assert_eq!(v.files.len(), 2);
        assert!(
            !v.files[0].primary,
            "`primary` is absent on non-primary files"
        );
        let f = &v.files[1];
        assert_eq!(f.size_bytes(), 6_938_065_160);
        assert_eq!(f.metadata.format.as_deref(), Some("SafeTensor"));
        assert_eq!(
            f.sha256().unwrap(),
            "6a35a7855770ae9820a3c931d4964c3817b6d9e3c6f9c4dabb5b3a94e5643b80"
        );
        assert_eq!(f.hash("autov2"), Some("6A35A78557"));
        assert!(f.primary);
        assert!(v.paid_access.is_none());
        assert!(!v.images[0].is_nsfw());
        assert!(v.images[1].is_nsfw());

        // Legacy string commercial use, tag objects, nulls everywhere.
        let lora = &page.items[1];
        assert_eq!(lora.kind, "LORA");
        assert!(lora.nsfw);
        assert!(
            lora.allow_commercial_use.allows("Image"),
            "legacy \"Sell\" implies Image"
        );
        assert!(lora.tags.iter().any(|t| t == "anime"));
        assert_eq!(
            lora.model_versions[0].trained_words,
            vec!["pnkstyle", "neon outline"]
        );

        let flux = page.items.iter().find(|m| m.id == 618692).unwrap();
        assert!(flux.stats.thumbs_up_ratio().is_none());
        assert!(flux.creator.is_none());
        assert_eq!(flux.model_versions[0].files.len(), 3);
        assert_eq!(flux.model_versions[0].trained_words, Vec::<String>::new());
    }

    #[test]
    fn early_access_detection() {
        let now = DateTime::parse_from_rfc3339("2026-09-28T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let page = page_fixture();
        let lora = &page.items[1].model_versions[0];
        assert!(lora.is_early_access(now), "availability EarlyAccess");
        assert!(!page.items[0].model_versions[0].is_early_access(now));

        let mut v = ModelVersion {
            early_access_ends_at: Some("2026-10-01T00:00:00.000Z".into()),
            ..Default::default()
        };
        assert!(v.is_early_access(now));
        v.early_access_ends_at = Some("2026-09-01T00:00:00Z".into());
        assert!(!v.is_early_access(now));
        let v = ModelVersion {
            early_access_time_frame: Some(5),
            published_at: Some("2026-09-26T00:00:00Z".into()),
            ..Default::default()
        };
        assert!(v.is_early_access(now));
        let v = ModelVersion {
            early_access_time_frame: Some(1),
            published_at: Some("2026-09-20T00:00:00Z".into()),
            ..Default::default()
        };
        assert!(!v.is_early_access(now));
        let v: ModelVersion =
            serde_json::from_str(r#"{"id":1,"availability":"Public","paidAccess":{"price":300}}"#)
                .unwrap();
        assert!(v.is_early_access(now), "paidAccess object");
        let v: ModelVersion = serde_json::from_str(r#"{"id":1,"availability":"Public","paidAccess":null,"earlyAccessConfig":{"timeframe":3}}"#).unwrap();
        assert!(
            !v.is_early_access(now),
            "a leftover earlyAccessConfig alone is not early access"
        );
    }

    #[test]
    fn parses_version_and_by_hash_fixtures() {
        let v: ModelVersion =
            serde_json::from_str(include_str!("../tests/fixtures/model_version.json")).unwrap();
        assert_eq!(v.id, 1759168);
        assert_eq!(v.model_id, 133005);
        assert_eq!(v.base_model, "SDXL 1.0");
        assert_eq!(v.model.as_ref().unwrap().kind, "Checkpoint");
        assert_eq!(v.model.as_ref().unwrap().name, "Juggernaut XL");
        assert_eq!(v.files[0].size_bytes(), 7_105_349_736);
        assert_eq!(v.name, "Ragnarok");
        assert!(v
            .download_url
            .as_deref()
            .unwrap()
            .starts_with("https://civitai.com/api/download/models/"));

        let h: ModelVersion =
            serde_json::from_str(include_str!("../tests/fixtures/by_hash_lora.json")).unwrap();
        assert_eq!(h.model.as_ref().unwrap().kind, "LORA");
        assert_eq!(h.base_model, "SD 1.5");
        assert_eq!(h.trained_words, vec!["watercolor"]);

        let m: Model = serde_json::from_str(include_str!("../tests/fixtures/model.json")).unwrap();
        assert_eq!(m.id, 133005);
        assert!(
            !m.allow_commercial_use.allows("Image"),
            "{{Rent}} set string without Image"
        );
        assert!(m.allow_commercial_use.allows("Rent"));
        assert_eq!(m.model_versions.len(), 2);
    }

    #[test]
    fn commercial_use_variants() {
        let p = |s: &str| serde_json::from_str::<CommercialUse>(s).unwrap();
        assert!(p(r#"["Image","Sell"]"#).allows("image"));
        assert!(!p(r#"["Sell"]"#).allows("Image"), "arrays are not expanded");
        assert!(p(r#""Sell""#).allows("Image"));
        assert!(p(r#""Rent""#).allows("Image"));
        assert!(p(r#""Image""#).allows("Image"));
        assert!(!p(r#""None""#).allows("Image"));
        assert!(p(r#""{Image,RentCivit}""#).allows("Image"));
        assert!(!p("null").allows("Image"));
        assert!(!p("7").allows("Image"));
    }

    #[test]
    fn cursor_from_next_page_url_and_numbers() {
        let p: ModelsPage = serde_json::from_str(r#"{"items":[],"metadata":{"nextPage":"https://civitai.com/api/v1/models?limit=24&cursor=3%7C99"}}"#).unwrap();
        assert_eq!(p.next_cursor().as_deref(), Some("3|99"));
        let p: ModelsPage =
            serde_json::from_str(r#"{"items":null,"metadata":{"nextCursor":12345}}"#).unwrap();
        assert_eq!(p.next_cursor().as_deref(), Some("12345"));
        let p: ModelsPage = serde_json::from_str(r#"{"items":[],"metadata":{}}"#).unwrap();
        assert_eq!(p.next_cursor(), None);
        let p: ModelsPage = serde_json::from_str(r#"{"items":[],"metadata":{"currentPage":1,"nextPage":"https://civitai.com/api/v1/models?query=neon&limit=50&page=2"}}"#).unwrap();
        assert_eq!(
            p.next_cursor().as_deref(),
            Some("page:2"),
            "text search pages by number"
        );
        let p: ModelsPage = serde_json::from_str(r#"{"items":[],"metadata":{"nextPage":"https://civitai.com/api/v1/models?query=neon&limit=50&page=21"}}"#).unwrap();
        assert_eq!(
            p.next_cursor(),
            None,
            "page x limit > 1000 would answer 429"
        );
        let p: ModelsPage = serde_json::from_str(r#"{}"#).unwrap();
        assert!(p.items.is_empty());
    }

    #[test]
    fn api_key_only_for_civitai() {
        let k = Some("k3y");
        assert_eq!(
            civitai_auth_header(k, "https://civitai.com/api/download/models/1"),
            Some(("Authorization".into(), "Bearer k3y".into()))
        );
        assert!(civitai_auth_header(k, "https://image.civitai.com/x.jpeg").is_some());
        assert!(civitai_auth_header(k, "https://huggingface.co/a/b").is_none());
        assert!(civitai_auth_header(k, "https://civitai.com.evil.example/x").is_none());
        assert!(civitai_auth_header(k, "https://evilcivitai.com/x").is_none());
        assert!(
            civitai_auth_header(k, "http://civitai.com/x").is_none(),
            "never over plain http"
        );
        assert!(civitai_auth_header(None, "https://civitai.com/x").is_none());
        assert!(civitai_auth_header(Some("  "), "https://civitai.com/x").is_none());
    }

    #[test]
    fn preview_urls() {
        assert!(is_preview_url(
            "https://image.civitai.com/xG1nkqKTMzGDvpLrqFT7WA/abc/width=450/1.jpeg"
        ));
        assert!(is_preview_url("https://civitai.com/x.png"));
        assert!(!is_preview_url("http://image.civitai.com/x.jpeg"));
        assert!(!is_preview_url("https://huggingface.co/x.png"));
        assert!(!is_preview_url(
            "https://image.civitai.com.evil.example/x.jpeg"
        ));
        assert!(!is_preview_url("https://user:pw@image.civitai.com/x.jpeg"));
        assert!(!is_preview_url("https://image.civitai.com:8443/x.jpeg"));
        assert!(!is_preview_url("file:///etc/passwd"));
        assert!(!is_preview_url("not a url"));
    }

    #[test]
    fn query_encoding() {
        assert_eq!(encode("Highest Rated"), "Highest%20Rated");
        assert_eq!(encode("SD 1.5"), "SD%201.5");
        assert_eq!(encode("2|17"), "2%7C17");
        assert_eq!(encode("a+b&c"), "a%2Bb%26c");
    }
}
