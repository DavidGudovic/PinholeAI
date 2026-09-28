//! Data folder resolution: portable (`Data/` next to the exe, if writable) or
//! installed (`%LOCALAPPDATA%\Pinhole\Data`, `~/.local/share/pinhole/Data`).

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::StoreError;

/// Environment override for the Data folder (tests / CI / power users).
pub const DATA_DIR_ENV: &str = "PINHOLE_DATA_DIR";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataDir {
    pub root: PathBuf,
    pub portable: bool,
}

/// Model sub-folders under `Data/models/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Checkpoint,
    Diffusion,
    TextEncoder,
    Vae,
    Lora,
    Upscaler,
    Captioner,
    Taesd,
}

impl ModelKind {
    pub const ALL: [ModelKind; 8] = [
        ModelKind::Checkpoint,
        ModelKind::Diffusion,
        ModelKind::TextEncoder,
        ModelKind::Vae,
        ModelKind::Lora,
        ModelKind::Upscaler,
        ModelKind::Captioner,
        ModelKind::Taesd,
    ];

    /// Folder name under `Data/models/` (TAESD files live with the VAEs).
    pub fn dir_name(self) -> &'static str {
        match self {
            ModelKind::Checkpoint => "checkpoints",
            ModelKind::Diffusion => "diffusion",
            ModelKind::TextEncoder => "text_encoders",
            ModelKind::Vae | ModelKind::Taesd => "vae",
            ModelKind::Lora => "loras",
            ModelKind::Upscaler => "upscalers",
            ModelKind::Captioner => "captioners",
        }
    }
}

/// Every folder SPEC §3 lists, relative to the Data root.
const LAYOUT: &[&str] = &[
    "models/checkpoints",
    "models/diffusion",
    "models/text_encoders",
    "models/vae",
    "models/loras",
    "models/upscalers",
    "models/captioners",
    "outputs",
    "presets",
    "styles",
    "config",
    "catalog",
    "engine",
];

impl DataDir {
    /// Resolution order:
    /// 1. `PINHOLE_DATA_DIR` (tests / CI), not portable;
    /// 2. portable: `<exe_dir>/Data` exists and is writable (on Linux also next
    ///    to the `.AppImage` file, since `exe_dir` is inside its read-only mount);
    /// 3. installed: `%LOCALAPPDATA%\Pinhole\Data` (Windows),
    ///    `$XDG_DATA_HOME/pinhole/Data` = `~/.local/share/pinhole/Data` (Linux),
    ///    `~/Library/Application Support/Pinhole/Data` (macOS),
    ///    else `~/.pinhole/Data`.
    ///
    /// Does not create anything; call [`DataDir::ensure_layout`] next.
    pub fn resolve(exe_dir: &Path) -> Result<Self, StoreError> {
        let env = std::env::var_os(DATA_DIR_ENV).filter(|v| !v.is_empty()).map(PathBuf::from);
        #[allow(unused_mut)]
        let mut portable = vec![exe_dir.join("Data")];
        #[cfg(target_os = "linux")]
        if let Some(appimage) = std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()) {
            if let Some(dir) = Path::new(&appimage).parent() {
                portable.push(dir.join("Data"));
            }
        }
        let installed = installed_root().or_else(|| dirs::home_dir().map(|h| h.join(".pinhole").join("Data")));
        resolve_with(env, &portable, installed)
    }

    /// Use an explicit root (tests).
    pub fn at(root: PathBuf, portable: bool) -> Self {
        Self { root, portable }
    }

    /// Create the folder layout from SPEC §3 if missing.
    pub fn ensure_layout(&self) -> Result<(), StoreError> {
        fs::create_dir_all(&self.root)?;
        for rel in LAYOUT {
            let dir = rel.split('/').fold(self.root.clone(), |p, c| p.join(c));
            fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    pub fn models(&self, kind: ModelKind) -> PathBuf {
        self.root.join("models").join(kind.dir_name())
    }
    pub fn outputs(&self) -> PathBuf { self.root.join("outputs") }
    pub fn presets(&self) -> PathBuf { self.root.join("presets") }
    pub fn styles(&self) -> PathBuf { self.root.join("styles") }
    pub fn config(&self) -> PathBuf { self.root.join("config") }
    pub fn settings_file(&self) -> PathBuf { self.config().join("settings.yaml") }
    pub fn overrides_file(&self) -> PathBuf { self.config().join("overrides.yaml") }
    pub fn installed_file(&self) -> PathBuf { self.root.join("catalog").join("installed.json") }
    pub fn engine(&self) -> PathBuf { self.root.join("engine") }

    /// `/`-separated path of `path` relative to the Data root (the form stored in
    /// `installed.json`), or `None` if `path` is not inside the Data folder.
    pub fn relative(&self, path: &Path) -> Option<String> {
        let rest = path.strip_prefix(&self.root).ok()?;
        let mut parts = Vec::new();
        for c in rest.components() {
            match c {
                Component::Normal(s) => parts.push(s.to_str()?.to_string()),
                Component::CurDir => {}
                _ => return None,
            }
        }
        (!parts.is_empty()).then(|| parts.join("/"))
    }
}

#[cfg(windows)]
fn installed_root() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("Pinhole").join("Data"))
}

#[cfg(target_os = "macos")]
fn installed_root() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("Pinhole").join("Data"))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn installed_root() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("pinhole").join("Data"))
}

fn resolve_with(env: Option<PathBuf>, portable: &[PathBuf], installed: Option<PathBuf>) -> Result<DataDir, StoreError> {
    if let Some(root) = env {
        let root = if root.is_absolute() { root } else { std::env::current_dir()?.join(root) };
        return Ok(DataDir { root, portable: false });
    }
    if let Some(root) = portable.iter().find(|p| p.is_dir() && is_writable_dir(p)) {
        return Ok(DataDir { root: root.clone(), portable: true });
    }
    installed.map(|root| DataDir { root, portable: false }).ok_or_else(|| {
        StoreError::Invalid(
            "Pinhole couldn't find a folder for its data. Create an empty folder named \"Data\" next to \
             the Pinhole program to use portable mode."
                .into(),
        )
    })
}

/// Probe by creating and deleting a file: permission bits lie (ACLs, read-only
/// media, Program Files virtualization), an actual write doesn't.
fn is_writable_dir(dir: &Path) -> bool {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let probe = dir.join(format!(".pinhole-write-test-{}-{nanos}", std::process::id()));
    match fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(f) => {
            drop(f);
            fs::remove_file(&probe).is_ok()
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let portable = tmp.path().join("exe").join("Data");
        fs::create_dir_all(&portable).unwrap();
        let env = tmp.path().join("ci-data");
        let d = resolve_with(Some(env.clone()), &[portable], Some(tmp.path().join("installed"))).unwrap();
        assert_eq!(d.root, env);
        assert!(!d.portable);
    }

    #[test]
    fn relative_env_override_is_made_absolute() {
        let d = resolve_with(Some(PathBuf::from("rel-data")), &[], None).unwrap();
        assert!(d.root.is_absolute());
        assert!(d.root.ends_with("rel-data"));
    }

    #[test]
    fn portable_when_data_next_to_exe_is_writable() {
        let tmp = tempfile::tempdir().unwrap();
        let portable = tmp.path().join("Data");
        fs::create_dir_all(&portable).unwrap();
        let d = resolve_with(None, std::slice::from_ref(&portable), Some(tmp.path().join("installed"))).unwrap();
        assert_eq!(d.root, portable);
        assert!(d.portable);
        // The probe file is cleaned up.
        assert_eq!(fs::read_dir(&portable).unwrap().count(), 0);
    }

    #[test]
    fn second_portable_candidate_used() {
        let tmp = tempfile::tempdir().unwrap();
        let second = tmp.path().join("appimage-dir").join("Data");
        fs::create_dir_all(&second).unwrap();
        let d = resolve_with(None, &[tmp.path().join("nope").join("Data"), second.clone()], None).unwrap();
        assert_eq!(d.root, second);
        assert!(d.portable);
    }

    #[test]
    fn installed_when_no_portable_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let installed = tmp.path().join("installed").join("Data");
        let d = resolve_with(None, &[tmp.path().join("Data")], Some(installed.clone())).unwrap();
        assert_eq!(d.root, installed);
        assert!(!d.portable);
    }

    #[test]
    fn portable_file_not_dir_is_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let not_dir = tmp.path().join("Data");
        fs::write(&not_dir, "x").unwrap();
        let installed = tmp.path().join("inst");
        let d = resolve_with(None, &[not_dir], Some(installed.clone())).unwrap();
        assert_eq!(d.root, installed);
    }

    #[cfg(unix)]
    #[test]
    fn read_only_portable_folder_is_ignored() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let portable = tmp.path().join("Data");
        fs::create_dir_all(&portable).unwrap();
        fs::set_permissions(&portable, fs::Permissions::from_mode(0o555)).unwrap();
        // Root can write anywhere; only assert when the permission actually bites.
        let writable = is_writable_dir(&portable);
        let installed = tmp.path().join("inst");
        let d = resolve_with(None, std::slice::from_ref(&portable), Some(installed.clone())).unwrap();
        if writable {
            assert_eq!(d.root, portable);
        } else {
            assert_eq!(d.root, installed);
        }
        fs::set_permissions(&portable, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn no_location_is_a_plain_error() {
        let e = resolve_with(None, &[], None).unwrap_err();
        assert!(matches!(e, StoreError::Invalid(_)));
    }

    #[test]
    fn default_installed_root_shape() {
        if let Some(root) = installed_root() {
            assert!(root.ends_with("Data"));
            #[cfg(target_os = "linux")]
            assert!(root.ends_with("pinhole/Data"));
        }
    }

    #[test]
    fn ensure_layout_creates_spec_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let d = DataDir::at(tmp.path().join("Data"), false);
        d.ensure_layout().unwrap();
        d.ensure_layout().unwrap(); // idempotent
        for kind in ModelKind::ALL {
            assert!(d.models(kind).is_dir(), "{kind:?}");
        }
        for p in [d.outputs(), d.presets(), d.styles(), d.config(), d.engine(), d.installed_file().parent().unwrap().to_path_buf()] {
            assert!(p.is_dir(), "{}", p.display());
        }
        for sub in ["checkpoints", "diffusion", "text_encoders", "vae", "loras", "upscalers", "captioners"] {
            assert!(d.root.join("models").join(sub).is_dir());
        }
    }

    #[test]
    fn model_dirs() {
        let d = DataDir::at(PathBuf::from("/data"), false);
        assert_eq!(d.models(ModelKind::Taesd), d.models(ModelKind::Vae));
        assert_eq!(d.models(ModelKind::Checkpoint), PathBuf::from("/data/models/checkpoints"));
        assert_eq!(d.models(ModelKind::TextEncoder), PathBuf::from("/data/models/text_encoders"));
        assert_eq!(d.models(ModelKind::Lora), PathBuf::from("/data/models/loras"));
    }

    #[test]
    fn relative_paths() {
        let d = DataDir::at(PathBuf::from("/data"), false);
        assert_eq!(d.relative(Path::new("/data/models/vae/ae.safetensors")).as_deref(), Some("models/vae/ae.safetensors"));
        assert_eq!(d.relative(Path::new("/elsewhere/x")), None);
        assert_eq!(d.relative(Path::new("/data")), None);
        assert_eq!(d.relative(Path::new("/data/../etc/passwd")), None);
    }
}
