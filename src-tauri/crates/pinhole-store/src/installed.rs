//! `Data/catalog/installed.json` — index of installed files: path, sha256,
//! family, CivitAI ids, observed VRAM. Never contains prompts.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::datadir::ModelKind;
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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledIndex {
    pub schema_version: u32,
    pub files: Vec<InstalledFile>,
}

/// Borrowed view used for saving (always writes the current schema version).
#[derive(Serialize)]
struct IndexOut<'a> {
    schema_version: u32,
    files: &'a [InstalledFile],
}

impl InstalledIndex {
    /// Empty index at the current schema version.
    pub fn new() -> Self {
        Self { schema_version: SCHEMA_VERSION, files: Vec::new() }
    }

    /// Load `installed.json`. Missing → empty index. Entries that can't be read
    /// are skipped. If the whole file is damaged it is kept aside as
    /// `installed.json.corrupt-<timestamp>` and an empty index is returned, so
    /// the app still starts (the model files themselves stay on disk and can be
    /// re-added with "Add a file I already have").
    pub fn load(dir: &DataDir) -> Result<Self, StoreError> {
        Self::load_from(&dir.installed_file())
    }

    /// [`InstalledIndex::load`] from an explicit index file. A damaged file is
    /// kept aside as `<file name>.corrupt-<timestamp>`.
    pub fn load_from(path: &Path) -> Result<Self, StoreError> {
        match Self::read_from(path)? {
            Some(index) => Ok(index),
            None => {
                let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "installed.json".into());
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
    pub fn save(&self, dir: &DataDir) -> Result<(), StoreError> {
        self.save_to(dir, &dir.installed_file())
    }

    /// Save to an explicit index file (moving the Models folder). Never creates
    /// a missing user-picked Models folder (an unmounted drive).
    pub fn save_to(&self, dir: &DataDir, path: &Path) -> Result<(), StoreError> {
        if dir.models_home.as_ref().is_some_and(|home| path.starts_with(home) && !home.is_dir()) {
            return Err(StoreError::Invalid(
                "Your Models folder isn't available. Connect or mount the drive it's on and restart Pinhole.".into(),
            ));
        }
        let out = IndexOut { schema_version: SCHEMA_VERSION, files: &self.files };
        let mut json = serde_json::to_vec_pretty(&out)
            .map_err(|e| StoreError::Invalid(format!("could not encode the installed-files index: {e}")))?;
        json.push(b'\n');
        write_atomic(path, &json)
    }

    pub fn get(&self, id: &str) -> Option<&InstalledFile> {
        self.files.iter().find(|f| f.id == id)
    }
    pub fn get_mut(&mut self, id: &str) -> Option<&mut InstalledFile> {
        self.files.iter_mut().find(|f| f.id == id)
    }
    pub fn find_by_sha(&self, sha256: &str) -> Option<&InstalledFile> {
        self.files.iter().find(|f| f.sha256.eq_ignore_ascii_case(sha256))
    }
    pub fn find_component(&self, component_id: &str) -> Option<&InstalledFile> {
        self.files.iter().find(|f| f.component_id.as_deref() == Some(component_id))
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
        Some(self.files.remove(pos))
    }
    /// Absolute path of `file` ([`DataDir::resolve_rel`]): `rel_path` is split
    /// on `/` (and `\`, for hand-edited files); empty, `.`, `..` and drive/root
    /// parts are dropped, so a damaged or hostile index can never point outside
    /// the Data folder or the Models folder.
    pub fn abs_path(&self, dir: &DataDir, file: &InstalledFile) -> PathBuf {
        dir.resolve_rel(&file.rel_path)
    }
    /// Main models: checkpoints + diffusion files.
    pub fn models(&self) -> impl Iterator<Item = &InstalledFile> {
        self.files.iter().filter(|f| matches!(f.kind, ModelKind::Checkpoint | ModelKind::Diffusion))
    }
    pub fn loras(&self) -> impl Iterator<Item = &InstalledFile> {
        self.files.iter().filter(|f| f.kind == ModelKind::Lora)
    }
}

fn parse_lenient(bytes: &[u8]) -> Option<InstalledIndex> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let obj = value.as_object()?;
    let files = match obj.get("files") {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| serde_json::from_value::<InstalledFile>(item.clone()).ok())
            .collect(),
        Some(_) => return None,
    };
    Some(InstalledIndex { schema_version: SCHEMA_VERSION, files })
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
            }),
            added_at: 1_700_000_000,
            last_used: None,
            observed_vram_gb: Some(7.5),
            dtype: Some("fp16".into()),
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
        idx.upsert(file("a", "models/checkpoints/a.safetensors", ModelKind::Checkpoint));
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
        assert_eq!(idx.models().map(|f| f.id.as_str()).collect::<Vec<_>>(), vec!["a", "d"]);
        assert_eq!(idx.loras().count(), 1);
    }

    #[test]
    fn abs_path_joins_components_and_stays_inside() {
        let d = DataDir::at(PathBuf::from("/data"), false);
        let idx = InstalledIndex::new();
        let p = idx.abs_path(&d, &file("a", "models/vae/ae.safetensors", ModelKind::Vae));
        assert_eq!(p, Path::new("/data").join("models").join("vae").join("ae.safetensors"));
        let p = idx.abs_path(&d, &file("a", "models\\loras\\x.safetensors", ModelKind::Lora));
        assert_eq!(p, Path::new("/data").join("models").join("loras").join("x.safetensors"));
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
        let good = serde_json::to_value(file("a", "models/checkpoints/a", ModelKind::Checkpoint)).unwrap();
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
        assert!(InstalledIndex::read_from(&dir.join("absent.json")).unwrap().unwrap().files.is_empty());

        assert!(InstalledIndex::load_from(&shared).unwrap().files.is_empty());
        let names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().any(|n| n.starts_with("pinhole-models.json.corrupt-")), "{names:?}");
        assert!(!names.iter().any(|n| n.starts_with("installed.json")), "{names:?}");
    }
}
