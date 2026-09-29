//! CivitAI browse / detail / preview images / API key. OWNER: catalog agent.
//!
//! PRIVACY: every request goes through `core.http` (allow-list + Offline mode).
//! Browsing never sends the API key; downloads (and the permission probe)
//! send it as an `Authorization` header to civitai.com only. The key lives in
//! the OS keychain and in RAM, never in `Data/`, logs or errors.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use pinhole_catalog::api::CivitaiClient;
use pinhole_catalog::browse::BrowseError;
use pinhole_catalog::cache::{CachedSource, PageCache};
use pinhole_catalog::cards::RegistryEnv;
use pinhole_catalog::families::{self, FamilyResolution};
use pinhole_catalog::plan::{self, PlanEnv};
use pinhole_catalog::{browse as browse_mod, local, select, CatalogFilters};
use pinhole_net::NetError;
use pinhole_store::keychain;

pub use pinhole_catalog::filters::{BrowseQuery, CatalogKind, ContentMode, PriceMode};
pub use pinhole_catalog::view::{BrowsePage, CatalogCard, CatalogFilterOptions, InstallPlan};

use crate::{AppCore, CoreError, CoreResult, InstallStarted};

/// Largest preview image we fetch (CLAUDE.md: previews come through Rust).
pub const MAX_PREVIEW_BYTES: usize = 15 * 1024 * 1024;

/// `catalog-filters.yaml`, loaded once per run.
pub fn filters(core: &AppCore) -> CoreResult<Arc<CatalogFilters>> {
    if let Some(f) = core.models.filters.get() {
        return Ok(f.clone());
    }
    let loaded = CatalogFilters::load(&core.shipped.config_dir.join("catalog-filters.yaml"))
        .map_err(|e| {
            CoreError::invalid("Pinhole's catalog settings couldn't be loaded. Reinstall Pinhole.")
                .with_details(e.to_string())
        })?;
    Ok(core.models.filters.get_or_init(|| Arc::new(loaded)).clone())
}

/// CivitAI errors in plain words.
pub fn net_error(e: NetError) -> CoreError {
    match e {
        NetError::Status(429) => CoreError::new(
            "network",
            "CivitAI is busy right now. Wait a minute and try again.",
        ),
        NetError::Status(404) => {
            CoreError::not_found("CivitAI couldn't find that model. It may have been removed.")
        }
        NetError::Status(s) if s >= 500 => CoreError::new(
            "network",
            "CivitAI is having problems right now. Try again later.",
        ),
        NetError::Timeout => CoreError::new(
            "network",
            "CivitAI took too long to answer. Check your connection and try again.",
        ),
        other => other.into(),
    }
}

pub(crate) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> CoreResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| CoreError::internal("A background task stopped unexpectedly. Try again."))
}

// ------------------------------------------------------------------ API key

/// The CivitAI API key (cached in RAM after the first keychain read).
/// Keychain problems count as "no key".
pub async fn api_key(core: &AppCore) -> Option<String> {
    if let Some(cached) = core.models.api_key.lock().clone() {
        return cached;
    }
    match blocking(keychain::get_civitai_key).await {
        Ok(Ok(key)) => {
            *core.models.api_key.lock() = Some(key.clone());
            key
        }
        _ => None,
    }
}

/// Client for metadata + downloads (carries the key, sent only where needed).
pub async fn civitai_client(core: &AppCore) -> CivitaiClient {
    CivitaiClient::new(core.http.clone(), api_key(core).await)
}

pub async fn has_civitai_key(core: &AppCore) -> CoreResult<bool> {
    Ok(api_key(core).await.is_some())
}

pub async fn set_civitai_key(core: &AppCore, key: String) -> CoreResult<()> {
    let key = key.trim().to_string();
    let stored = key.clone();
    blocking(move || keychain::set_civitai_key(&stored)).await??;
    *core.models.api_key.lock() = Some(Some(key));
    Ok(())
}

pub async fn clear_civitai_key(core: &AppCore) -> CoreResult<()> {
    blocking(keychain::delete_civitai_key).await??;
    *core.models.api_key.lock() = Some(None);
    Ok(())
}

// ------------------------------------------------------------------ browse

pub fn catalog_filters(core: &AppCore) -> CoreResult<CatalogFilterOptions> {
    Ok(filters(core)?.options())
}

/// The RAM cache of CivitAI answers (sized by `catalog-filters.yaml`).
fn page_cache(core: &AppCore, filters: &CatalogFilters) -> Arc<PageCache> {
    core.models
        .page_cache
        .get_or_init(|| Arc::new(PageCache::new(filters.cache_ttl, filters.cache_pages)))
        .clone()
}

/// Drop expired cache entries once they are stale, so idle RAM goes back down.
fn schedule_cache_purge(core: &AppCore, cache: &Arc<PageCache>) {
    if cache.is_empty() || core.models.cache_purge_pending.swap(true, Ordering::SeqCst) {
        return;
    }
    let Ok(rt) = tokio::runtime::Handle::try_current() else {
        core.models
            .cache_purge_pending
            .store(false, Ordering::SeqCst);
        return;
    };
    let cache = cache.clone();
    let ttl = cache.ttl();
    let pending = core.models.cache_purge_pending.clone();
    rt.spawn(async move {
        tokio::time::sleep(ttl + std::time::Duration::from_secs(1)).await;
        cache.purge_expired();
        pending.store(false, Ordering::SeqCst);
    });
}

/// Drop cached CivitAI answers (Offline mode switched, installed models changed…).
pub fn clear_browse_cache(core: &AppCore) {
    if let Some(c) = core.models.page_cache.get() {
        c.clear();
    }
}

/// One Browse page. Offline mode: no request, `offline: true`. A request that a
/// newer Browse request replaced stops paging and answers `cancelled`.
pub async fn browse(core: &AppCore, query: BrowseQuery) -> CoreResult<BrowsePage> {
    let generation = core.models.browse_gen.fetch_add(1, Ordering::SeqCst) + 1;
    if core.offline.get() {
        return Ok(BrowsePage::offline(query.cursor));
    }
    let filters = filters(core)?;
    let registry = core.registry();
    let hw = crate::app::hw_context(core);
    let index = core.installed.lock().clone();
    let env = RegistryEnv::new(&registry, hw, &index);
    // Browsing is anonymous: no API key.
    let client = CivitaiClient::new(core.http.clone(), None);
    let cache = page_cache(core, &filters);
    let source = CachedSource {
        inner: &client,
        cache: &cache,
    };
    let base_models = registry.all_civitai_base_models();
    let current = || core.models.browse_gen.load(Ordering::SeqCst) == generation;
    let out = browse_mod::browse(
        &source,
        &filters,
        &query,
        &base_models,
        &env,
        chrono::Utc::now(),
        current,
    )
    .await;
    schedule_cache_purge(core, &cache);
    out.map_err(|e| match e {
        BrowseError::Net(e) => net_error(e),
        BrowseError::Superseded => CoreError::new("cancelled", "Replaced by a newer search."),
    })
}

/// Formats card previews may come back in (`optimized=true` renditions are
/// negotiated; AVIF is left out because not every Linux WebView decodes it).
const PREVIEW_ACCEPT: &str = "image/webp,image/jpeg,image/png;q=0.9,*/*;q=0.5";

/// Preview image bytes (the WebView makes no network calls). Only https
/// CivitAI image hosts; at most 15 MB.
pub async fn fetch_preview(core: &AppCore, url: &str) -> CoreResult<Vec<u8>> {
    if !is_preview_url(url) {
        return Err(CoreError::invalid(
            "Only CivitAI preview images can be loaded.",
        ));
    }
    core.http
        .get_bytes(url, &[("accept", PREVIEW_ACCEPT)], MAX_PREVIEW_BYTES)
        .await
        .map_err(|e| match e {
            NetError::TooLarge => CoreError::invalid("This preview is too large to show."),
            other => net_error(other),
        })
}

pub use pinhole_catalog::api::is_preview_url;

// ------------------------------------------------------------------ install

type VersionAndModel = (
    pinhole_catalog::api::ModelVersion,
    Option<pinhole_catalog::api::Model>,
);

/// How long a fetched version (+ its model) is reused: the Install dialog
/// plans again for every file pick and Install fetches it once more.
const VERSION_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(180);
const VERSION_CACHE_MAX: usize = 8;

/// Recent CivitAI `/model-versions/{id}` + `/models/{id}` answers. RAM only.
#[derive(Default)]
pub(crate) struct VersionCache(
    parking_lot::Mutex<Vec<(u64, std::time::Instant, Arc<VersionAndModel>)>>,
);

impl VersionCache {
    fn get(&self, version_id: u64) -> Option<Arc<VersionAndModel>> {
        let mut entries = self.0.lock();
        entries.retain(|(_, at, _)| at.elapsed() < VERSION_CACHE_TTL);
        entries
            .iter()
            .find(|(id, _, _)| *id == version_id)
            .map(|(_, _, v)| v.clone())
    }

    fn put(&self, version_id: u64, value: Arc<VersionAndModel>) {
        let mut entries = self.0.lock();
        entries.retain(|(id, _, _)| *id != version_id);
        entries.push((version_id, std::time::Instant::now(), value));
        if entries.len() > VERSION_CACHE_MAX {
            entries.remove(0);
        }
    }
}

async fn fetch_version(
    cache: &VersionCache,
    client: &CivitaiClient,
    version_id: u64,
) -> CoreResult<Arc<VersionAndModel>> {
    if let Some(hit) = cache.get(version_id) {
        return Ok(hit);
    }
    let version = client.model_version(version_id).await.map_err(net_error)?;
    // Best effort: license / commercial use / type live on the model.
    let model = if version.model_id > 0 {
        client.model(version.model_id).await.ok()
    } else {
        None
    };
    // Kept only when complete, so a failed model fetch is tried again.
    let complete = model.is_some() || version.model_id == 0;
    let value = Arc::new((version, model));
    if complete {
        cache.put(version_id, value.clone());
    }
    Ok(value)
}

/// Everything the Install dialog shows before downloading (SPEC §5.4).
pub async fn plan_civitai_install(
    core: &AppCore,
    version_id: u64,
    file_id: Option<u64>,
) -> CoreResult<InstallPlan> {
    let filters = filters(core)?;
    let client = civitai_client(core).await;
    let fetched = fetch_version(&core.models.versions, &client, version_id).await?;
    let (version, model) = (&fetched.0, &fetched.1);
    let registry = core.registry();
    let hw = crate::app::hw_context(core);
    let index = core.installed.lock().clone();

    let free = local::free_space(&core.data.models_root());
    let env = PlanEnv {
        registry: &registry,
        index: &index,
        hw: &hw,
        filters: &filters,
    };
    let plan = plan::build_plan(&env, version, model.as_ref(), free, false, file_id);
    // Does the download need a key (401/403)? Only asked when there is
    // something to download.
    let picked = plan.file_options.iter().find(|o| o.selected).and_then(|o| {
        version
            .files
            .iter()
            .find(|f| f.id == o.file_id && f.name == o.name)
    });
    let already = picked
        .and_then(|f| f.sha256())
        .is_some_and(|h| index.find_by_sha(&h).is_some());
    let needs_api_key = match picked.filter(|_| !already) {
        Some(f) if !f.download_url.is_empty() => {
            matches!(client.probe_download(&f.download_url).await, Ok(401 | 403))
        }
        _ => false,
    };
    Ok(InstallPlan {
        needs_api_key,
        ..plan
    })
}

/// Download a CivitAI version (safe file + missing components) as one group;
/// files are registered when it finishes. `family_id` = the user's pick when
/// the plan listed several candidates.
pub async fn install_civitai(
    core: &Arc<AppCore>,
    version_id: u64,
    family_id: Option<String>,
    file_id: Option<u64>,
) -> CoreResult<InstallStarted> {
    let filters = filters(core)?;
    let client = civitai_client(core).await;
    let fetched = fetch_version(&core.models.versions, &client, version_id).await?;
    let (version, model) = (&fetched.0, &fetched.1);
    let registry = core.registry();
    let hw = crate::app::hw_context(core);
    let index = core.installed.lock().clone();

    let kind = plan::version_kind(version, model.as_ref());
    let is_lora = filters.is_lora_type(&kind);
    let file = select::select_file(&version.files, &filters.allowed_file_formats)
        .map_err(CoreError::invalid)?;
    let family = match family_id.filter(|f| !f.is_empty()) {
        Some(f) if registry.family(&f).is_some() => Some(f),
        Some(_) => {
            return Err(CoreError::invalid(
                "That model type isn't known to Pinhole. Pick one from the list.",
            ))
        }
        None => match families::resolve_family(
            &registry,
            file.sha256().as_deref(),
            Some(&version.base_model),
            None,
        ) {
            FamilyResolution::Resolved(f) => Some(f),
            FamilyResolution::Ambiguous(_) if is_lora => None,
            FamilyResolution::Ambiguous(_) => {
                return Err(CoreError::invalid(
                    "Pick which kind of model this is first.",
                ))
            }
            FamilyResolution::Unsupported(base) => {
                return Err(CoreError::invalid(families::unsupported_message(
                    base.as_deref(),
                )))
            }
        },
    };
    let env = PlanEnv {
        registry: &registry,
        index: &index,
        hw: &hw,
        filters: &filters,
    };
    let install =
        plan::civitai_install_files(&env, version, model.as_ref(), family.as_deref(), file_id)
            .map_err(CoreError::invalid)?;
    if install.files.is_empty() {
        return Err(CoreError::invalid("This model is already installed."));
    }
    let civitai = install.civitai.clone();
    let items = install
        .files
        .into_iter()
        .map(|f| {
            let r = f.component_id.is_none().then(|| civitai.clone());
            (f, r)
        })
        .collect();
    crate::models::start_install(
        core,
        install.label,
        items,
        client.api_key().map(str::to_string),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{test_core, Recorder};

    /// Re-planning for another file pick (and Install) reuses the version and
    /// model it just fetched instead of asking CivitAI again.
    #[tokio::test]
    async fn install_dialog_reuses_the_fetched_version() {
        use pinhole_net::testutil::{MockResponse, MockServer};
        let srv = MockServer::start(|req| match req.path.as_str() {
            "/model-versions/7" => {
                MockResponse::json(&serde_json::json!({ "id": 7, "modelId": 70 }))
            }
            "/model-versions/8" => {
                MockResponse::json(&serde_json::json!({ "id": 8, "modelId": 80 }))
            }
            "/models/70" => MockResponse::json(&serde_json::json!({ "id": 70, "name": "M" })),
            _ => MockResponse::status(500),
        })
        .await;
        let http =
            pinhole_net::HttpClient::new_for_tests(pinhole_net::OfflineFlag::new(false), true)
                .unwrap();
        let client = CivitaiClient::with_base(http, None, srv.url(""));
        let cache = VersionCache::default();
        let count = |p: &str| srv.requests().iter().filter(|r| r.path == p).count();
        for _ in 0..3 {
            let got = fetch_version(&cache, &client, 7).await.unwrap();
            assert_eq!(got.0.id, 7);
            assert_eq!(got.1.as_ref().map(|m| m.id), Some(70));
        }
        assert_eq!(count("/model-versions/7"), 1);
        assert_eq!(count("/models/70"), 1);
        // The model fetch failed: not kept, so the next plan tries again.
        for _ in 0..2 {
            let got = fetch_version(&cache, &client, 8).await.unwrap();
            assert!(got.1.is_none());
        }
        assert_eq!(count("/model-versions/8"), 2);
    }

    #[tokio::test]
    async fn offline_browse_and_preview_guard() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        let opts = catalog_filters(&core).unwrap();
        assert_eq!(opts.looks.len(), 5);
        core.offline.set(true);
        let page = browse(&core, BrowseQuery::default()).await.unwrap();
        assert!(page.offline && page.items.is_empty());
        assert_eq!(
            fetch_preview(&core, "https://evil.example/x.jpeg")
                .await
                .unwrap_err()
                .code,
            "invalid"
        );
        assert_eq!(
            fetch_preview(&core, "http://image.civitai.com/x.jpeg")
                .await
                .unwrap_err()
                .code,
            "invalid"
        );
        assert_eq!(
            fetch_preview(&core, "https://image.civitai.com/x/width=450/1.jpeg")
                .await
                .unwrap_err()
                .code,
            "offline"
        );
        assert_eq!(
            plan_civitai_install(&core, 1, None).await.unwrap_err().code,
            "offline"
        );
        assert_eq!(
            install_civitai(&core, 1, None, None)
                .await
                .unwrap_err()
                .code,
            "offline"
        );
    }

    #[test]
    fn plain_network_errors() {
        assert!(net_error(NetError::Status(429)).message.contains("busy"));
        assert_eq!(net_error(NetError::Status(404)).code, "not_found");
        assert_eq!(net_error(NetError::Unauthorized(401)).code, "unauthorized");
        assert_eq!(net_error(NetError::Offline).code, "offline");
    }
}
