//! Installed models, add-a-file, delete, recommended picks, install flows,
//! CivitAI resource resolution (paste). OWNER: catalog agent.
//!
//! The pure logic lives in `pinhole_catalog` (inventory, recommend, paste,
//! local); this module wires it to `AppCore` (index, registry, hardware,
//! downloads, events).
//!
//! PRIVACY: nothing here sees prompt text. Pasted resources carry ids/hashes
//! and model names only.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::Mutex;
use pinhole_catalog::cache::PageCache;
use pinhole_catalog::families::{self, FamilyResolution};
use pinhole_catalog::recommend::{self, FileToGet, PickAction};
use pinhole_catalog::{inventory, local, paste, CatalogFilters};
use pinhole_net::download::{check_free_space, ContentCheck, DownloadSpec, DownloadedFile};
use pinhole_registry::detect;
use pinhole_store::datadir::ModelKind;
use pinhole_store::installed::{CivitaiRef, InstalledFile};

pub use pinhole_catalog::view::{
    AddFileResult, DeleteFile, DeletePreview, DeleteReason, FamilyChoice, InstalledLora, InstalledModel, NeedsChoice,
    PastedResource, RecommendedPick, ResolvedResource, ResolvedResources,
};

use crate::{AppCore, CoreError, CoreEvent, CoreResult, InstallStarted};

/// Catalog / model-install state. RAM only.
#[derive(Default)]
pub struct ModelsState {
    /// "Add a file" results waiting for the user to pick a family (token → file).
    pending: Mutex<HashMap<String, PendingAdd>>,
    /// Files currently queued or downloading: key (`url:<url>` / `component:<id>`) → group id.
    inflight: Mutex<HashMap<String, String>>,
    /// Destination paths of files queued or downloading (until registered), so
    /// two installs never pick the same file name.
    inflight_dests: Mutex<HashSet<PathBuf>>,
    /// CivitAI API key cache: `None` = not read yet. Never written anywhere.
    pub(crate) api_key: Mutex<Option<Option<String>>>,
    /// `catalog-filters.yaml`, loaded once.
    pub(crate) filters: OnceLock<Arc<CatalogFilters>>,
    /// Recent CivitAI `/models` answers (RAM only; see `pinhole_catalog::cache`).
    pub(crate) page_cache: OnceLock<Arc<PageCache>>,
    /// Bumped by every Browse request; an older one still paging stops early.
    pub(crate) browse_gen: AtomicU64,
    /// A purge of expired cache entries is scheduled.
    pub(crate) cache_purge_pending: Arc<AtomicBool>,
    /// Adding / deleting files holds a read lock; moving the Models folder
    /// takes the write lock, so neither starts while the other runs.
    pub(crate) folder_lock: tokio::sync::RwLock<()>,
}

/// Read side of [`ModelsState::folder_lock`]: fails fast while the models move.
pub(crate) fn folder_read(core: &AppCore) -> CoreResult<tokio::sync::RwLockReadGuard<'_, ()>> {
    core.models.folder_lock.try_read().map_err(|_| CoreError::invalid("Pinhole is moving your models. Try again when it has restarted."))
}

/// `DataDir::models_dir_for_write`, refused while the Models folder moves.
pub(crate) fn models_dir_for_write(core: &AppCore, kind: ModelKind) -> CoreResult<PathBuf> {
    drop(folder_read(core)?);
    Ok(core.data.models_dir_for_write(kind)?)
}

/// Drop "Add a file" copies still waiting for a family choice (the Models
/// folder is about to move; the app restarts afterwards).
pub(crate) fn discard_pending(core: &AppCore) {
    for (_, p) in core.models.pending.lock().drain() {
        remove_copy(&p);
    }
}

/// A file copied into `Data/models/` whose family the user still has to pick.
#[derive(Debug, Clone)]
struct PendingAdd {
    path: PathBuf,
    /// We made the copy (so we may delete it if the add is abandoned).
    copied: bool,
    sha256: String,
    size_bytes: u64,
    kind: ModelKind,
    file_name: String,
    friendly_name: String,
    dtype: Option<String>,
    civitai: Option<CivitaiRef>,
    candidates: Vec<String>,
    created: std::time::Instant,
}

/// Unanswered family choices are dropped (and their copies deleted) after this.
const PENDING_TTL: Duration = Duration::from_secs(30 * 60);

/// Forget expired "Add a file" choices and delete the copies we made for them.
fn purge_expired_pending(core: &AppCore) {
    let expired: Vec<PendingAdd> = {
        let mut pending = core.models.pending.lock();
        let keys: Vec<String> = pending.iter().filter(|(_, p)| p.created.elapsed() > PENDING_TTL).map(|(k, _)| k.clone()).collect();
        keys.into_iter().filter_map(|k| pending.remove(&k)).collect()
    };
    expired.iter().for_each(remove_copy);
}

/// What a finished download is, for registering it in `installed.json`.
#[derive(Debug, Clone)]
pub struct Registration {
    pub kind: ModelKind,
    pub friendly_name: String,
    pub family: Option<String>,
    pub component_id: Option<String>,
    pub civitai: Option<CivitaiRef>,
    pub dtype: Option<String>,
}

/// `Data`-relative, `/`-separated path of a file inside the Data folder.
fn rel_path_for(core: &AppCore, path: &Path) -> CoreResult<String> {
    if let Some(rel) = core.data.relative(path) {
        return Ok(rel);
    }
    // Symlinked / non-canonical roots: compare canonical forms.
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let dir = pinhole_store::DataDir::at(canon(&core.data.root), core.data.portable)
        .with_models_home(core.data.models_home.as_deref().map(canon));
    if let Some(rel) = path.canonicalize().ok().and_then(|file| dir.relative(&file)) {
        return Ok(rel);
    }
    Err(CoreError::invalid("This file isn't inside Pinhole's Data folder, so it can't be registered."))
}

/// Add a downloaded (already verified, already in `Data/models/...`) file to
/// `installed.json`, save it, and emit `ModelsChanged`. Shared by every install
/// flow (catalog, recommended, captioner, upscaler). Re-registering the same
/// path updates the existing entry (keeps its id).
pub fn register_download(core: &AppCore, file: &DownloadedFile, reg: Registration) -> CoreResult<InstalledFile> {
    let rel_path = rel_path_for(core, &file.path)?;
    let sha256 = file.sha256.trim().to_ascii_lowercase();
    let entry = {
        let mut index = core.installed.lock();
        let previous = index.files.iter().find(|f| f.rel_path == rel_path).cloned();
        let same_file = previous.as_ref().is_some_and(|p| p.sha256.eq_ignore_ascii_case(&sha256));
        let entry = InstalledFile {
            id: previous.as_ref().map(|p| p.id.clone()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            rel_path,
            kind: reg.kind,
            sha256,
            size_bytes: file.size_bytes,
            family: reg.family,
            component_id: reg.component_id,
            friendly_name: reg.friendly_name,
            civitai: reg.civitai,
            added_at: chrono::Utc::now().timestamp(),
            last_used: previous.as_ref().and_then(|p| p.last_used),
            // A measured peak only stays valid for the very same file.
            observed_vram_gb: previous.as_ref().filter(|_| same_file).and_then(|p| p.observed_vram_gb),
            dtype: reg.dtype,
        };
        let before = index.files.clone();
        index.upsert(entry.clone());
        if let Err(e) = index.save(&core.data) {
            index.files = before;
            return Err(e.into());
        }
        entry
    };
    core.emit(CoreEvent::ModelsChanged);
    Ok(entry)
}

/// Main model files: record what the header says (all-in-one checkpoint vs
/// diffusion-only file, dtype) instead of assuming the family's layout — e.g.
/// CivitAI "Flux" checkpoints are sometimes full files with VAE + encoders.
fn refine_main_registration(registry: &pinhole_registry::Registry, path: &Path, mut reg: Registration) -> Registration {
    if reg.component_id.is_some() || !matches!(reg.kind, ModelKind::Checkpoint | ModelKind::Diffusion) {
        return reg;
    }
    if let Ok(header) = detect::read_header(path) {
        let d = detect::detect(registry, &header);
        if !d.is_lora && d.is_component.is_none() && (!d.candidates.is_empty() || d.has_vae || d.has_text_encoders) {
            reg.kind = match d.layout {
                pinhole_registry::Layout::AllInOne => ModelKind::Checkpoint,
                pinhole_registry::Layout::DiffusionOnly => ModelKind::Diffusion,
            };
        }
        if reg.dtype.is_none() && !d.dtype.is_empty() {
            reg.dtype = Some(d.dtype);
        }
    }
    reg
}

fn snapshot(core: &AppCore) -> pinhole_store::InstalledIndex {
    core.installed.lock().clone()
}

/// Installed main models (checkpoints + diffusion files), by name.
pub fn list_models(core: &AppCore) -> CoreResult<Vec<InstalledModel>> {
    let registry = core.registry();
    let hw = crate::app::hw_context(core);
    let index = snapshot(core);
    let mut out: Vec<InstalledModel> = index.models().map(|f| inventory::installed_model_view(&registry, &index, f, &hw)).collect();
    out.sort_by_key(|m| m.friendly_name.to_lowercase());
    Ok(out)
}

pub fn list_loras(core: &AppCore) -> CoreResult<Vec<InstalledLora>> {
    let index = snapshot(core);
    let mut out: Vec<InstalledLora> = index.loras().map(inventory::installed_lora_view).collect();
    out.sort_by_key(|l| l.friendly_name.to_lowercase());
    Ok(out)
}

/// `InstalledHelper` in src/lib/types.ts: a helper model that isn't picked
/// on Generate (the Describe model, the upscaler), shown on Models → Installed.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledHelper {
    /// `describe`, or the installed file id.
    pub id: String,
    pub friendly_name: String,
    /// `describe` | `upscale`
    pub purpose: String,
    pub size_bytes: u64,
}

/// Helper id of the default Describe model (its model + vision files together).
pub const DESCRIBE_HELPER_ID: &str = "describe";

fn helper_files<'a>(index: &'a pinhole_store::InstalledIndex, helper_id: &str) -> Vec<&'a InstalledFile> {
    let describe = [crate::describe::DEFAULT_MODEL_ID, crate::describe::DEFAULT_MMPROJ_ID];
    index
        .files
        .iter()
        .filter(|f| matches!(f.kind, ModelKind::Captioner | ModelKind::Upscaler))
        .filter(|f| {
            let is_describe = f.component_id.as_deref().is_some_and(|c| describe.contains(&c));
            if helper_id == DESCRIBE_HELPER_ID { is_describe } else { !is_describe && f.id == helper_id }
        })
        .collect()
}

/// Installed helpers: the Describe model (one row for its two files) and
/// upscalers / other captioner files.
pub fn list_helpers(core: &AppCore) -> CoreResult<Vec<InstalledHelper>> {
    let index = snapshot(core);
    let mut out = Vec::new();
    let describe = helper_files(&index, DESCRIBE_HELPER_ID);
    if !describe.is_empty() {
        out.push(InstalledHelper {
            id: DESCRIBE_HELPER_ID.into(),
            friendly_name: "Describe model".into(),
            purpose: "describe".into(),
            size_bytes: describe.iter().map(|f| f.size_bytes).sum(),
        });
    }
    let describe_ids: Vec<&str> = describe.iter().map(|f| f.id.as_str()).collect();
    for f in index.files.iter().filter(|f| matches!(f.kind, ModelKind::Captioner | ModelKind::Upscaler)) {
        if describe_ids.contains(&f.id.as_str()) {
            continue;
        }
        out.push(InstalledHelper {
            id: f.id.clone(),
            friendly_name: f.friendly_name.clone(),
            purpose: if f.kind == ModelKind::Upscaler { "upscale" } else { "describe" }.into(),
            size_bytes: f.size_bytes,
        });
    }
    Ok(out)
}

/// Delete a helper's files. Refused while it's in use; the engine that has it
/// open is stopped first (Windows can't delete an open file).
pub async fn delete_helper(core: &AppCore, helper_id: &str) -> CoreResult<()> {
    let _folder = folder_read(core)?;
    if helper_files(&snapshot(core), helper_id).is_empty() {
        return Err(CoreError::not_found("That helper isn't installed any more."));
    }
    if helper_id == DESCRIBE_HELPER_ID {
        if core.describe.busy.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(CoreError::invalid("Wait for the picture description to finish, then delete the Describe model."));
        }
        crate::describe::shutdown(core).await;
    } else {
        if core.gen.active.lock().is_some() {
            return Err(CoreError::invalid("Wait for the current pictures to finish, then delete it."));
        }
        crate::generate::shutdown(core).await;
    }
    let failed;
    {
        let mut index = core.installed.lock();
        let ids: Vec<String> = helper_files(&index, helper_id).into_iter().map(|f| f.id.clone()).collect();
        failed = remove_entries(core, &mut index, &ids);
        index.save(&core.data)?;
    }
    core.emit(CoreEvent::ModelsChanged);
    if !failed.is_empty() {
        return Err(CoreError::new("io", format!("Couldn't delete {}. Close any program using it and try again.", failed.join(", "))));
    }
    Ok(())
}

pub fn view_of(core: &AppCore, file: &InstalledFile) -> AddFileResult {
    let registry = core.registry();
    let hw = crate::app::hw_context(core);
    let index = snapshot(core);
    if file.kind == ModelKind::Lora {
        AddFileResult { model: None, lora: Some(inventory::installed_lora_view(file)), needs_choice: None }
    } else {
        AddFileResult { model: Some(inventory::installed_model_view(&registry, &index, file, &hw)), lora: None, needs_choice: None }
    }
}

// ------------------------------------------------------------------ recommended

pub fn get_recommended(core: &AppCore) -> CoreResult<Vec<RecommendedPick>> {
    let registry = core.registry();
    let hw = crate::app::hw_context(core);
    let index = snapshot(core);
    Ok(recommend::recommend(&registry, &index, &hw).into_iter().map(|p| p.pick).collect())
}

/// One-click install of a role's pick (first run, empty Create/Edit/Describe).
pub async fn install_recommended(core: &Arc<AppCore>, role: &str) -> CoreResult<InstallStarted> {
    let action = {
        let registry = core.registry();
        let hw = crate::app::hw_context(core);
        let index = snapshot(core);
        let plan = recommend::recommend_role(&registry, &index, &hw, role)
            .ok_or_else(|| CoreError::not_found(format!("There is no recommended model for “{role}”.")))?;
        match plan.action {
            PickAction::Nothing if plan.pick.installed => return Err(CoreError::invalid("This model is already installed.")),
            PickAction::Nothing => {
                return Err(CoreError::new(
                    "vram",
                    plan.pick.unavailable_reason.unwrap_or_else(|| "No recommended model fits this computer.".into()),
                ))
            }
            other => other,
        }
    };
    match action {
        PickAction::Download { label, files } => {
            let items = files.into_iter().map(|f| (f, None)).collect();
            start_install(core, label, items, None).await
        }
        PickAction::Civitai { version_id, family_id } => crate::catalog::install_civitai(core, version_id, Some(family_id), None).await,
        PickAction::Captioner => crate::describe::install_captioner(core).await,
        PickAction::Nothing => Err(CoreError::internal("Nothing to install.")),
    }
}

// ------------------------------------------------------------------ download groups

fn inflight_key(f: &FileToGet) -> String {
    match &f.component_id {
        Some(id) => format!("component:{id}"),
        None => format!("url:{}", f.url),
    }
}

fn disk_space_error(e: pinhole_net::download::DownloadError, url: &str) -> CoreError {
    CoreError::new(e.code(), e.user_message(url))
}

/// Shown when a downloaded model file fails [`model_file_check`].
pub const INVALID_MODEL_FILE: &str = "The downloaded file isn't a valid model file, so Pinhole removed it.";

/// Download content check: the file must parse as a safetensors / GGUF model
/// (bounded header read, see `pinhole_registry::detect::read_header`) with at
/// least one tensor, in a weight layout the engine can load
/// (`detect::unsupported_weights`: e.g. CivitAI int8 files must be ComfyUI
/// `int8_tensorwise`). On failure the downloader deletes the file.
pub fn model_file_check() -> ContentCheck {
    Arc::new(|path: &Path| match detect::read_header(path) {
        Ok(h) if !h.tensor_names.is_empty() => match detect::unsupported_weights(&h) {
            Some(why) => Err(why.to_string()),
            None => Ok(()),
        },
        _ => Err(INVALID_MODEL_FILE.to_string()),
    })
}

/// Queue one download group (model + missing components), then register every
/// file when it finishes (background task) and emit `models-changed`.
/// Components another running group is already fetching are skipped; a second
/// click on the same model returns the running group.
pub(crate) async fn start_install(
    core: &Arc<AppCore>,
    label: String,
    items: Vec<(FileToGet, Option<CivitaiRef>)>,
    api_key: Option<String>,
) -> CoreResult<InstallStarted> {
    let index = snapshot(core);
    let (group_id, planned) = {
        let mut inflight = core.models.inflight.lock();
        if let Some((main, _)) = items.iter().find(|(f, _)| f.component_id.is_none()) {
            if let Some(group) = inflight.get(&inflight_key(main)) {
                return Ok(InstallStarted { group_id: group.clone() });
            }
        }
        let items: Vec<(FileToGet, Option<CivitaiRef>)> =
            items.into_iter().filter(|(f, _)| !inflight.contains_key(&inflight_key(f))).collect();
        if items.is_empty() {
            return Err(CoreError::invalid("This is already downloading."));
        }
        let total: u64 = items.iter().map(|(f, _)| f.size_bytes).sum();
        let models_root = models_dir_for_write(core, ModelKind::Checkpoint).map(|_| core.data.models_root())?;
        check_free_space(&models_root, total).map_err(|e| disk_space_error(e, &items[0].0.url))?;

        let mut specs = Vec::new();
        let mut planned: Vec<(PathBuf, String, Registration)> = Vec::new();
        let mut dests = core.models.inflight_dests.lock();
        for (f, civitai) in items {
            let dir = models_dir_for_write(core, f.kind)?;
            // Never overwrite a registered file, one planned in this group or one
            // another running install is downloading to.
            let dest = local::unique_path(&dir, &f.file_name, |p| {
                planned.iter().any(|(d, _, _)| d == p)
                    || dests.contains(p)
                    || (p.exists() && core.data.relative(p).is_some_and(|rel| index.files.iter().any(|x| x.rel_path == rel)))
            });
            let headers: Vec<(String, String)> =
                pinhole_catalog::api::civitai_auth_header(api_key.as_deref(), &f.url).into_iter().collect();
            // CivitAI files (the hash only proves what the uploader sent) and model
            // files without a pinned hash must parse as safetensors/GGUF.
            let check = civitai.is_some() || (f.component_id.is_none() && f.sha256.is_none());
            specs.push(DownloadSpec {
                url: f.url.clone(),
                dest: dest.clone(),
                sha256: f.sha256.clone(),
                // Registry `size_mb` and CivitAI `sizeKB` are rounded: estimates only.
                size_bytes: None,
                approx_size_bytes: (f.size_bytes > 0).then_some(f.size_bytes),
                label: f.friendly_name.clone(),
                headers,
                content_check: check.then(model_file_check),
            });
            let reg = Registration {
                kind: f.kind,
                friendly_name: f.friendly_name.clone(),
                family: f.family.clone(),
                component_id: f.component_id.clone(),
                civitai,
                dtype: f.dtype.clone(),
            };
            planned.push((dest, inflight_key(&f), reg));
        }
        let group_id = core.downloads.enqueue_kind(label, pinhole_net::download::DownloadKind::Model, specs);
        for (dest, key, _) in &planned {
            inflight.insert(key.clone(), group_id.clone());
            dests.insert(dest.clone());
        }
        (group_id, planned)
    };

    let task_core = core.clone();
    let task_group = group_id.clone();
    tokio::spawn(async move {
        let result = task_core.downloads.wait(&task_group).await;
        {
            let mut inflight = task_core.models.inflight.lock();
            for (_, key, _) in &planned {
                if inflight.get(key) == Some(&task_group) {
                    inflight.remove(key);
                }
            }
        }
        // Failures are reported through the group's `download-progress` status.
        let planned_dests: Vec<PathBuf> = planned.iter().map(|(d, _, _)| d.clone()).collect();
        if let Ok(files) = result {
            let same_len = files.len() == planned.len();
            for (i, (dest, _, reg)) in planned.into_iter().enumerate() {
                let file = files.iter().find(|f| f.path == dest).or_else(|| files.get(i).filter(|_| same_len));
                if let Some(file) = file.cloned() {
                    let registry = task_core.registry();
                    let path = file.path.clone();
                    let fallback = reg.clone();
                    let reg = tokio::task::spawn_blocking(move || refine_main_registration(&registry, &path, reg))
                        .await
                        .unwrap_or(fallback);
                    let _ = register_download(&task_core, &file, reg);
                }
            }
        }
        // Only now (registered, or failed) may another install pick these names.
        let mut dests = task_core.models.inflight_dests.lock();
        for d in &planned_dests {
            dests.remove(d);
        }
    });
    Ok(InstallStarted { group_id })
}

// ------------------------------------------------------------------ add a file I already have

fn plain_name(path: &Path) -> String {
    path.file_stem().and_then(|s| s.to_str()).map(|s| s.replace(['_', '-'], " ").trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "Model".into())
}

fn remove_copy(p: &PendingAdd) {
    if p.copied {
        let _ = std::fs::remove_file(&p.path);
    }
}

fn register_pending(core: &AppCore, p: &PendingAdd, family: Option<String>) -> CoreResult<AddFileResult> {
    let file = DownloadedFile { path: p.path.clone(), sha256: p.sha256.clone(), size_bytes: p.size_bytes };
    let entry = register_download(
        core,
        &file,
        Registration {
            kind: p.kind,
            friendly_name: p.friendly_name.clone(),
            family,
            component_id: None,
            civitai: p.civitai.clone(),
            dtype: p.dtype.clone(),
        },
    )?;
    Ok(view_of(core, &entry))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> CoreResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|_| CoreError::internal("A background task stopped unexpectedly. Try again."))
}

/// "Add a file I already have": check the extension, read the header, copy the
/// file into `Data/models/<kind>/` while hashing it (the user's file is never
/// moved or changed), then resolve the family: known hash → CivitAI by-hash
/// (online only, failures ignored) → header sniffing → ask (`needsChoice`).
pub async fn add_local_model(core: &Arc<AppCore>, path: &str) -> CoreResult<AddFileResult> {
    let _folder = folder_read(core)?;
    let src = PathBuf::from(path.trim());
    let ext = local::allowed_extension(&src).ok_or_else(|| {
        CoreError::invalid("Only .safetensors and .gguf files can be added. Older .ckpt/.pt files can hide harmful code.")
    })?;
    if !src.is_file() {
        return Err(CoreError::not_found("Pinhole can't find that file. Check that it still exists and try again."));
    }
    let registry = core.registry();
    purge_expired_pending(core);

    let header_src = src.clone();
    let reg_for_detect = registry.clone();
    let (header, detection) = blocking(move || {
        detect::read_header(&header_src).map(|h| {
            let d = detect::detect(&reg_for_detect, &h);
            (h, d)
        })
    })
    .await?
    .map_err(|e| CoreError::invalid("This file isn't a model Pinhole can read.").with_details(e.to_string()))?;
    if let Some(kind) = detection.is_component.as_deref() {
        return Err(CoreError::invalid(format!(
            "This file is a {} (a part of a model), not a model. Pinhole downloads those parts by itself.",
            families::component_kind_label(kind).to_lowercase()
        )));
    }
    let kind = if detection.is_lora {
        ModelKind::Lora
    } else {
        match detection.layout {
            pinhole_registry::Layout::AllInOne => ModelKind::Checkpoint,
            pinhole_registry::Layout::DiffusionOnly => ModelKind::Diffusion,
        }
    };

    // Copy (or hash in place when the file already lives in the Data folder).
    let data_root = core.data.root.canonicalize().unwrap_or_else(|_| core.data.root.clone());
    let models_root = core.data.models_root().canonicalize().unwrap_or_else(|_| core.data.models_root());
    let src_canon = src.canonicalize().unwrap_or_else(|_| src.clone());
    // With a picked Models folder only files already in it count as in place;
    // anything else (even a leftover in Data/models) is copied into it.
    let in_place = src_canon.starts_with(&models_root) || (core.data.models_home.is_none() && src_canon.starts_with(&data_root));
    let (dest, sha256, size_bytes) = if in_place {
        let p = src_canon.clone();
        let (sha, size) = blocking(move || local::hash_file(&p)).await??;
        (src_canon, sha, size)
    } else {
        let dir = core.data.models_dir_for_write(kind)?; // folder lock held above
        check_free_space(&dir, header.file_size).map_err(|e| disk_space_error(e, ""))?;
        let name = local::sanitize_file_name(src.file_name().and_then(|n| n.to_str()).unwrap_or("model"), ext);
        let dest = local::unique_path(&dir, &name, |p| p.exists());
        let (from, to) = (src.clone(), dest.clone());
        let (sha, size) = blocking(move || local::copy_and_hash(&from, &to))
            .await?
            .map_err(|e| CoreError::new("io", "Pinhole couldn't copy the file into its Data folder.").with_details(e.to_string()))?;
        (dest, sha, size)
    };
    let mut pending = PendingAdd {
        path: dest,
        copied: !in_place,
        sha256: sha256.clone(),
        size_bytes,
        kind,
        file_name: src.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string(),
        friendly_name: plain_name(&src),
        dtype: Some(detection.dtype.clone()).filter(|d| !d.is_empty()),
        civitai: None,
        candidates: Vec::new(),
        created: std::time::Instant::now(),
    };

    // Already installed (same bytes)? Keep the existing entry.
    if let Some(existing) = snapshot(core).find_by_sha(&sha256).cloned() {
        if pending.copied && core.data.relative(&pending.path).is_some_and(|rel| rel != existing.rel_path) {
            remove_copy(&pending);
        }
        return Ok(view_of(core, &existing));
    }

    // 1. known hash
    let mut resolution = families::resolve_family(&registry, Some(&sha256), None, None);
    if let Some(k) = registry.known_file(&sha256) {
        if let Some(name) = k.friendly_name.clone() {
            pending.friendly_name = name;
        }
    }
    // 2. CivitAI by hash (online only; failures are ignored).
    if !matches!(resolution, FamilyResolution::Resolved(_)) && !core.offline.get() {
        let client = crate::catalog::civitai_client(core).await;
        let lookup = tokio::time::timeout(Duration::from_secs(20), client.by_hash(&sha256)).await;
        if let Ok(Ok(Some(v))) = lookup {
            let name = pinhole_catalog::plan::model_name(&v, None);
            pending.friendly_name = pinhole_catalog::plan::friendly_name(&v, None);
            pending.civitai = Some(CivitaiRef {
                model_id: v.model_id,
                version_id: v.id,
                model_name: Some(name),
                version_name: Some(v.name.clone()).filter(|n| !n.is_empty()),
                base_model: Some(v.base_model.clone()).filter(|b| !b.is_empty()),
                trained_words: v.trained_words.clone(),
                license: None,
            });
            resolution = families::resolve_family(&registry, Some(&sha256), Some(&v.base_model), Some(&detection.candidates));
        }
    }
    // 3. header sniffing
    if matches!(resolution, FamilyResolution::Unsupported(None)) {
        resolution = families::resolve_family(&registry, None, None, Some(&detection.candidates));
    }
    match resolution {
        FamilyResolution::Resolved(family) => register_pending(core, &pending, Some(family)).inspect_err(|_| remove_copy(&pending)),
        FamilyResolution::Ambiguous(candidates) => {
            // 4. ask
            let token = uuid::Uuid::new_v4().to_string();
            let choices: Vec<FamilyChoice> = candidates.iter().filter_map(|id| families::family_choice(&registry, id)).collect();
            pending.candidates = candidates;
            let file_name = pending.file_name.clone();
            let replaced = core.models.pending.lock().insert(token.clone(), pending);
            if let Some(old) = replaced {
                remove_copy(&old);
            }
            Ok(AddFileResult { model: None, lora: None, needs_choice: Some(NeedsChoice { token, file_name, candidates: choices }) })
        }
        // A LoRA of unknown base still works; it just can't be checked for compatibility.
        FamilyResolution::Unsupported(_) if kind == ModelKind::Lora => register_pending(core, &pending, None),
        FamilyResolution::Unsupported(base) => {
            remove_copy(&pending);
            Err(CoreError::invalid(families::unsupported_message(base.as_deref())))
        }
    }
}

/// Finish an ambiguous "Add a file" with the family the user picked.
pub fn confirm_family(core: &AppCore, token: &str, family_id: &str) -> CoreResult<AddFileResult> {
    let registry = core.registry();
    if registry.family(family_id).is_none() {
        return Err(CoreError::invalid("That model type isn't known to Pinhole. Pick one from the list."));
    }
    purge_expired_pending(core);
    let pending = core
        .models
        .pending
        .lock()
        .remove(token)
        .ok_or_else(|| CoreError::not_found("This choice has expired. Add the file again."))?;
    register_pending(core, &pending, Some(family_id.to_string())).inspect_err(|_| remove_copy(&pending))
}

// ------------------------------------------------------------------ delete

pub fn preview_delete(core: &AppCore, model_id: &str) -> CoreResult<DeletePreview> {
    let registry = core.registry();
    let index = snapshot(core);
    inventory::delete_preview(&registry, &index, model_id)
        .ok_or_else(|| CoreError::not_found("That model isn't installed any more."))
}

/// Delete a model and the components no other installed model needs. The
/// engine unloads it first.
pub async fn delete_model(core: &AppCore, model_id: &str) -> CoreResult<()> {
    let _folder = folder_read(core)?;
    let registry = core.registry();
    let paths: Vec<PathBuf> = {
        let index = core.installed.lock();
        let ids = delete_ids(&registry, &index, model_id)?;
        ids.iter().filter_map(|id| index.get(id)).map(|f| index.abs_path(&core.data, f)).collect()
    };
    crate::generate::unload_model(core, model_id, &paths).await;
    let failed;
    {
        let mut index = core.installed.lock();
        let ids = delete_ids(&registry, &index, model_id)?;
        failed = remove_entries(core, &mut index, &ids);
        index.save(&core.data)?;
    }
    core.emit(CoreEvent::ModelsChanged);
    if !failed.is_empty() {
        return Err(CoreError::new(
            "io",
            format!("Couldn't delete {}. Close any program using it and try again.", failed.join(", ")),
        ));
    }
    Ok(())
}

/// Ids of the entries deleting `model_id` removes: the model and the
/// components no other model needs (by id: two entries can share a path).
fn delete_ids(registry: &pinhole_registry::Registry, index: &pinhole_store::InstalledIndex, model_id: &str) -> CoreResult<Vec<String>> {
    if index.get(model_id).is_none() {
        return Err(CoreError::not_found("That model isn't installed any more."));
    }
    let orphans = inventory::orphaned_components(registry, index, model_id);
    Ok(std::iter::once(model_id.to_string()).chain(orphans.into_iter().map(|f| f.id.clone())).collect())
}

/// Delete the files of these index entries (and their `.part` leftovers) and
/// drop the entries; returns the names of files that couldn't be deleted
/// (their entries stay). A file another remaining entry also points at is
/// kept. The caller saves the index.
fn remove_entries(core: &AppCore, index: &mut pinhole_store::InstalledIndex, ids: &[String]) -> Vec<String> {
    let mut failed = Vec::new();
    for id in ids {
        let Some(f) = index.get(id).cloned() else { continue };
        let shared = index.files.iter().any(|o| o.id != f.id && !ids.contains(&o.id) && o.rel_path == f.rel_path);
        if inventory::is_safe_rel_path(&f.rel_path) && !shared {
            let abs = index.abs_path(&core.data, &f);
            match std::fs::remove_file(&abs) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => {
                    failed.push(f.friendly_name.clone());
                    continue;
                }
            }
            let _ = std::fs::remove_file(local::part_path(&abs));
        }
        index.remove(id);
    }
    failed
}

// ------------------------------------------------------------------ paste from CivitAI

/// Match pasted CivitAI resources to installed files or installable versions.
pub async fn resolve_civitai_resources(core: &AppCore, resources: Vec<PastedResource>) -> CoreResult<ResolvedResources> {
    let registry = core.registry();
    let hw = crate::app::hw_context(core);
    let index = snapshot(core);
    let filters = crate::catalog::filters(core)?;
    let env = paste::PasteEnv { registry: &registry, index: &index, hw: &hw, filters: &filters };
    let client = if core.offline.get() { None } else { Some(crate::catalog::civitai_client(core).await) };
    Ok(paste::resolve_resources(&env, client.as_ref(), &resources).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{test_core, Recorder};

    /// Minimal safetensors file with these tensor names (F16, 2 elements each).
    fn safetensors(path: &Path, names: &[&str]) {
        let mut header = serde_json::Map::new();
        for (i, n) in names.iter().enumerate() {
            header.insert(
                n.to_string(),
                serde_json::json!({ "dtype": "F16", "shape": [2], "data_offsets": [i * 4, i * 4 + 4] }),
            );
        }
        let h = serde_json::to_vec(&serde_json::Value::Object(header)).unwrap();
        let mut bytes = (h.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(&h);
        bytes.extend(std::iter::repeat_n(0u8, names.len() * 4));
        std::fs::write(path, bytes).unwrap();
    }

    const SDXL_TENSORS: &[&str] = &[
        "model.diffusion_model.input_blocks.0.0.weight",
        "conditioner.embedders.1.model.ln_final.weight",
        "conditioner.embedders.0.transformer.text_model.embeddings.token_embedding.weight",
    ];

    fn events(rec: &Recorder) -> usize {
        rec.0.lock().iter().filter(|e| **e == "models-changed").count()
    }

    #[test]
    fn register_download_round_trip() {
        let rec = Arc::new(Recorder::default());
        let (_t, core) = test_core(rec.clone());
        let dir = core.data.models(ModelKind::Checkpoint);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("m.safetensors");
        std::fs::write(&path, b"abc").unwrap();
        let file = DownloadedFile { path: path.clone(), sha256: "AB".repeat(32), size_bytes: 3 };
        let reg = Registration {
            kind: ModelKind::Checkpoint,
            friendly_name: "My model".into(),
            family: Some("sdxl".into()),
            component_id: None,
            civitai: Some(CivitaiRef {
                model_id: 1,
                version_id: 2,
                model_name: Some("M".into()),
                version_name: None,
                base_model: Some("SDXL 1.0".into()),
                trained_words: vec![],
                license: None,
            }),
            dtype: Some("f16".into()),
        };
        let entry = register_download(&core, &file, reg.clone()).unwrap();
        assert_eq!(entry.rel_path, "models/checkpoints/m.safetensors");
        assert_eq!(entry.sha256, "ab".repeat(32));
        assert!(uuid::Uuid::parse_str(&entry.id).is_ok());
        assert_eq!(events(&rec), 1);

        // Persisted: reload from disk.
        let reloaded = pinhole_store::InstalledIndex::load(&core.data).unwrap();
        assert_eq!(reloaded.files, vec![entry.clone()]);
        let text = std::fs::read_to_string(core.data.installed_file()).unwrap();
        assert!(!text.contains(&core.data.root.to_string_lossy().to_string()), "paths are Data-relative");

        // Same path again → same id (no duplicates).
        let again = register_download(&core, &file, reg).unwrap();
        assert_eq!(again.id, entry.id);
        assert_eq!(core.installed.lock().files.len(), 1);

        // Listed as an installed model.
        let models = list_models(&core).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].family_label.as_deref(), Some("SDXL"));
        assert_eq!(models[0].civitai_version_id, Some(2));
        assert!(!models[0].missing_components.is_empty(), "the SDXL fp16-fix VAE isn't installed");

        // Files outside Data are refused.
        let outside = DownloadedFile { path: _t.path().join("elsewhere.safetensors"), sha256: "cd".repeat(32), size_bytes: 1 };
        let reg = Registration { kind: ModelKind::Lora, friendly_name: "x".into(), family: None, component_id: None, civitai: None, dtype: None };
        assert_eq!(register_download(&core, &outside, reg).unwrap_err().code, "invalid");
    }

    #[tokio::test]
    async fn add_local_file_copies_hashes_and_asks_or_resolves() {
        let rec = Arc::new(Recorder::default());
        let (tmp, core) = test_core(rec.clone());
        core.offline.set(true); // no CivitAI lookup in tests

        // Wrong extension.
        let bad = tmp.path().join("m.ckpt");
        std::fs::write(&bad, b"x").unwrap();
        assert_eq!(add_local_model(&core, bad.to_str().unwrap()).await.unwrap_err().code, "invalid");

        // SDXL-shaped header → a family is resolved or the user is asked.
        let src = tmp.path().join("My Model (v2).safetensors");
        safetensors(&src, SDXL_TENSORS);
        let before = std::fs::read(&src).unwrap();
        let out = add_local_model(&core, src.to_str().unwrap()).await.unwrap();
        assert_eq!(std::fs::read(&src).unwrap(), before, "the user's file is untouched");
        let model = match out.needs_choice {
            Some(choice) => {
                assert_eq!(choice.file_name, "My Model (v2).safetensors");
                assert!(choice.candidates.iter().any(|c| c.family_id == "sdxl"), "{:?}", choice.candidates);
                assert!(core.installed.lock().files.is_empty(), "nothing registered until the user picks");
                assert_eq!(confirm_family(&core, "bogus", "sdxl").unwrap_err().code, "not_found");
                assert_eq!(confirm_family(&core, &choice.token, "nope").unwrap_err().code, "invalid");
                let done = confirm_family(&core, &choice.token, "sdxl_pony").unwrap();
                assert_eq!(confirm_family(&core, &choice.token, "sdxl").unwrap_err().code, "not_found", "token is single-use");
                done.model.unwrap()
            }
            None => out.model.unwrap(),
        };
        let idx = core.installed.lock().clone();
        let f = idx.get(&model.id).unwrap();
        assert_eq!(f.rel_path, "models/checkpoints/My Model (v2).safetensors");
        assert_eq!(f.sha256, local::hash_file(&src).unwrap().0);
        assert!(core.data.root.join(&f.rel_path).exists());
        assert_eq!(model.friendly_name, "My Model (v2)");

        // Adding the same bytes again returns the existing model, no second copy.
        let again = add_local_model(&core, src.to_str().unwrap()).await.unwrap();
        assert_eq!(again.model.unwrap().id, model.id);
        let copies = std::fs::read_dir(core.data.models(ModelKind::Checkpoint)).unwrap().count();
        assert_eq!(copies, 1);

        // Unknown tensors → refused, copy cleaned up.
        let odd = tmp.path().join("odd.safetensors");
        safetensors(&odd, &["some.random.tensor"]);
        let err = add_local_model(&core, odd.to_str().unwrap()).await.unwrap_err();
        assert_eq!(err.code, "invalid", "{}", err.message);
        let left: Vec<_> = std::fs::read_dir(core.data.root.join("models"))
            .unwrap()
            .flat_map(|d| std::fs::read_dir(d.unwrap().path()).unwrap())
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with("odd"))
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[tokio::test]
    async fn delete_removes_model_and_orphans() {
        let rec = Arc::new(Recorder::default());
        let (_t, core) = test_core(rec.clone());
        let put = |kind: ModelKind, name: &str, reg: Registration| {
            let dir = core.data.models(kind);
            std::fs::create_dir_all(&dir).unwrap();
            let p = dir.join(name);
            std::fs::write(&p, name.as_bytes()).unwrap();
            let sha = local::hash_file(&p).unwrap().0;
            register_download(&core, &DownloadedFile { path: p, sha256: sha, size_bytes: name.len() as u64 }, reg).unwrap()
        };
        let comp = |id: &str| Registration {
            kind: ModelKind::TextEncoder,
            friendly_name: id.into(),
            family: None,
            component_id: Some(id.into()),
            civitai: None,
            dtype: None,
        };
        let main = |fam: &str| Registration {
            kind: ModelKind::Diffusion,
            friendly_name: fam.into(),
            family: Some(fam.into()),
            component_id: None,
            civitai: None,
            dtype: None,
        };
        let zit = put(ModelKind::Diffusion, "zit.gguf", main("z_image_turbo"));
        let kontext = put(ModelKind::Diffusion, "kontext.gguf", main("flux1_kontext"));
        let ae = put(ModelKind::Vae, "ae.safetensors", Registration { kind: ModelKind::Vae, ..comp("flux_ae") });
        let qwen3 = put(ModelKind::TextEncoder, "qwen_3_4b.safetensors", comp("qwen3_4b"));

        let preview = preview_delete(&core, &zit.id).unwrap();
        let paths: Vec<&str> = preview.files.iter().map(|f| f.rel_path.as_str()).collect();
        assert_eq!(paths, ["models/diffusion/zit.gguf", "models/text_encoders/qwen_3_4b.safetensors"]);

        delete_model(&core, &zit.id).await.unwrap();
        let idx = core.installed.lock().clone();
        assert!(idx.get(&zit.id).is_none() && idx.get(&qwen3.id).is_none());
        assert!(idx.get(&kontext.id).is_some() && idx.get(&ae.id).is_some(), "shared VAE stays");
        assert!(!core.data.root.join(&zit.rel_path).exists());
        assert!(!core.data.root.join(&qwen3.rel_path).exists());
        assert!(core.data.root.join(&ae.rel_path).exists());
        assert_eq!(pinhole_store::InstalledIndex::load(&core.data).unwrap().files.len(), 2);
        assert_eq!(preview_delete(&core, &zit.id).unwrap_err().code, "not_found");
        assert_eq!(delete_model(&core, &zit.id).await.unwrap_err().code, "not_found");

        // A hostile rel_path never deletes outside Data (the entry is just dropped).
        let outside = _t.path().join("victim.txt");
        std::fs::write(&outside, b"keep").unwrap();
        {
            let mut idx = core.installed.lock();
            let mut evil = idx.get(&kontext.id).unwrap().clone();
            evil.id = "evil".into();
            evil.family = None;
            evil.rel_path = "../victim.txt".into();
            idx.upsert(evil);
        }
        delete_model(&core, "evil").await.unwrap();
        assert!(outside.exists());
    }

    #[tokio::test]
    async fn delete_uses_the_id_not_a_shared_path() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        let dir = core.data.models(ModelKind::Checkpoint);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.safetensors");
        std::fs::write(&p, b"theirs").unwrap();
        let reg = Registration { kind: ModelKind::Checkpoint, friendly_name: "a".into(), family: Some("sd15".into()), component_id: None, civitai: None, dtype: None };
        let theirs = register_download(&core, &DownloadedFile { path: p.clone(), sha256: "ab".repeat(32), size_bytes: 6 }, reg).unwrap();
        // An older index could hold a second (missing) entry with the same path.
        let mut lost = theirs.clone();
        lost.id = "lost".into();
        lost.sha256 = "cd".repeat(32);
        core.installed.lock().files.insert(0, lost);

        delete_model(&core, "lost").await.unwrap();
        let idx = core.installed.lock().clone();
        assert!(idx.get("lost").is_none(), "the entry asked for is removed");
        assert!(idx.get(&theirs.id).is_some(), "the other entry stays");
        assert_eq!(std::fs::read(&p).unwrap(), b"theirs", "and so does its file");
    }

    #[test]
    fn downloaded_model_files_must_parse() {
        let (tmp, _core) = test_core(Arc::new(Recorder::default()));
        let check = model_file_check();
        let good = tmp.path().join("good.safetensors");
        safetensors(&good, SDXL_TENSORS);
        assert_eq!(check(&good), Ok(()));
        // An HTML error page, a pickle, an empty header, a missing file.
        let html = tmp.path().join("page.safetensors");
        std::fs::write(&html, b"<!DOCTYPE html><html>Please log in</html>").unwrap();
        let pickle = tmp.path().join("m.safetensors");
        std::fs::write(&pickle, b"\x80\x02}q\x00(X\x05\x00\x00\x00model").unwrap();
        let empty = tmp.path().join("empty.safetensors");
        safetensors(&empty, &[]);
        for bad in [&html, &pickle, &empty, &tmp.path().join("missing.gguf")] {
            assert_eq!(check(bad), Err(INVALID_MODEL_FILE.to_string()), "{}", bad.display());
        }
        // Valid safetensors in a layout the engine can't load (U8-packed NVFP4 weight).
        let nvfp4 = tmp.path().join("k_nvfp4.safetensors");
        let h = br#"{"blocks.0.attn.wq.weight":{"dtype":"U8","shape":[4],"data_offsets":[0,4]}}"#;
        let mut bytes = (h.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(h);
        bytes.extend_from_slice(&[0u8; 4]);
        std::fs::write(&nvfp4, bytes).unwrap();
        assert_eq!(check(&nvfp4), Err(detect::UNSUPPORTED_WEIGHTS.to_string()));
    }

    #[test]
    fn main_files_keep_their_real_layout() {
        let (tmp, core) = test_core(Arc::new(Recorder::default()));
        let p = tmp.path().join("sdxl.safetensors");
        safetensors(&p, SDXL_TENSORS);
        let reg = Registration {
            kind: ModelKind::Diffusion,
            friendly_name: "x".into(),
            family: Some("sdxl".into()),
            component_id: None,
            civitai: None,
            dtype: None,
        };
        let out = refine_main_registration(&core.registry(), &p, reg.clone());
        assert_eq!(out.kind, ModelKind::Checkpoint, "SDXL files with text encoders are all-in-one");
        assert!(out.dtype.is_some());
        // Components and unreadable files are left alone.
        let comp = Registration { component_id: Some("flux_ae".into()), ..reg.clone() };
        assert_eq!(refine_main_registration(&core.registry(), &p, comp).kind, ModelKind::Diffusion);
        assert_eq!(refine_main_registration(&core.registry(), &tmp.path().join("missing"), reg).kind, ModelKind::Diffusion);
    }

    #[test]
    fn recommended_through_core() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        let picks = get_recommended(&core).unwrap();
        assert_eq!(picks.iter().map(|p| p.role.as_str()).collect::<Vec<_>>(), ["realistic", "anime", "edit", "describe"]);
        // Before hardware detection: CPU only → the small SD 1.5 (runs on the
        // processor) and the captioner; the big anime / edit models don't fit.
        assert_eq!(picks[0].family_id.as_deref(), Some("sd15"));
        assert_eq!(picks[0].fit, Some(pinhole_registry::vram::Fit::Tight));
        assert!(picks[0].vram.unwrap().on_cpu);
        assert!(picks[1].unavailable_reason.is_some() && picks[2].unavailable_reason.is_some());
        assert_eq!(picks[3].title.as_deref(), Some("Image describer"));
    }

    #[tokio::test]
    async fn install_recommended_errors_are_plain() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        // CPU only: no anime model runs acceptably on the processor.
        let e = install_recommended(&core, "anime").await.unwrap_err();
        assert_eq!(e.code, "vram");
        assert!(e.message.contains("graphics card"), "{}", e.message);
        assert_eq!(install_recommended(&core, "nope").await.unwrap_err().code, "not_found");
    }

    #[tokio::test]
    async fn install_recommended_queues_one_group_with_components() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        core.offline.set(true); // downloads fail fast, nothing leaves the machine
        // Tiny sizes so the free-disk check passes on small CI disks.
        let yaml = std::fs::read_to_string(core.shipped.config_dir.join("models.yaml"))
            .unwrap()
            // 8 GB → Z-Image Q4_K (the Q8 is Tight there, Q4 closer to fitting).
            .replace("size_mb: 3864", "size_mb: 1")
            .replace("size_mb: 335", "size_mb: 1")
            // 8 GB → the Qwen3-4B Q4_K_M GGUF text encoder (bf16 only from 20 GB).
            .replace("size_mb: 2497", "size_mb: 1");
        *core.registry.write() = Arc::new(pinhole_registry::Registry::from_yaml(&yaml, None).unwrap());
        {
            let mut s = core.settings.write();
            s.vram_override_gb = Some(8.0);
            s.gpu = "auto".into();
            // The VRAM override only counts with a GPU backend (no hardware detected here).
            s.engine_backend = "cuda".into();
        }
        let started = install_recommended(&core, "realistic").await.unwrap();
        let status = core.downloads.status();
        let g = status.iter().find(|g| g.group_id == started.group_id).unwrap();
        assert_eq!(g.label, "Z-Image Turbo");
        assert_eq!(g.file_count, 3, "model + VAE + text encoder");
        // Registry sizes are rounded: they only feed the UI total (not `size_bytes`).
        assert_eq!(g.total_bytes, 3_000_000);
        // A second click returns the same running group.
        let again = install_recommended(&core, "realistic").await;
        if let Ok(again) = again {
            assert_eq!(again.group_id, started.group_id);
        }
        // Offline → the group fails; nothing gets registered.
        let _ = core.downloads.wait(&started.group_id).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(core.installed.lock().files.is_empty());
    }

    fn lora_file(url: &str, name: &str) -> FileToGet {
        FileToGet {
            url: url.into(),
            file_name: name.into(),
            sha256: None,
            size_bytes: 1,
            kind: ModelKind::Lora,
            friendly_name: name.into(),
            family: None,
            component_id: None,
            dtype: None,
        }
    }

    #[tokio::test]
    async fn queued_installs_with_the_same_file_name_get_different_paths() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        core.offline.set(true); // downloads fail fast, nothing leaves the machine
        let a = lora_file("https://civitai.com/api/download/models/1", "style.safetensors");
        let b = lora_file("https://civitai.com/api/download/models/2", "style.safetensors");
        let ga = start_install(&core, "A".into(), vec![(a, None)], None).await.unwrap();
        let gb = start_install(&core, "B".into(), vec![(b, None)], None).await.unwrap();
        assert_ne!(ga.group_id, gb.group_id);
        let dir = core.data.models(ModelKind::Lora);
        let dests = core.models.inflight_dests.lock().clone();
        assert_eq!(dests, HashSet::from([dir.join("style.safetensors"), dir.join("style-2.safetensors")]));
        // Once the groups end (offline: failed), the names are free again.
        let _ = core.downloads.wait(&ga.group_id).await;
        let _ = core.downloads.wait(&gb.group_id).await;
        for _ in 0..50 {
            if core.models.inflight_dests.lock().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(core.models.inflight_dests.lock().is_empty());
    }

    #[tokio::test]
    async fn paste_offline() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        core.offline.set(true);
        let out = resolve_civitai_resources(
            &core,
            vec![PastedResource { kind: "checkpoint".into(), model_version_id: Some(1759168), ..Default::default() }],
        )
        .await
        .unwrap();
        assert!(out.checkpoint.unwrap().problem.unwrap().contains("Offline"));
    }

    #[tokio::test]
    async fn describe_model_is_listed_as_a_helper_and_can_be_deleted() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        let dir = core.data.models(ModelKind::Captioner);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, comp) in [("m.gguf", crate::describe::DEFAULT_MODEL_ID), ("p.gguf", crate::describe::DEFAULT_MMPROJ_ID)] {
            std::fs::write(dir.join(name), b"1234").unwrap();
            let file = DownloadedFile { path: dir.join(name), sha256: "ab".repeat(32), size_bytes: 4 };
            let reg = Registration {
                kind: ModelKind::Captioner,
                friendly_name: "Describe model".into(),
                family: None,
                component_id: Some(comp.into()),
                civitai: None,
                dtype: None,
            };
            register_download(&core, &file, reg).unwrap();
        }
        // Not a main model, but shown as one helper row with both files' size.
        assert!(list_models(&core).unwrap().is_empty());
        let helpers = list_helpers(&core).unwrap();
        assert_eq!(helpers.len(), 1);
        assert_eq!((helpers[0].id.as_str(), helpers[0].purpose.as_str(), helpers[0].size_bytes), (DESCRIBE_HELPER_ID, "describe", 8));

        delete_helper(&core, DESCRIBE_HELPER_ID).await.unwrap();
        assert!(list_helpers(&core).unwrap().is_empty());
        assert!(!dir.join("m.gguf").exists() && !dir.join("p.gguf").exists());
        assert!(delete_helper(&core, DESCRIBE_HELPER_ID).await.is_err());
    }
}
