//! "Use models from another app": the user points Pinhole at a ComfyUI, A1111,
//! Forge or Stability Matrix models folder and Pinhole uses what it can run,
//! in place (SPEC §3). OWNER: catalog agent.
//!
//! Read-only: nothing in a linked folder is ever written, moved or deleted.
//! What Pinhole found is kept in `Data/catalog/linked-folders.json` (see
//! `pinhole_store::installed`). Style add-ons are linked (or, when the drive
//! can't link, copied) into a hidden `.pinhole-linked` folder in Pinhole's own
//! add-on folder when a picture uses them, because sd-server only loads add-ons
//! from one folder; that folder is emptied at every start.
//!
//! PRIVACY: nothing here goes online (no by-hash lookups; only notes other
//! apps already saved next to their files). No prompt text anywhere.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use pinhole_catalog::linked::{self as scan, FoundFile, Skipped};
use pinhole_catalog::local;
use pinhole_registry::detect;
use pinhole_store::datadir::ModelKind;
use pinhole_store::installed::{
    linked_folder_id, linked_rel_path, FileStamp, InstalledFile, LinkedFolder,
};
use serde::{Deserialize, Serialize};

use crate::{AppCore, CoreError, CoreEvent, CoreResult};

/// Hidden folder inside Pinhole's add-on folder holding links to linked add-ons.
pub const LORA_LINKS_DIR: &str = ".pinhole-linked";

/// RAM-only state of the linked folders.
#[derive(Default)]
pub struct LinkedRuntime {
    /// Folder ids being looked through right now.
    scanning: Mutex<HashSet<String>>,
    /// Another look was asked for while one ran.
    again: Mutex<HashSet<String>>,
    /// Files found last time that Pinhole can't use, by `rel_path`: not read
    /// again while unchanged.
    skipped: Mutex<HashMap<String, (FileStamp, Skipped)>>,
}

/// `LinkedFolder` in src/lib/types.ts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedFolderView {
    pub id: String,
    pub path: String,
    /// Last part of the path ("ComfyUI", "models"…).
    pub name: String,
    /// The folder is there (its drive is connected).
    pub available: bool,
    /// Pinhole is looking through it.
    pub scanning: bool,
    pub models: u32,
    pub addons: u32,
    /// Parts (VAE, text encoders) the models share with Pinhole's.
    pub parts: u32,
    /// Files found last time that Pinhole can't use (ControlNets, upscalers…).
    pub not_used: u32,
}

fn folder_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

fn view(core: &AppCore, folder: &LinkedFolder) -> LinkedFolderView {
    let index = core.installed.lock();
    let mine = || {
        index
            .files
            .iter()
            .filter(|f| linked_folder_id(&f.rel_path) == Some(folder.id.as_str()))
    };
    let count = |pred: &dyn Fn(&InstalledFile) -> bool| mine().filter(|f| pred(f)).count() as u32;
    let prefix = linked_rel_path(&folder.id, &[]) + "/";
    let not_used = core
        .linked
        .skipped
        .lock()
        .keys()
        .filter(|k| k.starts_with(&prefix))
        .count() as u32;
    LinkedFolderView {
        id: folder.id.clone(),
        path: folder.path.clone(),
        name: folder_name(&folder.path),
        available: Path::new(&folder.path).is_dir(),
        scanning: core.linked.scanning.lock().contains(&folder.id),
        models: count(&|f| matches!(f.kind, ModelKind::Checkpoint | ModelKind::Diffusion)),
        addons: count(&|f| f.kind == ModelKind::Lora),
        parts: count(&|f| f.component_id.is_some()),
        not_used,
    }
}

/// The linked folders, in the order they were added.
pub fn list(core: &AppCore) -> Vec<LinkedFolderView> {
    let folders = core.installed.lock().linked.folders.clone();
    folders.iter().map(|f| view(core, f)).collect()
}

fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// Add another app's models folder and start looking through it.
pub fn add(core: &Arc<AppCore>, path: &str) -> CoreResult<LinkedFolderView> {
    let _folder = crate::models::folder_read(core)?;
    let raw = PathBuf::from(path.trim());
    if !raw.is_absolute() || !raw.is_dir() {
        return Err(CoreError::not_found(
            "Pinhole can't find that folder. Check that its drive is connected and try again.",
        ));
    }
    let picked = canon(&raw);
    let own = [canon(&core.data.root), canon(&core.data.models_root())];
    if own
        .iter()
        .any(|o| o.starts_with(&picked) || picked.starts_with(o))
    {
        return Err(CoreError::invalid(
            "That's Pinhole's own folder (or holds it). Pick the models folder of the other app, for example ComfyUI/models.",
        ));
    }
    let folder = {
        let mut index = core.installed.lock();
        for f in &index.linked.folders {
            let other = canon(Path::new(&f.path));
            if other == picked {
                return Err(CoreError::invalid("You've already added this folder."));
            }
            if picked.starts_with(&other) || other.starts_with(&picked) {
                return Err(CoreError::invalid(format!(
                    "This folder overlaps “{}”, which you've already added. Remove that one first to use this one.",
                    folder_name(&f.path)
                )));
            }
        }
        let folder = LinkedFolder {
            id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
            path: picked.display().to_string(),
            added_at: chrono::Utc::now().timestamp(),
        };
        index.linked.folders.push(folder.clone());
        if let Err(e) = index.save_linked(&core.data) {
            index.linked.folders.retain(|f| f.id != folder.id);
            return Err(e.into());
        }
        folder
    };
    core.linked.scanning.lock().insert(folder.id.clone());
    spawn_scan(core, folder.id.clone());
    core.emit(CoreEvent::ModelsChanged);
    Ok(view(core, &folder))
}

/// Stop using a linked folder (its files stay where they are).
pub async fn remove(core: &AppCore, id: &str) -> CoreResult<()> {
    let _folder = crate::models::folder_read(core)?;
    let paths: Vec<PathBuf> = {
        let index = core.installed.lock();
        if index.linked.folder(id).is_none() {
            return Err(CoreError::not_found(
                "That folder isn't in the list any more.",
            ));
        }
        index
            .files
            .iter()
            .filter(|f| linked_folder_id(&f.rel_path) == Some(id))
            .map(|f| index.abs_path(&core.data, f))
            .collect()
    };
    if crate::generate::unload_interrupts_job(core, "", &paths).await {
        return Err(CoreError::invalid(
            "Wait for the current pictures to finish, then remove the folder.",
        ));
    }
    crate::generate::unload_model(core, "", &paths).await;
    {
        let mut index = core.installed.lock();
        let before = index.clone();
        let ids: Vec<String> = index
            .files
            .iter()
            .filter(|f| linked_folder_id(&f.rel_path) == Some(id))
            .map(|f| f.id.clone())
            .collect();
        for fid in &ids {
            index.remove(fid);
        }
        index.linked.folders.retain(|f| f.id != id);
        index
            .linked
            .parked
            .retain(|e| linked_folder_id(&e.file.rel_path) != Some(id));
        if let Err(e) = index.save_linked(&core.data) {
            *index = before;
            return Err(e.into());
        }
    }
    let prefix = linked_rel_path(id, &[]) + "/";
    core.linked
        .skipped
        .lock()
        .retain(|k, _| !k.starts_with(&prefix));
    let _ = std::fs::remove_dir_all(lora_links_root(core).join(id));
    core.emit(CoreEvent::ModelsChanged);
    Ok(())
}

/// Look through every linked folder again (new or changed files). Unchanged
/// files aren't read again.
pub fn rescan_all(core: &Arc<AppCore>) {
    let ids: Vec<String> = core
        .installed
        .lock()
        .linked
        .folders
        .iter()
        .map(|f| f.id.clone())
        .collect();
    for id in ids {
        let fresh = core.linked.scanning.lock().insert(id.clone());
        if fresh {
            spawn_scan(core, id);
        } else {
            core.linked.again.lock().insert(id);
        }
    }
    core.emit(CoreEvent::ModelsChanged);
}

/// At start: empty the add-on links folder, then look through the folders.
pub fn start(core: &Arc<AppCore>) {
    let _ = std::fs::remove_dir_all(lora_links_root(core));
    if !core.installed.lock().linked.folders.is_empty() {
        rescan_all(core);
    }
}

fn spawn_scan(core: &Arc<AppCore>, id: String) {
    let core = core.clone();
    std::thread::spawn(move || loop {
        scan_folder(&core, &id);
        let again = core.linked.again.lock().remove(&id);
        if !again {
            core.linked.scanning.lock().remove(&id);
            core.emit(CoreEvent::ModelsChanged);
            break;
        }
    });
}

/// Look through one folder and replace its entries in the index. Blocking.
fn scan_folder(core: &AppCore, id: &str) {
    let (folder, previous, taken_shas) = {
        let index = core.installed.lock();
        let Some(folder) = index.linked.folder(id).cloned() else {
            return;
        };
        let previous: HashMap<String, (InstalledFile, FileStamp)> = index
            .files
            .iter()
            .filter(|f| linked_folder_id(&f.rel_path) == Some(id))
            .map(|f| {
                let stamp = index.linked.stamps.get(&f.id).copied().unwrap_or_default();
                (f.rel_path.clone(), (f.clone(), stamp))
            })
            .collect();
        // Files Pinhole already has (installed, or in another linked folder).
        let taken: HashSet<String> = index
            .files
            .iter()
            .filter(|f| linked_folder_id(&f.rel_path) != Some(id))
            .map(|f| f.sha256.to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        (folder, previous, taken)
    };
    let root = PathBuf::from(&folder.path);
    if !root.is_dir() {
        return;
    }
    let registry = core.registry();
    let found = scan::walk(&root);
    let mut kept: Vec<(InstalledFile, FileStamp)> = Vec::new();
    let mut shas: HashSet<String> = taken_shas;
    let mut skipped: HashMap<String, (FileStamp, Skipped)> = HashMap::new();
    let old_skipped = core.linked.skipped.lock().clone();
    for f in &found {
        let rel = linked_rel_path(id, &f.parts);
        let stamp = FileStamp {
            size: f.size,
            mtime: f.mtime,
        };
        let entry = match previous.get(&rel) {
            Some((prev, s)) if *s == stamp => Ok(prev.clone()),
            _ => match old_skipped.get(&rel) {
                Some((s, why)) if *s == stamp => Err(*why),
                _ => recognise(&registry, f, previous.get(&rel).map(|p| &p.0), &rel),
            },
        };
        match entry {
            Ok(entry) => {
                let sha = entry.sha256.to_ascii_lowercase();
                // The same file twice (e.g. a copy Pinhole installed): keep one.
                if !sha.is_empty() && !shas.insert(sha) {
                    continue;
                }
                kept.push((entry, stamp));
            }
            Err(why) => {
                skipped.insert(rel, (stamp, why));
            }
        }
    }
    {
        let mut index = core.installed.lock();
        if index.linked.folder(id).is_none() {
            return; // removed meanwhile
        }
        let old: Vec<String> = index
            .files
            .iter()
            .filter(|f| linked_folder_id(&f.rel_path) == Some(id))
            .map(|f| f.id.clone())
            .collect();
        let unchanged = old.len() == kept.len()
            && kept.iter().all(|(f, s)| {
                index.get(&f.id) == Some(f) && index.linked.stamps.get(&f.id) == Some(s)
            });
        if !unchanged {
            // Keep what changed meanwhile (last used, measured memory, trigger words).
            let live: HashMap<String, InstalledFile> = old
                .iter()
                .filter_map(|fid| index.get(fid).cloned())
                .map(|f| (f.id.clone(), f))
                .collect();
            for fid in &old {
                index.remove(fid);
            }
            for (mut f, stamp) in kept {
                if let Some(now) = live.get(&f.id) {
                    f.last_used = now.last_used;
                    f.trigger_words = now.trigger_words.clone();
                    if previous.get(&f.rel_path).is_some_and(|(_, s)| *s == stamp) {
                        f.observed_vram_gb = now.observed_vram_gb;
                    }
                }
                index.linked.stamps.insert(f.id.clone(), stamp);
                index.files.push(f);
            }
            let _ = index.save_linked(&core.data);
        }
    }
    {
        let prefix = linked_rel_path(id, &[]) + "/";
        let mut all = core.linked.skipped.lock();
        all.retain(|k, _| !k.starts_with(&prefix));
        all.extend(skipped);
    }
    core.emit(CoreEvent::ModelsChanged);
}

fn recognise(
    registry: &pinhole_registry::Registry,
    f: &FoundFile,
    previous: Option<&InstalledFile>,
    rel: &str,
) -> Result<InstalledFile, Skipped> {
    let header = detect::read_header(&f.abs).map_err(|_| Skipped::NotUsable)?;
    let note = scan::read_note(&f.abs, f.size);
    let r = scan::recognise(registry, f, &header, note.as_ref(), &mut |p| {
        local::hash_file(p).ok().map(|(h, _)| h)
    })?;
    Ok(InstalledFile {
        id: previous
            .map(|p| p.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        rel_path: rel.to_string(),
        kind: r.kind,
        sha256: r.sha256,
        size_bytes: f.size,
        family: r.family,
        component_id: r.component_id,
        friendly_name: r.friendly_name,
        civitai: r.civitai,
        added_at: previous
            .map(|p| p.added_at)
            .unwrap_or_else(|| chrono::Utc::now().timestamp()),
        last_used: previous.and_then(|p| p.last_used),
        observed_vram_gb: None,
        dtype: r.dtype,
        trigger_words: previous.and_then(|p| p.trigger_words.clone()),
    })
}

// ------------------------------------------------------------------ add-ons

fn lora_links_root(core: &AppCore) -> PathBuf {
    core.data.models(ModelKind::Lora).join(LORA_LINKS_DIR)
}

/// Same file (size and modification time).
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => {
            x.len() == y.len()
                && x.modified().ok().is_some()
                && x.modified().ok() == y.modified().ok()
        }
        _ => false,
    }
}

/// A path inside Pinhole's add-on folder for linked add-on `file` (at `abs`),
/// which sd-server can load: a hard link, else a symbolic link, else a copy
/// (add-ons are small). Reused while the file is unchanged. Blocking.
pub fn lora_path_for_engine(
    core: &AppCore,
    file: &InstalledFile,
    abs: &Path,
) -> CoreResult<PathBuf> {
    let id = linked_folder_id(&file.rel_path)
        .ok_or_else(|| CoreError::internal("Not an add-on from another app's folder."))?;
    let norm = pinhole_store::datadir::normalize_rel(&file.rel_path);
    let dest = norm
        .split('/')
        .skip(2)
        .fold(lora_links_root(core).join(id), |p, c| p.join(c));
    if same_file(&dest, abs) {
        return Ok(dest);
    }
    let fail = |e: std::io::Error| {
        CoreError::new(
            "io",
            "Pinhole couldn't get this add-on ready from the other app's folder. Check that the drive is connected and that Pinhole's Data folder has free space.",
        )
        .with_details(e.to_string())
    };
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    let _ = std::fs::remove_file(&dest);
    if std::fs::hard_link(abs, &dest).is_ok() {
        return Ok(dest);
    }
    #[cfg(unix)]
    if std::os::unix::fs::symlink(abs, &dest).is_ok() {
        return Ok(dest);
    }
    #[cfg(windows)]
    if std::os::windows::fs::symlink_file(abs, &dest).is_ok() {
        return Ok(dest);
    }
    let part = local::part_path(&dest);
    std::fs::copy(abs, &part).map_err(fail)?;
    if let Ok(t) = std::fs::metadata(abs).and_then(|m| m.modified()) {
        if let Ok(f) = std::fs::File::options().write(true).open(&part) {
            let _ = f.set_modified(t);
        }
    }
    std::fs::rename(&part, &dest).map_err(fail)?;
    Ok(dest)
}

/// Test helpers: small but real model files for a linked folder.
#[cfg(test)]
pub(crate) mod fixtures {
    use std::path::Path;

    /// A `.safetensors` file with these tensors (zeros) and metadata.
    pub fn write_safetensors(path: &Path, tensors: &[(&str, &[u64])], metadata: &[(&str, &str)]) {
        let mut obj = serde_json::Map::new();
        let mut offset = 0u64;
        for (name, shape) in tensors {
            let n = shape.iter().product::<u64>().max(1) * 2;
            obj.insert(
                (*name).into(),
                serde_json::json!({ "dtype": "F16", "shape": shape, "data_offsets": [offset, offset + n] }),
            );
            offset += n;
        }
        if !metadata.is_empty() {
            let m: serde_json::Map<String, serde_json::Value> = metadata
                .iter()
                .map(|(k, v)| ((*k).to_string(), serde_json::Value::from(*v)))
                .collect();
            obj.insert("__metadata__".into(), serde_json::Value::Object(m));
        }
        let header = serde_json::to_vec(&serde_json::Value::Object(obj)).unwrap();
        let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(&header);
        bytes.resize(bytes.len() + offset as usize, 0);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    pub fn sdxl(path: &Path) {
        write_safetensors(
            path,
            &[
                ("model.diffusion_model.input_blocks.0.0.weight", &[320, 4, 3, 3]),
                ("model.diffusion_model.middle_block.1.norm.weight", &[1280]),
                (
                    "conditioner.embedders.0.transformer.text_model.embeddings.token_embedding.weight",
                    &[4, 768],
                ),
                ("conditioner.embedders.1.model.token_embedding.weight", &[4, 1280]),
                ("first_stage_model.encoder.conv_in.weight", &[128, 3, 3, 3]),
            ],
            &[],
        );
    }

    pub fn sdxl_lora(path: &Path) {
        write_safetensors(
            path,
            &[
                (
                    "lora_unet_input_blocks_1_1_proj_in.lora_down.weight",
                    &[8, 320],
                ),
                (
                    "lora_unet_input_blocks_1_1_proj_in.lora_up.weight",
                    &[320, 8],
                ),
                ("lora_unet_input_blocks_1_1_proj_in.alpha", &[]),
            ],
            &[("ss_base_model_version", "sdxl_base_v1-0")],
        );
    }

    /// A ComfyUI-like folder: a Pony checkpoint, a style add-on, a ControlNet,
    /// and a checkpoint whose CivitAI note marks a real person.
    pub fn comfy(root: &Path) {
        sdxl(&root.join("models/checkpoints/ponyDiffusionV6XL.safetensors"));
        sdxl_lora(&root.join("models/loras/watercolor.safetensors"));
        sdxl(&root.join("models/controlnet/cn.safetensors"));
        sdxl(&root.join("models/checkpoints/someone.safetensors"));
        std::fs::write(
            root.join("models/checkpoints/someone.civitai.info"),
            serde_json::to_vec(&serde_json::json!({
                "id": 9, "modelId": 8, "baseModel": "SDXL 1.0",
                "model": {"name": "Someone", "poi": true}
            }))
            .unwrap(),
        )
        .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use pinhole_store::DataDir;

    use super::*;
    use crate::{NullSink, ShippedPaths};

    pub(crate) fn new_core(data: &Path) -> Arc<AppCore> {
        AppCore::new(
            ShippedPaths {
                config_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../config"),
            },
            DataDir::at(data.to_path_buf(), false),
            Arc::new(NullSink),
        )
        .unwrap()
    }

    pub(crate) fn wait_scans(core: &AppCore) {
        let start = Instant::now();
        while !core.linked.scanning.lock().is_empty() {
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "scan never finished"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[tokio::test]
    async fn a_comfyui_folder_is_used_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        let comfy = tmp.path().join("ComfyUI");
        fixtures::comfy(&comfy);
        let listing = |p: &Path| {
            let mut v: Vec<_> = scan::walk(p)
                .into_iter()
                .map(|f| (f.parts, f.size, f.mtime))
                .collect();
            v.sort();
            v
        };
        let before = listing(&comfy);
        let core = new_core(&tmp.path().join("Data"));

        let view = add(&core, &comfy.display().to_string()).unwrap();
        assert_eq!(view.name, "ComfyUI");
        wait_scans(&core);

        let models = crate::models::list_models(&core).unwrap();
        assert_eq!(models.len(), 1, "{models:?}");
        assert_eq!(models[0].family_id.as_deref(), Some("sdxl_pony"));
        assert_eq!(models[0].linked_folder.as_deref(), Some("ComfyUI"));
        assert!(models[0].vram.is_some() && models[0].fit.is_some());
        let loras = crate::models::list_loras(&core).unwrap();
        assert_eq!(loras.len(), 1);
        assert_eq!(loras[0].family_id.as_deref(), Some("sdxl"));
        let v = &list(&core)[0];
        // The ControlNet folder isn't looked into; the flagged checkpoint is not used.
        assert_eq!(
            (v.models, v.addons, v.not_used, v.scanning),
            (1, 1, 1, false)
        );

        // The file is read where it is.
        {
            let index = core.installed.lock();
            let f = index.get(&models[0].id).unwrap();
            assert_eq!(
                index.abs_path(&core.data, f),
                comfy
                    .join("models")
                    .join("checkpoints")
                    .join("ponyDiffusionV6XL.safetensors")
            );
        }
        // Never deleted by Pinhole.
        let e = crate::models::delete_model(&core, &models[0].id)
            .await
            .unwrap_err();
        assert!(e.message.contains("another app's models folder"), "{e:?}");

        // Adding it again, a folder inside it, or Pinhole's own folder is refused.
        assert!(add(&core, &comfy.display().to_string()).is_err());
        assert!(add(&core, &comfy.join("models").display().to_string()).is_err());
        assert!(add(&core, &core.data.root.join("models").display().to_string()).is_err());
        assert!(add(&core, "relative/path").is_err());

        // Kept across restarts (Data/catalog/linked-folders.json), not in installed.json.
        let installed = std::fs::read_to_string(core.data.installed_file()).unwrap_or_default();
        assert!(!installed.contains("pony"), "{installed}");
        let again = new_core(&tmp.path().join("Data"));
        assert_eq!(
            crate::models::list_models(&again).unwrap()[0].id,
            models[0].id
        );

        // A new file shows up on the next look; unchanged ones keep their entry.
        fixtures::sdxl(&comfy.join("models/checkpoints/illustrious/mix.safetensors"));
        rescan_all(&again);
        wait_scans(&again);
        let models2 = crate::models::list_models(&again).unwrap();
        assert_eq!(models2.len(), 2);
        assert!(models2.iter().any(|m| m.id == models[0].id));
        assert!(models2
            .iter()
            .any(|m| m.family_id.as_deref() == Some("sdxl_illustrious")));

        // Removing the folder forgets its files and leaves them on disk.
        remove(&again, &view.id).await.unwrap();
        assert!(crate::models::list_models(&again).unwrap().is_empty());
        assert!(crate::models::list_loras(&again).unwrap().is_empty());
        assert!(list(&again).is_empty());
        let mut after = listing(&comfy);
        after.retain(|(p, _, _)| !p.iter().any(|x| x == "illustrious"));
        assert_eq!(before, after, "nothing in the folder changed");
        assert!(comfy
            .join("models/checkpoints/someone.civitai.info")
            .is_file());
    }

    #[test]
    fn an_add_on_is_linked_into_pinholes_add_on_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let comfy = tmp.path().join("Comfy");
        fixtures::sdxl_lora(&comfy.join("loras/a/w.safetensors"));
        let core = new_core(&tmp.path().join("Data"));
        add(&core, &comfy.display().to_string()).unwrap();
        wait_scans(&core);
        let (file, abs) = {
            let index = core.installed.lock();
            let f = index.loras().next().unwrap().clone();
            let abs = index.abs_path(&core.data, &f);
            (f, abs)
        };
        let p = lora_path_for_engine(&core, &file, &abs).unwrap();
        assert!(p.starts_with(core.data.models(ModelKind::Lora).join(LORA_LINKS_DIR)));
        assert!(p.ends_with(Path::new("a").join("w.safetensors")));
        assert_eq!(std::fs::read(&p).unwrap(), std::fs::read(&abs).unwrap());
        // Reused while unchanged.
        assert_eq!(lora_path_for_engine(&core, &file, &abs).unwrap(), p);
        // Emptied at start.
        start(&core);
        wait_scans(&core);
        assert!(!p.exists());
        assert!(abs.is_file());
    }
}
