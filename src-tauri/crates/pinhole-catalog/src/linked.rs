//! Models folders of other apps (ComfyUI, A1111, Forge, Stability Matrix…),
//! used in place (SPEC §3 "Models from another app"). Pure logic: walking the
//! folder, reading the notes other apps keep next to model files (CivitAI
//! data they fetched earlier), and working out what each file is. Nothing here
//! writes to the folder or goes online.
//!
//! What a file is, in order:
//! - **Style add-ons (LoRAs):** the header says LoRA; the family comes from a
//!   note next to it (CivitAI `baseModel`), else the training metadata inside
//!   the file (`ss_base_model_version`), else a word in its name or folder.
//! - **Main models:** the header's family candidates, narrowed by a note's
//!   `baseModel` / SHA-256, then by words in the file or folder name
//!   (`name_hints` in models.yaml: `pony`, `kontext`, `turbo`…), else the base
//!   family (first candidate).
//! - **Parts (VAE, text encoders):** only a file that is byte-for-byte a part
//!   Pinhole knows (same kind, same size, then same SHA-256) is used; the hash
//!   is the only full read of a file, and only for such size matches.
//!
//! Skipped: anything else (ControlNets, upscalers, embeddings…), and models a
//! note says CivitAI marks as showing a real person or someone under 18
//! (RELEASE-SPEC §5; no lookups: only data the other app already fetched).

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use pinhole_registry::detect::{self, Detection, HeaderInfo};
use pinhole_registry::{Layout, Registry};
use pinhole_store::datadir::ModelKind;
use pinhole_store::installed::CivitaiRef;
use serde_json::Value;

use crate::api::{self, ModelVersion};
use crate::families::{self, FamilyResolution};

/// Deepest folder level looked at below the picked folder.
pub const MAX_DEPTH: usize = 8;
/// Most model files looked at in one folder.
pub const MAX_FILES: usize = 20_000;
/// Most folders visited (a folder with a huge unrelated tree stays quick).
pub const MAX_DIRS: usize = 20_000;
/// Notes next to model files are small JSON; bigger ones are ignored.
const MAX_NOTE_BYTES: u64 = 4 * 1024 * 1024;

/// Folder names never looked into (lower case): tools, outputs, and model
/// kinds Pinhole can't use.
const SKIP_DIRS: &[&str] = &[
    "__pycache__",
    "venv",
    "env",
    "node_modules",
    "custom_nodes",
    "extensions",
    "extensions-builtin",
    "repositories",
    "output",
    "outputs",
    "input",
    "temp",
    "tmp",
    "cache",
    "embeddings",
    "textual_inversion",
    "hypernetworks",
    "controlnet",
    "t2i_adapter",
    "clip_vision",
    "ipadapter",
    "insightface",
    "facerestore_models",
    "facedetection",
    "gfpgan",
    "codeformer",
    "upscale_models",
    "esrgan",
    "realesrgan",
    "swinir",
    "ldsr",
    "sams",
    "ultralytics",
    "style_models",
    "photomaker",
    "gligen",
    "animatediff_models",
    "animatediff_motion_lora",
    "motion_lora",
];

/// A model file found in a linked folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundFile {
    pub abs: PathBuf,
    /// Path inside the picked folder, one name per part.
    pub parts: Vec<String>,
    pub size: u64,
    /// Unix seconds (0 when unknown).
    pub mtime: i64,
}

/// Every `.safetensors` / `.gguf` file under `root` (following links, never
/// twice into the same folder, never into `exclude`, given as canonical
/// paths), sorted by path. Blocking.
pub fn walk(root: &Path, exclude: &[PathBuf]) -> Vec<FoundFile> {
    let mut out = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut dirs = 0usize;
    let mut stack: Vec<(PathBuf, Vec<String>)> = vec![(root.to_path_buf(), Vec::new())];
    while let Some((dir, parts)) = stack.pop() {
        dirs += 1;
        if dirs > MAX_DIRS || out.len() >= MAX_FILES {
            break;
        }
        let canon = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        // Pinhole's own folders (a link to them inside the picked folder).
        if exclude.iter().any(|e| canon.starts_with(e)) || !seen.insert(canon) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            // Follows links (people link their model drives into ComfyUI).
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let mut child = parts.clone();
            child.push(name.clone());
            if meta.is_dir() {
                if parts.len() < MAX_DEPTH && !SKIP_DIRS.contains(&name.to_lowercase().as_str()) {
                    stack.push((path, child));
                }
            } else if meta.is_file() && crate::local::allowed_extension(&path).is_some() {
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
                out.push(FoundFile {
                    abs: path,
                    parts: child,
                    size: meta.len(),
                    mtime,
                });
                if out.len() >= MAX_FILES {
                    break;
                }
            }
        }
    }
    out.sort_by(|a, b| a.parts.cmp(&b.parts));
    out
}

// ------------------------------------------------------------------ notes

/// What another app wrote down about a model file (from CivitAI, earlier).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Note {
    pub sha256: Option<String>,
    pub base_model: Option<String>,
    pub model_id: u64,
    pub version_id: u64,
    pub model_name: Option<String>,
    pub version_name: Option<String>,
    pub trained_words: Vec<String>,
    /// CivitAI marks the model as showing a real person or someone under 18.
    pub person_or_minor: bool,
}

impl Note {
    fn civitai(&self) -> Option<CivitaiRef> {
        (self.model_id > 0 && self.version_id > 0).then(|| CivitaiRef {
            model_id: self.model_id,
            version_id: self.version_id,
            model_name: self.model_name.clone(),
            version_name: self.version_name.clone(),
            base_model: self.base_model.clone(),
            trained_words: self.trained_words.clone(),
            license: None,
            creator_notes: None,
            sfw_only: false,
        })
    }

    fn friendly_name(&self) -> Option<String> {
        let m = self
            .model_name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())?;
        Some(
            match self
                .version_name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
            {
                Some(v) => format!("{m} · {v}"),
                None => m.to_string(),
            },
        )
    }
}

/// Read the notes next to `model` (first one found):
/// - `<name>.civitai.info`: A1111 / Forge "Civitai Helper" (a CivitAI version);
/// - `<name>.metadata.json`: ComfyUI "LoRA Manager" (`sha256`, `base_model`, `civitai`);
/// - `<name>.cm-info.json`: Stability Matrix (`ModelId`, `BaseModel`, `Hashes`…);
/// - `<name>.json`: A1111's own card data (`sd version`, `activation text`).
///
/// Blocking. Damaged or unexpected notes are ignored.
pub fn read_note(model: &Path, size: u64) -> Option<Note> {
    let stem = model.file_stem()?.to_str()?;
    let file_name = model.file_name()?.to_str()?;
    let dir = model.parent()?;
    let read = |suffix: &str| -> Option<Value> {
        let p = dir.join(format!("{stem}{suffix}"));
        let meta = std::fs::metadata(&p).ok()?;
        if !meta.is_file() || meta.len() > MAX_NOTE_BYTES {
            return None;
        }
        serde_json::from_slice(&std::fs::read(p).ok()?).ok()
    };
    if let Some(v) = read(".civitai.info") {
        if let Some(n) = note_from_version(&v, file_name, size) {
            return Some(n);
        }
    }
    if let Some(v) = read(".metadata.json") {
        if let Some(n) = note_from_lora_manager(&v, file_name, size) {
            return Some(n);
        }
    }
    if let Some(v) = read(".cm-info.json") {
        if let Some(n) = note_from_stability_matrix(&v) {
            return Some(n);
        }
    }
    read(".json").and_then(|v| note_from_a1111(&v))
}

/// A CivitAI model version (`/model-versions/{id}` shape).
pub fn note_from_version(v: &Value, file_name: &str, size: u64) -> Option<Note> {
    if !v.is_object() {
        return None;
    }
    let version: ModelVersion = serde_json::from_value(v.clone()).ok()?;
    if version.id == 0 && version.base_model.is_empty() {
        return None;
    }
    // The version can have several files: only the one that is this file
    // (same name or same size) gives its hash.
    let sha256 = version
        .files
        .iter()
        .find(|f| {
            f.name.eq_ignore_ascii_case(file_name)
                || (f.size_kb > 0.0 && (f.size_kb * 1024.0 - size as f64).abs() < 2048.0)
        })
        .and_then(|f| {
            f.hashes
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("sha256"))
                .and_then(|(_, h)| families::normalize_sha(h))
        });
    let model_name = version
        .model
        .as_ref()
        .map(|m| m.name.clone())
        .filter(|n| !n.trim().is_empty());
    Some(Note {
        sha256,
        base_model: Some(version.base_model.clone()).filter(|b| !b.trim().is_empty()),
        model_id: version.model_id,
        version_id: version.id,
        model_name,
        version_name: Some(version.name.clone()).filter(|n| !n.trim().is_empty()),
        trained_words: version.trained_words.clone(),
        person_or_minor: api::version_is_person_or_minor(&version, None),
    })
}

fn str_at<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn u64_at(v: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|k| {
            v.get(*k).and_then(|x| {
                x.as_u64()
                    .or_else(|| x.as_str().and_then(|s| s.trim().parse().ok()))
            })
        })
        .unwrap_or(0)
}

fn strings_at(v: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(Value::as_array))
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// ComfyUI LoRA Manager: `{ sha256, base_model, model_name, civitai: {version} }`.
fn note_from_lora_manager(v: &Value, file_name: &str, size: u64) -> Option<Note> {
    let mut note = v
        .get("civitai")
        .filter(|c| c.is_object())
        .and_then(|c| note_from_version(c, file_name, size))
        .unwrap_or_default();
    if let Some(h) = str_at(v, &["sha256"]).and_then(families::normalize_sha) {
        note.sha256 = Some(h);
    }
    if note.base_model.is_none() {
        note.base_model = str_at(v, &["base_model"]).map(str::to_string);
    }
    if note.model_name.is_none() {
        note.model_name = str_at(v, &["model_name"]).map(str::to_string);
    }
    (note.sha256.is_some() || note.base_model.is_some() || note.version_id > 0).then_some(note)
}

/// Stability Matrix: `{ ModelName, ModelId, VersionName, VersionId, BaseModel,
/// Hashes: { SHA256 }, TrainedWords }`.
fn note_from_stability_matrix(v: &Value) -> Option<Note> {
    let note = Note {
        sha256: v
            .get("Hashes")
            .and_then(|h| str_at(h, &["SHA256", "sha256"]))
            .and_then(families::normalize_sha),
        base_model: str_at(v, &["BaseModel"]).map(str::to_string),
        model_id: u64_at(v, &["ModelId"]),
        version_id: u64_at(v, &["VersionId"]),
        model_name: str_at(v, &["ModelName"]).map(str::to_string),
        version_name: str_at(v, &["VersionName"]).map(str::to_string),
        trained_words: strings_at(v, &["TrainedWords"]),
        person_or_minor: false,
    };
    (note.sha256.is_some() || note.base_model.is_some() || note.version_id > 0).then_some(note)
}

/// A1111's own card data: `{ "sd version": "SDXL" | "SD1" | …, "activation text": "a, b" }`.
fn note_from_a1111(v: &Value) -> Option<Note> {
    let base = match str_at(v, &["sd version"])?.to_ascii_uppercase().as_str() {
        "SD1" => Some("SD 1.5"),
        "SDXL" => Some("SDXL 1.0"),
        _ => None,
    };
    let words: Vec<String> = str_at(v, &["activation text"])
        .map(|t| {
            t.split(',')
                .map(|w| w.trim().to_string())
                .filter(|w| !w.is_empty())
                .collect()
        })
        .unwrap_or_default();
    (base.is_some() || !words.is_empty()).then(|| Note {
        base_model: base.map(str::to_string),
        trained_words: words,
        ..Note::default()
    })
}

// ------------------------------------------------------------------ names

/// Words of a file or folder name, lower case: split at anything that isn't a
/// letter or digit, and where a lower-case letter or digit meets an upper-case one
/// (`ponyDiffusionV6XL` → `pony`, `diffusion`, `v6`, `xl`).
pub fn name_words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur).to_lowercase());
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !cur.is_empty() {
            out.push(std::mem::take(&mut cur).to_lowercase());
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        cur.push(c);
    }
    if !cur.is_empty() {
        out.push(cur.to_lowercase());
    }
    out
}

/// The one candidate whose `name_hints` appear among `words`, if exactly one does.
pub fn hint_pick(registry: &Registry, candidates: &[String], words: &[String]) -> Option<String> {
    let hits: Vec<&String> = candidates
        .iter()
        .filter(|id| {
            registry.family(id).is_some_and(|f| {
                f.name_hints
                    .iter()
                    .any(|h| words.iter().any(|w| w.eq_ignore_ascii_case(h.trim())))
            })
        })
        .collect();
    (hits.len() == 1).then(|| hits[0].clone())
}

/// Family a LoRA's training metadata names (kohya `ss_base_model_version` /
/// `modelspec.architecture`), first family in YAML order.
pub fn lora_family_from_metadata(
    registry: &Registry,
    meta: &BTreeMap<String, String>,
) -> Option<String> {
    let values: Vec<String> = ["ss_base_model_version", "modelspec.architecture"]
        .iter()
        .filter_map(|k| meta.get(*k))
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
        .collect();
    registry
        .families_in_order()
        .find(|f| {
            f.lora_metadata.iter().any(|m| {
                let m = m.trim().to_ascii_lowercase();
                !m.is_empty() && values.iter().any(|v| v.starts_with(&m))
            })
        })
        .map(|f| f.id.clone())
}

/// Words of the file name, then of the folders it is in (nearest first).
fn word_groups(parts: &[String]) -> Vec<Vec<String>> {
    let mut groups = Vec::new();
    if let Some((file, dirs)) = parts.split_last() {
        let stem = file.rsplit_once('.').map_or(file.as_str(), |(s, _)| s);
        groups.push(name_words(stem));
        for d in dirs.iter().rev() {
            groups.push(name_words(d));
        }
    }
    groups
}

/// "cool_model-v2" → "cool model v2".
pub fn plain_name(file_name: &str) -> String {
    let stem = file_name.rsplit_once('.').map_or(file_name, |(s, _)| s);
    let s = stem.replace(['_', '-'], " ").trim().to_string();
    if s.is_empty() {
        "Model".into()
    } else {
        s
    }
}

// ------------------------------------------------------------------ classify

/// What a linked file turned out to be.
#[derive(Debug, Clone, PartialEq)]
pub struct Recognised {
    pub kind: ModelKind,
    pub family: Option<String>,
    pub component_id: Option<String>,
    pub friendly_name: String,
    pub dtype: Option<String>,
    /// Empty when the file wasn't hashed (the usual case).
    pub sha256: String,
    pub civitai: Option<CivitaiRef>,
}

/// Why a file in a linked folder isn't listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skipped {
    /// Not a model Pinhole can run (ControlNet, upscaler, unknown part…).
    NotUsable,
    /// CivitAI marks it as showing a real person or someone under 18.
    PersonOrMinor,
}

/// Components `detection` could be, by kind and size (sizes in models.yaml are
/// rounded to the MB).
pub fn component_candidates<'a>(
    registry: &'a Registry,
    detection: &Detection,
    size: u64,
) -> Vec<(&'a String, &'a pinhole_registry::Component)> {
    let Some(kind) = detection.is_component.as_deref() else {
        return Vec::new();
    };
    registry
        .components()
        .iter()
        // The upscaler is Pinhole's own pinned download, never taken from elsewhere.
        .filter(|(_, c)| c.kind == kind && c.kind != "upscaler")
        .filter(|(_, c)| families::normalize_sha(&c.sha256).is_some())
        .filter(|(_, c)| (size as f64 - c.size_mb as f64 * 1e6).abs() <= 2.5e6)
        .collect()
}

/// Work out what `found` is from its header, the note next to it and its
/// name. `hash` returns the file's SHA-256 (it reads the whole file). Every usable file is
/// hashed: parts to match them, models and add-ons for the CivitAI lookup (a note's own hash
/// isn't trusted for that). Blocking only through `hash`.
pub fn recognise(
    registry: &Registry,
    found: &FoundFile,
    header: &HeaderInfo,
    note: Option<&Note>,
    hash: &mut dyn FnMut(&Path) -> Option<String>,
) -> Result<Recognised, Skipped> {
    if note.is_some_and(|n| n.person_or_minor) {
        return Err(Skipped::PersonOrMinor);
    }
    let file_name = found.parts.last().map(String::as_str).unwrap_or("model");
    let detection = detect::detect(registry, header);
    let dtype = Some(detection.dtype.clone()).filter(|d| !d.is_empty());
    let named = note
        .and_then(Note::friendly_name)
        .unwrap_or_else(|| plain_name(file_name));
    let civitai = note.and_then(Note::civitai);

    if detection.is_component.is_some() {
        let candidates = component_candidates(registry, &detection, found.size);
        if candidates.is_empty() {
            return Err(Skipped::NotUsable);
        }
        let sha = hash(&found.abs).ok_or(Skipped::NotUsable)?;
        let (id, comp) = candidates
            .into_iter()
            .find(|(_, c)| families::normalize_sha(&c.sha256).as_deref() == Some(sha.as_str()))
            .ok_or(Skipped::NotUsable)?;
        return Ok(Recognised {
            kind: families::component_model_kind(&comp.kind),
            family: None,
            component_id: Some(id.clone()),
            friendly_name: families::component_label(comp),
            dtype,
            sha256: sha,
            civitai: None,
        });
    }

    let groups = word_groups(&found.parts);
    if detection.is_lora {
        let family = note
            .and_then(|n| n.base_model.as_deref())
            .and_then(
                |b| match families::resolve_family(registry, None, Some(b), None) {
                    FamilyResolution::Resolved(f) => Some(f),
                    FamilyResolution::Ambiguous(c) => c.into_iter().next(),
                    FamilyResolution::Unsupported(_) => None,
                },
            )
            .or_else(|| lora_family_from_metadata(registry, &header.metadata))
            .or_else(|| {
                // A word naming a family's own hint ("pony", "flux"…): only the
                // architecture matters for an add-on.
                let all: Vec<String> = registry.families_in_order().map(|f| f.id.clone()).collect();
                groups.iter().find_map(|w| hint_pick(registry, &all, w))
            });
        let sha256 = hash(&found.abs).ok_or(Skipped::NotUsable)?;
        return Ok(Recognised {
            kind: ModelKind::Lora,
            family,
            component_id: None,
            friendly_name: named,
            dtype,
            sha256,
            civitai,
        });
    }

    // A main model: the header must match a family (ControlNets and the like don't).
    if detection.candidates.is_empty() {
        return Err(Skipped::NotUsable);
    }
    let candidates = detection.candidates.clone();
    let sha256 = hash(&found.abs).ok_or(Skipped::NotUsable)?;
    let resolution = families::resolve_family(
        registry,
        Some(sha256.as_str()),
        note.and_then(|n| n.base_model.as_deref()),
        Some(&candidates),
    );
    let family = match resolution {
        // The header wins over a note that names another architecture.
        FamilyResolution::Resolved(f) if candidates.contains(&f) => f,
        FamilyResolution::Resolved(_) => groups
            .iter()
            .find_map(|w| hint_pick(registry, &candidates, w))
            .unwrap_or_else(|| candidates[0].clone()),
        FamilyResolution::Ambiguous(list) => groups
            .iter()
            .find_map(|w| hint_pick(registry, &list, w))
            .or_else(|| list.first().cloned())
            .ok_or(Skipped::NotUsable)?,
        // The note names a base model Pinhole can't run.
        FamilyResolution::Unsupported(_) => return Err(Skipped::NotUsable),
    };
    let friendly_name = registry
        .known_file(&sha256)
        .and_then(|k| k.friendly_name.clone())
        .filter(|_| note.and_then(Note::friendly_name).is_none())
        .unwrap_or(named);
    Ok(Recognised {
        kind: match detection.layout {
            Layout::AllInOne => ModelKind::Checkpoint,
            Layout::DiffusionOnly => ModelKind::Diffusion,
        },
        family: Some(family),
        component_id: None,
        friendly_name,
        dtype,
        sha256,
        civitai,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split_names() {
        assert_eq!(
            name_words("ponyDiffusionV6XL_v6"),
            vec!["pony", "diffusion", "v6", "xl", "v6"]
        );
        assert_eq!(name_words("NoobAI-XL"), vec!["noob", "ai", "xl"]);
        assert_eq!(
            name_words("flux1-kontext-dev_Q4_K_M"),
            vec!["flux1", "kontext", "dev", "q4", "k", "m"]
        );
        assert_eq!(
            name_words("sdxl_lightning_4step"),
            vec!["sdxl", "lightning", "4step"]
        );
        assert_eq!(name_words("PDXL"), vec!["pdxl"]);
        assert_eq!(name_words("drawingStyle"), vec!["drawing", "style"]);
    }

    #[test]
    fn plain_names() {
        assert_eq!(plain_name("cool_model-v2.safetensors"), "cool model v2");
        assert_eq!(plain_name("_.gguf"), "Model");
    }

    #[test]
    fn civitai_helper_note() {
        let v = serde_json::json!({
            "id": 22, "modelId": 11, "name": "v6", "baseModel": "Pony",
            "trainedWords": ["score_9"],
            "model": {"name": "Pony Diffusion", "type": "Checkpoint", "poi": false},
            "files": [
                {"name": "other.safetensors", "sizeKB": 10.0, "hashes": {"SHA256": "AA"}},
                {"name": "pony.safetensors", "sizeKB": 1.0,
                 "hashes": {"SHA256": "67AB2FD8EC439A89B3FEDB15CC65F54336AF163C7EB5E4F2ACC98F090A29B0B3"}}
            ]
        });
        let n = note_from_version(&v, "pony.safetensors", 999_999).unwrap();
        assert_eq!(n.base_model.as_deref(), Some("Pony"));
        assert_eq!(
            n.sha256.as_deref(),
            Some("67ab2fd8ec439a89b3fedb15cc65f54336af163c7eb5e4f2acc98f090a29b0b3")
        );
        assert_eq!(n.friendly_name().as_deref(), Some("Pony Diffusion · v6"));
        assert!(!n.person_or_minor);
        assert_eq!(n.civitai().unwrap().version_id, 22);

        // A renamed file: matched by size instead.
        let n = note_from_version(&v, "renamed.safetensors", 10 * 1024).unwrap();
        assert_eq!(n.sha256, None, "AA isn't a SHA-256");

        let mut flagged = v.clone();
        flagged["model"]["poi"] = serde_json::json!(true);
        assert!(
            note_from_version(&flagged, "pony.safetensors", 0)
                .unwrap()
                .person_or_minor
        );
        assert!(note_from_version(&serde_json::json!([]), "x", 0).is_none());
    }

    #[test]
    fn other_apps_notes() {
        let sm = serde_json::json!({
            "ModelName": "M", "ModelId": 5, "VersionName": "V", "VersionId": "6",
            "BaseModel": "SDXL 1.0", "Hashes": {"SHA256": "ab".repeat(32)}, "TrainedWords": ["tw"]
        });
        let n = note_from_stability_matrix(&sm).unwrap();
        assert_eq!((n.model_id, n.version_id), (5, 6));
        assert_eq!(n.sha256, Some("ab".repeat(32)));
        assert_eq!(n.trained_words, vec!["tw"]);

        let lm = serde_json::json!({"sha256": "CD".repeat(32), "base_model": "Illustrious", "model_name": "X"});
        let n = note_from_lora_manager(&lm, "x.safetensors", 1).unwrap();
        assert_eq!(n.base_model.as_deref(), Some("Illustrious"));
        assert_eq!(n.sha256, Some("cd".repeat(32)));
        assert_eq!(n.civitai(), None);

        let a = serde_json::json!({"sd version": "SDXL", "activation text": "red hat, , blue"});
        let n = note_from_a1111(&a).unwrap();
        assert_eq!(n.base_model.as_deref(), Some("SDXL 1.0"));
        assert_eq!(n.trained_words, vec!["red hat", "blue"]);
        assert!(note_from_a1111(&serde_json::json!({"sd version": "Unknown"})).is_none());
    }

    #[test]
    fn walk_finds_models_and_skips_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let mk = |rel: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        };
        mk("models/checkpoints/a.safetensors");
        mk("models/loras/sub/b.SAFETENSORS");
        mk("models/unet/c.gguf");
        mk("models/checkpoints/old.ckpt");
        mk("models/controlnet/cn.safetensors");
        mk("custom_nodes/x/model.safetensors");
        mk(".git/y.safetensors");
        let found = walk(root, &[]);
        let rels: Vec<String> = found.iter().map(|f| f.parts.join("/")).collect();
        assert_eq!(
            rels,
            vec![
                "models/checkpoints/a.safetensors",
                "models/loras/sub/b.SAFETENSORS",
                "models/unet/c.gguf"
            ]
        );
        assert_eq!(found[0].size, 1);
        // Pinhole's own folder inside the picked one is left out.
        let own = root.join("models/unet").canonicalize().unwrap();
        assert_eq!(walk(root, &[own]).len(), 2);
    }

    #[test]
    fn a_note_is_read_next_to_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let model = tmp.path().join("m.safetensors");
        std::fs::write(&model, b"x").unwrap();
        assert!(read_note(&model, 1).is_none());
        std::fs::write(
            tmp.path().join("m.civitai.info"),
            serde_json::to_vec(&serde_json::json!({"id": 1, "modelId": 2, "baseModel": "SD 1.5"}))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            read_note(&model, 1).unwrap().base_model.as_deref(),
            Some("SD 1.5")
        );
    }

    // ---------------------------------------------------------------- recognise

    fn header(tensors: &[(&str, &str, &[u64])], metadata: &[(&str, &str)]) -> HeaderInfo {
        let mut obj = serde_json::Map::new();
        let mut offset = 0u64;
        for (name, dtype, shape) in tensors {
            let n: u64 = shape.iter().product::<u64>().max(1) * 2;
            obj.insert(
                (*name).into(),
                serde_json::json!({ "dtype": dtype, "shape": shape, "data_offsets": [offset, offset + n] }),
            );
            offset += n;
        }
        if !metadata.is_empty() {
            let m: serde_json::Map<String, Value> = metadata
                .iter()
                .map(|(k, v)| ((*k).to_string(), Value::from(*v)))
                .collect();
            obj.insert("__metadata__".into(), Value::Object(m));
        }
        let h = serde_json::to_vec(&Value::Object(obj)).unwrap();
        let mut bytes = (h.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(&h);
        let size = bytes.len() as u64 + offset;
        detect::parse_header_bytes(&bytes, size).unwrap()
    }

    fn sdxl() -> HeaderInfo {
        header(
            &[
                ("model.diffusion_model.input_blocks.0.0.weight", "F16", &[320, 4, 3, 3]),
                ("model.diffusion_model.middle_block.1.norm.weight", "F16", &[1280]),
                (
                    "conditioner.embedders.0.transformer.text_model.embeddings.token_embedding.weight",
                    "F16",
                    &[49408, 768],
                ),
                ("conditioner.embedders.1.model.token_embedding.weight", "F16", &[49408, 1280]),
                ("first_stage_model.encoder.conv_in.weight", "F16", &[128, 3, 3, 3]),
            ],
            &[],
        )
    }

    fn found(rel: &str, size: u64) -> FoundFile {
        FoundFile {
            abs: PathBuf::from("/x").join(rel),
            parts: rel.split('/').map(str::to_string).collect(),
            size,
            mtime: 0,
        }
    }

    fn no_hash(_: &Path) -> Option<String> {
        panic!("only parts are hashed")
    }

    #[test]
    fn main_models_get_a_family_from_notes_and_names() {
        let reg = crate::testkit::registry();
        let family = |rel: &str, note: Option<&Note>| {
            recognise(&reg, &found(rel, 1), &sdxl(), note, &mut no_hash)
                .unwrap()
                .family
                .unwrap()
        };
        assert_eq!(
            family("checkpoints/juggernautXL_v9.safetensors", None),
            "sdxl"
        );
        assert_eq!(
            family("Stable-diffusion/ponyDiffusionV6XL.safetensors", None),
            "sdxl_pony"
        );
        assert_eq!(
            family("checkpoints/Pony/someMix_v2.safetensors", None),
            "sdxl_pony"
        );
        assert_eq!(
            family("checkpoints/noobaiXL_vPred.safetensors", None),
            "sdxl_illustrious"
        );
        let note = Note {
            base_model: Some("Illustrious".into()),
            ..Note::default()
        };
        assert_eq!(
            family("checkpoints/x.safetensors", Some(&note)),
            "sdxl_illustrious"
        );
        // A note naming another architecture doesn't override the header.
        let flux = Note {
            base_model: Some("Flux.1 D".into()),
            ..Note::default()
        };
        assert_eq!(family("checkpoints/x.safetensors", Some(&flux)), "sdxl");

        let r = recognise(
            &reg,
            &found("checkpoints/a_b.safetensors", 1),
            &sdxl(),
            None,
            &mut no_hash,
        )
        .unwrap();
        assert_eq!(r.kind, ModelKind::Checkpoint);
        assert_eq!(r.friendly_name, "a b");
        assert!(r.sha256.is_empty());

        let flux_dev = header(
            &[
                ("double_blocks.0.img_attn.qkv.weight", "BF16", &[9216, 3072]),
                ("single_blocks.0.linear1.weight", "BF16", &[21504, 3072]),
                ("img_in.weight", "BF16", &[3072, 64]),
                ("txt_in.weight", "BF16", &[3072, 4096]),
                ("guidance_in.in_layer.weight", "BF16", &[3072, 256]),
            ],
            &[],
        );
        let pick = |rel: &str| {
            let r = recognise(&reg, &found(rel, 1), &flux_dev, None, &mut no_hash).unwrap();
            assert_eq!(r.kind, ModelKind::Diffusion);
            r.family.unwrap()
        };
        assert_eq!(pick("unet/flux1-kontext-dev.safetensors"), "flux1_kontext");
        assert_eq!(pick("diffusion_models/flux1-dev.safetensors"), "flux1_dev");
    }

    #[test]
    fn loras_parts_and_skips() {
        let reg = crate::testkit::registry();
        let lora = |meta: &[(&str, &str)]| {
            header(
                &[
                    (
                        "lora_unet_input_blocks_1_1_proj_in.lora_down.weight",
                        "F16",
                        &[8, 320],
                    ),
                    (
                        "lora_unet_input_blocks_1_1_proj_in.lora_up.weight",
                        "F16",
                        &[320, 8],
                    ),
                    ("lora_unet_input_blocks_1_1_proj_in.alpha", "F16", &[]),
                ],
                meta,
            )
        };
        let r = recognise(
            &reg,
            &found("loras/x.safetensors", 1),
            &lora(&[("ss_base_model_version", "sdxl_base_v1-0")]),
            None,
            &mut no_hash,
        )
        .unwrap();
        assert_eq!(r.kind, ModelKind::Lora);
        assert_eq!(r.family.as_deref(), Some("sdxl"));
        let r = recognise(
            &reg,
            &found("Lora/pony/y.safetensors", 1),
            &lora(&[]),
            None,
            &mut no_hash,
        )
        .unwrap();
        assert_eq!(r.family.as_deref(), Some("sdxl_pony"));
        let r = recognise(
            &reg,
            &found("Lora/z.safetensors", 1),
            &lora(&[]),
            None,
            &mut no_hash,
        )
        .unwrap();
        assert_eq!(r.family, None);
        let note = Note {
            base_model: Some("SD 1.5".into()),
            model_id: 3,
            version_id: 4,
            model_name: Some("Hat".into()),
            trained_words: vec!["red hat".into()],
            ..Note::default()
        };
        let r = recognise(
            &reg,
            &found("Lora/z.safetensors", 1),
            &lora(&[]),
            Some(&note),
            &mut no_hash,
        )
        .unwrap();
        assert_eq!(r.family.as_deref(), Some("sd15"));
        assert_eq!(r.friendly_name, "Hat");
        assert_eq!(r.civitai.unwrap().trained_words, vec!["red hat"]);

        let flagged = Note {
            person_or_minor: true,
            ..note
        };
        assert_eq!(
            recognise(
                &reg,
                &found("Lora/z.safetensors", 1),
                &lora(&[]),
                Some(&flagged),
                &mut no_hash
            ),
            Err(Skipped::PersonOrMinor)
        );

        // A VAE that is byte-for-byte the FLUX one; hashed only when the size matches.
        let vae = header(
            &[
                (
                    "encoder.down.0.block.0.conv1.weight",
                    "F32",
                    &[128, 128, 3, 3],
                ),
                (
                    "decoder.up.0.block.0.conv1.weight",
                    "F32",
                    &[4096, 4096, 3, 3],
                ),
                ("decoder.conv_out.weight", "F32", &[3, 128, 3, 3]),
            ],
            &[],
        );
        let flux_ae = reg.component("flux_ae").unwrap();
        let size = flux_ae.size_mb * 1_000_000;
        let sha = flux_ae.sha256.clone();
        let mut hashed = 0;
        let r = recognise(
            &reg,
            &found("vae/ae.safetensors", size),
            &vae,
            None,
            &mut |_| {
                hashed += 1;
                Some(sha.clone())
            },
        )
        .unwrap();
        assert_eq!(hashed, 1);
        assert_eq!(r.component_id.as_deref(), Some("flux_ae"));
        assert_eq!(r.kind, ModelKind::Vae);
        assert_eq!(r.sha256, sha);
        // Same size, other bytes: not used.
        assert_eq!(
            recognise(
                &reg,
                &found("vae/ae.safetensors", size),
                &vae,
                None,
                &mut |_| Some("0".repeat(64))
            ),
            Err(Skipped::NotUsable)
        );
        // Another size: never read.
        assert_eq!(
            recognise(
                &reg,
                &found("vae/other.safetensors", 123),
                &vae,
                None,
                &mut no_hash
            ),
            Err(Skipped::NotUsable)
        );
        // Not a model at all (unknown tensors).
        let junk = header(&[("foo.bar", "F16", &[4])], &[]);
        assert_eq!(
            recognise(&reg, &found("x.safetensors", 1), &junk, None, &mut no_hash),
            Err(Skipped::NotUsable)
        );
    }
}
