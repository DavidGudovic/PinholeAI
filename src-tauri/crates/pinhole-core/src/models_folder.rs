//! Settings → Models folder: keep models in a folder the user picks
//! (e.g. a partition shared by Windows and Linux on a dual-boot PC), so two
//! Pinhole installs use the same files instead of downloading them twice.
//!
//! The picked folder holds the model sub-folders plus its own index
//! (`pinhole-models.json`, same format as `installed.json`). Index paths stay
//! `models/<sub>/<file>`, so they read the same wherever the folder is mounted.
//! Engines, settings, presets and styles stay in each install's Data folder.
//!
//! Changing the folder moves every installed model: a rename on the same
//! drive, else a copy that is checked against the file's SHA-256 before the
//! original is removed. Files the target folder already has (same SHA-256,
//! e.g. put there by the other install) are not copied twice. Any failure
//! puts everything back. The app restarts afterwards (the Data paths are fixed
//! for the life of an `AppCore`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pinhole_catalog::local;
use pinhole_net::download::check_free_space;
use pinhole_store::datadir::{is_writable_dir, ModelsFolderProblem};
use pinhole_store::{DataDir, InstalledFile, InstalledIndex};
use serde::{Deserialize, Serialize};

use crate::events::ModelsMoveProgress;
use crate::{AppCore, CoreError, CoreEvent, CoreResult};

/// `ModelsFolderInfo` in src/lib/types.ts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelsFolderInfo {
    /// Folder holding the model sub-folders.
    pub path: String,
    /// A folder the user picked (not `Data/models`).
    pub custom: bool,
    pub problem: Option<ModelsFolderProblem>,
}

/// `ModelsFolderPreview` in src/lib/types.ts: what "Change…" would do.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelsFolderPreview {
    /// The folder models would go to (`Data/models` for the default).
    pub path: String,
    pub is_default: bool,
    /// Installed files that move (models, parts, add-ons, helpers).
    pub files: u32,
    pub bytes: u64,
    /// Models already in the target folder (e.g. from another Pinhole install).
    pub existing_models: u32,
    /// Moving to another drive copies the files (slower); same drive renames.
    pub same_drive: bool,
}

pub fn info(core: &AppCore) -> ModelsFolderInfo {
    ModelsFolderInfo {
        path: core.data.models_root().display().to_string(),
        custom: core.data.models_home.is_some(),
        problem: core.data.models_problem(),
    }
}

/// Target `DataDir` for `folder` (`None` = the default `Data/models`). The
/// default folder picked by path counts as the default.
fn target_dir(core: &AppCore, folder: Option<&str>) -> CoreResult<DataDir> {
    let default = DataDir::at(core.data.root.clone(), core.data.portable);
    let Some(raw) = folder.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(default);
    };
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return Err(CoreError::invalid("Pick a folder with the folder chooser."));
    }
    if !path.is_dir() {
        return Err(CoreError::invalid("That folder doesn't exist. Pick an existing folder."));
    }
    // Compare canonical forms, but keep the path as picked (on Windows
    // `canonicalize` gives `\\?\` paths, which file managers don't open).
    let canon = canon_or(&path);
    let default_models = canon_or(&default.models_root());
    if canon == default_models {
        return Ok(default);
    }
    // Inside the Data folder (but not Data/models itself) would mix models
    // with engines, outputs and settings.
    if canon.starts_with(canon_or(&core.data.root)) || canon_or(&core.data.root).starts_with(&canon) {
        return Err(CoreError::invalid(
            "Pick a folder outside Pinhole's Data folder, for example a new folder on your shared drive.",
        ));
    }
    Ok(default.with_models_home(Some(path)))
}

fn canon_or(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

fn check_target(core: &AppCore, to: &DataDir) -> CoreResult<()> {
    let (from_root, to_root) = (canon_or(&core.data.models_root()), canon_or(&to.models_root()));
    if from_root == to_root {
        return Err(CoreError::invalid("Your models are already in that folder."));
    }
    if from_root.starts_with(&to_root) || to_root.starts_with(&from_root) {
        return Err(CoreError::invalid("Pick a folder that isn't inside the current Models folder (or the other way round)."));
    }
    if to.models_home.is_some() && !is_writable_dir(&to.models_root()) {
        return Err(CoreError::invalid(
            "Pinhole can't write to that folder. Pick another one. If it's on a drive Windows also uses, turn off Fast Startup in Windows and shut it down fully.",
        ));
    }
    Ok(())
}

/// Refuse a move when either models list was saved by a newer Pinhole (this
/// version can't save it). Returns the target's index, read without changing
/// anything (a damaged one reads as empty; the move sets it aside).
fn check_versions(core: &AppCore, to: &DataDir) -> CoreResult<InstalledIndex> {
    if core.installed.lock().is_newer() {
        return Err(CoreError::invalid(
            "Your models list was saved by a newer version of Pinhole. Update Pinhole, then change the Models folder.",
        ));
    }
    let to_index = InstalledIndex::read_from(&to.installed_file())?.unwrap_or_else(InstalledIndex::new);
    if to_index.is_newer() {
        return Err(CoreError::invalid(
            "The models list in that folder was saved by a newer version of Pinhole. Update Pinhole, then pick that folder again.",
        ));
    }
    Ok(to_index)
}

pub fn preview(core: &AppCore, folder: Option<&str>) -> CoreResult<ModelsFolderPreview> {
    let to = target_dir(core, folder)?;
    check_target(core, &to)?;
    let to_index = check_versions(core, &to)?;
    let from_index = core.installed.lock().clone();
    let plan = plan(&core.data, &from_index, &to, &to_index);
    Ok(ModelsFolderPreview {
        path: to.models_root().display().to_string(),
        is_default: to.models_home.is_none(),
        files: from_index.files.len() as u32,
        bytes: plan.iter().filter(|s| s.action == Action::Move).map(|s| s.entry.size_bytes).sum(),
        existing_models: to_index.models().count() as u32,
        same_drive: same_drive(&core.data.models_root(), &to.models_root()),
    })
}

/// Move every model to `folder` (`None` = back to `Data/models`), save the
/// new location in settings, then return; the caller restarts the app.
/// Refuses while a download, generation or description is running; stops the
/// engines first (Windows can't move a file a running engine has open).
pub async fn change(core: &Arc<AppCore>, folder: Option<String>) -> CoreResult<ModelsFolderInfo> {
    let to = target_dir(core, folder.as_deref())?;
    check_target(core, &to)?;
    check_versions(core, &to)?;
    if core.downloads.status().iter().any(|g| !g.state.is_finished()) {
        return Err(CoreError::invalid("Wait for your downloads to finish (or cancel them), then change the Models folder."));
    }
    if core.gen.active.lock().is_some() || core.describe.busy.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(CoreError::invalid("Wait for the current pictures to finish, then change the Models folder."));
    }
    let _folder = core
        .models
        .folder_lock
        .try_write()
        .map_err(|_| CoreError::invalid("Wait for Pinhole to finish adding or deleting a model (or making or describing a picture), then change the Models folder."))?;
    crate::generate::shutdown(core).await;
    crate::describe::shutdown(core).await;
    crate::models::discard_pending(core);

    let core2 = core.clone();
    tokio::task::spawn_blocking(move || move_all(&core2, to))
        .await
        .map_err(|_| CoreError::internal("A background task stopped unexpectedly. Try again."))??;
    Ok(info(core))
}

fn load_target_index(to: &DataDir) -> CoreResult<InstalledIndex> {
    let path = to.installed_file();
    if !path.is_file() {
        return Ok(InstalledIndex::new());
    }
    InstalledIndex::load_from(&path).map_err(CoreError::from)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// Rename or copy `src` → `dest`.
    Move,
    /// The target already has this file (same SHA-256): keep its entry, drop ours.
    Duplicate,
    /// Our file is missing on disk: carry the entry over (at a free path).
    Missing,
}

#[derive(Debug, Clone)]
struct Step {
    action: Action,
    /// Entry as it will be in the target index (for `Duplicate`: the target's entry).
    entry: InstalledFile,
    /// Our id (presets refer to it).
    old_id: String,
    src: PathBuf,
    dest: PathBuf,
}

/// Pure planning: what happens to each entry of `from_index`.
fn plan(from: &DataDir, from_index: &InstalledIndex, to: &DataDir, to_index: &InstalledIndex) -> Vec<Step> {
    let mut taken: HashSet<PathBuf> = to_index.rel_paths().map(|p| to.resolve_rel(p)).collect();
    let mut steps = Vec::new();
    for f in &from_index.files {
        let src = from.resolve_rel(&f.rel_path);
        let dup = to_index.find_by_sha(&f.sha256).filter(|t| {
            !f.sha256.is_empty()
                && std::fs::metadata(to.resolve_rel(&t.rel_path)).is_ok_and(|m| m.is_file() && m.len() == f.size_bytes)
        });
        if let Some(t) = dup {
            steps.push(Step { action: Action::Duplicate, entry: t.clone(), old_id: f.id.clone(), src, dest: to.resolve_rel(&t.rel_path) });
            continue;
        }
        // `models/<sub>/<name>` → same sub-folder in the target, name made
        // unique. Entries registered elsewhere in Data go to their kind's folder.
        // Missing files get a unique path too, so their entry never shares a
        // path with another file (deleting it would delete that file).
        let in_models = f.rel_path.split(['/', '\\']).next() == Some("models");
        let wanted = if in_models {
            to.resolve_rel(&f.rel_path)
        } else {
            to.models(f.kind).join(src.file_name().unwrap_or_else(|| std::ffi::OsStr::new("model")))
        };
        let dest = free_dest(&wanted, to, &mut taken);
        let mut entry = f.clone();
        entry.rel_path = to.relative(&dest).unwrap_or_else(|| f.rel_path.clone());
        let action = if src.is_file() { Action::Move } else { Action::Missing };
        steps.push(Step { action, entry, old_id: f.id.clone(), src, dest });
    }
    steps
}

/// `wanted`, or a free name next to it in the target (not used by an entry,
/// a file or a `.part`); marks it taken.
fn free_dest(wanted: &Path, to: &DataDir, taken: &mut HashSet<PathBuf>) -> PathBuf {
    let dir = wanted.parent().map(Path::to_path_buf).unwrap_or_else(|| to.models_root());
    let name = wanted.file_name().and_then(|n| n.to_str()).unwrap_or("model").to_string();
    let dest = local::unique_path(&dir, &name, |p| taken.contains(p) || p.exists() || local::part_path(p).exists());
    taken.insert(dest.clone());
    dest
}

/// Same file system (rename works). Windows: same drive/share prefix.
fn same_drive(a: &Path, b: &Path) -> bool {
    let (a, b) = (existing_ancestor(a), existing_ancestor(b));
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(&a), std::fs::metadata(&b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        let prefix = |p: &Path| canon_or(p).components().next().map(|c| c.as_os_str().to_ascii_lowercase());
        prefix(&a) == prefix(&b)
    }
}

fn existing_ancestor(p: &Path) -> PathBuf {
    let mut cur = p;
    loop {
        if cur.exists() {
            return cur.to_path_buf();
        }
        match cur.parent() {
            Some(parent) => cur = parent,
            None => return p.to_path_buf(),
        }
    }
}

enum Done {
    Renamed { src: PathBuf, dest: PathBuf },
    Copied { dest: PathBuf },
}

/// Undo the moves; `false` if something couldn't be put back.
#[must_use]
fn rollback(done: &[Done]) -> bool {
    let mut ok = true;
    for d in done.iter().rev() {
        ok &= match d {
            Done::Renamed { src, dest } => std::fs::rename(dest, src).is_ok(),
            Done::Copied { dest } => std::fs::remove_file(dest).is_ok() || !dest.exists(),
        };
    }
    ok
}

/// `e`, or — when rolling back failed — an error that says where files are.
fn after_rollback(e: CoreError, clean: bool, from: &Path, to: &Path) -> CoreError {
    if clean {
        return e;
    }
    CoreError::new(
        "io",
        format!(
            "Moving your models failed and Pinhole couldn't put every file back. Some model files may be in {} instead of {}. Move them back by hand, then restart Pinhole.",
            to.display(),
            from.display()
        ),
    )
    .with_details(e.details.clone().unwrap_or(e.message))
}

/// Plain message for a failed move (the OS error goes to details).
fn move_error(e: &std::io::Error) -> CoreError {
    let too_big = matches!(e.raw_os_error(), Some(27)) && cfg!(unix) || matches!(e.raw_os_error(), Some(223)) && cfg!(windows);
    let read_only = e.kind() == std::io::ErrorKind::ReadOnlyFilesystem || matches!(e.raw_os_error(), Some(30)) && cfg!(unix);
    let message = if too_big {
        "That drive can't hold files bigger than 4 GB (it's formatted as FAT32). Use a folder on an NTFS or exFAT drive. Your models weren't moved."
    } else if read_only {
        "That folder is read-only right now. If you dual boot with Windows, turn off Fast Startup in Windows and shut it down fully. Your models weren't moved."
    } else if e.kind() == std::io::ErrorKind::StorageFull {
        "The drive filled up while moving. Free some space and try again. Your models weren't moved."
    } else {
        "Pinhole couldn't move your models, so it put everything back. Check that the folder is writable and try again."
    };
    CoreError::new("io", message).with_details(e.to_string())
}

#[cfg(test)]
thread_local! {
    /// Runs on each copied file before it is checked (tests damage the copy).
    static AFTER_COPY: std::cell::Cell<Option<fn(&Path)>> = const { std::cell::Cell::new(None) };
}

fn move_all(core: &AppCore, to: DataDir) -> CoreResult<()> {
    let copy_needed = !same_drive(&core.data.models_root(), &to.models_root());
    move_all_with(core, to, copy_needed)
}

fn move_all_with(core: &AppCore, to: DataDir, copy_needed: bool) -> CoreResult<()> {
    let from = &core.data;
    // `change` checked already; checked again here, where the move happens.
    check_versions(core, &to)?;
    let from_index = core.installed.lock().clone();
    std::fs::create_dir_all(to.models_root())?;
    let to_index = load_target_index(&to)?;
    let steps = plan(from, &from_index, &to, &to_index);

    let moving: Vec<&Step> = steps.iter().filter(|s| s.action == Action::Move).collect();
    let total: u64 = moving.iter().map(|s| s.entry.size_bytes).sum();
    if copy_needed {
        check_free_space(&to.models_root(), total).map_err(|_| {
            CoreError::new("disk_space", "There isn't enough free space in that folder for your models. Free some space or pick another folder.")
        })?;
    }

    // 1. Files.
    let mut done_bytes = 0u64;
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let mut emit = |done: u64, name: &str, force: bool| {
        if force || last_emit.elapsed() >= Duration::from_millis(200) {
            last_emit = Instant::now();
            core.emit(CoreEvent::ModelsMove(ModelsMoveProgress { done_bytes: done, total_bytes: total, file_name: name.to_string() }));
        }
    };
    let mut done: Vec<Done> = Vec::new();
    for s in &moving {
        let name = s.dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        emit(done_bytes, &name, true);
        let res = (|| -> Result<Done, CoreError> {
            if let Some(dir) = s.dest.parent() {
                std::fs::create_dir_all(dir).map_err(|e| move_error(&e))?;
            }
            if !copy_needed && std::fs::rename(&s.src, &s.dest).is_ok() {
                return Ok(Done::Renamed { src: s.src.clone(), dest: s.dest.clone() });
            }
            if !copy_needed {
                // Same drive but the rename failed (e.g. a bind mount): copy instead.
                check_free_space(s.dest.parent().unwrap_or(&s.dest), s.entry.size_bytes).map_err(|_| {
                    CoreError::new("disk_space", "There isn't enough free space in that folder for your models. Free some space or pick another folder.")
                })?;
            }
            let base = done_bytes;
            let mut copied = 0u64;
            let (sha, _) = local::copy_and_hash_with(&s.src, &s.dest, |n| {
                copied += n;
                emit(base + copied, &name, false);
            })
            .map_err(|e| move_error(&e))?;
            #[cfg(test)]
            if let Some(hook) = AFTER_COPY.get() {
                hook(&s.dest);
            }
            // The copy was synced to disk (`sync_all`) before its rename. The
            // original is deleted later, so check what was written, not only
            // what was read: re-read the copy. No recorded hash: compare with
            // the hash computed while copying.
            let recorded = s.entry.sha256.trim().to_ascii_lowercase();
            let expected = if recorded.len() == 64 { recorded } else { sha.clone() };
            let written = local::hash_file(&s.dest).map(|(h, _)| h);
            if sha != expected || written.as_ref().ok() != Some(&expected) {
                let _ = std::fs::remove_file(&s.dest);
                if let Err(e) = written {
                    return Err(move_error(&e));
                }
                return Err(CoreError::new(
                    "io",
                    format!("A copied file didn't match the original ({name}). Your models weren't moved. Check the drive and try again."),
                ));
            }
            Ok(Done::Copied { dest: s.dest.clone() })
        })();
        match res {
            Ok(d) => done.push(d),
            Err(e) => {
                let clean = rollback(&done);
                return Err(after_rollback(e, clean, &from.models_root(), &to.models_root()));
            }
        }
        done_bytes += s.entry.size_bytes;
    }
    emit(total, "", true);

    // 2. Target index: what was there + ours (duplicates keep the target's entry).
    let target_file = to.installed_file();
    let previous_target = std::fs::read(&target_file).ok();
    let mut merged = to_index.clone();
    for s in &steps {
        if s.action != Action::Duplicate {
            merged.upsert(s.entry.clone());
        }
    }
    let restore_target = || match &previous_target {
        Some(bytes) => {
            let _ = pinhole_store::write_atomic(&target_file, bytes);
        }
        None => {
            let _ = std::fs::remove_file(&target_file);
        }
    };
    if let Err(e) = merged.save_to(&to, &target_file) {
        let clean = rollback(&done);
        return Err(after_rollback(e.into(), clean, &from.models_root(), &to.models_root()));
    }

    // 3. Settings (this is what makes the next start use the new folder).
    {
        let mut settings = core.settings.write();
        let mut next = settings.clone();
        next.models_folder = to.models_home.as_ref().map(|p| p.display().to_string());
        if let Err(e) = pinhole_store::settings::save(&core.data, &next) {
            restore_target();
            let clean = rollback(&done);
            return Err(after_rollback(e.into(), clean, &from.models_root(), &to.models_root()));
        }
        *settings = next;
    }

    // 4. Point this install's presets at the target's ids for duplicates.
    let remap: HashMap<&str, &str> =
        steps.iter().filter(|s| s.action == Action::Duplicate && s.old_id != s.entry.id).map(|s| (s.old_id.as_str(), s.entry.id.as_str())).collect();
    if !remap.is_empty() {
        if let Ok(presets) = pinhole_store::presets::list(&core.shipped.presets(), &core.data) {
            for mut p in presets.into_iter().filter(|p| !p.builtin) {
                let mut changed = false;
                let ids = std::iter::once(&mut p.model_id).chain(p.loras.iter_mut().map(|l| &mut l.lora_id));
                for id in ids {
                    if let Some(new_id) = id.as_deref().and_then(|old| remap.get(old)) {
                        *id = Some((*new_id).to_string());
                        changed = true;
                    }
                }
                if changed {
                    let _ = pinhole_store::presets::save(&core.data, p);
                }
            }
        }
    }

    // 5. Clean up: originals of copies and of duplicates, then the old index.
    for (s, d) in moving.iter().zip(&done) {
        if let Done::Copied { .. } = d {
            let _ = std::fs::remove_file(&s.src);
        }
    }
    for s in steps.iter().filter(|s| s.action == Action::Duplicate) {
        if canon_or(&s.src) != canon_or(&s.dest) {
            let _ = std::fs::remove_file(&s.src);
        }
    }
    // Entries this version can't read stay behind with their files (we never
    // move files we don't understand): the old index keeps only them, e.g.
    // for the newer install on the other OS sharing the old folder.
    let old_index = from.installed_file();
    if old_index != target_file {
        if from_index.unknown.is_empty() {
            let _ = std::fs::remove_file(&old_index);
        } else {
            let left = InstalledIndex { files: Vec::new(), ..from_index.clone() };
            let _ = left.save_to(from, &old_index);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_core() -> (tempfile::TempDir, Arc<AppCore>) {
        crate::app::tests::test_core(Arc::new(crate::NullSink))
    }

    fn add(core: &AppCore, rel: &str, bytes: &[u8]) -> InstalledFile {
        let path = core.data.resolve_rel(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        let sha = pinhole_net::download::sha256_file(&path).unwrap();
        let f = InstalledFile {
            id: uuid::Uuid::new_v4().to_string(),
            rel_path: rel.into(),
            kind: pinhole_store::datadir::ModelKind::Checkpoint,
            sha256: sha,
            size_bytes: bytes.len() as u64,
            family: Some("sd15".into()),
            component_id: None,
            friendly_name: rel.into(),
            civitai: None,
            added_at: 0,
            last_used: None,
            observed_vram_gb: None,
            dtype: None,
        };
        let mut idx = core.installed.lock();
        idx.upsert(f.clone());
        idx.save(&core.data).unwrap();
        f
    }

    #[tokio::test]
    async fn moves_models_and_merges_the_shared_index() {
        let (_tmp, core) = test_core();
        let a = add(&core, "models/checkpoints/a.safetensors", b"aaaa");
        let shared = tempfile::tempdir().unwrap();
        // The other install already put `b` and the same bytes as `a` there.
        let other = DataDir::at(PathBuf::from("/unused"), false).with_models_home(Some(shared.path().canonicalize().unwrap()));
        std::fs::create_dir_all(other.models(pinhole_store::datadir::ModelKind::Checkpoint)).unwrap();
        std::fs::write(other.resolve_rel("models/checkpoints/b.safetensors"), b"bbbbbb").unwrap();
        std::fs::write(other.resolve_rel("models/checkpoints/a-other.safetensors"), b"aaaa").unwrap();
        let mut theirs = InstalledIndex::new();
        let mut b = a.clone();
        b.id = "b".into();
        b.rel_path = "models/checkpoints/b.safetensors".into();
        b.sha256 = pinhole_net::download::sha256_file(&other.resolve_rel(&b.rel_path)).unwrap();
        b.size_bytes = 6;
        let mut a2 = a.clone();
        a2.id = "a-other".into();
        a2.rel_path = "models/checkpoints/a-other.safetensors".into();
        theirs.upsert(b);
        theirs.upsert(a2);
        theirs.save(&other).unwrap();

        // A preset of this install that uses `a` as its model and as an add-on.
        let preset = pinhole_store::presets::save(
            &core.data,
            pinhole_store::presets::Preset {
                id: String::new(),
                name: "Mine".into(),
                family: Some("sd15".into()),
                model_id: Some(a.id.clone()),
                civitai_version_id: None,
                style_id: None,
                shape: None,
                quality: None,
                stick: None,
                count: None,
                fine_tune: Default::default(),
                loras: vec![pinhole_store::presets::PresetLora { lora_id: Some(a.id.clone()), civitai_version_id: None, name: "x".into(), weight: 1.0 }],
                builtin: false,
            },
        )
        .unwrap();

        let p = preview(&core, Some(shared.path().to_str().unwrap())).unwrap();
        assert_eq!((p.files, p.bytes, p.existing_models, p.is_default), (1, 0, 2, false));

        change(&core, Some(shared.path().to_string_lossy().into_owned())).await.unwrap();
        // Duplicate: ours removed, theirs kept; settings point at the folder.
        assert!(!core.data.resolve_rel(&a.rel_path).exists());
        assert!(!core.data.default_installed_file().exists());
        let merged = InstalledIndex::load_from(&other.installed_file()).unwrap();
        assert_eq!(merged.files.len(), 2);
        assert_eq!(core.settings.read().models_folder.as_deref(), Some(shared.path().to_str().unwrap()));
        let saved = pinhole_store::settings::load(&core.data).unwrap();
        assert!(saved.models_folder.is_some());
        // The preset now points at the shared folder's entry for the same file.
        let p = pinhole_store::presets::get(&core.shipped.presets(), &core.data, &preset.id).unwrap();
        assert_eq!(p.model_id.as_deref(), Some("a-other"));
        assert_eq!(p.loras[0].lora_id.as_deref(), Some("a-other"));
    }

    #[tokio::test]
    async fn moves_files_and_rewrites_paths() {
        let (_tmp, core) = test_core();
        let a = add(&core, "models/checkpoints/a.safetensors", b"aaaa");
        let shared = tempfile::tempdir().unwrap();
        // A foreign file with the same name must not be overwritten.
        std::fs::create_dir_all(shared.path().join("checkpoints")).unwrap();
        std::fs::write(shared.path().join("checkpoints").join("a.safetensors"), b"not ours").unwrap();
        change(&core, Some(shared.path().to_string_lossy().into_owned())).await.unwrap();
        let to = DataDir::at(core.data.root.clone(), false).with_models_home(Some(shared.path().canonicalize().unwrap()));
        let idx = InstalledIndex::load(&to).unwrap();
        let moved = idx.get(&a.id).unwrap();
        assert_eq!(moved.rel_path, "models/checkpoints/a-2.safetensors");
        assert_eq!(std::fs::read(to.resolve_rel(&moved.rel_path)).unwrap(), b"aaaa");
        assert_eq!(std::fs::read(shared.path().join("checkpoints").join("a.safetensors")).unwrap(), b"not ours");
        assert!(!core.data.resolve_rel(&a.rel_path).exists());
    }

    #[test]
    fn copies_across_drives_and_checks_the_hash() {
        let (_tmp, core) = test_core();
        let a = add(&core, "models/checkpoints/a.safetensors", b"aaaa");
        let mut bad = add(&core, "models/vae/b.safetensors", b"bbbb");
        let shared = tempfile::tempdir().unwrap();
        let to = DataDir::at(core.data.root.clone(), false).with_models_home(Some(shared.path().canonicalize().unwrap()));

        // A wrong recorded hash: the copy is rejected and everything is put back.
        bad.sha256 = "0".repeat(64);
        core.installed.lock().upsert(bad.clone());
        let e = move_all_with(&core, to.clone(), true).unwrap_err();
        assert!(e.message.contains("didn't match"), "{}", e.message);
        assert!(core.data.resolve_rel(&a.rel_path).is_file());
        assert!(core.data.resolve_rel(&bad.rel_path).is_file());
        assert!(!to.resolve_rel(&a.rel_path).exists());
        assert!(!to.installed_file().exists());
        assert!(core.settings.read().models_folder.is_none());

        core.installed.lock().remove(&bad.id);
        move_all_with(&core, to.clone(), true).unwrap();
        assert!(!core.data.resolve_rel(&a.rel_path).exists());
        assert_eq!(std::fs::read(to.resolve_rel(&a.rel_path)).unwrap(), b"aaaa");
        assert!(!local::part_path(&to.resolve_rel(&a.rel_path)).exists());
    }

    #[tokio::test]
    async fn entries_from_a_newer_pinhole_in_the_target_are_kept() {
        let (_tmp, core) = test_core();
        let a = add(&core, "models/checkpoints/a.safetensors", b"aaaa");
        let shared = tempfile::tempdir().unwrap();
        let to = DataDir::at(core.data.root.clone(), false).with_models_home(Some(shared.path().canonicalize().unwrap()));
        let newer = serde_json::json!({"id": "n", "relPath": "models/checkpoints/a.safetensors", "kind": "video"});
        let json = serde_json::json!({"schema_version": 1, "files": [newer.clone()]});
        std::fs::write(to.installed_file(), serde_json::to_vec(&json).unwrap()).unwrap();

        change(&core, Some(shared.path().to_string_lossy().into_owned())).await.unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(to.installed_file()).unwrap()).unwrap();
        assert!(saved["files"].as_array().unwrap().contains(&newer));
        // Its path counts as taken: ours got another name.
        let merged = InstalledIndex::load(&to).unwrap();
        assert_eq!(merged.get(&a.id).unwrap().rel_path, "models/checkpoints/a-2.safetensors");
    }

    #[test]
    fn unreadable_entries_of_our_own_stay_behind() {
        let (_tmp, core) = test_core();
        let a = add(&core, "models/checkpoints/a.safetensors", b"aaaa");
        let v = core.data.resolve_rel("models/video/v.safetensors");
        std::fs::create_dir_all(v.parent().unwrap()).unwrap();
        std::fs::write(&v, b"video").unwrap();
        let newer = serde_json::json!({"id": "v", "relPath": "models/video/v.safetensors", "kind": "video", "extra": 7});
        core.installed.lock().unknown.push(newer.clone());
        let shared = tempfile::tempdir().unwrap();
        let to = DataDir::at(core.data.root.clone(), false).with_models_home(Some(shared.path().canonicalize().unwrap()));

        let p = preview(&core, Some(shared.path().to_str().unwrap())).unwrap();
        assert_eq!(p.files, 1, "only the models this version can move");
        move_all_with(&core, to.clone(), true).unwrap();
        assert_eq!(std::fs::read(to.resolve_rel(&a.rel_path)).unwrap(), b"aaaa");
        // Its file and entry stay where they were: the old index keeps only it.
        assert_eq!(std::fs::read(&v).unwrap(), b"video");
        let old: serde_json::Value = serde_json::from_slice(&std::fs::read(core.data.installed_file()).unwrap()).unwrap();
        assert_eq!(old["files"], serde_json::json!([newer]));
        let moved = InstalledIndex::load(&to).unwrap();
        assert!(moved.unknown.is_empty() && moved.get(&a.id).is_some());
    }

    #[tokio::test]
    async fn a_models_list_from_a_newer_pinhole_is_refused_before_anything_changes() {
        let (_tmp, core) = test_core();
        let a = add(&core, "models/checkpoints/a.safetensors", b"aaaa");
        let shared = tempfile::tempdir().unwrap();
        let to = DataDir::at(core.data.root.clone(), false).with_models_home(Some(shared.path().canonicalize().unwrap()));
        let json = serde_json::json!({"schema_version": 99, "files": []});
        std::fs::write(to.installed_file(), serde_json::to_vec(&json).unwrap()).unwrap();
        let folder = shared.path().to_string_lossy().into_owned();
        for e in [preview(&core, Some(&folder)).unwrap_err(), change(&core, Some(folder.clone())).await.unwrap_err()] {
            assert!(e.message.contains("in that folder was saved by a newer version"), "{}", e.message);
        }
        assert_eq!(std::fs::read(to.installed_file()).unwrap(), serde_json::to_vec(&json).unwrap());
        assert!(core.data.resolve_rel(&a.rel_path).is_file());

        std::fs::remove_file(to.installed_file()).unwrap();
        core.installed.lock().schema_version = pinhole_store::installed::SCHEMA_VERSION + 1;
        let e = change(&core, Some(folder)).await.unwrap_err();
        assert!(e.message.starts_with("Your models list was saved by a newer version"), "{}", e.message);
        assert!(core.data.resolve_rel(&a.rel_path).is_file());
    }

    #[test]
    fn a_copy_damaged_on_write_is_rejected() {
        let (_tmp, core) = test_core();
        let a = add(&core, "models/checkpoints/a.safetensors", b"aaaa");
        let shared = tempfile::tempdir().unwrap();
        let to = DataDir::at(core.data.root.clone(), false).with_models_home(Some(shared.path().canonicalize().unwrap()));
        // The bytes read from the source hash fine, but the written copy differs.
        AFTER_COPY.set(Some(|p: &Path| std::fs::write(p, b"aaab").unwrap()));
        let e = move_all_with(&core, to.clone(), true).unwrap_err();
        AFTER_COPY.set(None);
        assert!(e.message.contains("didn't match"), "{}", e.message);
        assert_eq!(std::fs::read(core.data.resolve_rel(&a.rel_path)).unwrap(), b"aaaa");
        assert!(!to.resolve_rel(&a.rel_path).exists());
        assert!(core.settings.read().models_folder.is_none());
    }

    #[test]
    fn refuses_data_folder_and_same_folder() {
        let (_tmp, core) = test_core();
        let inside = core.data.root.join("outputs");
        assert!(preview(&core, Some(inside.to_str().unwrap())).is_err());
        let default = core.data.models_root();
        assert!(preview(&core, Some(default.to_str().unwrap())).is_err());
        assert!(preview(&core, Some("relative/path")).is_err());
        assert!(preview(&core, Some("/definitely/not/here/pinhole")).is_err());
    }

    #[test]
    fn preview_leaves_a_damaged_shared_index_alone() {
        let (_tmp, core) = test_core();
        add(&core, "models/checkpoints/a.safetensors", b"aaaa");
        let shared = tempfile::tempdir().unwrap();
        let to = DataDir::at(core.data.root.clone(), false).with_models_home(Some(shared.path().to_path_buf()));
        std::fs::write(to.installed_file(), b"{ damaged").unwrap();
        let p = preview(&core, Some(shared.path().to_str().unwrap())).unwrap();
        assert_eq!(p.existing_models, 0);
        assert_eq!(std::fs::read(to.installed_file()).unwrap(), b"{ damaged");
        assert_eq!(std::fs::read_dir(shared.path()).unwrap().count(), 1, "no backup file made");
    }

    #[test]
    fn entries_outside_models_go_to_their_kind_folder() {
        let (_tmp, core) = test_core();
        let a = add(&core, "outputs/odd.safetensors", b"aaaa");
        let shared = tempfile::tempdir().unwrap();
        let to = DataDir::at(core.data.root.clone(), false).with_models_home(Some(shared.path().to_path_buf()));
        let steps = plan(&core.data, &core.installed.lock().clone(), &to, &InstalledIndex::new());
        assert_eq!(steps[0].dest, shared.path().join("checkpoints").join("odd.safetensors"));
        assert_eq!(steps[0].entry.rel_path, "models/checkpoints/odd.safetensors");
        assert_eq!(steps[0].old_id, a.id);
    }

    #[tokio::test]
    async fn refused_while_a_file_is_being_added() {
        let (_tmp, core) = test_core();
        let shared = tempfile::tempdir().unwrap();
        let guard = core.models.folder_lock.try_read().unwrap();
        let e = change(&core, Some(shared.path().to_string_lossy().into_owned())).await.unwrap_err();
        assert!(e.message.contains("adding or deleting"), "{}", e.message);
        drop(guard);
    }

    #[tokio::test]
    async fn pictures_describe_and_updates_wait_for_a_move() {
        let (_tmp, core) = test_core();
        let moving = core.models.folder_lock.try_write().unwrap();
        let req = crate::generate::GenerateRequest::txt2img("m", "a lighthouse");
        let errors = [
            crate::generate::generate(&core, req).await.unwrap_err(),
            crate::generate::upscale_image(&core, "img", 2).await.unwrap_err(),
            crate::describe::describe_image(&core, "img", crate::describe::DescribeStyle::Sentence).await.unwrap_err(),
            crate::update::ensure_idle(&core, None).unwrap_err(),
        ];
        for e in errors {
            assert!(e.message.contains("moving your models"), "{}", e.message);
        }
        drop(moving);
        assert!(crate::update::ensure_idle(&core, None).is_ok());
    }

    #[test]
    fn plan_skips_missing_files() {
        let (_tmp, core) = test_core();
        let a = add(&core, "models/checkpoints/a.safetensors", b"aaaa");
        std::fs::remove_file(core.data.resolve_rel(&a.rel_path)).unwrap();
        let shared = tempfile::tempdir().unwrap();
        let to = DataDir::at(core.data.root.clone(), false).with_models_home(Some(shared.path().to_path_buf()));
        let steps = plan(&core.data, &core.installed.lock().clone(), &to, &InstalledIndex::new());
        assert_eq!(steps[0].action, Action::Missing);
    }

    #[tokio::test]
    async fn missing_entries_never_share_a_path_with_a_file_in_the_target() {
        let (_tmp, core) = test_core();
        let lost = add(&core, "models/checkpoints/a.safetensors", b"aaaa");
        std::fs::remove_file(core.data.resolve_rel(&lost.rel_path)).unwrap();
        // The shared folder already has another install's `a.safetensors`.
        let shared = tempfile::tempdir().unwrap();
        let to = DataDir::at(core.data.root.clone(), false).with_models_home(Some(shared.path().canonicalize().unwrap()));
        std::fs::create_dir_all(to.models(pinhole_store::datadir::ModelKind::Checkpoint)).unwrap();
        std::fs::write(to.resolve_rel("models/checkpoints/a.safetensors"), b"theirs").unwrap();
        let mut theirs = InstalledIndex::new();
        let mut t = lost.clone();
        t.id = "theirs".into();
        t.sha256 = pinhole_net::download::sha256_file(&to.resolve_rel(&t.rel_path)).unwrap();
        t.size_bytes = 6;
        theirs.upsert(t);
        theirs.save(&to).unwrap();

        change(&core, Some(shared.path().to_string_lossy().into_owned())).await.unwrap();
        let merged = InstalledIndex::load(&to).unwrap();
        assert_eq!(merged.files.len(), 2);
        assert_eq!(merged.get(&lost.id).unwrap().rel_path, "models/checkpoints/a-2.safetensors");
        assert_eq!(merged.get("theirs").unwrap().rel_path, "models/checkpoints/a.safetensors");
    }
}
