//! Installed models, add-a-file, delete, recommended picks, install flows,
//! CivitAI resource resolution (paste). OWNER: catalog agent.
//!
//! The pure logic lives in `pinhole_catalog` (inventory, recommend, paste,
//! local); this module wires it to `AppCore` (index, registry, hardware,
//! downloads, events).
//!
//! PRIVACY: nothing here sees prompt text. Pasted resources carry ids/hashes
//! and model names only.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::Mutex;
use pinhole_catalog::families::{self, FamilyResolution};
use pinhole_catalog::recommend::{self, FileToGet, PickAction};
use pinhole_catalog::{inventory, local, paste, CatalogFilters};
use pinhole_net::download::{check_free_space, DownloadSpec, DownloadedFile};
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
    /// CivitAI API key cache: `None` = not read yet. Never written anywhere.
    pub(crate) api_key: Mutex<Option<Option<String>>>,
    /// `catalog-filters.yaml`, loaded once.
    pub(crate) filters: OnceLock<Arc<CatalogFilters>>,
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
    let root = core.data.root.canonicalize().ok();
    let file = path.canonicalize().ok();
    if let (Some(root), Some(file)) = (root, file) {
        if let Ok(rest) = file.strip_prefix(&root) {
            let parts: Vec<String> = rest.components().filter_map(|c| c.as_os_str().to_str().map(str::to_string)).collect();
            if !parts.is_empty() {
                return Ok(parts.join("/"));
            }
        }
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
        PickAction::Civitai { version_id, family_id } => crate::catalog::install_civitai(core, version_id, Some(family_id)).await,
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
        let models_root = core.data.root.join("models");
        check_free_space(&models_root, total).map_err(|e| disk_space_error(e, &items[0].0.url))?;

        let mut specs = Vec::new();
        let mut planned: Vec<(PathBuf, String, Registration)> = Vec::new();
        for (f, civitai) in items {
            let dir = core.data.models(f.kind);
            std::fs::create_dir_all(&dir)?;
            // Never overwrite a registered file (or one planned in this group).
            let dest = local::unique_path(&dir, &f.file_name, |p| {
                planned.iter().any(|(d, _, _)| d == p)
                    || (p.exists() && core.data.relative(p).is_some_and(|rel| index.files.iter().any(|x| x.rel_path == rel)))
            });
            let headers: Vec<(String, String)> =
                pinhole_catalog::api::civitai_auth_header(api_key.as_deref(), &f.url).into_iter().collect();
            specs.push(DownloadSpec {
                url: f.url.clone(),
                dest: dest.clone(),
                sha256: f.sha256.clone(),
                size_bytes: (f.size_bytes > 0).then_some(f.size_bytes),
                label: f.friendly_name.clone(),
                headers,
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
        let group_id = core.downloads.enqueue(label, specs);
        for (_, key, _) in &planned {
            inflight.insert(key.clone(), group_id.clone());
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
        if let Ok(files) = result {
            for (i, (dest, _, reg)) in planned.into_iter().enumerate() {
                let file = files.iter().find(|f| f.path == dest).or_else(|| files.get(i));
                if let Some(file) = file {
                    let _ = register_download(&task_core, file, reg);
                }
            }
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
    let src = PathBuf::from(path.trim());
    let ext = local::allowed_extension(&src).ok_or_else(|| {
        CoreError::invalid("Only .safetensors and .gguf files can be added. Older .ckpt/.pt files can hide harmful code.")
    })?;
    if !src.is_file() {
        return Err(CoreError::not_found("Pinhole can't find that file. Check that it still exists and try again."));
    }
    let registry = core.registry();

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
    let src_canon = src.canonicalize().unwrap_or_else(|_| src.clone());
    let in_place = src_canon.starts_with(&data_root);
    let (dest, sha256, size_bytes) = if in_place {
        let p = src_canon.clone();
        let (sha, size) = blocking(move || local::hash_file(&p)).await??;
        (src_canon, sha, size)
    } else {
        let dir = core.data.models(kind);
        std::fs::create_dir_all(&dir)?;
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
    preview_delete(core, model_id)?;
    crate::generate::unload_model(core, model_id).await;
    let registry = core.registry();
    let mut failed: Vec<String> = Vec::new();
    {
        let mut index = core.installed.lock();
        let Some(preview) = inventory::delete_preview(&registry, &index, model_id) else {
            return Err(CoreError::not_found("That model isn't installed any more."));
        };
        let ids: Vec<String> = preview
            .files
            .iter()
            .filter_map(|df| index.files.iter().find(|f| f.rel_path == df.rel_path).map(|f| f.id.clone()))
            .collect();
        for id in ids {
            let Some(f) = index.get(&id).cloned() else { continue };
            if inventory::is_safe_rel_path(&f.rel_path) {
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
            index.remove(&id);
        }
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

    #[test]
    fn recommended_through_core() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        let picks = get_recommended(&core).unwrap();
        assert_eq!(picks.iter().map(|p| p.role.as_str()).collect::<Vec<_>>(), ["realistic", "anime", "edit", "describe"]);
        // Before hardware detection: CPU only → nothing fits except the captioner.
        assert!(picks[0].unavailable_reason.is_some());
        assert_eq!(picks[3].title.as_deref(), Some("Image describer"));
    }

    #[tokio::test]
    async fn install_recommended_errors_are_plain() {
        let (_t, core) = test_core(Arc::new(Recorder::default()));
        let e = install_recommended(&core, "realistic").await.unwrap_err();
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
            .replace("size_mb: 6577", "size_mb: 1")
            .replace("size_mb: 335", "size_mb: 1")
            .replace("size_mb: 8045", "size_mb: 1");
        *core.registry.write() = Arc::new(pinhole_registry::Registry::from_yaml(&yaml, None).unwrap());
        {
            let mut s = core.settings.write();
            s.vram_override_gb = Some(8.0);
            s.gpu = "auto".into();
        }
        let started = install_recommended(&core, "realistic").await.unwrap();
        let status = core.downloads.status();
        let g = status.iter().find(|g| g.group_id == started.group_id).unwrap();
        assert_eq!(g.label, "Z-Image Turbo");
        assert_eq!(g.file_count, 3, "model + VAE + text encoder");
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
}
