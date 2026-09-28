//! CivitAI browse / detail / preview images / API key. OWNER: catalog agent.
//!
//! PRIVACY: every request goes through `core.http` (allow-list + Offline mode).
//! Browsing never sends the API key; downloads (and the permission probe)
//! send it as an `Authorization` header to civitai.com only. The key lives in
//! the OS keychain and in RAM, never in `Data/`, logs or errors.

use std::sync::Arc;

use pinhole_catalog::api::CivitaiClient;
use pinhole_catalog::cards::RegistryEnv;
use pinhole_catalog::families::{self, FamilyResolution};
use pinhole_catalog::plan::{self, PlanEnv};
use pinhole_catalog::{browse as browse_mod, local, select, CatalogFilters};
use pinhole_net::NetError;
use pinhole_store::keychain;

pub use pinhole_catalog::filters::{BrowseQuery, CatalogKind, ContentMode, PriceMode};
pub use pinhole_catalog::gallery::ModelGallery;
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
        .map_err(|e| CoreError::invalid("Pinhole's catalog settings couldn't be loaded. Reinstall Pinhole.").with_details(e.to_string()))?;
    Ok(core.models.filters.get_or_init(|| Arc::new(loaded)).clone())
}

/// CivitAI errors in plain words.
pub fn net_error(e: NetError) -> CoreError {
    match e {
        NetError::Status(429) => CoreError::new("network", "CivitAI is busy right now. Wait a minute and try again."),
        NetError::Status(404) => CoreError::not_found("CivitAI couldn't find that model. It may have been removed."),
        NetError::Status(s) if s >= 500 => CoreError::new("network", "CivitAI is having problems right now. Try again later."),
        NetError::Timeout => CoreError::new("network", "CivitAI took too long to answer. Check your connection and try again."),
        other => other.into(),
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> CoreResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|_| CoreError::internal("A background task stopped unexpectedly. Try again."))
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

/// One Browse page. Offline mode: no request, `offline: true`.
pub async fn browse(core: &AppCore, query: BrowseQuery) -> CoreResult<BrowsePage> {
    if core.offline.get() {
        return Ok(BrowsePage { items: Vec::new(), next_cursor: query.cursor, offline: true, partial: false });
    }
    let filters = filters(core)?;
    let registry = core.registry();
    let hw = crate::app::hw_context(core);
    let index = core.installed.lock().clone();
    let env = RegistryEnv::new(&registry, hw, &index);
    // Browsing is anonymous: no API key.
    let client = CivitaiClient::new(core.http.clone(), None);
    let base_models = registry.all_civitai_base_models();
    browse_mod::browse(&client, &filters, &query, &base_models, &env, chrono::Utc::now()).await.map_err(net_error)
}

/// Preview image bytes (the WebView makes no network calls). Only https
/// CivitAI image hosts; at most 15 MB.
pub async fn fetch_preview(core: &AppCore, url: &str) -> CoreResult<Vec<u8>> {
    if !is_preview_url(url) {
        return Err(CoreError::invalid("Only CivitAI preview images can be loaded."));
    }
    core.http.get_bytes(url, &[], MAX_PREVIEW_BYTES).await.map_err(|e| match e {
        NetError::TooLarge => CoreError::invalid("This preview is too large to show."),
        other => net_error(other),
    })
}

pub use pinhole_catalog::api::is_preview_url;

// ------------------------------------------------------------------ details

/// The details page's gallery: the version's preview images with their
/// generation data (in memory only). Anonymous, like browsing. Offline mode:
/// no request, `offline: true`.
pub async fn model_gallery(core: &AppCore, version_id: u64, content: ContentMode, model_nsfw: bool) -> CoreResult<ModelGallery> {
    if core.offline.get() {
        return Ok(ModelGallery { items: Vec::new(), hidden_nsfw: 0, trained_words: Vec::new(), offline: true });
    }
    let filters = filters(core)?;
    let client = CivitaiClient::new(core.http.clone(), None);
    let version = client.model_version(version_id).await.map_err(net_error)?;
    Ok(pinhole_catalog::gallery::gallery(&version, content, model_nsfw, filters.preview_width))
}

/// `https://civitai.com/models/…` (civitai.red for NSFW models), for the
/// system browser. Built here so the UI can't open arbitrary URLs.
pub fn civitai_page_url(model_id: u64, version_id: Option<u64>, nsfw: bool) -> CoreResult<String> {
    if model_id == 0 {
        return Err(CoreError::invalid("That model has no CivitAI page."));
    }
    Ok(pinhole_catalog::gallery::civitai_page_url(model_id, version_id, nsfw))
}

// ------------------------------------------------------------------ install

async fn fetch_version(client: &CivitaiClient, version_id: u64) -> CoreResult<(pinhole_catalog::api::ModelVersion, Option<pinhole_catalog::api::Model>)> {
    let version = client.model_version(version_id).await.map_err(net_error)?;
    // Best effort: license / commercial use / type live on the model.
    let model = if version.model_id > 0 { client.model(version.model_id).await.ok() } else { None };
    Ok((version, model))
}

/// Everything the Install dialog shows before downloading (SPEC §5.4).
pub async fn plan_civitai_install(core: &AppCore, version_id: u64) -> CoreResult<InstallPlan> {
    let filters = filters(core)?;
    let client = civitai_client(core).await;
    let (version, model) = fetch_version(&client, version_id).await?;
    let registry = core.registry();
    let hw = crate::app::hw_context(core);
    let index = core.installed.lock().clone();

    // Does the download need a key (401/403)? Only asked when there is
    // something to download.
    let picked = select::select_file(&version.files, &filters.allowed_file_formats).ok();
    let already = picked.and_then(|f| f.sha256()).is_some_and(|h| index.find_by_sha(&h).is_some());
    let needs_api_key = match picked.filter(|_| !already) {
        Some(f) if !f.download_url.is_empty() => matches!(client.probe_download(&f.download_url).await, Ok(401 | 403)),
        _ => false,
    };
    let free = local::free_space(&core.data.root.join("models"));
    let env = PlanEnv { registry: &registry, index: &index, hw: &hw, filters: &filters };
    Ok(plan::build_plan(&env, &version, model.as_ref(), free, needs_api_key))
}

/// Download a CivitAI version (safe file + missing components) as one group;
/// files are registered when it finishes. `family_id` = the user's pick when
/// the plan listed several candidates.
pub async fn install_civitai(core: &Arc<AppCore>, version_id: u64, family_id: Option<String>) -> CoreResult<InstallStarted> {
    let filters = filters(core)?;
    let client = civitai_client(core).await;
    let (version, model) = fetch_version(&client, version_id).await?;
    let registry = core.registry();
    let hw = crate::app::hw_context(core);
    let index = core.installed.lock().clone();

    let kind = plan::version_kind(&version, model.as_ref());
    let is_lora = filters.is_lora_type(&kind);
    let file = select::select_file(&version.files, &filters.allowed_file_formats).map_err(CoreError::invalid)?;
    let family = match family_id.filter(|f| !f.is_empty()) {
        Some(f) if registry.family(&f).is_some() => Some(f),
        Some(_) => return Err(CoreError::invalid("That model type isn't known to Pinhole. Pick one from the list.")),
        None => match families::resolve_family(&registry, file.sha256().as_deref(), Some(&version.base_model), None) {
            FamilyResolution::Resolved(f) => Some(f),
            FamilyResolution::Ambiguous(_) if is_lora => None,
            FamilyResolution::Ambiguous(_) => return Err(CoreError::invalid("Pick which kind of model this is first.")),
            FamilyResolution::Unsupported(base) => return Err(CoreError::invalid(families::unsupported_message(base.as_deref()))),
        },
    };
    let env = PlanEnv { registry: &registry, index: &index, hw: &hw, filters: &filters };
    let install = plan::civitai_install_files(&env, &version, model.as_ref(), family.as_deref()).map_err(CoreError::invalid)?;
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
    crate::models::start_install(core, install.label, items, client.api_key().map(str::to_string)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{test_core, Recorder};

    #[tokio::test]
    async fn offline_browse_and_preview_guard() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        let opts = catalog_filters(&core).unwrap();
        assert_eq!(opts.looks.len(), 5);
        core.offline.set(true);
        let page = browse(&core, BrowseQuery::default()).await.unwrap();
        assert!(page.offline && page.items.is_empty());
        assert_eq!(fetch_preview(&core, "https://evil.example/x.jpeg").await.unwrap_err().code, "invalid");
        assert_eq!(fetch_preview(&core, "http://image.civitai.com/x.jpeg").await.unwrap_err().code, "invalid");
        assert_eq!(fetch_preview(&core, "https://image.civitai.com/x/width=450/1.jpeg").await.unwrap_err().code, "offline");
        assert_eq!(plan_civitai_install(&core, 1).await.unwrap_err().code, "offline");
        assert_eq!(install_civitai(&core, 1, None).await.unwrap_err().code, "offline");
        assert!(model_gallery(&core, 1, ContentMode::Safe, false).await.unwrap().offline);
    }

    #[test]
    fn plain_network_errors() {
        assert!(net_error(NetError::Status(429)).message.contains("busy"));
        assert_eq!(net_error(NetError::Status(404)).code, "not_found");
        assert_eq!(net_error(NetError::Unauthorized(401)).code, "unauthorized");
        assert_eq!(net_error(NetError::Offline).code, "offline");
    }
}
