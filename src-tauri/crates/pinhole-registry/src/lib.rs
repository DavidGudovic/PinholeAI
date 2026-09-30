//! Model registry (SPEC §6). Loads `config/models.yaml`, deep-merges
//! `Data/config/overrides.yaml`, resolves `inherits`, and answers every
//! "what is this model / how do I run it" question. Model knowledge lives in
//! YAML; this crate only interprets it.
//!
//! OWNER: registry agent. Public signatures below are the cross-crate contract
//! (see docs/ARCHITECTURE.md). Extend freely; don't break them.
//!
//! Loading pipeline ([`Registry::from_yaml`]):
//! 1. parse both documents as `serde_yaml::Value`, expand anchors and `<<` merge keys;
//! 2. deep-merge the overrides over the shipped file ([`merge::deep_merge`]:
//!    maps merge recursively, the user wins, scalars and sequences replace;
//!    a family or component set to `null` is removed);
//! 3. resolve `inherits:` per family (multi-level, cycle-checked; the parent's
//!    `download` and `civitai_base_models` are *not* inherited);
//! 4. resolve `detect: { same_as: X }` (copy X's rules, keep the family distinct);
//! 5. deserialize every family into [`Family`] with `id` set.

pub mod detect;
pub mod merge;
pub mod model;
pub mod style;
pub mod vram;
pub mod wiring;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_yaml::{Mapping, Value};

pub use model::*;

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("could not read {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid registry YAML: {0}")]
    Yaml(String),
    #[error("family `{0}` inherits from unknown family `{1}`")]
    BadInherit(String, String),
    #[error("{0}")]
    Invalid(String),
}

/// The loaded, fully resolved registry. Cheap to clone behind an `Arc`.
#[derive(Debug, Clone)]
pub struct Registry {
    pub(crate) file: RegistryFile,
    /// Families with `inherits` / `same_as` already applied.
    pub(crate) families: BTreeMap<String, Family>,
    /// Family ids in YAML order (shipped order, then ids added by overrides).
    pub(crate) family_order: Vec<String>,
}

/// Keys a child family never takes from its `inherits:` parent: they describe
/// one concrete file / CivitAI category, not the architecture.
const NON_INHERITED_KEYS: &[&str] = &["download", "civitai_base_models"];

/// Used when a registry has no `hardware_profiles` at all.
static DEFAULT_PROFILE: HardwareProfile = HardwareProfile {
    name: String::new(),
    max_vram_gb: f32::MAX,
    flags: Vec::new(),
    prefer_quant: None,
};

impl Registry {
    /// Load `<config_dir>/models.yaml` and deep-merge `overrides` (user wins).
    /// A missing overrides file is treated as "no overrides".
    pub fn load(config_dir: &Path, overrides: Option<&Path>) -> Result<Self, RegistryError> {
        let path = config_dir.join("models.yaml");
        let models = std::fs::read_to_string(&path).map_err(|source| RegistryError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let over = match overrides {
            Some(p) => match std::fs::read_to_string(p) {
                Ok(s) => Some(s),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(source) => {
                    return Err(RegistryError::Io {
                        path: p.display().to_string(),
                        source,
                    })
                }
            },
            None => None,
        };
        Self::from_yaml(&models, over.as_deref())
    }

    /// Same as [`Registry::load`] but from strings (tests).
    pub fn from_yaml(
        models_yaml: &str,
        overrides_yaml: Option<&str>,
    ) -> Result<Self, RegistryError> {
        let mut root = parse_doc(models_yaml, "models.yaml")?;
        if !root.is_mapping() {
            return Err(RegistryError::Yaml(
                "models.yaml: the top level must be a mapping".into(),
            ));
        }
        if let Some(over) = overrides_yaml {
            let over = parse_doc(over, "overrides.yaml")?;
            match over {
                Value::Null => {}
                Value::Mapping(_) => merge::deep_merge(&mut root, over),
                _ => {
                    return Err(RegistryError::Yaml(
                        "overrides.yaml: the top level must be a mapping".into(),
                    ))
                }
            }
        }

        // `families: { x: null }` / `components: { x: null }` in overrides removes an entry.
        for section in ["families", "components"] {
            if let Some(Value::Mapping(m)) = root.get_mut(section) {
                m.retain(|_, v| !v.is_null());
            }
        }

        let family_order: Vec<String> = match root.get("families") {
            Some(Value::Mapping(m)) => m
                .keys()
                .filter_map(|k| k.as_str().map(str::to_owned))
                .collect(),
            Some(Value::Null) | None => Vec::new(),
            Some(_) => return Err(RegistryError::Yaml("`families` must be a mapping".into())),
        };

        let mut file: RegistryFile =
            serde_yaml::from_value(root).map_err(|e| RegistryError::Yaml(e.to_string()))?;
        file.hardware_profiles
            .sort_by(|a, b| a.max_vram_gb.total_cmp(&b.max_vram_gb));

        let families = resolve_families(&file.families, &family_order)?;
        Ok(Self {
            file,
            families,
            family_order,
        })
    }

    pub fn family(&self, id: &str) -> Option<&Family> {
        self.families.get(id)
    }

    pub fn families(&self) -> impl Iterator<Item = &Family> {
        self.families.values()
    }

    /// Families in YAML order (the order detection candidates and pickers use).
    pub fn families_in_order(&self) -> impl Iterator<Item = &Family> {
        self.family_order
            .iter()
            .filter_map(|id| self.families.get(id))
    }

    pub fn component(&self, id: &str) -> Option<&Component> {
        self.file.components.get(id)
    }

    pub fn components(&self) -> &BTreeMap<String, Component> {
        &self.file.components
    }

    /// Families whose `civitai_base_models` contains `base_model` (exact, case-insensitive).
    /// Returned in YAML order (base families before their finetunes).
    pub fn families_for_base_model(&self, base_model: &str) -> Vec<&Family> {
        let wanted = base_model.trim();
        if wanted.is_empty() {
            return Vec::new();
        }
        self.families_in_order()
            .filter(|f| {
                f.civitai_base_models
                    .iter()
                    .any(|b| b.trim().eq_ignore_ascii_case(wanted))
            })
            .collect()
    }

    /// Every CivitAI baseModel string Pinhole can run ("Compatibility" filter).
    /// Sorted and de-duplicated.
    pub fn all_civitai_base_models(&self) -> Vec<String> {
        let set: BTreeSet<String> = self
            .families
            .values()
            .flat_map(|f| f.civitai_base_models.iter())
            .map(|b| b.trim().to_owned())
            .filter(|b| !b.is_empty())
            .collect();
        set.into_iter().collect()
    }

    pub fn known_file(&self, sha256: &str) -> Option<&KnownFile> {
        let wanted = sha256.trim();
        if wanted.is_empty() || wanted.eq_ignore_ascii_case("TODO") {
            return None;
        }
        self.file
            .known_files
            .iter()
            .find(|k| k.sha256.trim().eq_ignore_ascii_case(wanted))
    }

    /// Hardware tier for this much VRAM (first profile with `max_vram_gb >= vram_gb`;
    /// the largest tier when none is big enough).
    pub fn hardware_profile(&self, vram_gb: f32) -> &HardwareProfile {
        let profiles = &self.file.hardware_profiles;
        profiles
            .iter()
            .find(|p| p.max_vram_gb >= vram_gb)
            .or_else(|| profiles.last())
            .unwrap_or(&DEFAULT_PROFILE)
    }

    pub fn hardware_profiles(&self) -> &[HardwareProfile] {
        &self.file.hardware_profiles
    }

    /// Ranked candidates per role: `realistic`, `anime`, `edit`, `describe`.
    pub fn recommended(&self) -> &BTreeMap<String, Vec<RecommendedCandidate>> {
        &self.file.recommended
    }

    pub fn captioner(&self) -> &CaptionerSpec {
        &self.file.captioner
    }

    /// `natural` → "{prompt}. Style: {style}", `tags` → "{prompt}, {style}".
    pub fn style_template(&self, name: &str) -> Option<&str> {
        self.file.style_templates.get(name).map(String::as_str)
    }

    /// Optional test-only models (engine smoke test), from `test_models:`.
    pub fn test_models(&self) -> &BTreeMap<String, DownloadSpec> {
        &self.file.test_models
    }

    pub fn engine_features(&self) -> &EngineFeatures {
        &self.file.engine_features
    }

    pub fn known_files(&self) -> &[KnownFile] {
        &self.file.known_files
    }

    /// Consistency problems in the loaded data (dangling component / family
    /// references, invalid sampler names, sizes the engine would re-align…).
    /// Empty = OK. Loading never fails on these so a bad user override can't
    /// brick the app; CI asserts the shipped file is clean.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let comp_ok = |id: &str| self.file.components.contains_key(id);
        let mut helper_ids = std::collections::HashSet::new();
        for h in &self.file.captioner.helpers {
            if !helper_ids.insert(h.id.as_str()) {
                problems.push(format!("captioner helper `{}` is listed twice", h.id));
            }
            if h.default != h.components.is_empty() {
                problems.push(format!(
                    "captioner helper `{}`: set either `default: true` or two `components`",
                    h.id
                ));
            }
            if !h.components.is_empty() && h.components.len() != 2 {
                problems.push(format!(
                    "captioner helper `{}`: `components` is the model and its vision projector",
                    h.id
                ));
            }
            for c in &h.components {
                if !comp_ok(c) {
                    problems.push(format!(
                        "captioner helper `{}` refers to unknown component `{c}`",
                        h.id
                    ));
                }
            }
        }
        for f in self.families_in_order() {
            let id = &f.id;
            for (kind, choice) in &f.components {
                let ids: Vec<&String> = match choice {
                    ComponentChoice::Fixed(c) => vec![c],
                    ComponentChoice::ByVram(m) => m.values().collect(),
                };
                for c in ids {
                    if !comp_ok(c) {
                        problems.push(format!(
                            "family `{id}`: component `{kind}` refers to unknown component `{c}`"
                        ));
                    }
                }
                if let ComponentChoice::ByVram(m) = choice {
                    if !m.contains_key("else") && !m.contains_key("default") {
                        problems.push(format!(
                            "family `{id}`: component `{kind}` has no `else` choice"
                        ));
                    }
                }
            }
            if let Some(t) = &f.taesd {
                if !comp_ok(t) {
                    problems.push(format!("family `{id}`: unknown taesd component `{t}`"));
                }
            }
            if !self.file.style_templates.contains_key(&f.style_template) {
                problems.push(format!(
                    "family `{id}`: unknown style_template `{}`",
                    f.style_template
                ));
            }
            for other in &f.detect.ambiguous_with {
                if !self.families.contains_key(other) {
                    problems.push(format!(
                        "family `{id}`: ambiguous_with unknown family `{other}`"
                    ));
                }
            }
            if !f.detect.has_positive_rule() {
                problems.push(format!("family `{id}`: detect has no positive rule (header sniffing can never pick it)"));
            }
            if f.detect.whole_checkpoint && f.layout != Layout::AllInOne {
                problems.push(format!(
                    "family `{id}`: detect.whole_checkpoint needs layout: all_in_one"
                ));
            }
            if let Some(s) = &f.defaults.sampler {
                if !wiring::SAMPLERS.contains(&s.as_str()) {
                    problems.push(format!(
                        "family `{id}`: sampler `{s}` is not an sd.cpp sample method"
                    ));
                }
            }
            if let Some(s) = &f.defaults.scheduler {
                if !wiring::SCHEDULERS.contains(&s.as_str()) {
                    problems.push(format!(
                        "family `{id}`: scheduler `{s}` is not an sd.cpp scheduler"
                    ));
                }
            }
            let multiple = wiring::size_multiple(f);
            for (shape, [w, h]) in &f.dials.shape {
                if w % multiple != 0 || h % multiple != 0 {
                    problems.push(format!(
                        "family `{id}`: shape `{shape}` {w}x{h} is not a multiple of {multiple}"
                    ));
                }
            }
            if f.dials.shape.is_empty() {
                problems.push(format!("family `{id}`: no dials.shape"));
            }
            if f.dials.quality.is_none() {
                problems.push(format!("family `{id}`: no dials.quality"));
            }
            if f.dials.cfg_fixed.is_none() && f.dials.cfg_range.is_none() {
                problems.push(format!(
                    "family `{id}`: neither dials.cfg_fixed nor dials.cfg_range"
                ));
            }
            for flag in &f.flags {
                if flag.starts_with("--") && !wiring::is_known_flag(flag) {
                    problems.push(format!("family `{id}`: unknown sd-server flag `{flag}`"));
                }
            }
        }
        for p in &self.file.hardware_profiles {
            for flag in &p.flags {
                if flag.starts_with("--") && !wiring::is_known_flag(flag) {
                    problems.push(format!(
                        "hardware profile `{}`: unknown sd-server flag `{flag}`",
                        p.name
                    ));
                }
            }
        }
        for (role, list) in &self.file.recommended {
            for c in list {
                if let Some(fam) = &c.family {
                    match self.families.get(fam) {
                        None => problems.push(format!("recommended `{role}`: unknown family `{fam}`")),
                        // A registry pick is a one-click download of the family's own file.
                        Some(f) if c.source.as_deref() == Some("registry") && f.download.is_none() => problems.push(
                            format!("recommended `{role}`: family `{fam}` has no `download` for a registry pick"),
                        ),
                        Some(_) => {}
                    }
                }
            }
        }
        for c in &self.file.captioner.prefer_reuse {
            if !comp_ok(c) {
                problems.push(format!("captioner.prefer_reuse: unknown component `{c}`"));
            }
        }
        for (name, t) in &self.file.test_models {
            match &t.family {
                Some(fam) if self.families.contains_key(fam) => {}
                Some(fam) => problems.push(format!("test_models `{name}`: unknown family `{fam}`")),
                None => problems.push(format!("test_models `{name}`: missing family")),
            }
        }
        for k in &self.file.known_files {
            if !self.families.contains_key(&k.family) {
                problems.push(format!(
                    "known_files {}: unknown family `{}`",
                    k.sha256, k.family
                ));
            }
        }
        problems
    }
}

fn parse_doc(text: &str, what: &str) -> Result<Value, RegistryError> {
    let mut v: Value =
        serde_yaml::from_str(text).map_err(|e| RegistryError::Yaml(format!("{what}: {e}")))?;
    v.apply_merge()
        .map_err(|e| RegistryError::Yaml(format!("{what}: {e}")))?;
    Ok(v)
}

/// Resolve `inherits` and `detect.same_as` for every family and deserialize.
fn resolve_families(
    raw: &BTreeMap<String, Value>,
    order: &[String],
) -> Result<BTreeMap<String, Family>, RegistryError> {
    // Every id, YAML order first (then anything only the BTreeMap knows about).
    let mut ids: Vec<&String> = order.iter().filter(|id| raw.contains_key(*id)).collect();
    ids.extend(raw.keys().filter(|k| !order.contains(k)));

    let mut merged: BTreeMap<String, Value> = BTreeMap::new();
    for id in &ids {
        resolve_inherits(id, raw, &mut merged, &mut Vec::new())?;
    }
    let mut detects: BTreeMap<String, Value> = BTreeMap::new();
    for id in &ids {
        resolve_detect(id, &merged, &mut detects, &mut Vec::new())?;
    }

    let mut families = BTreeMap::new();
    for id in &ids {
        let mut v = merged[*id].clone();
        let map = v.as_mapping_mut().expect("checked in resolve_inherits");
        map.insert(Value::from("id"), Value::from(id.as_str()));
        map.insert(Value::from("detect"), detects[*id].clone());
        let mut fam: Family = serde_yaml::from_value(v)
            .map_err(|e| RegistryError::Yaml(format!("family `{id}`: {e}")))?;
        fam.id = (*id).clone();
        families.insert((*id).clone(), fam);
    }

    // ambiguous_with: drop self, include the same_as target, make it symmetric.
    let mut pairs: BTreeSet<(String, String)> = BTreeSet::new();
    for f in families.values() {
        let mut others: Vec<String> = f.detect.ambiguous_with.clone();
        if let Some(t) = &f.detect.same_as {
            others.push(t.clone());
        }
        for o in others {
            if o != f.id && families.contains_key(&o) {
                pairs.insert((f.id.clone(), o.clone()));
                pairs.insert((o, f.id.clone()));
            }
        }
    }
    for f in families.values_mut() {
        let unknown: Vec<String> = f
            .detect
            .ambiguous_with
            .iter()
            .filter(|o| **o != f.id && !raw.contains_key(*o))
            .cloned()
            .collect();
        let mut list: Vec<String> = ids
            .iter()
            .filter(|o| pairs.contains(&(f.id.clone(), (**o).clone())))
            .map(|o| (*o).clone())
            .collect();
        list.extend(unknown); // kept so `validate()` can report them
        f.detect.ambiguous_with = list;
    }
    Ok(families)
}

fn resolve_inherits(
    id: &str,
    raw: &BTreeMap<String, Value>,
    merged: &mut BTreeMap<String, Value>,
    stack: &mut Vec<String>,
) -> Result<(), RegistryError> {
    if merged.contains_key(id) {
        return Ok(());
    }
    if stack.iter().any(|s| s == id) {
        stack.push(id.to_owned());
        return Err(RegistryError::Invalid(format!(
            "`inherits` cycle: {}",
            stack.join(" → ")
        )));
    }
    let own = raw.get(id).cloned().unwrap_or(Value::Null);
    let own = match own {
        Value::Mapping(m) => m,
        Value::Null => Mapping::new(),
        _ => {
            return Err(RegistryError::Yaml(format!(
                "family `{id}` must be a mapping"
            )))
        }
    };
    let parent = match own.get("inherits") {
        None | Some(Value::Null) => None,
        Some(Value::String(p)) => Some(p.clone()),
        Some(_) => {
            return Err(RegistryError::Invalid(format!(
                "family `{id}`: `inherits` must be a family id"
            )))
        }
    };
    let value = match parent {
        None => Value::Mapping(own),
        Some(parent) => {
            if !raw.contains_key(&parent) {
                return Err(RegistryError::BadInherit(id.to_owned(), parent));
            }
            stack.push(id.to_owned());
            resolve_inherits(&parent, raw, merged, stack)?;
            stack.pop();
            let mut base = merged[&parent].clone();
            if let Some(m) = base.as_mapping_mut() {
                for k in NON_INHERITED_KEYS {
                    m.remove(*k);
                }
                m.remove("inherits");
            }
            merge::deep_merge(&mut base, Value::Mapping(own));
            base
        }
    };
    merged.insert(id.to_owned(), value);
    Ok(())
}

fn resolve_detect(
    id: &str,
    merged: &BTreeMap<String, Value>,
    detects: &mut BTreeMap<String, Value>,
    stack: &mut Vec<String>,
) -> Result<(), RegistryError> {
    if detects.contains_key(id) {
        return Ok(());
    }
    if stack.iter().any(|s| s == id) {
        stack.push(id.to_owned());
        return Err(RegistryError::Invalid(format!(
            "`detect.same_as` cycle: {}",
            stack.join(" → ")
        )));
    }
    let own = match merged.get(id).and_then(|f| f.get("detect")) {
        Some(Value::Mapping(m)) => m.clone(),
        None | Some(Value::Null) => Mapping::new(),
        Some(_) => {
            return Err(RegistryError::Yaml(format!(
                "family `{id}`: `detect` must be a mapping"
            )))
        }
    };
    let target = match own.get("same_as") {
        None | Some(Value::Null) => None,
        Some(Value::String(t)) => Some(t.clone()),
        Some(_) => {
            return Err(RegistryError::Invalid(format!(
                "family `{id}`: `detect.same_as` must be a family id"
            )))
        }
    };
    let value = match target {
        None => Value::Mapping(own),
        Some(target) => {
            if !merged.contains_key(&target) {
                return Err(RegistryError::Invalid(format!(
                    "family `{id}`: detect.same_as refers to unknown family `{target}`"
                )));
            }
            stack.push(id.to_owned());
            resolve_detect(&target, merged, detects, stack)?;
            stack.pop();
            let mut base = detects[&target].clone();
            if let Some(m) = base.as_mapping_mut() {
                // Family-specific keys are not copied.
                for k in ["same_as", "decisive_tensor", "ambiguous_with"] {
                    m.remove(k);
                }
            }
            let mut own = own;
            own.remove("same_as");
            merge::deep_merge(&mut base, Value::Mapping(own));
            if let Some(m) = base.as_mapping_mut() {
                m.insert(Value::from("same_as"), Value::from(target));
            }
            base
        }
    };
    detects.insert(id.to_owned(), value);
    Ok(())
}
