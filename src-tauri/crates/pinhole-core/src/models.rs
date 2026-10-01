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
use pinhole_store::datadir::{normalize_rel, ModelKind};
use pinhole_store::installed::{CivitaiRef, InstalledFile, Lookup};

pub use pinhole_catalog::view::{
    AddFileResult, DeleteFile, DeletePreview, DeleteReason, FamilyChoice, InstalledLora,
    InstalledModel, NeedsChoice, PastedResource, RecommendedPick, ResolvedResource,
    ResolvedResources,
};

use crate::lookup::Outcome;
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
    /// Recent CivitAI version + model answers for the Install dialog (RAM only).
    pub(crate) versions: crate::catalog::VersionCache,
    /// Bumped by every Browse request; an older one still paging stops early.
    pub(crate) browse_gen: AtomicU64,
    /// A purge of expired cache entries is scheduled.
    pub(crate) cache_purge_pending: Arc<AtomicBool>,
    /// Adding / deleting files holds a read lock; moving the Models folder
    /// takes the write lock, so neither starts while the other runs.
    pub(crate) folder_lock: tokio::sync::RwLock<()>,
    /// Files whose CivitAI lookup is running (`lookup::look_up_pending`).
    pub(crate) lookups: Mutex<HashSet<String>>,
    /// Tests: CivitAI API base of a mock server.
    #[cfg(test)]
    pub(crate) test_civitai: Mutex<Option<String>>,
}

/// Read side of [`ModelsState::folder_lock`]: fails fast while the models move.
pub(crate) fn folder_read(core: &AppCore) -> CoreResult<tokio::sync::RwLockReadGuard<'_, ()>> {
    core.models.folder_lock.try_read().map_err(|_| {
        CoreError::invalid("Pinhole is moving your models. Try again when it has restarted.")
    })
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
    /// The CivitAI lookup's result (`None` for a file Pinhole offers itself).
    lookup: Option<Lookup>,
    candidates: Vec<String>,
    created: std::time::Instant,
}

/// Unanswered family choices are dropped (and their copies deleted) after this.
const PENDING_TTL: Duration = Duration::from_secs(30 * 60);

/// Forget expired "Add a file" choices and delete the copies we made for them.
fn purge_expired_pending(core: &AppCore) {
    let expired: Vec<PendingAdd> = {
        let mut pending = core.models.pending.lock();
        let keys: Vec<String> = pending
            .iter()
            .filter(|(_, p)| p.created.elapsed() > PENDING_TTL)
            .map(|(k, _)| k.clone())
            .collect();
        keys.into_iter()
            .filter_map(|k| pending.remove(&k))
            .collect()
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
    /// Files added by hand: the CivitAI lookup's result (`lookup.rs`). `None` for downloads.
    pub lookup: Option<Lookup>,
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
    if let Some(rel) = path
        .canonicalize()
        .ok()
        .and_then(|file| dir.relative(&file))
    {
        return Ok(rel);
    }
    Err(CoreError::invalid(
        "This file isn't inside Pinhole's Data folder, so it can't be registered.",
    ))
}

/// Add a downloaded (already verified, already in `Data/models/...`) file to
/// `installed.json`, save it, and emit `ModelsChanged`. Shared by every install
/// flow (catalog, recommended, captioner, upscaler). Re-registering the same
/// path updates the existing entry (keeps its id).
pub fn register_download(
    core: &AppCore,
    file: &DownloadedFile,
    reg: Registration,
) -> CoreResult<InstalledFile> {
    let rel_path = rel_path_for(core, &file.path)?;
    let sha256 = file.sha256.trim().to_ascii_lowercase();
    let entry = {
        let mut index = core.installed.lock();
        let previous = index.files.iter().find(|f| f.rel_path == rel_path).cloned();
        let same_file = previous
            .as_ref()
            .is_some_and(|p| p.sha256.eq_ignore_ascii_case(&sha256));
        let entry = InstalledFile {
            id: previous
                .as_ref()
                .map(|p| p.id.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
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
            observed_vram_gb: previous
                .as_ref()
                .filter(|_| same_file)
                .and_then(|p| p.observed_vram_gb),
            dtype: reg.dtype,
            // The user's own trigger words describe the file, so keep them for the same file.
            trigger_words: previous
                .as_ref()
                .filter(|_| same_file)
                .and_then(|p| p.trigger_words.clone()),
            lookup: reg.lookup,
        };
        let before = index.files.clone();
        index.upsert(entry.clone());
        // A part downloaded again replaces an entry whose file was deleted by hand (it may
        // have had another name), so the part is found by its new file.
        if let Some(cid) = entry.component_id.as_deref() {
            let data = &core.data;
            let stale: Vec<String> = index
                .files
                .iter()
                .filter(|f| f.id != entry.id && !f.is_linked())
                .filter(|f| f.component_id.as_deref() == Some(cid))
                .filter(|f| !index.abs_path(data, f).is_file())
                .map(|f| f.id.clone())
                .collect();
            index.files.retain(|f| !stale.contains(&f.id));
        }
        // An unreadable entry at this path described the file that was there.
        let replaced = index.remove_unknown_at(&entry.rel_path);
        if let Err(e) = index.save(&core.data) {
            index.files = before;
            index.unknown.extend(replaced);
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
fn refine_main_registration(
    registry: &pinhole_registry::Registry,
    path: &Path,
    mut reg: Registration,
) -> Registration {
    if reg.component_id.is_some()
        || !matches!(reg.kind, ModelKind::Checkpoint | ModelKind::Diffusion)
    {
        return reg;
    }
    if let Ok(header) = detect::read_header(path) {
        let d = detect::detect(registry, &header);
        if !d.is_lora
            && d.is_component.is_none()
            && (!d.candidates.is_empty() || d.has_vae || d.has_text_encoders)
        {
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

/// The index without parts (VAE, encoders…) whose file is gone from Pinhole's folders, so a
/// part deleted by hand shows as missing and "Get missing parts" downloads it again (as
/// Generate already reports it). Main models and add-ons stay listed so they can be deleted.
pub(crate) fn snapshot_present(core: &AppCore) -> pinhole_store::InstalledIndex {
    let mut index = snapshot(core);
    let data = &core.data;
    let gone: Vec<String> = index
        .files
        .iter()
        .filter(|f| {
            !f.is_linked()
                && !matches!(
                    f.kind,
                    ModelKind::Checkpoint | ModelKind::Diffusion | ModelKind::Lora
                )
        })
        .filter(|f| !index.abs_path(data, f).is_file())
        .map(|f| f.id.clone())
        .collect();
    index.files.retain(|f| !gone.contains(&f.id));
    index
}

/// Installed main models (checkpoints + diffusion files), by name.
pub fn list_models(core: &AppCore) -> CoreResult<Vec<InstalledModel>> {
    let registry = core.registry();
    let hw = crate::app::hw_context(core);
    let index = snapshot_present(core);
    let mut out: Vec<InstalledModel> = index
        .models()
        .map(|f| inventory::installed_model_view(&registry, &index, f, &hw))
        .collect();
    out.sort_by_key(|m| m.friendly_name.to_lowercase());
    Ok(out)
}

pub fn list_loras(core: &AppCore) -> CoreResult<Vec<InstalledLora>> {
    let index = snapshot(core);
    let mut out: Vec<InstalledLora> = index
        .loras()
        .map(|f| inventory::installed_lora_view(&index, f))
        .collect();
    out.sort_by_key(|l| l.friendly_name.to_lowercase());
    Ok(out)
}

/// Replace an installed add-on's trigger words with the user's own list
/// (trimmed, empty and duplicate words dropped). An empty list means "none".
pub fn set_lora_trigger_words(
    core: &AppCore,
    lora_id: &str,
    words: Vec<String>,
) -> CoreResult<InstalledLora> {
    let mut clean: Vec<String> = Vec::new();
    for w in words {
        let w = w.trim();
        if w.chars().count() > 200 {
            return Err(CoreError::invalid(
                "That trigger word is too long. Keep each one short.",
            ));
        }
        if !w.is_empty() && !clean.iter().any(|c| c.eq_ignore_ascii_case(w)) {
            clean.push(w.to_string());
        }
    }
    if clean.len() > 50 {
        return Err(CoreError::invalid(
            "That's too many trigger words. Keep the ones the add-on needs.",
        ));
    }
    let mut index = core.installed.lock();
    index.check_savable(&core.data)?;
    let file = index
        .files
        .iter_mut()
        .find(|f| f.id == lora_id && f.kind == ModelKind::Lora)
        .ok_or_else(|| CoreError::not_found("That add-on isn't installed any more."))?;
    let old = file.trigger_words.replace(clean);
    let file = file.clone();
    let view = inventory::installed_lora_view(&index, &file);
    if let Err(e) = index.save(&core.data) {
        // Not saved: keep the old words in memory too, so a later save can't write them.
        if let Some(f) = index.files.iter_mut().find(|f| f.id == lora_id) {
            f.trigger_words = old;
        }
        return Err(e.into());
    }
    Ok(view)
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

/// Component ids of a catalog helper (`captioner.helpers`), `None` for other helper files.
fn helper_group(core: &AppCore, helper_id: &str) -> Option<Vec<String>> {
    core.registry()
        .captioner()
        .helpers
        .iter()
        .find(|h| h.id == helper_id)
        .map(|h| {
            let (m, p) = crate::describe::helper_components(h);
            vec![m, p]
        })
}

/// Files of one helper that Pinhole downloaded as a helper (kind Captioner / Upscaler).
/// A Describe model's files that arrived with another model (Qwen Image Edit's encoder)
/// belong to that model and are not listed.
pub(crate) fn helper_files<'a>(
    core: &AppCore,
    index: &'a pinhole_store::InstalledIndex,
    helper_id: &str,
) -> Vec<&'a InstalledFile> {
    let group = helper_group(core, helper_id);
    let all_groups: Vec<String> = core
        .registry()
        .captioner()
        .helpers
        .iter()
        .flat_map(|h| {
            let (m, p) = crate::describe::helper_components(h);
            [m, p]
        })
        .collect();
    // A file shared with another helper whose model file is installed stays with that one
    // (the 7B vision file serves both 7B helpers): not listed or deleted here.
    let kept: Vec<String> = core
        .registry()
        .captioner()
        .helpers
        .iter()
        .filter(|h| h.id != helper_id)
        .filter_map(|h| {
            let (m, p) = crate::describe::helper_components(h);
            index.find_component(&m).is_some().then_some(p)
        })
        .collect();
    index
        .files
        .iter()
        .filter(|f| matches!(f.kind, ModelKind::Captioner | ModelKind::Upscaler))
        .filter(|f| {
            !f.component_id
                .as_deref()
                .is_some_and(|c| kept.iter().any(|x| x == c))
        })
        .filter(|f| match &group {
            Some(g) => f
                .component_id
                .as_deref()
                .is_some_and(|c| g.iter().any(|x| x == c)),
            None => {
                f.id == helper_id
                    && !f
                        .component_id
                        .as_deref()
                        .is_some_and(|c| all_groups.iter().any(|x| x == c))
            }
        })
        .collect()
}

/// The name of an installed model whose family uses one of `files` as a component.
fn model_using_helper_files(
    core: &AppCore,
    index: &pinhole_store::InstalledIndex,
    files: &[&InstalledFile],
) -> Option<String> {
    let registry = core.registry();
    index.models().find_map(|m| {
        let family = registry.family(m.family.as_deref()?)?;
        let used = families::family_component_ids(family);
        files
            .iter()
            .any(|f| f.component_id.as_deref().is_some_and(|c| used.contains(c)))
            .then(|| m.friendly_name.clone())
    })
}

/// Installed helpers: one row per Describe / Improve model (its two files together) and
/// upscalers / other captioner files.
pub fn list_helpers(core: &AppCore) -> CoreResult<Vec<InstalledHelper>> {
    let index = snapshot(core);
    let mut out = Vec::new();
    let mut listed: Vec<&str> = Vec::new();
    for h in core.registry().captioner().helpers.iter() {
        let mut files = helper_files(core, &index, &h.id);
        if files.is_empty() {
            continue;
        }
        // A vision file shared with another installed helper is kept on Remove, but its size
        // still shows once, on the first row that uses it.
        let (_, p) = crate::describe::helper_components(h);
        if let Some(shared) = index
            .files
            .iter()
            .find(|f| f.kind == ModelKind::Captioner && f.component_id.as_deref() == Some(&p))
        {
            if !listed.contains(&shared.id.as_str()) && !files.iter().any(|f| f.id == shared.id) {
                files.push(shared);
            }
        }
        listed.extend(files.iter().map(|f| f.id.as_str()));
        out.push(InstalledHelper {
            id: h.id.clone(),
            friendly_name: if h.default {
                "Describe model".into()
            } else {
                h.title.clone()
            },
            purpose: "describe".into(),
            size_bytes: files.iter().map(|f| f.size_bytes).sum(),
        });
    }
    let all_groups: Vec<String> = core
        .registry()
        .captioner()
        .helpers
        .iter()
        .flat_map(|h| {
            let (m, p) = crate::describe::helper_components(h);
            [m, p]
        })
        .collect();
    for f in index
        .files
        .iter()
        .filter(|f| matches!(f.kind, ModelKind::Captioner | ModelKind::Upscaler))
    {
        if listed.contains(&f.id.as_str())
            || f.component_id
                .as_deref()
                .is_some_and(|c| all_groups.iter().any(|x| x == c))
        {
            continue;
        }
        out.push(InstalledHelper {
            id: f.id.clone(),
            friendly_name: f.friendly_name.clone(),
            purpose: if f.kind == ModelKind::Upscaler {
                "upscale"
            } else {
                "describe"
            }
            .into(),
            size_bytes: f.size_bytes,
        });
    }
    Ok(out)
}

/// Delete a helper's files. Refused while it's in use; the engine that has it
/// open is stopped first (Windows can't delete an open file).
pub async fn delete_helper(core: &AppCore, helper_id: &str) -> CoreResult<()> {
    let _folder = folder_read(core)?;
    {
        let index = core.installed.lock();
        let files = helper_files(core, &index, helper_id);
        if files.is_empty() {
            return Err(CoreError::not_found(
                "That helper isn't installed any more.",
            ));
        }
        if files.iter().any(|f| f.is_linked()) {
            return Err(CoreError::invalid(LINKED_DELETE));
        }
        // A model can reuse a Describe helper's files (Qwen Image Edit reads the 7B model):
        // they stay while that model is installed.
        if let Some(user) = model_using_helper_files(core, &index, &files) {
            return Err(CoreError::invalid(format!(
                "{user} also uses this helper. Delete {user} first, then delete the helper."
            )));
        }
        // Files are deleted before the index is saved: make sure it can be.
        index.check_savable(&core.data)?;
    }
    if helper_group(core, helper_id).is_some() {
        if core.describe.is_busy() {
            return Err(CoreError::invalid(
                "Wait for the picture description to finish, then delete the Describe model.",
            ));
        }
        crate::describe::shutdown(core).await;
    } else {
        if core.gen.active.lock().is_some() {
            return Err(CoreError::invalid(
                "Wait for the current pictures to finish, then delete it.",
            ));
        }
        crate::generate::shutdown(core).await;
        crate::engine_setup::emit_status(core);
    }
    let failed;
    {
        let mut index = core.installed.lock();
        let ids: Vec<String> = helper_files(core, &index, helper_id)
            .into_iter()
            .map(|f| f.id.clone())
            .collect();
        index.check_savable(&core.data)?;
        failed = remove_entries(core, &mut index, &ids);
        index.save(&core.data)?;
    }
    core.emit(CoreEvent::ModelsChanged);
    if !failed.is_empty() {
        return Err(CoreError::new(
            "io",
            format!(
                "Couldn't delete {}. Close any program using it and try again.",
                failed.join(", ")
            ),
        ));
    }
    Ok(())
}

pub fn view_of(core: &AppCore, file: &InstalledFile) -> AddFileResult {
    let registry = core.registry();
    let hw = crate::app::hw_context(core);
    let index = snapshot(core);
    if file.kind == ModelKind::Lora {
        AddFileResult {
            model: None,
            lora: Some(inventory::installed_lora_view(&index, file)),
            needs_choice: None,
        }
    } else {
        AddFileResult {
            model: Some(inventory::installed_model_view(
                &registry, &index, file, &hw,
            )),
            lora: None,
            needs_choice: None,
        }
    }
}

// ------------------------------------------------------------------ recommended

pub fn get_recommended(core: &AppCore) -> CoreResult<Vec<RecommendedPick>> {
    let registry = core.registry();
    let hw = crate::app::hw_context(core);
    let index = snapshot(core);
    Ok(recommend::recommend(&registry, &index, &hw)
        .into_iter()
        .map(|p| p.pick)
        .collect())
}

/// One-click install of a role's pick (first run, empty Create/Edit/Describe).
pub async fn install_recommended(core: &Arc<AppCore>, role: &str) -> CoreResult<InstallStarted> {
    let (action, family_id) = {
        let registry = core.registry();
        let hw = crate::app::hw_context(core);
        let index = snapshot(core);
        let plan = recommend::recommend_role(&registry, &index, &hw, role).ok_or_else(|| {
            CoreError::not_found(format!("There is no recommended model for “{role}”."))
        })?;
        match plan.action {
            PickAction::Nothing if plan.pick.installed => {
                return Err(CoreError::invalid("This model is already installed."))
            }
            PickAction::Nothing => {
                return Err(CoreError::new(
                    "vram",
                    plan.pick
                        .unavailable_reason
                        .unwrap_or_else(|| "No recommended model fits this computer.".into()),
                ))
            }
            other => (other, plan.pick.family_id),
        }
    };
    match action {
        PickAction::Download { label, files } => {
            crate::licence::require_family(core, family_id.as_deref())?;
            let items = files.into_iter().map(|f| (f, None)).collect();
            start_install(core, label, items, None).await
        }
        PickAction::Civitai {
            version_id,
            family_id,
        } => crate::catalog::install_civitai(core, version_id, Some(family_id), None).await,
        PickAction::Captioner => crate::describe::install_captioner(core, None).await,
        PickAction::Nothing => Err(CoreError::internal("Nothing to install.")),
    }
}

/// Download the parts (VAE, text encoders) an installed model still needs,
/// from its family's registry list (models added from disk or another app's
/// folder, which have no CivitAI version to install from).
pub async fn install_missing_parts(
    core: &Arc<AppCore>,
    model_id: &str,
) -> CoreResult<InstallStarted> {
    let (label, files) = {
        let registry = core.registry();
        let hw = crate::app::hw_context(core);
        let index = snapshot_present(core);
        let model = index
            .get(model_id)
            .ok_or_else(|| CoreError::not_found("That model isn't installed any more."))?;
        let family = model
            .family
            .as_deref()
            .and_then(|f| registry.family(f))
            .ok_or_else(|| {
                CoreError::invalid("Pinhole doesn't know which parts this model needs.")
            })?;
        // A licensed family's parts download only after its licence was accepted.
        crate::licence::require_family(core, Some(&family.id))?;
        let files = recommend::parts_to_run(&registry, family, &hw, &index);
        (model.friendly_name.clone(), files)
    };
    if files.is_empty() {
        return Err(CoreError::invalid(
            "This model already has every part it needs.",
        ));
    }
    let items = files.into_iter().map(|f| (f, None)).collect();
    start_install(core, format!("Parts for {label}"), items, None).await
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
pub const INVALID_MODEL_FILE: &str =
    "The downloaded file isn't a valid model file, so Pinhole removed it.";

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

/// Does this download get [`model_file_check`]? CivitAI files (the hash only
/// proves what the uploader sent) and any file without a pinned hash (model
/// or component, e.g. `sha256: TODO` in the registry) must parse as
/// safetensors/GGUF.
fn needs_content_check(f: &FileToGet, from_civitai: bool) -> bool {
    from_civitai || f.sha256.is_none()
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
    let (group_id, planned) = {
        let mut inflight = core.models.inflight.lock();
        if let Some((main, _)) = items.iter().find(|(f, _)| f.component_id.is_none()) {
            if let Some(group) = inflight.get(&inflight_key(main)) {
                return Ok(InstallStarted {
                    group_id: group.clone(),
                });
            }
        }
        let items: Vec<(FileToGet, Option<CivitaiRef>)> = items
            .into_iter()
            .filter(|(f, _)| !inflight.contains_key(&inflight_key(f)))
            .collect();
        if items.is_empty() {
            return Err(CoreError::invalid("This is already downloading."));
        }
        let total: u64 = items.iter().map(|(f, _)| f.size_bytes).sum();
        let models_root =
            models_dir_for_write(core, ModelKind::Checkpoint).map(|_| core.data.models_root())?;
        check_free_space(&models_root, total).map_err(|e| disk_space_error(e, &items[0].0.url))?;

        let mut specs = Vec::new();
        let mut planned: Vec<(PathBuf, String, Registration)> = Vec::new();
        let mut dests = core.models.inflight_dests.lock();
        // Read the index while holding `dests`: a finishing install registers its
        // file before releasing its name, so every name is in one or the other.
        let index = snapshot(core);
        // The files are registered once downloaded: say now if that can't work
        // (e.g. installed.json is from a newer Pinhole), not after the download.
        index.check_savable(&core.data)?;
        // "Add a file" copies waiting for a family choice (on disk, not indexed yet).
        let pending: Vec<PathBuf> = core
            .models
            .pending
            .lock()
            .values()
            .map(|pa| pa.path.clone())
            .collect();
        for (f, civitai) in items {
            let dir = models_dir_for_write(core, f.kind)?;
            // Never overwrite a registered file, one planned in this group, one
            // another running install is downloading to or a pending added file.
            let dest = local::unique_path(&dir, &f.file_name, |p| {
                planned.iter().any(|(d, _, _)| d == p)
                    || dests.contains(p)
                    || pending.iter().any(|d| d == p)
                    || (p.exists()
                        && core
                            .data
                            .relative(p)
                            .is_some_and(|rel| index.has_rel_path(&rel)))
            });
            let headers: Vec<(String, String)> =
                pinhole_catalog::api::civitai_auth_header(api_key.as_deref(), &f.url)
                    .into_iter()
                    .collect();
            let check = needs_content_check(&f, civitai.is_some());
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
                lookup: None,
            };
            planned.push((dest, inflight_key(&f), reg));
        }
        let group_id =
            core.downloads
                .enqueue_kind(label, pinhole_net::download::DownloadKind::Model, specs);
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
        // Failures are reported through the group's `download-progress` status,
        // registration failures too (the group is marked failed after all).
        let planned_dests: Vec<PathBuf> = planned.iter().map(|(d, _, _)| d.clone()).collect();
        let mut register_error: Option<CoreError> = None;
        if let Ok(files) = result {
            let same_len = files.len() == planned.len();
            for (i, (dest, _, reg)) in planned.into_iter().enumerate() {
                let file = files
                    .iter()
                    .find(|f| f.path == dest)
                    .or_else(|| files.get(i).filter(|_| same_len));
                if let Some(file) = file.cloned() {
                    let registry = task_core.registry();
                    let path = file.path.clone();
                    let fallback = reg.clone();
                    let reg = tokio::task::spawn_blocking(move || {
                        refine_main_registration(&registry, &path, reg)
                    })
                    .await
                    .unwrap_or(fallback);
                    if let Err(e) = register_download(&task_core, &file, reg) {
                        register_error.get_or_insert(e);
                    }
                }
            }
        }
        if let Some(e) = register_error {
            task_core
                .downloads
                .fail_done(&task_group, &e.code, &e.message);
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
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.replace(['_', '-'], " ").trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Model".into())
}

fn remove_copy(p: &PendingAdd) {
    if p.copied {
        let _ = std::fs::remove_file(&p.path);
    }
}

fn register_pending(
    core: &AppCore,
    p: &PendingAdd,
    family: Option<String>,
) -> CoreResult<AddFileResult> {
    let file = DownloadedFile {
        path: p.path.clone(),
        sha256: p.sha256.clone(),
        size_bytes: p.size_bytes,
    };
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
            lookup: p.lookup,
        },
    )?;
    Ok(view_of(core, &entry))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> CoreResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| CoreError::internal("A background task stopped unexpectedly. Try again."))
}

/// A name in [`ModelsState::inflight_dests`], released on drop.
struct ReservedDest<'a>(&'a AppCore, PathBuf);

impl Drop for ReservedDest<'_> {
    fn drop(&mut self) {
        self.0.models.inflight_dests.lock().remove(&self.1);
    }
}

/// "Add a file I already have": check the extension, read the header, copy the
/// file into `Data/models/<kind>/` while hashing it (the user's file is never
/// moved or changed), look it up on CivitAI by hash (every file Pinhole doesn't offer
/// itself, RELEASE-SPEC §5: a real person or minor is refused; no match, Offline mode or a
/// failed lookup leave it "safe images only"), then resolve the family: known hash →
/// CivitAI's base model → header sniffing → ask (`needsChoice`).
pub async fn add_local_model(core: &Arc<AppCore>, path: &str) -> CoreResult<AddFileResult> {
    let _folder = folder_read(core)?;
    let src = PathBuf::from(path.trim());
    let ext = local::allowed_extension(&src).ok_or_else(|| {
        CoreError::invalid("Only .safetensors and .gguf files can be added. Older .ckpt/.pt files can hide harmful code.")
    })?;
    if !src.is_file() {
        return Err(CoreError::not_found(
            "Pinhole can't find that file. Check that it still exists and try again.",
        ));
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
    .map_err(|e| {
        CoreError::invalid("This file isn't a model Pinhole can read.").with_details(e.to_string())
    })?;
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
    let data_root = core
        .data
        .root
        .canonicalize()
        .unwrap_or_else(|_| core.data.root.clone());
    let models_root = core
        .data
        .models_root()
        .canonicalize()
        .unwrap_or_else(|_| core.data.models_root());
    let src_canon = src.canonicalize().unwrap_or_else(|_| src.clone());
    // With a picked Models folder only files already in it count as in place;
    // anything else (even a leftover in Data/models) is copied into it.
    let in_place = src_canon.starts_with(&models_root)
        || (core.data.models_home.is_none() && src_canon.starts_with(&data_root));
    let (dest, sha256, size_bytes, _reserved) = if in_place {
        let p = src_canon.clone();
        let (sha, size) = blocking(move || local::hash_file(&p)).await??;
        (src_canon, sha, size, None)
    } else {
        let dir = core.data.models_dir_for_write(kind)?; // folder lock held above
        check_free_space(&dir, header.file_size).map_err(|e| disk_space_error(e, ""))?;
        let name = local::sanitize_file_name(
            src.file_name().and_then(|n| n.to_str()).unwrap_or("model"),
            ext,
        );
        // Not a name a download is writing to (only its `.part` exists yet),
        // and reserved like one until this add is registered or pending, so a
        // download starting meanwhile doesn't pick it either.
        let dest = {
            let mut dests = core.models.inflight_dests.lock();
            let dest = local::unique_path(&dir, &name, |p| {
                p.exists() || local::part_path(p).exists() || dests.contains(p)
            });
            dests.insert(dest.clone());
            dest
        };
        let reserved = ReservedDest(core, dest.clone());
        let (from, to) = (src.clone(), dest.clone());
        let (sha, size) = blocking(move || local::copy_and_hash(&from, &to))
            .await?
            .map_err(|e| {
                CoreError::new("io", "Pinhole couldn't copy the file into its Data folder.")
                    .with_details(e.to_string())
            })?;
        (dest, sha, size, Some(reserved))
    };
    let mut pending = PendingAdd {
        path: dest,
        copied: !in_place,
        sha256: sha256.clone(),
        size_bytes,
        kind,
        file_name: src
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string(),
        friendly_name: plain_name(&src),
        dtype: Some(detection.dtype.clone()).filter(|d| !d.is_empty()),
        civitai: None,
        lookup: None,
        candidates: Vec::new(),
        created: std::time::Instant::now(),
    };

    // Already installed (same bytes)? Keep the existing entry; adding it again retries a
    // lookup that couldn't run before.
    if let Some(existing) = snapshot(core).find_by_sha(&sha256).cloned() {
        if pending.copied
            && core
                .data
                .relative(&pending.path)
                .is_some_and(|rel| rel != existing.rel_path)
        {
            remove_copy(&pending);
        }
        if existing.lookup == Some(Lookup::NotYet) {
            let outcome = crate::lookup::look_up_one(core, &existing.id, &existing.sha256).await;
            if outcome == Outcome::PersonOrMinor {
                return Err(CoreError::invalid(
                    pinhole_catalog::api::PERSON_OR_MINOR_REASON,
                ));
            }
        }
        let existing = snapshot(core).get(&existing.id).cloned().ok_or_else(|| {
            CoreError::not_found("That file was just removed from Pinhole. Add it again.")
        })?;
        crate::lookup::refuse_if_flagged(&existing)?;
        return Ok(view_of(core, &existing));
    }

    // 1. known hash
    let mut resolution = families::resolve_family(&registry, Some(&sha256), None, None);
    if let Some(k) = registry.known_file(&sha256) {
        if let Some(name) = k.friendly_name.clone() {
            pending.friendly_name = name;
        }
    }
    // 2. CivitAI by hash, whether or not the family is known (RELEASE-SPEC §5). A file
    // Pinhole offers itself is looked up too, but never counts as "safe images only".
    pending.lookup = crate::lookup::initial(&registry, kind, &sha256);
    if crate::lookup::looked_up_kind(kind) {
        let outcome = crate::lookup::look_up(core, &sha256).await;
        if pending.lookup.is_some() {
            pending.lookup = Some(outcome.lookup());
        }
        match outcome {
            Outcome::PersonOrMinor => {
                remove_copy(&pending);
                return Err(CoreError::invalid(
                    pinhole_catalog::api::PERSON_OR_MINOR_REASON,
                ));
            }
            Outcome::Found {
                civitai,
                friendly_name,
            } => {
                pending.friendly_name = friendly_name;
                if !matches!(resolution, FamilyResolution::Resolved(_)) {
                    resolution = families::resolve_family(
                        &registry,
                        Some(&sha256),
                        civitai.base_model.as_deref(),
                        Some(&detection.candidates),
                    );
                }
                pending.civitai = Some(*civitai);
            }
            Outcome::NoMatch | Outcome::Failed => {}
        }
    }
    // Added meanwhile (the lookup takes a moment): keep the entry that is there.
    if let Some(existing) = snapshot(core).find_by_sha(&sha256).cloned() {
        if core
            .data
            .relative(&pending.path)
            .is_some_and(|rel| rel != existing.rel_path)
        {
            remove_copy(&pending);
        }
        crate::lookup::refuse_if_flagged(&existing)?;
        return Ok(view_of(core, &existing));
    }
    // 3. header sniffing
    if matches!(resolution, FamilyResolution::Unsupported(None)) {
        resolution = families::resolve_family(&registry, None, None, Some(&detection.candidates));
    }
    match resolution {
        FamilyResolution::Resolved(family) => {
            register_pending(core, &pending, Some(family)).inspect_err(|_| remove_copy(&pending))
        }
        FamilyResolution::Ambiguous(candidates) => {
            // 4. ask
            let token = uuid::Uuid::new_v4().to_string();
            let choices: Vec<FamilyChoice> = candidates
                .iter()
                .filter_map(|id| families::family_choice(&registry, id))
                .collect();
            pending.candidates = candidates;
            let file_name = pending.file_name.clone();
            let replaced = core.models.pending.lock().insert(token.clone(), pending);
            if let Some(old) = replaced {
                remove_copy(&old);
            }
            Ok(AddFileResult {
                model: None,
                lora: None,
                needs_choice: Some(NeedsChoice {
                    token,
                    file_name,
                    candidates: choices,
                }),
            })
        }
        // A LoRA of unknown base still works; it just can't be checked for compatibility.
        FamilyResolution::Unsupported(_) if kind == ModelKind::Lora => {
            register_pending(core, &pending, None)
        }
        FamilyResolution::Unsupported(base) => {
            remove_copy(&pending);
            Err(CoreError::invalid(families::unsupported_message(
                base.as_deref(),
            )))
        }
    }
}

/// Finish an ambiguous "Add a file" with the family the user picked.
pub fn confirm_family(core: &AppCore, token: &str, family_id: &str) -> CoreResult<AddFileResult> {
    let registry = core.registry();
    if registry.family(family_id).is_none() {
        return Err(CoreError::invalid(
            "That model type isn't known to Pinhole. Pick one from the list.",
        ));
    }
    purge_expired_pending(core);
    let pending = core
        .models
        .pending
        .lock()
        .remove(token)
        .ok_or_else(|| CoreError::not_found("This choice has expired. Add the file again."))?;
    register_pending(core, &pending, Some(family_id.to_string()))
        .inspect_err(|_| remove_copy(&pending))
}

/// The family question was closed without a pick: the copy made for it is removed.
pub fn cancel_add(core: &AppCore, token: &str) {
    let pending = core.models.pending.lock().remove(token);
    if let Some(p) = pending {
        remove_copy(&p);
    }
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
        // Files are deleted before the index is saved: make sure it can be.
        index.check_savable(&core.data)?;
        ids.iter()
            .filter_map(|id| index.get(id))
            .map(|f| index.abs_path(&core.data, f))
            .collect()
    };
    // Stopping the engine under a job would fail it as if the engine crashed.
    if crate::generate::unload_interrupts_job(core, model_id, &paths).await {
        return Err(CoreError::invalid(
            "Wait for the current pictures to finish, then delete it.",
        ));
    }
    crate::generate::unload_model(core, model_id, &paths).await;
    let failed;
    {
        let mut index = core.installed.lock();
        let ids = delete_ids(&registry, &index, model_id)?;
        index.check_savable(&core.data)?;
        failed = remove_entries(core, &mut index, &ids);
        index.save(&core.data)?;
    }
    core.emit(CoreEvent::ModelsChanged);
    if !failed.is_empty() {
        return Err(CoreError::new(
            "io",
            format!(
                "Couldn't delete {}. Close any program using it and try again.",
                failed.join(", ")
            ),
        ));
    }
    Ok(())
}

/// Pinhole never deletes files in another app's models folder.
pub(crate) const LINKED_DELETE: &str = "This model is in another app's models folder, so Pinhole doesn't delete it. Delete it in that app, or remove the folder from Pinhole's list.";

/// Ids of the entries deleting `model_id` removes: the model and the
/// components no other model needs (by id: two entries can share a path).
fn delete_ids(
    registry: &pinhole_registry::Registry,
    index: &pinhole_store::InstalledIndex,
    model_id: &str,
) -> CoreResult<Vec<String>> {
    let Some(target) = index.get(model_id) else {
        return Err(CoreError::not_found("That model isn't installed any more."));
    };
    if target.is_linked() {
        return Err(CoreError::invalid(LINKED_DELETE));
    }
    let orphans = inventory::orphaned_components(registry, index, model_id);
    Ok(std::iter::once(model_id.to_string())
        .chain(orphans.into_iter().map(|f| f.id.clone()))
        .collect())
}

/// Delete the files of these index entries (and their `.part` leftovers) and
/// drop the entries; returns the names of files that couldn't be deleted
/// (their entries stay). A file another remaining entry also points at is
/// kept. The caller saves the index.
fn remove_entries(
    core: &AppCore,
    index: &mut pinhole_store::InstalledIndex,
    ids: &[String],
) -> Vec<String> {
    let mut failed = Vec::new();
    for id in ids {
        let Some(f) = index.get(id).cloned() else {
            continue;
        };
        let same = |rel: &str| normalize_rel(rel) == normalize_rel(&f.rel_path);
        let shared = index
            .files
            .iter()
            .any(|o| o.id != f.id && !ids.contains(&o.id) && same(&o.rel_path))
            || index.unknown_uses(&f.rel_path);
        // Never delete a file in the user's other models folders.
        if inventory::is_safe_rel_path(&f.rel_path) && !shared && !f.is_linked() {
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
pub async fn resolve_civitai_resources(
    core: &AppCore,
    resources: Vec<PastedResource>,
) -> CoreResult<ResolvedResources> {
    let registry = core.registry();
    let hw = crate::app::hw_context(core);
    let index = snapshot(core);
    let filters = crate::catalog::filters(core)?;
    let env = paste::PasteEnv {
        registry: &registry,
        index: &index,
        hw: &hw,
        filters: &filters,
    };
    let client = if core.offline.get() {
        None
    } else {
        Some(crate::catalog::civitai_client(core).await)
    };
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
        rec.0
            .lock()
            .iter()
            .filter(|e| **e == "models-changed")
            .count()
    }

    #[test]
    fn register_download_round_trip() {
        let rec = Arc::new(Recorder::default());
        let (_t, core) = test_core(rec.clone());
        let dir = core.data.models(ModelKind::Checkpoint);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("m.safetensors");
        std::fs::write(&path, b"abc").unwrap();
        let file = DownloadedFile {
            path: path.clone(),
            sha256: "AB".repeat(32),
            size_bytes: 3,
        };
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
                creator_notes: None,
                sfw_only: false,
            }),
            dtype: Some("f16".into()),
            lookup: None,
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
        assert!(
            !text.contains(&core.data.root.to_string_lossy().to_string()),
            "paths are Data-relative"
        );

        // Same path again → same id (no duplicates), the user's trigger words kept.
        core.installed.lock().files[0].trigger_words = Some(vec!["mine".into()]);
        let again = register_download(&core, &file, reg).unwrap();
        assert_eq!(again.id, entry.id);
        assert_eq!(again.trigger_words, Some(vec!["mine".into()]));
        assert_eq!(core.installed.lock().files.len(), 1);

        // Listed as an installed model.
        let models = list_models(&core).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].family_label.as_deref(), Some("SDXL"));
        assert_eq!(models[0].civitai_version_id, Some(2));
        assert!(
            !models[0].missing_components.is_empty(),
            "the SDXL fp16-fix VAE isn't installed"
        );

        // Files outside Data are refused.
        let outside = DownloadedFile {
            path: _t.path().join("elsewhere.safetensors"),
            sha256: "cd".repeat(32),
            size_bytes: 1,
        };
        let reg = Registration {
            kind: ModelKind::Lora,
            friendly_name: "x".into(),
            family: None,
            component_id: None,
            civitai: None,
            dtype: None,
            lookup: None,
        };
        assert_eq!(
            register_download(&core, &outside, reg).unwrap_err().code,
            "invalid"
        );
    }

    #[tokio::test]
    async fn add_local_file_copies_hashes_and_asks_or_resolves() {
        let rec = Arc::new(Recorder::default());
        let (tmp, core) = test_core(rec.clone());
        core.offline.set(true); // no CivitAI lookup in tests

        // Wrong extension.
        let bad = tmp.path().join("m.ckpt");
        std::fs::write(&bad, b"x").unwrap();
        assert_eq!(
            add_local_model(&core, bad.to_str().unwrap())
                .await
                .unwrap_err()
                .code,
            "invalid"
        );

        // SDXL-shaped header → a family is resolved or the user is asked.
        let src = tmp.path().join("My Model (v2).safetensors");
        safetensors(&src, SDXL_TENSORS);
        let before = std::fs::read(&src).unwrap();
        let out = add_local_model(&core, src.to_str().unwrap()).await.unwrap();
        assert_eq!(
            std::fs::read(&src).unwrap(),
            before,
            "the user's file is untouched"
        );
        let model = match out.needs_choice {
            Some(choice) => {
                assert_eq!(choice.file_name, "My Model (v2).safetensors");
                assert!(
                    choice.candidates.iter().any(|c| c.family_id == "sdxl"),
                    "{:?}",
                    choice.candidates
                );
                assert!(
                    core.installed.lock().files.is_empty(),
                    "nothing registered until the user picks"
                );
                assert_eq!(
                    confirm_family(&core, "bogus", "sdxl").unwrap_err().code,
                    "not_found"
                );
                // Closing the question removes the copy made for it.
                let other = tmp.path().join("Other.safetensors");
                std::fs::copy(&src, &other).unwrap();
                let out2 = add_local_model(&core, other.to_str().unwrap())
                    .await
                    .unwrap();
                let copy = core
                    .data
                    .models(ModelKind::Checkpoint)
                    .join("Other.safetensors");
                assert!(copy.is_file());
                cancel_add(&core, &out2.needs_choice.unwrap().token);
                assert!(!copy.exists() && other.is_file());
                assert_eq!(
                    confirm_family(&core, &choice.token, "nope")
                        .unwrap_err()
                        .code,
                    "invalid"
                );
                let done = confirm_family(&core, &choice.token, "sdxl_pony").unwrap();
                assert_eq!(
                    confirm_family(&core, &choice.token, "sdxl")
                        .unwrap_err()
                        .code,
                    "not_found",
                    "token is single-use"
                );
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
        let copies = std::fs::read_dir(core.data.models(ModelKind::Checkpoint))
            .unwrap()
            .count();
        assert_eq!(copies, 1);

        // Unknown tensors → refused, copy cleaned up.
        let odd = tmp.path().join("odd.safetensors");
        safetensors(&odd, &["some.random.tensor"]);
        let err = add_local_model(&core, odd.to_str().unwrap())
            .await
            .unwrap_err();
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
            register_download(
                &core,
                &DownloadedFile {
                    path: p,
                    sha256: sha,
                    size_bytes: name.len() as u64,
                },
                reg,
            )
            .unwrap()
        };
        let comp = |id: &str| Registration {
            kind: ModelKind::TextEncoder,
            friendly_name: id.into(),
            family: None,
            component_id: Some(id.into()),
            civitai: None,
            dtype: None,
            lookup: None,
        };
        let main = |fam: &str| Registration {
            kind: ModelKind::Diffusion,
            friendly_name: fam.into(),
            family: Some(fam.into()),
            component_id: None,
            civitai: None,
            dtype: None,
            lookup: None,
        };
        let zit = put(ModelKind::Diffusion, "zit.gguf", main("z_image_turbo"));
        let kontext = put(ModelKind::Diffusion, "kontext.gguf", main("flux1_kontext"));
        let ae = put(
            ModelKind::Vae,
            "ae.safetensors",
            Registration {
                kind: ModelKind::Vae,
                ..comp("flux_ae")
            },
        );
        let qwen3 = put(
            ModelKind::TextEncoder,
            "qwen_3_4b.safetensors",
            comp("qwen3_4b"),
        );

        let preview = preview_delete(&core, &zit.id).unwrap();
        let paths: Vec<&str> = preview.files.iter().map(|f| f.rel_path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "models/diffusion/zit.gguf",
                "models/text_encoders/qwen_3_4b.safetensors"
            ]
        );

        delete_model(&core, &zit.id).await.unwrap();
        let idx = core.installed.lock().clone();
        assert!(idx.get(&zit.id).is_none() && idx.get(&qwen3.id).is_none());
        assert!(
            idx.get(&kontext.id).is_some() && idx.get(&ae.id).is_some(),
            "shared VAE stays"
        );
        assert!(!core.data.root.join(&zit.rel_path).exists());
        assert!(!core.data.root.join(&qwen3.rel_path).exists());
        assert!(core.data.root.join(&ae.rel_path).exists());
        assert_eq!(
            pinhole_store::InstalledIndex::load(&core.data)
                .unwrap()
                .files
                .len(),
            2
        );
        assert_eq!(
            preview_delete(&core, &zit.id).unwrap_err().code,
            "not_found"
        );
        assert_eq!(
            delete_model(&core, &zit.id).await.unwrap_err().code,
            "not_found"
        );

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
        let reg = Registration {
            kind: ModelKind::Checkpoint,
            friendly_name: "a".into(),
            family: Some("sd15".into()),
            component_id: None,
            civitai: None,
            dtype: None,
            lookup: None,
        };
        let theirs = register_download(
            &core,
            &DownloadedFile {
                path: p.clone(),
                sha256: "ab".repeat(32),
                size_bytes: 6,
            },
            reg,
        )
        .unwrap();
        // An older index could hold a second (missing) entry with the same path.
        let mut lost = theirs.clone();
        lost.id = "lost".into();
        lost.sha256 = "cd".repeat(32);
        core.installed.lock().files.insert(0, lost);

        delete_model(&core, "lost").await.unwrap();
        let idx = core.installed.lock().clone();
        assert!(idx.get("lost").is_none(), "the entry asked for is removed");
        assert!(idx.get(&theirs.id).is_some(), "the other entry stays");
        assert_eq!(
            std::fs::read(&p).unwrap(),
            b"theirs",
            "and so does its file"
        );
    }

    #[tokio::test]
    async fn delete_keeps_files_when_the_index_cant_be_saved_or_is_shared() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        let dir = core.data.models(ModelKind::Checkpoint);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.safetensors");
        std::fs::write(&p, b"model").unwrap();
        let reg = Registration {
            kind: ModelKind::Checkpoint,
            friendly_name: "a".into(),
            family: Some("sd15".into()),
            component_id: None,
            civitai: None,
            dtype: None,
            lookup: None,
        };
        let a = register_download(
            &core,
            &DownloadedFile {
                path: p.clone(),
                sha256: "ab".repeat(32),
                size_bytes: 5,
            },
            reg,
        )
        .unwrap();

        // Index from a newer Pinhole: refused before any file is touched.
        core.installed.lock().schema_version = pinhole_store::installed::SCHEMA_VERSION + 1;
        let e = delete_model(&core, &a.id).await.unwrap_err();
        assert!(e.message.contains("newer version"), "{}", e.message);
        assert!(p.exists() && core.installed.lock().get(&a.id).is_some());
        core.installed.lock().schema_version = pinhole_store::installed::SCHEMA_VERSION;

        // An entry this version can't read uses the same file: the file stays.
        // Written on Windows, by hand: the same file as a's.
        let same_file = format!(".\\{}", a.rel_path.replace('/', "\\"));
        core.installed
            .lock()
            .unknown
            .push(serde_json::json!({ "relPath": same_file, "kind": "future" }));
        delete_model(&core, &a.id).await.unwrap();
        assert!(core.installed.lock().get(&a.id).is_none());
        assert!(p.exists());

        // Registering a new file at that path replaces the unreadable entry.
        let reg = Registration {
            kind: ModelKind::Checkpoint,
            friendly_name: "b".into(),
            family: Some("sd15".into()),
            component_id: None,
            civitai: None,
            dtype: None,
            lookup: None,
        };
        register_download(
            &core,
            &DownloadedFile {
                path: p.clone(),
                sha256: "cd".repeat(32),
                size_bytes: 5,
            },
            reg,
        )
        .unwrap();
        assert!(core.installed.lock().unknown.is_empty());
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
            assert_eq!(
                check(bad),
                Err(INVALID_MODEL_FILE.to_string()),
                "{}",
                bad.display()
            );
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
            lookup: None,
        };
        let out = refine_main_registration(&core.registry(), &p, reg.clone());
        assert_eq!(
            out.kind,
            ModelKind::Checkpoint,
            "SDXL files with text encoders are all-in-one"
        );
        assert!(out.dtype.is_some());
        // Components and unreadable files are left alone.
        let comp = Registration {
            component_id: Some("flux_ae".into()),
            ..reg.clone()
        };
        assert_eq!(
            refine_main_registration(&core.registry(), &p, comp).kind,
            ModelKind::Diffusion
        );
        assert_eq!(
            refine_main_registration(&core.registry(), &tmp.path().join("missing"), reg).kind,
            ModelKind::Diffusion
        );
    }

    #[test]
    fn recommended_through_core() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        let picks = get_recommended(&core).unwrap();
        assert_eq!(
            picks.iter().map(|p| p.role.as_str()).collect::<Vec<_>>(),
            ["realistic", "anime", "edit", "describe"]
        );
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
        assert_eq!(
            install_recommended(&core, "nope").await.unwrap_err().code,
            "not_found"
        );
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
        *core.registry.write() =
            Arc::new(pinhole_registry::Registry::from_yaml(&yaml, None).unwrap());
        {
            let mut s = core.settings.write();
            s.vram_override_gb = Some(8.0);
            s.gpu = "auto".into();
            // The VRAM override only counts with a GPU backend (no hardware detected here).
            s.engine_backend = "cuda".into();
        }
        let started = install_recommended(&core, "realistic").await.unwrap();
        let status = core.downloads.status();
        let g = status
            .iter()
            .find(|g| g.group_id == started.group_id)
            .unwrap();
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

    #[test]
    fn files_without_a_pinned_hash_get_the_header_check() {
        let model = lora_file("https://huggingface.co/x/m.safetensors", "m.safetensors");
        let component = FileToGet {
            component_id: Some("clip_l".into()),
            ..model.clone()
        };
        // No hash (registry `sha256: TODO`): checked, model or component.
        assert!(needs_content_check(&model, false));
        assert!(needs_content_check(&component, false));
        // Pinned hash: the hash is enough; CivitAI files are always checked.
        let pinned = FileToGet {
            sha256: Some("ab".repeat(32)),
            ..component
        };
        assert!(!needs_content_check(&pinned, false));
        assert!(needs_content_check(&pinned, true));
    }

    #[tokio::test]
    async fn queued_installs_with_the_same_file_name_get_different_paths() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        core.offline.set(true); // downloads fail fast, nothing leaves the machine
        let a = lora_file(
            "https://civitai.com/api/download/models/1",
            "style.safetensors",
        );
        let b = lora_file(
            "https://civitai.com/api/download/models/2",
            "style.safetensors",
        );
        let ga = start_install(&core, "A".into(), vec![(a, None)], None)
            .await
            .unwrap();
        let gb = start_install(&core, "B".into(), vec![(b, None)], None)
            .await
            .unwrap();
        assert_ne!(ga.group_id, gb.group_id);
        let dir = core.data.models(ModelKind::Lora);
        let dests = core.models.inflight_dests.lock().clone();
        assert_eq!(
            dests,
            HashSet::from([
                dir.join("style.safetensors"),
                dir.join("style-2.safetensors")
            ])
        );
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
    async fn a_cancelled_install_keeps_its_name_to_resume() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        core.offline.set(true);
        let dir = core.data.models(ModelKind::Lora);
        std::fs::create_dir_all(&dir).unwrap();
        // Left by a cancelled download: the next install resumes it (a pinned
        // hash restarts from byte 0 if the bytes turn out to be wrong).
        std::fs::write(
            local::part_path(&dir.join("style.safetensors")),
            b"first bytes",
        )
        .unwrap();
        let a = FileToGet {
            sha256: Some("ab".repeat(32)),
            ..lora_file(
                "https://civitai.com/api/download/models/1",
                "style.safetensors",
            )
        };
        let ga = start_install(&core, "A".into(), vec![(a, None)], None)
            .await
            .unwrap();
        assert!(core
            .models
            .inflight_dests
            .lock()
            .contains(&dir.join("style.safetensors")));
        let _ = core.downloads.wait(&ga.group_id).await;
    }

    /// An install whose files couldn't be registered afterwards is refused
    /// before anything downloads (not a finished download nothing shows).
    #[tokio::test]
    async fn install_is_refused_when_the_index_cant_be_saved() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        core.offline.set(true);
        core.installed.lock().schema_version = pinhole_store::installed::SCHEMA_VERSION + 1;
        let a = lora_file(
            "https://civitai.com/api/download/models/1",
            "style.safetensors",
        );
        let e = start_install(&core, "A".into(), vec![(a, None)], None)
            .await
            .unwrap_err();
        assert!(e.message.contains("newer version"), "{}", e.message);
        assert!(core.downloads.status().is_empty(), "nothing queued");
        assert!(core.models.inflight_dests.lock().is_empty());
        assert!(core.models.inflight.lock().is_empty());
    }

    /// "Add a file" never picks the name a download is writing to (only its
    /// `.part` exists yet), and a download never picks the name of an added
    /// file waiting for a family choice.
    #[tokio::test]
    async fn add_a_file_and_downloads_never_share_a_name() {
        let (tmp, core) = test_core(Arc::new(Recorder::default()));
        core.offline.set(true); // no CivitAI lookup, downloads fail fast
        let dir = core.data.models(ModelKind::Checkpoint);
        std::fs::create_dir_all(&dir).unwrap();
        // A queued download of Clash.safetensors, a running one of Clash-2.
        core.models
            .inflight_dests
            .lock()
            .insert(dir.join("Clash.safetensors"));
        std::fs::write(local::part_path(&dir.join("Clash-2.safetensors")), b"bytes").unwrap();
        let src = tmp.path().join("Clash.safetensors");
        safetensors(&src, SDXL_TENSORS);
        let out = add_local_model(&core, src.to_str().unwrap()).await.unwrap();
        assert!(out.needs_choice.is_some(), "SDXL-shaped: waits for a pick");
        let added = core
            .models
            .pending
            .lock()
            .values()
            .next()
            .unwrap()
            .path
            .clone();
        assert_eq!(added, dir.join("Clash-3.safetensors"));
        assert_eq!(
            std::fs::read(local::part_path(&dir.join("Clash-2.safetensors"))).unwrap(),
            b"bytes",
            "the running download's .part is untouched"
        );
        assert_eq!(
            *core.models.inflight_dests.lock(),
            HashSet::from([dir.join("Clash.safetensors")]),
            "the add's own reservation is released"
        );
        // A new download of the same name skips the added file (pending or registered).
        core.models.inflight_dests.lock().clear();
        let f = FileToGet {
            kind: ModelKind::Checkpoint,
            ..lora_file(
                "https://civitai.com/api/download/models/3",
                "Clash-3.safetensors",
            )
        };
        let g = start_install(&core, "C".into(), vec![(f, None)], None)
            .await
            .unwrap();
        assert_eq!(
            *core.models.inflight_dests.lock(),
            HashSet::from([dir.join("Clash-3-2.safetensors")])
        );
        let _ = core.downloads.wait(&g.group_id).await;
    }

    #[tokio::test]
    async fn paste_offline() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        core.offline.set(true);
        let out = resolve_civitai_resources(
            &core,
            vec![PastedResource {
                kind: "checkpoint".into(),
                model_version_id: Some(1759168),
                ..Default::default()
            }],
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
        for (name, comp) in [
            ("m.gguf", crate::describe::DEFAULT_MODEL_ID),
            ("p.gguf", crate::describe::DEFAULT_MMPROJ_ID),
        ] {
            std::fs::write(dir.join(name), b"1234").unwrap();
            let file = DownloadedFile {
                path: dir.join(name),
                sha256: "ab".repeat(32),
                size_bytes: 4,
            };
            let reg = Registration {
                kind: ModelKind::Captioner,
                friendly_name: "Describe model".into(),
                family: None,
                component_id: Some(comp.into()),
                civitai: None,
                dtype: None,
                lookup: None,
            };
            register_download(&core, &file, reg).unwrap();
        }
        // Not a main model, but shown as one helper row with both files' size.
        assert!(list_models(&core).unwrap().is_empty());
        let helpers = list_helpers(&core).unwrap();
        assert_eq!(helpers.len(), 1);
        assert_eq!(
            (
                helpers[0].id.as_str(),
                helpers[0].purpose.as_str(),
                helpers[0].size_bytes
            ),
            (DESCRIBE_HELPER_ID, "describe", 8)
        );

        delete_helper(&core, DESCRIBE_HELPER_ID).await.unwrap();
        assert!(list_helpers(&core).unwrap().is_empty());
        assert!(!dir.join("m.gguf").exists() && !dir.join("p.gguf").exists());
        assert!(delete_helper(&core, DESCRIBE_HELPER_ID).await.is_err());
    }

    #[tokio::test]
    async fn a_picked_helper_model_is_used_only_while_installed() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        let models = crate::describe::list_helper_models(&core);
        assert_eq!(
            models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["describe", "qwen25_vl_7b"],
            "the Safe-mode-Off helper is hidden while Safe mode is On"
        );
        assert!(models.iter().all(|m| !m.installed && m.download_bytes > 0));
        let seven = &models[1];
        assert!(seven.size_bytes > 8_000_000_000, "{seven:?}");

        let dir = core.data.models(ModelKind::Captioner);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, comp) in [
            ("m7.gguf", "qwen25_vl_7b_q8"),
            ("p7.gguf", "qwen25_vl_7b_mmproj"),
        ] {
            std::fs::write(dir.join(name), b"1234").unwrap();
            let file = DownloadedFile {
                path: dir.join(name),
                sha256: "cd".repeat(32),
                size_bytes: 4,
            };
            let reg = Registration {
                kind: ModelKind::Captioner,
                friendly_name: "Qwen2.5-VL 7B".into(),
                family: None,
                component_id: Some(comp.into()),
                civitai: None,
                dtype: None,
                lookup: None,
            };
            register_download(&core, &file, reg).unwrap();
        }
        let models = crate::describe::list_helper_models(&core);
        assert!(models[1].installed && models[1].removable && models[1].download_bytes == 0);
        assert!(!models[0].installed);
        // One row for the two files, with the registry's name.
        let rows = list_helpers(&core).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "qwen25_vl_7b");
        assert_eq!(rows[0].friendly_name, "Qwen2.5-VL 7B");

        // The picked helper is used while installed; removing it reads as automatic.
        let mut st = core.settings.read().clone();
        st.improve_model = "qwen25_vl_7b".into();
        crate::app::set_settings(&core, st).unwrap();
        let s = crate::describe::captioner_status(&core, crate::describe::Purpose::Improve);
        assert_eq!(s.source.as_deref(), Some("reuse"));
        delete_helper(&core, "qwen25_vl_7b").await.unwrap();
        assert!(list_helpers(&core).unwrap().is_empty());
        let s = crate::describe::captioner_status(&core, crate::describe::Purpose::Improve);
        assert!(
            s.source.is_none(),
            "falls back to automatic, which has nothing installed"
        );
    }

    #[tokio::test]
    async fn a_helper_a_model_reuses_is_kept_until_the_model_is_deleted() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        let add = |kind: ModelKind, name: &str, family: Option<&str>, comp: Option<&str>| {
            let dir = core.data.models(kind);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(name), b"1234").unwrap();
            let file = DownloadedFile {
                path: dir.join(name),
                sha256: format!("{:0>64}", name.len()),
                size_bytes: 4,
            };
            let reg = Registration {
                kind,
                friendly_name: name.into(),
                family: family.map(Into::into),
                component_id: comp.map(Into::into),
                civitai: None,
                dtype: None,
                lookup: None,
            };
            register_download(&core, &file, reg).unwrap();
        };
        add(
            ModelKind::Captioner,
            "m7.gguf",
            None,
            Some("qwen25_vl_7b_q8"),
        );
        add(
            ModelKind::Captioner,
            "p7.gguf",
            None,
            Some("qwen25_vl_7b_mmproj"),
        );
        add(ModelKind::Diffusion, "qwen.gguf", Some("qwen_image"), None);

        let err = delete_helper(&core, "qwen25_vl_7b").await.unwrap_err();
        assert!(err.message.contains("qwen.gguf"), "{err:?}");
        let dir = core.data.models(ModelKind::Captioner);
        assert!(dir.join("m7.gguf").exists());

        // Deleting the model keeps the helper's files; then the helper can go.
        let id = list_models(&core).unwrap()[0].id.clone();
        delete_model(&core, &id).await.unwrap();
        assert!(dir.join("m7.gguf").exists() && dir.join("p7.gguf").exists());
        delete_helper(&core, "qwen25_vl_7b").await.unwrap();
        assert!(!dir.join("m7.gguf").exists());
    }

    #[tokio::test]
    async fn the_safe_mode_off_helper_shares_the_7b_vision_file_and_is_automatic_while_off() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        let mut st = core.settings.read().clone();
        st.content_mode = "all".into();
        crate::app::set_settings(&core, st).unwrap();
        let ids: Vec<String> = crate::describe::list_helper_models(&core)
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, ["describe", "qwen25_vl_7b", "qwen25_vl_7b_safe_off"]);

        let dir = core.data.models(ModelKind::Captioner);
        std::fs::create_dir_all(&dir).unwrap();
        let add = |name: &str, comp: &str| {
            std::fs::write(dir.join(name), b"1234").unwrap();
            let file = DownloadedFile {
                path: dir.join(name),
                sha256: "cd".repeat(32),
                size_bytes: 4,
            };
            let reg = Registration {
                kind: ModelKind::Captioner,
                friendly_name: "helper".into(),
                family: None,
                component_id: Some(comp.into()),
                civitai: None,
                dtype: None,
                lookup: None,
            };
            register_download(&core, &file, reg).unwrap();
        };
        add("ab.gguf", "qwen25_vl_7b_safe_off_q4km");
        add("p7.gguf", "qwen25_vl_7b_mmproj");
        // Only the Safe-mode-Off helper is listed, with the shared vision file.
        let rows = list_helpers(&core).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["qwen25_vl_7b_safe_off"]
        );
        assert_eq!(rows[0].size_bytes, 8);
        // Automatic uses it while Safe mode is Off.
        let s = crate::describe::captioner_status(&core, crate::describe::Purpose::Improve);
        assert_eq!(s.source.as_deref(), Some("reuse"));

        // With the 7B installed too, each row is removable, the vision file is counted once,
        // and removing one helper keeps it.
        add("m7.gguf", "qwen25_vl_7b_q8");
        let rows = list_helpers(&core).unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| (r.id.as_str(), r.size_bytes))
                .collect::<Vec<_>>(),
            [("qwen25_vl_7b", 8), ("qwen25_vl_7b_safe_off", 4)]
        );
        assert!(crate::describe::list_helper_models(&core)
            .iter()
            .filter(|m| m.id != "describe")
            .all(|m| m.installed && m.removable));
        delete_helper(&core, "qwen25_vl_7b_safe_off").await.unwrap();
        assert!(!dir.join("ab.gguf").exists());
        assert!(dir.join("p7.gguf").exists() && dir.join("m7.gguf").exists());
        let rows = list_helpers(&core).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].id.as_str(), rows[0].size_bytes),
            ("qwen25_vl_7b", 8)
        );

        // Safe mode On: the Safe-mode-Off helper is never picked, even when installed.
        add("ab.gguf", "qwen25_vl_7b_safe_off_q4km");
        delete_helper(&core, "qwen25_vl_7b").await.unwrap();
        assert!(
            dir.join("p7.gguf").exists(),
            "still used by the other helper"
        );
        let s = crate::describe::captioner_status(&core, crate::describe::Purpose::Describe);
        assert_eq!(s.source.as_deref(), Some("reuse"));
        let mut st = core.settings.read().clone();
        st.content_mode = "safe".into();
        crate::app::set_settings(&core, st).unwrap();
        assert!(crate::describe::list_helper_models(&core)
            .iter()
            .all(|m| m.id != "qwen25_vl_7b_safe_off"));
        let s = crate::describe::captioner_status(&core, crate::describe::Purpose::Describe);
        assert!(s.source.is_none(), "nothing else is installed");
        // Installed helpers still lists it, so it can be removed.
        assert_eq!(list_helpers(&core).unwrap()[0].id, "qwen25_vl_7b_safe_off");
    }
}
