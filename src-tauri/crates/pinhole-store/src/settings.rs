//! `Data/config/settings.yaml` — app settings only. No prompts, no history.

use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

use crate::{write_atomic, DataDir, StoreError};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub offline: bool,
    /// `auto` | `cpu` | `gpu:<index>`
    pub gpu: String,
    /// Manual VRAM override in GB (None = detected).
    pub vram_override_gb: Option<f32>,
    /// Safe mode default for Browse: `safe` (On) | `all` (Off).
    pub content_mode: String,
    pub show_paid: bool,
    /// `none` | `settings` ("Include generation settings (no prompt)")
    pub saved_metadata: String,
    /// `system` | `light` | `dark`
    pub theme: String,
    /// LoRA trigger words added automatically.
    pub add_trigger_words: bool,
    /// First-run flow finished or skipped.
    pub first_run_done: bool,
    /// Engine backend override: `auto` | `cuda` | `vulkan` | `cpu`
    pub engine_backend: String,
    /// Where the text encoder (reads the prompt) runs on a GPU backend:
    /// `auto` (graphics card; moves to the processor for the rest of the app
    /// session if the card runs out of memory while reading the prompt),
    /// `on` (always the processor), `off` (never moved automatically; family
    /// flags such as `--clip-on-cpu` still apply).
    pub text_encoder_on_cpu: String,
    /// Models folder the user picked (Settings → Models folder), absolute; `None` =
    /// `Data/models/`. Only changed by moving the models (`set_models_folder`
    /// in pinhole-core), never by a plain settings save.
    pub models_folder: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            offline: false,
            gpu: "auto".into(),
            vram_override_gb: None,
            content_mode: "safe".into(),
            show_paid: false,
            saved_metadata: "none".into(),
            theme: "system".into(),
            add_trigger_words: true,
            first_run_done: false,
            engine_backend: "auto".into(),
            text_encoder_on_cpu: "auto".into(),
            models_folder: None,
        }
    }
}

/// Largest VRAM override we accept (GB).
const MAX_VRAM_OVERRIDE_GB: f32 = 1024.0;

impl Settings {
    /// Replace unknown / out-of-range values with their defaults, so a
    /// hand-edited or older file can never put the app in a strange state
    /// (e.g. an unknown `savedMetadata` value falls back to `none`).
    pub fn normalized(mut self) -> Self {
        let d = Settings::default();
        let gpu = self.gpu.trim().to_ascii_lowercase();
        self.gpu = if gpu == "auto" || gpu == "cpu" || gpu_index(&gpu).is_some() { gpu } else { d.gpu };
        self.vram_override_gb = self
            .vram_override_gb
            .filter(|v| v.is_finite() && *v > 0.0)
            .map(|v| v.min(MAX_VRAM_OVERRIDE_GB));
        match self.content_mode.as_str() {
            "safe" | "all" => {}
            // Older builds had two 18+ modes; both meant Safe mode off.
            "include_18plus" | "only_18plus" => self.content_mode = "all".into(),
            _ => self.content_mode = d.content_mode,
        }
        if !matches!(self.saved_metadata.as_str(), "none" | "settings") {
            self.saved_metadata = d.saved_metadata;
        }
        if !matches!(self.theme.as_str(), "system" | "light" | "dark") {
            self.theme = d.theme;
        }
        if !matches!(self.engine_backend.as_str(), "auto" | "cuda" | "vulkan" | "cpu") {
            self.engine_backend = d.engine_backend;
        }
        if !matches!(self.text_encoder_on_cpu.as_str(), "auto" | "on" | "off") {
            self.text_encoder_on_cpu = d.text_encoder_on_cpu;
        }
        self.models_folder = self.models_folder.filter(|p| std::path::Path::new(p.trim()).is_absolute()).map(|p| p.trim().to_string());
        self
    }

    /// `gpu:<index>` → `Some(index)`.
    pub fn gpu_index(&self) -> Option<usize> {
        gpu_index(&self.gpu)
    }

    /// `gpu == "cpu"` (Settings → "Force CPU").
    pub fn force_cpu(&self) -> bool {
        self.gpu == "cpu"
    }
}

fn gpu_index(s: &str) -> Option<usize> {
    let n = s.strip_prefix("gpu:")?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    n.parse().ok()
}

const HEADER: &str = "# Pinhole settings. App preferences only: no prompts, no history.\n";

/// Load settings. A missing, unreadable or corrupt file gives the defaults
/// (never an error); a single bad value falls back to its default without
/// losing the others; unknown fields are ignored.
pub fn load(dir: &DataDir) -> Result<Settings, StoreError> {
    let text = match std::fs::read_to_string(dir.settings_file()) {
        Ok(t) => t,
        Err(_) => return Ok(Settings::default()),
    };
    Ok(parse_lenient(&text))
}

pub fn save(dir: &DataDir, settings: &Settings) -> Result<(), StoreError> {
    let normalized = settings.clone().normalized();
    let body = serde_yaml::to_string(&normalized).map_err(|e| StoreError::Invalid(format!("could not encode settings: {e}")))?;
    write_atomic(&dir.settings_file(), format!("{HEADER}{body}").as_bytes())
}

fn parse_lenient(text: &str) -> Settings {
    let Ok(Value::Mapping(user)) = serde_yaml::from_str::<Value>(text) else {
        return Settings::default();
    };
    let Ok(Value::Mapping(mut merged)) = serde_yaml::to_value(Settings::default()) else {
        return Settings::default();
    };
    for (key, value) in user {
        if !merged.contains_key(&key) {
            continue; // unknown field (newer/older version): ignore
        }
        let mut trial: Mapping = merged.clone();
        trial.insert(key.clone(), value.clone());
        if serde_yaml::from_value::<Settings>(Value::Mapping(trial)).is_ok() {
            merged.insert(key, value);
        }
    }
    serde_yaml::from_value::<Settings>(Value::Mapping(merged)).unwrap_or_default().normalized()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> (tempfile::TempDir, DataDir) {
        let tmp = tempfile::tempdir().unwrap();
        let d = DataDir::at(tmp.path().to_path_buf(), false);
        (tmp, d)
    }

    #[test]
    fn missing_file_gives_defaults() {
        let (_t, d) = data();
        assert_eq!(load(&d).unwrap(), Settings::default());
    }

    #[test]
    fn round_trip() {
        let (_t, d) = data();
        let s = Settings {
            offline: true,
            gpu: "gpu:1".into(),
            vram_override_gb: Some(12.0),
            content_mode: "all".into(),
            show_paid: true,
            saved_metadata: "settings".into(),
            theme: "dark".into(),
            add_trigger_words: false,
            first_run_done: true,
            engine_backend: "vulkan".into(),
            text_encoder_on_cpu: "on".into(),
            models_folder: Some(if cfg!(windows) { r"D:\Shared\Pinhole Models" } else { "/mnt/shared/Pinhole Models" }.into()),
        };
        save(&d, &s).unwrap();
        assert_eq!(load(&d).unwrap(), s);
        let text = std::fs::read_to_string(d.settings_file()).unwrap();
        assert!(text.starts_with("# Pinhole settings"));
        assert!(text.contains("vramOverrideGb: 12"));
        assert!(text.contains("firstRunDone: true"));
    }

    #[test]
    fn corrupt_file_gives_defaults() {
        let (_t, d) = data();
        std::fs::create_dir_all(d.config()).unwrap();
        for junk in ["{{{ not yaml", "- a\n- b\n", "42", "", "\u{0}\u{1}binary"] {
            std::fs::write(d.settings_file(), junk).unwrap();
            assert_eq!(load(&d).unwrap(), Settings::default(), "{junk:?}");
        }
    }

    #[test]
    fn unreadable_path_gives_defaults() {
        let (_t, d) = data();
        // settings.yaml is a directory → read fails → defaults.
        std::fs::create_dir_all(d.settings_file()).unwrap();
        assert_eq!(load(&d).unwrap(), Settings::default());
    }

    #[test]
    fn bad_field_keeps_the_rest_and_unknown_fields_are_ignored() {
        let (_t, d) = data();
        std::fs::create_dir_all(d.config()).unwrap();
        std::fs::write(
            d.settings_file(),
            "offline: maybe\ntheme: dark\nfirstRunDone: true\nsomeFutureField: [1, 2]\nvramOverrideGb: -3\n",
        )
        .unwrap();
        let s = load(&d).unwrap();
        assert!(!s.offline);
        assert_eq!(s.theme, "dark");
        assert!(s.first_run_done);
        assert_eq!(s.vram_override_gb, None);
    }

    #[test]
    fn normalization() {
        let s = Settings {
            gpu: " GPU:2 ".into(),
            content_mode: "everything".into(),
            saved_metadata: "prompt".into(),
            theme: "neon".into(),
            engine_backend: "rocm".into(),
            text_encoder_on_cpu: "maybe".into(),
            vram_override_gb: Some(f32::NAN),
            ..Settings::default()
        }
        .normalized();
        assert_eq!(s.gpu, "gpu:2");
        assert_eq!(s.gpu_index(), Some(2));
        assert_eq!(s.content_mode, "safe");
        assert_eq!(s.saved_metadata, "none");
        assert_eq!(s.theme, "system");
        assert_eq!(s.engine_backend, "auto");
        assert_eq!(s.text_encoder_on_cpu, "auto");
        assert_eq!(s.vram_override_gb, None);
        for old in ["include_18plus", "only_18plus"] {
            let s = Settings { content_mode: old.into(), ..Settings::default() }.normalized();
            assert_eq!(s.content_mode, "all", "{old} → Safe mode off");
        }
        for bad in ["gpu:", "gpu:x", "gpu:-1", "banana"] {
            let s = Settings { gpu: bad.into(), ..Settings::default() }.normalized();
            assert_eq!(s.gpu, "auto", "{bad}");
        }
        assert!(Settings { gpu: "cpu".into(), ..Settings::default() }.normalized().force_cpu());
        assert_eq!(Settings { vram_override_gb: Some(5000.0), ..Settings::default() }.normalized().vram_override_gb, Some(1024.0));
    }

    #[test]
    fn save_normalizes() {
        let (_t, d) = data();
        save(&d, &Settings { saved_metadata: "everything".into(), ..Settings::default() }).unwrap();
        assert_eq!(load(&d).unwrap().saved_metadata, "none");
    }

    #[test]
    fn camel_case_json_shape() {
        let v = serde_json::to_value(Settings::default()).unwrap();
        for key in [
            "offline", "gpu", "vramOverrideGb", "contentMode", "showPaid", "savedMetadata", "theme",
            "addTriggerWords", "firstRunDone", "engineBackend",
        ] {
            assert!(v.get(key).is_some(), "{key}");
        }
    }
}
