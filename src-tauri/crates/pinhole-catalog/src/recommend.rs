//! Recommended models per role (SPEC §6.1): for each role, the first (best)
//! candidate in `config/models.yaml → recommended` that fits this GPU, with the
//! best quant that fits (starting at the hardware tier's `prefer_quant`).
//! Download sizes count only missing files; shared components count once.

use pinhole_registry::vram::{self, VramNeed};
use pinhole_registry::wiring::HwContext;
use pinhole_registry::{CaptionerFile, Family, RecommendedCandidate, Registry};
use pinhole_store::datadir::ModelKind;
use pinhole_store::{InstalledFile, InstalledIndex};

use crate::families::{self, mb_to_bytes, normalize_sha, QuantOption};
use crate::view::RecommendedPick;

/// Display order of roles; other roles in the YAML follow alphabetically.
pub const ROLE_ORDER: [&str; 4] = ["realistic", "anime", "edit", "describe"];

/// One file to download and how to register it afterwards.
#[derive(Debug, Clone, PartialEq)]
pub struct FileToGet {
    pub url: String,
    pub file_name: String,
    /// Verified hash, `None` while the registry says `TODO`.
    pub sha256: Option<String>,
    pub size_bytes: u64,
    pub kind: ModelKind,
    pub friendly_name: String,
    pub family: Option<String>,
    pub component_id: Option<String>,
    pub dtype: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PickAction {
    /// Installed and complete (or nothing to offer).
    Nothing,
    /// Registry download: main file (if missing) + missing components.
    Download { label: String, files: Vec<FileToGet> },
    /// A verified CivitAI version (installed through the CivitAI flow).
    Civitai { version_id: u64, family_id: String },
    /// The default captioner (engine area installs it).
    Captioner,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PickPlan {
    pub pick: RecommendedPick,
    pub action: PickAction,
}

pub fn role_label(role: &str) -> String {
    match role {
        "realistic" => "Realistic".into(),
        "anime" => "Anime".into(),
        "edit" => "Edit".into(),
        "describe" => "Describe".into(),
        other => {
            let mut c = other.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    }
}

fn good_at(role: &str) -> Option<String> {
    Some(
        match role {
            "realistic" => "Photos and lifelike pictures",
            "anime" => "Anime and illustration",
            "edit" => "Changing a picture by describing the change",
            "describe" => "Turning a picture into a prompt",
            _ => return None,
        }
        .into(),
    )
}

/// Picks for every role, in [`ROLE_ORDER`].
pub fn recommend(registry: &Registry, index: &InstalledIndex, hw: &HwContext) -> Vec<PickPlan> {
    let mut roles: Vec<String> = ROLE_ORDER.iter().map(|r| r.to_string()).filter(|r| registry.recommended().contains_key(r)).collect();
    roles.extend(registry.recommended().keys().filter(|k| !ROLE_ORDER.contains(&k.as_str())).cloned());
    roles.iter().filter_map(|r| recommend_role(registry, index, hw, r)).collect()
}

enum Skip {
    Vram,
    NotVerified,
    Broken,
}

pub fn recommend_role(registry: &Registry, index: &InstalledIndex, hw: &HwContext, role: &str) -> Option<PickPlan> {
    let candidates = registry.recommended().get(role)?;
    let mut skips: Vec<Skip> = Vec::new();
    for cand in candidates {
        let outcome = if cand.captioner.is_some() {
            captioner_pick(registry, index, role, cand)
        } else if cand.source.as_deref() == Some("civitai") {
            civitai_pick(registry, index, hw, role, cand)
        } else {
            registry_pick(registry, index, hw, role, cand)
        };
        match outcome {
            Ok(plan) => return Some(plan),
            Err(s) => skips.push(s),
        }
    }
    let reason = if skips.iter().any(|s| matches!(s, Skip::Vram)) {
        if hw.vram_gb <= 0.0 {
            "No supported graphics card was found. Pinhole can still run models on the processor, but very slowly — pick one in the Models tab.".to_string()
        } else {
            format!(
                "None of the recommended models fit in {} GB of graphics memory. The Models tab shows smaller ones.",
                fmt_gb(hw.vram_gb)
            )
        }
    } else {
        format!("No {} model has been picked for Pinhole yet. Browse the Models tab instead.", role_label(role).to_lowercase())
    };
    Some(PickPlan { pick: empty_pick(role, Some(reason)), action: PickAction::Nothing })
}

fn fmt_gb(v: f32) -> String {
    if (v - v.round()).abs() < 0.05 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

fn empty_pick(role: &str, unavailable_reason: Option<String>) -> RecommendedPick {
    RecommendedPick {
        role: role.to_string(),
        role_label: role_label(role),
        title: None,
        family_id: None,
        good_at: good_at(role),
        download_bytes: 0,
        vram: None,
        fit: None,
        installed: false,
        quant: None,
        license_note: None,
        unavailable_reason,
    }
}

fn component_files(registry: &Registry, family: &Family, hw: &HwContext, index: &InstalledIndex) -> Vec<FileToGet> {
    families::missing_components(registry, family, hw, index, true)
        .into_iter()
        .map(|(id, c)| FileToGet {
            url: c.url.clone(),
            file_name: c.file.clone(),
            sha256: normalize_sha(&c.sha256),
            size_bytes: mb_to_bytes(c.size_mb),
            kind: families::component_model_kind(&c.kind),
            friendly_name: families::component_label(c),
            family: None,
            component_id: Some(id),
            dtype: None,
        })
        .collect()
}

/// Required components (not the optional preview decoder) are all installed.
fn required_complete(registry: &Registry, family: &Family, hw: &HwContext, index: &InstalledIndex) -> bool {
    families::missing_components(registry, family, hw, index, false).is_empty()
}

fn installed_of_family<'a>(index: &'a InstalledIndex, family_id: &str) -> Option<&'a InstalledFile> {
    index.models().find(|m| m.family.as_deref() == Some(family_id))
}

fn registry_pick(
    registry: &Registry,
    index: &InstalledIndex,
    hw: &HwContext,
    role: &str,
    cand: &RecommendedCandidate,
) -> Result<PickPlan, Skip> {
    let family_id = cand.family.as_deref().ok_or(Skip::Broken)?;
    let fam = registry.family(family_id).ok_or(Skip::Broken)?;
    let title = cand.title.clone().unwrap_or_else(|| fam.label.clone());
    let comps = component_files(registry, fam, hw, index);
    let comp_bytes: u64 = comps.iter().map(|f| f.size_bytes).sum();

    if let Some(m) = installed_of_family(index, family_id) {
        let need = families::installed_need(registry, fam, m, hw);
        let complete = required_complete(registry, fam, hw, index);
        let quant = m.dtype.clone().unwrap_or_else(|| families::quant_of_file(&m.rel_path));
        return Ok(PickPlan {
            pick: RecommendedPick {
                role: role.into(),
                role_label: role_label(role),
                title: Some(title.clone()),
                family_id: Some(family_id.into()),
                good_at: good_at(role),
                download_bytes: comp_bytes,
                vram: Some(need),
                fit: Some(vram::fit(&need, hw.vram_gb)),
                installed: complete,
                quant: Some(families::quant_of_file(&quant)).filter(|q| q != "unknown"),
                license_note: fam.license_note.clone(),
                unavailable_reason: None,
            },
            action: if comps.is_empty() { PickAction::Nothing } else { PickAction::Download { label: title, files: comps } },
        });
    }

    let spec = fam.download.as_ref().ok_or(Skip::Broken)?;
    let options = families::quant_options(spec);
    let prefer = registry.hardware_profile(hw.vram_gb).prefer_quant.clone();
    let need_of = |o: &QuantOption| -> VramNeed {
        o.vram
            .as_ref()
            .or(fam.vram_gb.as_ref())
            .map(families::need_from)
            .unwrap_or_else(|| vram::estimate(registry, fam, o.size_bytes, families::gpu_component_bytes(registry, fam, hw)))
    };
    let (opt, need) = families::choose_quant(&options, prefer.as_deref(), hw.vram_gb, need_of).ok_or(Skip::Vram)?;
    let main = FileToGet {
        url: opt.url.clone(),
        file_name: opt.file.clone(),
        sha256: opt.sha256.clone(),
        size_bytes: opt.size_bytes,
        kind: families::main_model_kind(fam),
        friendly_name: match families::quant_suffix(&opt.quant) {
            Some(s) => format!("{} ({s})", fam.label),
            None => fam.label.clone(),
        },
        family: Some(family_id.into()),
        component_id: None,
        dtype: Some(opt.quant.clone()).filter(|q| q != "unknown"),
    };
    let mut files = vec![main];
    files.extend(comps);
    Ok(PickPlan {
        pick: RecommendedPick {
            role: role.into(),
            role_label: role_label(role),
            title: Some(title.clone()),
            family_id: Some(family_id.into()),
            good_at: good_at(role),
            download_bytes: files.iter().map(|f| f.size_bytes).sum(),
            vram: Some(need),
            fit: Some(vram::fit(&need, hw.vram_gb)),
            installed: false,
            quant: Some(opt.quant.clone()).filter(|q| q != "unknown"),
            license_note: fam.license_note.clone(),
            unavailable_reason: None,
        },
        action: PickAction::Download { label: title, files },
    })
}

/// A YAML id that is a real number (`TODO` and other strings → `None`).
pub fn yaml_id(v: Option<&serde_yaml::Value>) -> Option<u64> {
    match v? {
        serde_yaml::Value::Number(n) => n.as_u64(),
        serde_yaml::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|id| *id > 0)
}

fn civitai_pick(
    registry: &Registry,
    index: &InstalledIndex,
    hw: &HwContext,
    role: &str,
    cand: &RecommendedCandidate,
) -> Result<PickPlan, Skip> {
    let version_id = yaml_id(cand.civitai_version_id.as_ref()).ok_or(Skip::NotVerified)?;
    let family_id = cand.family.as_deref().ok_or(Skip::Broken)?;
    let fam = registry.family(family_id).ok_or(Skip::Broken)?;
    let installed_file = index.models().find(|m| m.civitai.as_ref().is_some_and(|c| c.version_id == version_id));
    let size = cand.size_mb.map(mb_to_bytes);
    let need = match installed_file {
        Some(m) => Some(families::installed_need(registry, fam, m, hw)),
        None => cand.vram_gb.as_ref().or(fam.vram_gb.as_ref()).map(families::need_from).or_else(|| {
            size.map(|s| vram::estimate(registry, fam, s, families::gpu_component_bytes(registry, fam, hw)))
        }),
    };
    if installed_file.is_none() && !need.as_ref().is_some_and(|n| families::fits_at_all(n, hw.vram_gb)) {
        return Err(Skip::Vram);
    }
    let comps = component_files(registry, fam, hw, index);
    let comp_bytes: u64 = comps.iter().map(|f| f.size_bytes).sum();
    let installed = installed_file.is_some() && required_complete(registry, fam, hw, index);
    Ok(PickPlan {
        pick: RecommendedPick {
            role: role.into(),
            role_label: role_label(role),
            title: Some(cand.title.clone().unwrap_or_else(|| fam.label.clone())),
            family_id: Some(family_id.into()),
            good_at: good_at(role),
            download_bytes: if installed_file.is_some() { comp_bytes } else { size.unwrap_or(0) + comp_bytes },
            vram: need,
            fit: need.map(|n| vram::fit(&n, hw.vram_gb)),
            installed,
            quant: None,
            license_note: fam.license_note.clone(),
            unavailable_reason: None,
        },
        action: if installed { PickAction::Nothing } else { PickAction::Civitai { version_id, family_id: family_id.into() } },
    })
}

/// Default captioner files that are not installed (matched by hash or file name).
pub fn missing_captioner_files<'a>(registry: &'a Registry, index: &InstalledIndex) -> Vec<&'a CaptionerFile> {
    let Some(def) = registry.captioner().default.as_ref() else { return Vec::new() };
    [&def.model, &def.mmproj]
        .into_iter()
        .filter(|cf| {
            !index.files.iter().any(|f| {
                let name = f.rel_path.rsplit('/').next().unwrap_or(&f.rel_path);
                normalize_sha(&cf.sha256).is_some_and(|h| f.sha256.eq_ignore_ascii_case(&h)) || name.eq_ignore_ascii_case(&cf.file)
            })
        })
        .collect()
}

/// The edit model's Qwen2.5-VL encoder + vision projector are installed.
pub fn captioner_reuse_available(registry: &Registry, index: &InstalledIndex) -> bool {
    let ids = &registry.captioner().prefer_reuse;
    !ids.is_empty()
        && ids.iter().all(|id| registry.component(id).is_some_and(|c| families::installed_component(index, id, c).is_some()))
}

fn captioner_pick(registry: &Registry, index: &InstalledIndex, role: &str, cand: &RecommendedCandidate) -> Result<PickPlan, Skip> {
    let base = |title: &str, bytes: u64, installed: bool| RecommendedPick {
        role: role.into(),
        role_label: role_label(role),
        title: Some(title.into()),
        family_id: None,
        good_at: good_at(role),
        download_bytes: bytes,
        vram: None,
        fit: None,
        installed,
        quant: None,
        license_note: None,
        unavailable_reason: None,
    };
    match cand.captioner.as_deref() {
        Some("reuse") if captioner_reuse_available(registry, index) => {
            Ok(PickPlan { pick: base("Image describer (uses your edit model)", 0, true), action: PickAction::Nothing })
        }
        Some("reuse") => Err(Skip::NotVerified),
        Some("default") => {
            registry.captioner().default.as_ref().ok_or(Skip::Broken)?;
            let missing = missing_captioner_files(registry, index);
            let bytes = missing.iter().map(|f| mb_to_bytes(f.size_mb)).sum();
            let installed = missing.is_empty();
            Ok(PickPlan {
                pick: base("Image describer", bytes, installed),
                action: if installed { PickAction::Nothing } else { PickAction::Captioner },
            })
        }
        _ => Err(Skip::Broken),
    }
}
