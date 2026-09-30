//! `Data/catalog/installed.json` — index of installed files: path, sha256,
//! family, CivitAI ids, observed VRAM. Never contains prompts.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::datadir::{normalize_rel, ModelKind};
use crate::{write_atomic, DataDir, StoreError};

/// Current `installed.json` schema version.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CivitaiRef {
    pub model_id: u64,
    pub version_id: u64,
    #[serde(default)]
    pub model_name: Option<String>,
    #[serde(default)]
    pub version_name: Option<String>,
    #[serde(default)]
    pub base_model: Option<String>,
    /// LoRA trigger words (`trainedWords`).
    #[serde(default)]
    pub trained_words: Vec<String>,
    #[serde(default)]
    pub license: Option<String>,
    /// The creator's own description (HTML from CivitAI), kept so the model's
    /// page can show it offline. Shown only after the UI sanitizes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_notes: Option<CreatorNotes>,
    /// CivitAI marks the model "safe images only": the image check blocks intimate
    /// results while it (or a LoRA with this mark) is in use (RELEASE-SPEC §4).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sfw_only: bool,
}

/// Most bytes kept per description (CivitAI descriptions are usually a few KB).
pub const MAX_NOTES_BYTES: usize = 20_000;

/// What a model's creator wrote about it: the model page text and the notes of
/// this version. Raw CivitAI HTML; never prompt text.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreatorNotes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The model is made for adults: Safe mode hides these notes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub adult: bool,
}

impl CreatorNotes {
    /// Trims and size-caps both texts; `None` when both are empty.
    pub fn from_html(model: Option<&str>, version: Option<&str>, adult: bool) -> Option<Self> {
        let notes = Self {
            model: clean_notes(model),
            version: clean_notes(version),
            adult,
        };
        (notes.model.is_some() || notes.version.is_some()).then_some(notes)
    }
}

fn clean_notes(text: Option<&str>) -> Option<String> {
    let t = text?.trim();
    if t.is_empty() {
        return None;
    }
    let mut end = t.len().min(MAX_NOTES_BYTES);
    while !t.is_char_boundary(end) {
        end -= 1;
    }
    Some(t[..end].to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledFile {
    /// Stable id (uuid).
    pub id: String,
    /// Path relative to the Data root, `/`-separated.
    pub rel_path: String,
    pub kind: ModelKind,
    pub sha256: String,
    pub size_bytes: u64,
    /// Registry family id (main models and LoRAs).
    #[serde(default)]
    pub family: Option<String>,
    /// Registry component id when this file is a shared component.
    #[serde(default)]
    pub component_id: Option<String>,
    pub friendly_name: String,
    #[serde(default)]
    pub civitai: Option<CivitaiRef>,
    /// Unix seconds.
    pub added_at: i64,
    #[serde(default)]
    pub last_used: Option<i64>,
    /// Observed peak VRAM (GB) after a real run — a number only.
    #[serde(default)]
    pub observed_vram_gb: Option<f32>,
    /// Dominant dtype/quant from the header (`bf16`, `q4_k`…).
    #[serde(default)]
    pub dtype: Option<String>,
    /// LoRA trigger words the user typed in (overrides CivitAI's `trainedWords`;
    /// the only source for add-ons added from disk). Add-on metadata, never prompt text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_words: Option<Vec<String>>,
}

impl InstalledFile {
    /// LoRA trigger words: the user's own list if they set one, else CivitAI's.
    pub fn trigger_words(&self) -> &[String] {
        match (&self.trigger_words, &self.civitai) {
            (Some(w), _) => w,
            (None, Some(c)) => &c.trained_words,
            (None, None) => &[],
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledIndex {
    /// Version of the file as loaded (a newer Pinhole may have written it).
    pub schema_version: u32,
    /// Installed files, plus the files found in the user's other models folders
    /// ([`LinkedState`]; their `rel_path` starts with `linked/<folder id>/`).
    pub files: Vec<InstalledFile>,
    /// Entries this version can't read (e.g. written by a newer Pinhole on the
    /// other OS sharing the Models folder). Kept as they are and saved back.
    #[serde(skip)]
    pub unknown: Vec<serde_json::Value>,
    /// Models folders of other apps (ComfyUI, A1111, Forge) used in place.
    /// Saved to `Data/catalog/linked-folders.json`, never to the shared index.
    #[serde(skip)]
    pub linked: LinkedState,
}

/// First part of the `rel_path` of a file in a linked folder:
/// `linked/<folder id>/<path inside the folder>`.
pub const LINKED_PREFIX: &str = "linked";

/// Where the linked folders and what was found in them are kept (per install:
/// the paths are this computer's).
pub const LINKED_FILE: &str = "linked-folders.json";

/// A models folder of another app, read in place: Pinhole never writes, moves
/// or deletes anything in it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LinkedFolder {
    /// Short random id (the second part of its files' `rel_path`).
    pub id: String,
    /// Absolute path on this computer.
    pub path: String,
    /// Unix seconds.
    #[serde(default)]
    pub added_at: i64,
}

/// Size and modification time of a linked file when it was last looked at: an
/// unchanged file keeps its entry without being read again.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileStamp {
    pub size: u64,
    /// Unix seconds (0 when unknown).
    pub mtime: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LinkedState {
    pub folders: Vec<LinkedFolder>,
    /// File id → stamp, for the entries in `InstalledIndex::files`.
    pub stamps: HashMap<String, FileStamp>,
    /// Entries of folders that aren't available right now (drive not connected):
    /// left out of `files` and saved back as they are.
    pub parked: Vec<LinkedEntry>,
    /// Files found that Pinhole can't use, by `rel_path`: not read (or hashed)
    /// again while unchanged.
    pub not_used: BTreeMap<String, NotUsed>,
}

/// A file in a linked folder Pinhole doesn't use.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NotUsed {
    pub stamp: FileStamp,
    /// A note says CivitAI marks it as showing a real person or someone under 18.
    #[serde(default)]
    pub flagged: bool,
}

impl LinkedState {
    pub fn folder(&self, id: &str) -> Option<&LinkedFolder> {
        self.folders.iter().find(|f| f.id == id)
    }
}

/// One linked file as saved in [`LINKED_FILE`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LinkedEntry {
    pub file: InstalledFile,
    pub stamp: FileStamp,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct LinkedOnDisk {
    #[serde(default)]
    folders: Vec<LinkedFolder>,
    #[serde(default)]
    files: Vec<serde_json::Value>,
    #[serde(default)]
    not_used: BTreeMap<String, NotUsed>,
}

/// `rel_path` for `parts` (the path inside the folder) of linked folder `folder_id`.
pub fn linked_rel_path(folder_id: &str, parts: &[String]) -> String {
    let mut out = format!("{LINKED_PREFIX}/{folder_id}");
    for p in parts {
        out.push('/');
        out.push_str(p);
    }
    out
}

/// A linked folder id: letters, digits and `-` only.
pub fn is_folder_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Id of the linked folder `rel_path` points into, if it does.
pub fn linked_folder_id(rel_path: &str) -> Option<&str> {
    let mut parts = rel_path.split(['/', '\\']).filter(|p| !p.is_empty());
    (parts.next() == Some(LINKED_PREFIX))
        .then(|| parts.next())
        .flatten()
}

impl InstalledFile {
    /// The file is in one of the user's other models folders (read-only for Pinhole).
    pub fn is_linked(&self) -> bool {
        linked_folder_id(&self.rel_path).is_some()
    }
}

/// Borrowed view used for saving (always writes the current schema version).
#[derive(Serialize)]
struct IndexOut<'a> {
    schema_version: u32,
    files: Vec<FileOut<'a>>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum FileOut<'a> {
    Known(&'a InstalledFile),
    Unknown(&'a serde_json::Value),
}

impl InstalledIndex {
    /// Empty index at the current schema version.
    pub fn new() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            files: Vec::new(),
            unknown: Vec::new(),
            linked: LinkedState::default(),
        }
    }

    /// Is `rel_path` used by an entry (readable or not)?
    /// Paths are compared as [`DataDir::resolve_rel`] reads them (backslashes,
    /// `.` and empty parts don't matter).
    pub fn has_rel_path(&self, rel_path: &str) -> bool {
        let want = normalize_rel(rel_path);
        self.rel_paths().any(|p| normalize_rel(p) == want)
    }

    /// Does an entry this version can't read use `rel_path` (compared as
    /// [`DataDir::resolve_rel`] reads it)?
    pub fn unknown_uses(&self, rel_path: &str) -> bool {
        let want = normalize_rel(rel_path);
        self.unknown
            .iter()
            .filter_map(unknown_rel_path)
            .any(|p| normalize_rel(p) == want)
    }

    /// Paths of every entry, including the ones this version can't read.
    pub fn rel_paths(&self) -> impl Iterator<Item = &str> {
        self.files
            .iter()
            .map(|f| f.rel_path.as_str())
            .chain(self.unknown.iter().filter_map(unknown_rel_path))
    }

    /// Load `installed.json`. Missing → empty index. Entries that can't be read
    /// are kept as they are (in `unknown`) and saved back; a file from a newer
    /// schema can't be saved. If the whole file is damaged it is kept aside as
    /// `installed.json.corrupt-<timestamp>` and an empty index is returned, so
    /// the app still starts (the model files themselves stay on disk and can be
    /// re-added with "Add a file I already have").
    pub fn load(dir: &DataDir) -> Result<Self, StoreError> {
        let mut index = Self::load_from(&dir.installed_file())?;
        // Linked files only ever come from `linked-folders.json`.
        index.files.retain(|f| !f.is_linked());
        index.load_linked(&linked_file(dir));
        Ok(index)
    }

    /// Add the linked folders and their files from `path`. Missing or damaged →
    /// none (the folders' files are found again when the user adds them back).
    fn load_linked(&mut self, path: &Path) {
        let Some(disk) = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<LinkedOnDisk>(&b).ok())
        else {
            return;
        };
        let mut folders: Vec<LinkedFolder> = Vec::new();
        for f in disk.folders {
            let usable = is_folder_id(&f.id)
                && Path::new(&f.path).is_absolute()
                && !folders.iter().any(|o| o.id == f.id);
            if usable {
                folders.push(f);
            }
        }
        let available: Vec<&str> = folders
            .iter()
            .filter(|f| Path::new(&f.path).is_dir())
            .map(|f| f.id.as_str())
            .collect();
        let mut seen: HashSet<String> = HashSet::new();
        for value in disk.files {
            let Ok(entry) = serde_json::from_value::<LinkedEntry>(value) else {
                continue;
            };
            let Some(id) = linked_folder_id(&entry.file.rel_path) else {
                continue;
            };
            if !folders.iter().any(|f| f.id == id)
                || self.get(&entry.file.id).is_some()
                || !seen.insert(entry.file.id.clone())
                || !seen.insert(format!("path:{}", normalize_rel(&entry.file.rel_path)))
            {
                continue;
            }
            if available.contains(&id) {
                self.linked
                    .stamps
                    .insert(entry.file.id.clone(), entry.stamp);
                self.files.push(entry.file);
            } else {
                self.linked.parked.push(entry);
            }
        }
        self.linked.not_used = disk
            .not_used
            .into_iter()
            .filter(|(rel, _)| {
                linked_folder_id(rel).is_some_and(|id| folders.iter().any(|f| f.id == id))
            })
            .collect();
        self.linked.folders = folders;
    }

    /// Put the parked entries of linked folder `id` back (its drive is connected
    /// again), so a new look through it keeps their ids.
    pub fn unpark(&mut self, id: &str) {
        let (back, keep): (Vec<LinkedEntry>, Vec<LinkedEntry>) =
            std::mem::take(&mut self.linked.parked)
                .into_iter()
                .partition(|e| linked_folder_id(&e.file.rel_path) == Some(id));
        self.linked.parked = keep;
        for e in back {
            let taken = self.get(&e.file.id).is_some()
                || self.files.iter().any(|f| f.rel_path == e.file.rel_path);
            if !taken {
                self.linked.stamps.insert(e.file.id.clone(), e.stamp);
                self.files.push(e.file);
            }
        }
    }

    /// Save the linked folders and their files (`Data/catalog/linked-folders.json`).
    pub fn save_linked(&self, dir: &DataDir) -> Result<(), StoreError> {
        let path = linked_file(dir);
        if self.linked.folders.is_empty() && !path.exists() {
            return Ok(());
        }
        let files = self
            .files
            .iter()
            .filter(|f| f.is_linked())
            .map(|f| LinkedEntry {
                file: f.clone(),
                stamp: self.linked.stamps.get(&f.id).copied().unwrap_or_default(),
            })
            .chain(self.linked.parked.iter().cloned())
            .filter_map(|e| serde_json::to_value(e).ok())
            .collect();
        let out = LinkedOnDisk {
            folders: self.linked.folders.clone(),
            files,
            not_used: self.linked.not_used.clone(),
        };
        let mut json = serde_json::to_vec_pretty(&out).map_err(|e| {
            StoreError::Invalid(format!("could not encode the linked folders: {e}"))
        })?;
        json.push(b'\n');
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_atomic(&path, &json)
    }

    /// [`InstalledIndex::load`] from an explicit index file. A damaged file is
    /// kept aside as `<file name>.corrupt-<timestamp>`.
    pub fn load_from(path: &Path) -> Result<Self, StoreError> {
        match Self::read_from(path)? {
            Some(index) => Ok(index),
            None => {
                let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "installed.json".into());
                let backup = path.with_file_name(format!("{name}.corrupt-{stamp}"));
                let _ = std::fs::rename(path, &backup);
                Ok(Self::new())
            }
        }
    }

    /// Read an index file without changing anything on disk (previews).
    /// Missing → empty index; damaged → `None`.
    pub fn read_from(path: &Path) -> Result<Option<Self>, StoreError> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Some(Self::new())),
            Err(e) => return Err(e.into()),
        };
        Ok(parse_lenient(&bytes))
    }

    /// Atomic, pretty-printed JSON at schema version 1.
    /// Also saves the linked folders ([`InstalledIndex::save_linked`]).
    pub fn save(&self, dir: &DataDir) -> Result<(), StoreError> {
        self.save_to(dir, &dir.installed_file())?;
        self.save_linked(dir)
    }

    /// Save to an explicit index file (moving the Models folder). Never creates
    /// a missing user-picked Models folder (an unmounted drive). Linked files
    /// are left out: they are saved by [`InstalledIndex::save_linked`].
    pub fn save_to(&self, dir: &DataDir, path: &Path) -> Result<(), StoreError> {
        self.check_savable_to(dir, path)?;
        let files = self
            .files
            .iter()
            .filter(|f| !f.is_linked())
            .map(FileOut::Known)
            .chain(self.unknown.iter().map(FileOut::Unknown))
            .collect();
        let out = IndexOut {
            schema_version: SCHEMA_VERSION,
            files,
        };
        let mut json = serde_json::to_vec_pretty(&out).map_err(|e| {
            StoreError::Invalid(format!("could not encode the installed-files index: {e}"))
        })?;
        json.push(b'\n');
        write_atomic(path, &json)
    }

    /// Would [`InstalledIndex::save`] be refused (newer schema, Models folder
    /// not mounted)? Check before changing files on disk.
    pub fn check_savable(&self, dir: &DataDir) -> Result<(), StoreError> {
        self.check_savable_to(dir, &dir.installed_file())
    }

    fn check_savable_to(&self, dir: &DataDir, path: &Path) -> Result<(), StoreError> {
        if dir
            .models_home
            .as_ref()
            .is_some_and(|home| path.starts_with(home) && !home.is_dir())
        {
            return Err(StoreError::Invalid(
                "Your Models folder isn't available. Connect or mount the drive it's on and restart Pinhole.".into(),
            ));
        }
        if self.is_newer() {
            return Err(StoreError::Invalid(
                "Your models list was saved by a newer version of Pinhole. Update Pinhole to add, move or delete models.".into(),
            ));
        }
        Ok(())
    }

    /// Written by a newer Pinhole (a schema this version can't save).
    pub fn is_newer(&self) -> bool {
        self.schema_version > SCHEMA_VERSION
    }

    /// Drop the unreadable entries at `rel_path` (the file there was replaced)
    /// and return them (to put back if saving fails).
    pub fn remove_unknown_at(&mut self, rel_path: &str) -> Vec<serde_json::Value> {
        let want = normalize_rel(rel_path);
        let (gone, keep) = std::mem::take(&mut self.unknown)
            .into_iter()
            .partition(|v| unknown_rel_path(v).is_some_and(|p| normalize_rel(p) == want));
        self.unknown = keep;
        gone
    }

    pub fn get(&self, id: &str) -> Option<&InstalledFile> {
        self.files.iter().find(|f| f.id == id)
    }
    pub fn get_mut(&mut self, id: &str) -> Option<&mut InstalledFile> {
        self.files.iter_mut().find(|f| f.id == id)
    }
    /// Never matches an empty hash (linked files are hashed only when needed).
    pub fn find_by_sha(&self, sha256: &str) -> Option<&InstalledFile> {
        let sha256 = sha256.trim();
        if sha256.is_empty() {
            return None;
        }
        self.files
            .iter()
            .find(|f| f.sha256.eq_ignore_ascii_case(sha256))
    }
    pub fn find_component(&self, component_id: &str) -> Option<&InstalledFile> {
        self.files
            .iter()
            .find(|f| f.component_id.as_deref() == Some(component_id))
    }
    /// Insert or replace (by id).
    pub fn upsert(&mut self, file: InstalledFile) {
        match self.files.iter_mut().find(|f| f.id == file.id) {
            Some(existing) => *existing = file,
            None => self.files.push(file),
        }
    }
    pub fn remove(&mut self, id: &str) -> Option<InstalledFile> {
        let pos = self.files.iter().position(|f| f.id == id)?;
        self.linked.stamps.remove(id);
        Some(self.files.remove(pos))
    }
    /// Absolute path of `file` ([`DataDir::resolve_rel`]): `rel_path` is split
    /// on `/` (and `\`, for hand-edited files); empty, `.`, `..` and drive/root
    /// parts are dropped, so a damaged or hostile index can never point outside
    /// the Data folder or the Models folder.
    ///
    /// A linked file (`linked/<id>/...`) resolves inside that linked folder, the
    /// same way.
    pub fn abs_path(&self, dir: &DataDir, file: &InstalledFile) -> PathBuf {
        if let Some(folder) = linked_folder_id(&file.rel_path).and_then(|id| self.linked.folder(id))
        {
            let parts = crate::datadir::normalize_rel(&file.rel_path);
            return parts
                .split('/')
                .skip(2)
                .fold(PathBuf::from(&folder.path), |p, c| p.join(c));
        }
        dir.resolve_rel(&file.rel_path)
    }
    /// Main models: checkpoints + diffusion files.
    pub fn models(&self) -> impl Iterator<Item = &InstalledFile> {
        self.files
            .iter()
            .filter(|f| matches!(f.kind, ModelKind::Checkpoint | ModelKind::Diffusion))
    }
    pub fn loras(&self) -> impl Iterator<Item = &InstalledFile> {
        self.files.iter().filter(|f| f.kind == ModelKind::Lora)
    }
}

/// `Data/catalog/linked-folders.json` (always in this install's Data folder).
pub fn linked_file(dir: &DataDir) -> PathBuf {
    dir.root.join("catalog").join(LINKED_FILE)
}

/// `relPath` of an entry this version can't read, if it has one.
pub fn unknown_rel_path(entry: &serde_json::Value) -> Option<&str> {
    entry.get("relPath").and_then(|p| p.as_str())
}

fn parse_lenient(bytes: &[u8]) -> Option<InstalledIndex> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let obj = value.as_object()?;
    let (mut files, mut unknown) = (Vec::new(), Vec::new());
    match obj.get("files") {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::Array(items)) => {
            for item in items {
                match serde_json::from_value::<InstalledFile>(item.clone()) {
                    Ok(f) => files.push(f),
                    Err(_) => unknown.push(item.clone()),
                }
            }
        }
        Some(_) => return None,
    }
    let version = obj
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .map_or(SCHEMA_VERSION, |v| u32::try_from(v).unwrap_or(u32::MAX));
    Some(InstalledIndex {
        schema_version: version.max(SCHEMA_VERSION),
        files,
        unknown,
        linked: LinkedState::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> (tempfile::TempDir, DataDir) {
        let tmp = tempfile::tempdir().unwrap();
        let d = DataDir::at(tmp.path().to_path_buf(), false);
        (tmp, d)
    }

    fn file(id: &str, rel: &str, kind: ModelKind) -> InstalledFile {
        InstalledFile {
            id: id.into(),
            rel_path: rel.into(),
            kind,
            sha256: format!("ABCDEF{id}"),
            size_bytes: 1234,
            family: Some("sdxl".into()),
            component_id: None,
            friendly_name: format!("Model {id}"),
            civitai: Some(CivitaiRef {
                model_id: 1,
                version_id: 2,
                model_name: Some("M".into()),
                version_name: None,
                base_model: Some("SDXL 1.0".into()),
                trained_words: vec!["tw".into()],
                license: None,
                creator_notes: None,
                sfw_only: false,
            }),
            added_at: 1_700_000_000,
            last_used: None,
            observed_vram_gb: Some(7.5),
            dtype: Some("fp16".into()),
            trigger_words: None,
        }
    }

    #[test]
    fn missing_file_is_empty_v1() {
        let (_t, d) = data();
        let idx = InstalledIndex::load(&d).unwrap();
        assert_eq!(idx.schema_version, 1);
        assert!(idx.files.is_empty());
    }

    #[test]
    fn save_load_round_trip() {
        let (_t, d) = data();
        let mut idx = InstalledIndex::default(); // schema 0 in memory
        idx.upsert(file(
            "a",
            "models/checkpoints/a.safetensors",
            ModelKind::Checkpoint,
        ));
        idx.upsert(file("b", "models/loras/b.safetensors", ModelKind::Lora));
        idx.save(&d).unwrap();
        let text = std::fs::read_to_string(d.installed_file()).unwrap();
        assert!(text.contains("\"schema_version\": 1"));
        assert!(text.contains("\"relPath\": \"models/checkpoints/a.safetensors\""));
        assert!(text.contains("\"kind\": \"checkpoint\""));
        let back = InstalledIndex::load(&d).unwrap();
        assert_eq!(back.schema_version, 1);
        assert_eq!(back.files, idx.files);
    }

    #[test]
    fn upsert_replaces_and_remove() {
        let mut idx = InstalledIndex::new();
        idx.upsert(file("a", "x", ModelKind::Checkpoint));
        let mut changed = file("a", "y", ModelKind::Checkpoint);
        changed.last_used = Some(5);
        idx.upsert(changed.clone());
        assert_eq!(idx.files.len(), 1);
        assert_eq!(idx.get("a"), Some(&changed));
        idx.get_mut("a").unwrap().observed_vram_gb = Some(9.0);
        assert_eq!(idx.get("a").unwrap().observed_vram_gb, Some(9.0));
        assert!(idx.remove("nope").is_none());
        assert_eq!(idx.remove("a").unwrap().rel_path, "y");
        assert!(idx.files.is_empty());
    }

    #[test]
    fn lookups() {
        let mut idx = InstalledIndex::new();
        idx.upsert(file("a", "models/checkpoints/a", ModelKind::Checkpoint));
        idx.upsert(file("d", "models/diffusion/d", ModelKind::Diffusion));
        idx.upsert(file("l", "models/loras/l", ModelKind::Lora));
        let mut vae = file("v", "models/vae/ae.safetensors", ModelKind::Vae);
        vae.component_id = Some("flux_ae".into());
        idx.upsert(vae);
        assert_eq!(idx.find_by_sha("abcdefa").unwrap().id, "a");
        assert_eq!(idx.find_component("flux_ae").unwrap().id, "v");
        assert_eq!(
            idx.models().map(|f| f.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "d"]
        );
        assert_eq!(idx.loras().count(), 1);
    }

    #[test]
    fn abs_path_joins_components_and_stays_inside() {
        let d = DataDir::at(PathBuf::from("/data"), false);
        let idx = InstalledIndex::new();
        let p = idx.abs_path(&d, &file("a", "models/vae/ae.safetensors", ModelKind::Vae));
        assert_eq!(
            p,
            Path::new("/data")
                .join("models")
                .join("vae")
                .join("ae.safetensors")
        );
        let p = idx.abs_path(
            &d,
            &file("a", "models\\loras\\x.safetensors", ModelKind::Lora),
        );
        assert_eq!(
            p,
            Path::new("/data")
                .join("models")
                .join("loras")
                .join("x.safetensors")
        );
        let p = idx.abs_path(&d, &file("a", "../../etc/passwd", ModelKind::Vae));
        assert_eq!(p, Path::new("/data").join("etc").join("passwd"));
        let p = idx.abs_path(&d, &file("a", "/abs/./x", ModelKind::Vae));
        assert_eq!(p, Path::new("/data").join("abs").join("x"));
        let p = idx.abs_path(&d, &file("a", "C:/Windows/x", ModelKind::Vae));
        assert_eq!(p, Path::new("/data").join("Windows").join("x"));
    }

    #[test]
    fn bad_entries_are_skipped() {
        let (_t, d) = data();
        std::fs::create_dir_all(d.installed_file().parent().unwrap()).unwrap();
        let good =
            serde_json::to_value(file("a", "models/checkpoints/a", ModelKind::Checkpoint)).unwrap();
        let json = serde_json::json!({
            "schema_version": 1,
            "files": [good, {"id": "broken"}, {"id": "k", "kind": "spaceship"}],
            "futureField": true
        });
        std::fs::write(d.installed_file(), serde_json::to_vec(&json).unwrap()).unwrap();
        let idx = InstalledIndex::load(&d).unwrap();
        assert_eq!(idx.files.len(), 1);
        assert_eq!(idx.files[0].id, "a");
    }

    #[test]
    fn unreadable_entries_survive_a_save() {
        let (_t, d) = data();
        std::fs::create_dir_all(d.installed_file().parent().unwrap()).unwrap();
        let good =
            serde_json::to_value(file("a", "models/checkpoints/a", ModelKind::Checkpoint)).unwrap();
        // E.g. a kind a newer Pinhole on the other OS knows about.
        let newer = serde_json::json!({"id": "k", "relPath": "models/video/k.safetensors", "kind": "video"});
        let json = serde_json::json!({ "schema_version": 1, "files": [good, newer.clone()] });
        std::fs::write(d.installed_file(), serde_json::to_vec(&json).unwrap()).unwrap();

        let mut idx = InstalledIndex::load(&d).unwrap();
        assert_eq!(idx.files.len(), 1);
        assert!(
            idx.has_rel_path("models/video/k.safetensors"),
            "its path counts as taken"
        );
        idx.upsert(file("b", "models/loras/b", ModelKind::Lora));
        idx.save(&d).unwrap();
        let text: serde_json::Value =
            serde_json::from_slice(&std::fs::read(d.installed_file()).unwrap()).unwrap();
        let files = text["files"].as_array().unwrap();
        assert_eq!(files.len(), 3);
        assert!(files.contains(&newer), "{files:?}");
        assert_eq!(InstalledIndex::load(&d).unwrap().files.len(), 2);
    }

    #[test]
    fn a_newer_schema_is_not_overwritten() {
        let (_t, d) = data();
        std::fs::create_dir_all(d.installed_file().parent().unwrap()).unwrap();
        let json = serde_json::json!({ "schema_version": SCHEMA_VERSION + 1, "files": [] });
        let bytes = serde_json::to_vec(&json).unwrap();
        std::fs::write(d.installed_file(), &bytes).unwrap();
        let mut idx = InstalledIndex::load(&d).unwrap();
        idx.upsert(file("a", "models/checkpoints/a", ModelKind::Checkpoint));
        let e = idx.save(&d).unwrap_err();
        assert!(e.to_string().contains("newer version of Pinhole"), "{e}");
        assert_eq!(std::fs::read(d.installed_file()).unwrap(), bytes);
    }

    #[test]
    fn corrupt_file_is_kept_aside() {
        let (_t, d) = data();
        let dir = d.installed_file().parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(d.installed_file(), b"{ this is not json").unwrap();
        let idx = InstalledIndex::load(&d).unwrap();
        assert!(idx.files.is_empty());
        assert!(!d.installed_file().exists());
        let backups: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("installed.json.corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1);
    }

    #[test]
    fn read_from_changes_nothing_and_backups_keep_the_file_name() {
        let (_t, d) = data();
        let dir = d.installed_file().parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let shared = dir.join("pinhole-models.json");
        std::fs::write(&shared, b"{ damaged").unwrap();
        assert!(InstalledIndex::read_from(&shared).unwrap().is_none());
        assert!(shared.exists(), "a preview read leaves the file alone");
        assert!(InstalledIndex::read_from(&dir.join("absent.json"))
            .unwrap()
            .unwrap()
            .files
            .is_empty());

        assert!(InstalledIndex::load_from(&shared).unwrap().files.is_empty());
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names
                .iter()
                .any(|n| n.starts_with("pinhole-models.json.corrupt-")),
            "{names:?}"
        );
        assert!(
            !names.iter().any(|n| n.starts_with("installed.json")),
            "{names:?}"
        );
    }

    #[test]
    fn creator_notes_are_trimmed_and_capped() {
        assert_eq!(CreatorNotes::from_html(Some("  "), None, false), None);
        let n = CreatorNotes::from_html(Some(" <p>hi</p> "), Some(""), false).unwrap();
        assert_eq!(n.model.as_deref(), Some("<p>hi</p>"));
        assert_eq!(n.version, None);
        let long = "é".repeat(MAX_NOTES_BYTES);
        let n = CreatorNotes::from_html(None, Some(&long), false).unwrap();
        assert!(n.version.unwrap().len() <= MAX_NOTES_BYTES);
    }

    #[test]
    fn linked_files_are_kept_apart_and_resolve_in_their_folder() {
        let (tmp, d) = data();
        let other = tmp.path().join("ComfyUI");
        std::fs::create_dir_all(&other).unwrap();
        let mut idx = InstalledIndex::new();
        idx.upsert(file("a", "models/checkpoints/a", ModelKind::Checkpoint));
        idx.linked.folders.push(LinkedFolder {
            id: "f1".into(),
            path: other.display().to_string(),
            added_at: 1,
        });
        let rel = linked_rel_path(
            "f1",
            &["models".into(), "loras".into(), "x.safetensors".into()],
        );
        assert_eq!(rel, "linked/f1/models/loras/x.safetensors");
        let mut l = file("l", &rel, ModelKind::Lora);
        l.sha256 = String::new();
        assert!(l.is_linked() && !idx.get("a").unwrap().is_linked());
        idx.upsert(l.clone());
        idx.linked
            .stamps
            .insert("l".into(), FileStamp { size: 5, mtime: 7 });
        assert_eq!(
            idx.abs_path(&d, &l),
            other.join("models").join("loras").join("x.safetensors")
        );
        // An unhashed file never matches an empty hash.
        assert!(idx.find_by_sha("").is_none());
        idx.save(&d).unwrap();

        let shared = std::fs::read_to_string(d.installed_file()).unwrap();
        assert!(!shared.contains("linked/"), "{shared}");
        let back = InstalledIndex::load(&d).unwrap();
        assert_eq!(back.files.len(), 2);
        assert_eq!(back.get("l"), Some(&l));
        assert_eq!(back.linked.stamps["l"], FileStamp { size: 5, mtime: 7 });
        assert_eq!(back.linked.folders.len(), 1);

        // A folder that isn't there (drive not connected): its files are parked, kept on save.
        std::fs::remove_dir_all(&other).unwrap();
        let parked = InstalledIndex::load(&d).unwrap();
        assert_eq!(parked.files.len(), 1);
        assert_eq!(parked.linked.parked.len(), 1);
        parked.save(&d).unwrap();
        // Put back when the drive is there again (same id, same stamp).
        let mut again = parked.clone();
        again.unpark("f1");
        assert_eq!(again.get("l"), Some(&l));
        assert!(again.linked.parked.is_empty());
        assert_eq!(again.linked.stamps["l"], FileStamp { size: 5, mtime: 7 });
        std::fs::create_dir_all(&other).unwrap();
        assert_eq!(InstalledIndex::load(&d).unwrap().get("l"), Some(&l));

        // A hand-edited installed.json can't claim linked paths, and bad folder ids are dropped.
        let mut shared: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(d.installed_file()).unwrap()).unwrap();
        shared["files"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::to_value(file("z", "linked/f1/evil", ModelKind::Lora)).unwrap());
        std::fs::write(d.installed_file(), serde_json::to_vec(&shared).unwrap()).unwrap();
        let mut linked: serde_json::Value =
            serde_json::from_slice(&std::fs::read(linked_file(&d)).unwrap()).unwrap();
        linked["folders"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"id": "..", "path": "/etc"}));
        std::fs::write(linked_file(&d), serde_json::to_vec(&linked).unwrap()).unwrap();
        let back = InstalledIndex::load(&d).unwrap();
        assert!(back.get("z").is_none());
        assert_eq!(back.linked.folders.len(), 1);
        assert!(!is_folder_id("..") && !is_folder_id("a/b") && is_folder_id("ab12-c"));
    }
}
