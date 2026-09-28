//! Presets (SPEC §7): model + style reference + dial and Fine-tune values +
//! LoRAs. NEVER the prompt and NEVER the negative prompt (CLAUDE.md rule 1).
//!
//! The types below have no prompt fields, and serde drops unknown fields on
//! deserialization, so a `prompt` / `fineTune.negativePrompt` sent by the UI
//! can never reach the file.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::files::{self, builtin_stem, BUILTIN_PREFIX};
use crate::{slugify, write_atomic, DataDir, StoreError};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PresetLora {
    /// Installed LoRA id, or CivitAI version id for one-click install.
    pub lora_id: Option<String>,
    pub civitai_version_id: Option<u64>,
    pub name: String,
    pub weight: f32,
}

/// Fine-tune values that may be stored (no negative prompt!).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct PresetFineTune {
    pub sampler: Option<String>,
    pub scheduler: Option<String>,
    pub steps: Option<u32>,
    pub cfg: Option<f32>,
    pub guidance: Option<f32>,
    pub seed: Option<i64>,
    pub flow_shift: Option<f32>,
    pub clip_skip: Option<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub hires: Option<bool>,
    pub vae_tiling: Option<bool>,
    pub auto_prompt_prefix: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// Family the preset is for.
    pub family: Option<String>,
    /// Installed model id (if any) and CivitAI version for one-click install.
    pub model_id: Option<String>,
    pub civitai_version_id: Option<u64>,
    pub style_id: Option<String>,
    /// `square` | `portrait` | `landscape` | `wide`
    pub shape: Option<String>,
    /// `fast` | `balanced` | `best`
    pub quality: Option<String>,
    pub stick: Option<f32>,
    pub count: Option<u32>,
    #[serde(default)]
    pub fine_tune: PresetFineTune,
    #[serde(default)]
    pub loras: Vec<PresetLora>,
    #[serde(default)]
    pub builtin: bool,
}

const MAX_NAME_CHARS: usize = 120;
const SHAPES: &[&str] = &["square", "portrait", "landscape", "wide"];
const QUALITIES: &[&str] = &["fast", "balanced", "best"];

const HEADER: &str = "# Pinhole preset: model, style and settings. Never contains a prompt. Edit freely.\n";

/// Built-ins (sorted by name) followed by user presets (sorted by name).
/// Files that can't be read are skipped.
pub fn list(builtin_dir: &Path, dir: &DataDir) -> Result<Vec<Preset>, StoreError> {
    let mut builtins = read_all(builtin_dir, true)?;
    let mut users = read_all(&dir.presets(), false)?;
    builtins.sort_by(|a, b| files::name_order(&a.name, &a.id, &b.name, &b.id));
    users.sort_by(|a, b| files::name_order(&a.name, &a.id, &b.name, &b.id));
    builtins.extend(users);
    Ok(builtins)
}

/// Create or update a user preset. Empty `id` → new slug from name
/// (deduplicated); a non-empty `id` must name an existing user preset and is
/// kept. Built-in presets can't be changed. Out-of-range values are dropped
/// (stored as "use the default").
pub fn save(dir: &DataDir, preset: Preset) -> Result<Preset, StoreError> {
    let id = preset.id.trim().to_string();
    if builtin_stem(&id).is_some() {
        return Err(StoreError::Invalid(
            "Built-in presets can't be changed. Save it under a new name to make your own copy.".into(),
        ));
    }
    let mut preset = sanitize(preset)?;
    let presets_dir = dir.presets();

    let (id, fresh) = if id.is_empty() {
        (files::reserve_unique(&presets_dir, &slugify(&preset.name))?, true)
    } else {
        if !files::is_safe_stem(&id) || !presets_dir.join(format!("{id}.yaml")).is_file() {
            return Err(StoreError::NotFound("Preset".into()));
        }
        (id, false)
    };
    preset.id = id;
    preset.builtin = false;

    let path = presets_dir.join(format!("{}.yaml", preset.id));
    let written = files::to_library_yaml(&preset, HEADER).and_then(|yaml| write_atomic(&path, yaml.as_bytes()));
    if let Err(e) = written {
        if fresh {
            let _ = std::fs::remove_file(&path);
        }
        return Err(e);
    }
    Ok(preset)
}

pub fn delete(dir: &DataDir, id: &str) -> Result<(), StoreError> {
    if builtin_stem(id).is_some() {
        return Err(StoreError::Invalid("Built-in presets can't be deleted.".into()));
    }
    if !files::is_safe_stem(id) {
        return Err(StoreError::NotFound("Preset".into()));
    }
    match std::fs::remove_file(dir.presets().join(format!("{id}.yaml"))) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(StoreError::NotFound("Preset".into())),
        Err(e) => Err(e.into()),
    }
}

/// Look up any preset (built-in or user) by id.
pub fn get(builtin_dir: &Path, dir: &DataDir, id: &str) -> Result<Preset, StoreError> {
    let (folder, stem, builtin) = match builtin_stem(id) {
        Some(stem) => (builtin_dir.to_path_buf(), stem, true),
        None => (dir.presets(), id, false),
    };
    if !files::is_safe_stem(stem) {
        return Err(StoreError::NotFound("Preset".into()));
    }
    read_one(&folder.join(format!("{stem}.yaml")), stem, builtin)
}

fn read_all(folder: &Path, builtin: bool) -> Result<Vec<Preset>, StoreError> {
    Ok(files::yaml_files(folder)?
        .into_iter()
        .filter_map(|(stem, path)| read_one(&path, &stem, builtin).ok())
        .collect())
}

fn read_one(path: &Path, stem: &str, builtin: bool) -> Result<Preset, StoreError> {
    let text = files::read_text_or_not_found(path, "Preset")?;
    let mut preset: Preset = serde_yaml::from_str(&text).map_err(|e| files::parse_error(path, e))?;
    preset.id = if builtin { format!("{BUILTIN_PREFIX}{stem}") } else { stem.to_string() };
    preset.builtin = builtin;
    Ok(preset)
}

fn finite(v: Option<f32>) -> Option<f32> {
    v.filter(|x| x.is_finite())
}

fn sanitize(p: Preset) -> Result<Preset, StoreError> {
    let name = p.name.trim().to_string();
    if name.is_empty() {
        return Err(StoreError::Invalid("Give the preset a name.".into()));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(StoreError::Invalid(format!("The preset name is too long (max {MAX_NAME_CHARS} characters).")));
    }
    let ft = p.fine_tune;
    let fine_tune = PresetFineTune {
        sampler: files::clean_opt(ft.sampler),
        scheduler: files::clean_opt(ft.scheduler),
        steps: ft.steps.filter(|s| (1..=500).contains(s)),
        cfg: finite(ft.cfg).filter(|v| *v >= 0.0),
        guidance: finite(ft.guidance).filter(|v| *v >= 0.0),
        seed: ft.seed,
        flow_shift: finite(ft.flow_shift),
        clip_skip: ft.clip_skip.filter(|c| (-1..=12).contains(c)),
        width: ft.width.filter(|w| (64..=8192).contains(w)),
        height: ft.height.filter(|h| (64..=8192).contains(h)),
        hires: ft.hires,
        vae_tiling: ft.vae_tiling,
        auto_prompt_prefix: ft.auto_prompt_prefix,
    };
    let loras = p
        .loras
        .into_iter()
        .filter_map(|l| {
            let name = l.name.trim().to_string();
            let lora_id = files::clean_opt(l.lora_id);
            if name.is_empty() && lora_id.is_none() && l.civitai_version_id.is_none() {
                return None;
            }
            let weight = if l.weight.is_finite() { l.weight.clamp(-10.0, 10.0) } else { 1.0 };
            Some(PresetLora { lora_id, civitai_version_id: l.civitai_version_id, name, weight })
        })
        .collect();
    Ok(Preset {
        id: p.id,
        name,
        family: files::clean_opt(p.family),
        model_id: files::clean_opt(p.model_id),
        civitai_version_id: p.civitai_version_id,
        style_id: files::clean_opt(p.style_id),
        shape: files::clean_opt(p.shape).filter(|s| SHAPES.contains(&s.as_str())),
        quality: files::clean_opt(p.quality).filter(|q| QUALITIES.contains(&q.as_str())),
        stick: finite(p.stick).map(|s| s.clamp(0.0, 1.0)),
        count: p.count.map(|c| c.clamp(1, 4)),
        fine_tune,
        loras,
        builtin: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const SENTINEL: &str = "PINHOLE_SENTINEL_7f3a";

    fn setup() -> (tempfile::TempDir, PathBuf, DataDir) {
        let tmp = tempfile::tempdir().unwrap();
        let builtin = tmp.path().join("config").join("presets");
        std::fs::create_dir_all(&builtin).unwrap();
        std::fs::write(builtin.join("photo-portrait.yaml"), "name: Photo portrait\nfamily: z_image_turbo\nstyleId: builtin:film-photo\nshape: portrait\n").unwrap();
        let data = DataDir::at(tmp.path().join("Data"), false);
        data.ensure_layout().unwrap();
        (tmp, builtin, data)
    }

    fn preset(id: &str, name: &str) -> Preset {
        Preset {
            id: id.into(),
            name: name.into(),
            family: Some("sdxl".into()),
            model_id: Some("m1".into()),
            civitai_version_id: Some(42),
            style_id: Some("builtin:film-photo".into()),
            shape: Some("portrait".into()),
            quality: Some("best".into()),
            stick: Some(0.7),
            count: Some(2),
            fine_tune: PresetFineTune { steps: Some(30), cfg: Some(6.0), seed: Some(-1), ..Default::default() },
            loras: vec![PresetLora { lora_id: Some("l1".into()), civitai_version_id: None, name: "Detail".into(), weight: 0.8 }],
            builtin: false,
        }
    }

    #[test]
    fn shipped_presets_parse_and_reference_builtin_styles() {
        let config = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../config");
        let data = DataDir::at(PathBuf::from("/nonexistent-pinhole-test"), false);
        let presets = list(&config.join("presets"), &data).unwrap();
        let ids: Vec<_> = presets.iter().map(|p| p.id.as_str()).collect();
        for id in ["builtin:photo-portrait", "builtin:anime-illustration", "builtin:product-white"] {
            assert!(ids.contains(&id), "{id} missing from {ids:?}");
        }
        let styles = crate::styles::list(&config.join("styles"), &data).unwrap();
        for p in &presets {
            assert!(p.builtin);
            assert!(p.family.is_some(), "{}", p.id);
            let style_id = p.style_id.as_deref().expect("built-in presets reference a style");
            assert!(styles.iter().any(|s| s.id == style_id), "{} references unknown style {style_id}", p.id);
            // Values survive sanitizing unchanged (they're valid).
            let mut clean = sanitize(p.clone()).unwrap();
            clean.id = p.id.clone();
            clean.builtin = true;
            assert_eq!(&clean, p);
        }
    }

    #[test]
    fn round_trip_and_update() {
        let (_t, builtin, data) = setup();
        let saved = save(&data, preset("", "My SDXL setup")).unwrap();
        assert_eq!(saved.id, "my-sdxl-setup");
        assert_eq!(get(&builtin, &data, &saved.id).unwrap(), saved);
        let again = save(&data, preset("", "My SDXL setup")).unwrap();
        assert_eq!(again.id, "my-sdxl-setup-2");

        let mut changed = saved.clone();
        changed.name = "Renamed".into();
        changed.quality = Some("fast".into());
        let changed = save(&data, changed).unwrap();
        assert_eq!(changed.id, "my-sdxl-setup");
        let all = list(&builtin, &data).unwrap();
        let names: Vec<_> = all.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["Photo portrait", "My SDXL setup", "Renamed"]);
        assert_eq!(all[0].id, "builtin:photo-portrait");
    }

    #[test]
    fn prompt_fields_from_ui_never_reach_disk() {
        let (_t, _b, data) = setup();
        let json = serde_json::json!({
            "id": "",
            "name": "Sneaky",
            "prompt": SENTINEL,
            "negativePrompt": SENTINEL,
            "family": "sdxl",
            "modelId": null,
            "civitaiVersionId": null,
            "styleId": null,
            "shape": "square",
            "quality": "balanced",
            "stick": 0.5,
            "count": 1,
            "fineTune": { "negativePrompt": SENTINEL, "steps": 20, "hiresScale": 1.5 },
            "loras": [],
            "builtin": false
        });
        let p: Preset = serde_json::from_value(json).unwrap();
        let saved = save(&data, p).unwrap();
        let text = std::fs::read_to_string(data.presets().join(format!("{}.yaml", saved.id))).unwrap();
        assert!(!text.contains(SENTINEL));
        assert!(!text.to_lowercase().contains("prompt:"));
        assert!(text.contains("steps: 20"));
        assert!(!serde_json::to_string(&saved).unwrap().contains(SENTINEL));
    }

    #[test]
    fn sanitizing() {
        let (_t, _b, data) = setup();
        let mut p = preset("", "  Odd values ");
        p.shape = Some("hexagon".into());
        p.quality = Some("ultra".into());
        p.stick = Some(3.0);
        p.count = Some(99);
        p.family = Some("   ".into());
        p.fine_tune.cfg = Some(f32::NAN);
        p.fine_tune.steps = Some(0);
        p.fine_tune.width = Some(10);
        p.loras.push(PresetLora { lora_id: None, civitai_version_id: None, name: "  ".into(), weight: 1.0 });
        p.loras.push(PresetLora { lora_id: None, civitai_version_id: Some(7), name: "x".into(), weight: f32::INFINITY });
        let s = save(&data, p).unwrap();
        assert_eq!(s.name, "Odd values");
        assert_eq!(s.shape, None);
        assert_eq!(s.quality, None);
        assert_eq!(s.stick, Some(1.0));
        assert_eq!(s.count, Some(4));
        assert_eq!(s.family, None);
        assert_eq!(s.fine_tune.cfg, None);
        assert_eq!(s.fine_tune.steps, None);
        assert_eq!(s.fine_tune.width, None);
        assert_eq!(s.loras.len(), 2);
        assert_eq!(s.loras[1].weight, 1.0);
        assert!(matches!(save(&data, preset("", " ")), Err(StoreError::Invalid(_))));
    }

    #[test]
    fn builtins_read_only_and_delete() {
        let (_t, builtin, data) = setup();
        let b = get(&builtin, &data, "builtin:photo-portrait").unwrap();
        assert!(b.builtin);
        assert!(matches!(save(&data, b), Err(StoreError::Invalid(_))));
        assert!(matches!(delete(&data, "builtin:photo-portrait"), Err(StoreError::Invalid(ref m)) if m == "Built-in presets can't be deleted."));
        let s = save(&data, preset("", "Mine")).unwrap();
        delete(&data, &s.id).unwrap();
        assert!(matches!(delete(&data, &s.id), Err(StoreError::NotFound(_))));
        assert!(matches!(delete(&data, "../x"), Err(StoreError::NotFound(_))));
        assert!(matches!(save(&data, preset("gone", "Mine")), Err(StoreError::NotFound(_))));
    }

    #[test]
    fn minimal_hand_written_file_loads() {
        let (_t, builtin, data) = setup();
        std::fs::write(data.presets().join("tiny.yaml"), "name: Tiny\n").unwrap();
        std::fs::write(data.presets().join("broken.yaml"), "shape: square\n").unwrap(); // no name → skipped
        let all = list(&builtin, &data).unwrap();
        let tiny = all.iter().find(|p| p.id == "tiny").unwrap();
        assert_eq!(tiny.fine_tune, PresetFineTune::default());
        assert!(tiny.loras.is_empty());
        assert!(!all.iter().any(|p| p.id == "broken"));
    }
}
