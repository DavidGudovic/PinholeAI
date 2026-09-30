//! Recommended models per role (SPEC §6.1): for each role, the first (best)
//! candidate in `config/models.yaml → recommended` that fits this GPU, with the
//! best quant that fits (starting at the hardware tier's `prefer_quant`).
//! Download sizes count only missing files; shared components count once.

use pinhole_registry::vram::{self, Fit, VramNeed};
use pinhole_registry::wiring::HwContext;
use pinhole_registry::{CaptionerFile, Family, RecommendedCandidate, Registry};
use pinhole_store::datadir::ModelKind;
use pinhole_store::{InstalledFile, InstalledIndex};

use crate::families::{self, mb_to_bytes, normalize_sha, QuantOption};
use crate::view::RecommendedPick;

/// Display order of roles; other roles in the YAML follow alphabetically.
pub const ROLE_ORDER: [&str; 5] = ["realistic", "realistic_fast", "anime", "edit", "describe"];

/// Extra cards next to a main role (the fast Realistic model next to
/// Qwen-Image 2.1). [`recommend`] leaves them out when none of their candidates
/// fits this machine: no card at all rather than a "doesn't fit" card.
pub const OPTIONAL_ROLES: [&str; 1] = ["realistic_fast"];

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
    Download {
        label: String,
        files: Vec<FileToGet>,
    },
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
        "realistic" | "realistic_fast" => "Realistic".into(),
        "anime" => "Anime".into(),
        "edit" => "Edit".into(),
        "describe" => "Describe".into(),
        other => {
            let mut c = other.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        }
    }
}

fn good_at(role: &str) -> Option<String> {
    Some(
        match role {
            "realistic" => "Photos and lifelike pictures",
            "realistic_fast" => {
                "Photos and lifelike pictures, several times faster; a little less detail"
            }
            "anime" => "Anime and illustration",
            "edit" => "Changing a picture by describing the change",
            "describe" => "Turning a picture into a prompt",
            _ => return None,
        }
        .into(),
    )
}

/// Picks for every role, in [`ROLE_ORDER`]. [`OPTIONAL_ROLES`] with nothing
/// that fits are left out.
pub fn recommend(registry: &Registry, index: &InstalledIndex, hw: &HwContext) -> Vec<PickPlan> {
    let mut roles: Vec<String> = ROLE_ORDER
        .iter()
        .map(|r| r.to_string())
        .filter(|r| registry.recommended().contains_key(r))
        .collect();
    roles.extend(
        registry
            .recommended()
            .keys()
            .filter(|k| !ROLE_ORDER.contains(&k.as_str()))
            .cloned(),
    );
    let picks: Vec<PickPlan> = roles
        .iter()
        .filter_map(|r| recommend_role(registry, index, hw, r))
        .filter(|p| !(OPTIONAL_ROLES.contains(&p.pick.role.as_str()) && p.pick.title.is_none()))
        .collect();
    // A second card that repeats the first one's model (the fast model is
    // already the Realistic pick on a smaller card) is left out.
    let realistic = picks
        .iter()
        .find(|p| p.pick.role == "realistic")
        .and_then(|p| p.pick.family_id.clone());
    picks
        .into_iter()
        .filter(|p| p.pick.role != "realistic_fast" || p.pick.family_id != realistic)
        .collect()
}

enum Skip {
    Vram,
    NotVerified,
    Broken,
}

pub fn recommend_role(
    registry: &Registry,
    index: &InstalledIndex,
    hw: &HwContext,
    role: &str,
) -> Option<PickPlan> {
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
    // No "go to the Models tab" here: these cards are shown on the Models tab too,
    // and every place that shows them has its own browse button.
    let reason = if skips.iter().any(|s| matches!(s, Skip::Vram)) {
        too_big_reason(registry, index, candidates, role, hw)
    } else {
        format!(
            "No {} model has been picked for Pinhole yet. You can browse CivitAI for one instead.",
            role_label(role).to_lowercase()
        )
    };
    Some(PickPlan {
        pick: empty_pick(role, Some(reason)),
        action: PickAction::Nothing,
    })
}

/// Why no candidate of `role` fits this machine.
fn too_big_reason(
    registry: &Registry,
    index: &InstalledIndex,
    candidates: &[RecommendedCandidate],
    role: &str,
    hw: &HwContext,
) -> String {
    if !hw.cpu_only() {
        return format!(
            "None of the recommended models fit in {} GB of graphics memory. Smaller ones are available when you browse CivitAI.",
            fmt_gb(hw.vram_gb)
        );
    }
    // A processor-friendly candidate was skipped too: not enough RAM for it.
    let small = candidates
        .iter()
        .filter_map(|c| registry.family(c.family.as_deref()?))
        .find(|f| f.cpu_friendly);
    if let Some(f) = small {
        let size = f
            .download
            .as_ref()
            .map(|d| mb_to_bytes(d.size_mb))
            .unwrap_or(0);
        let need = vram::cpu_need(f, families::cpu_weight_bytes(registry, f, hw, index, size)).gb
            + vram::CPU_SPARE_RAM_GB;
        return format!(
            "Pinhole didn't find a graphics card it can use, and this computer doesn't have enough memory to run even the small model on the processor. It needs at least {} GB of RAM.",
            fmt_gb(need.ceil())
        );
    }
    match role {
        "edit" => "Editing by description needs a graphics card, and Pinhole didn't find one it can use. Restyle still works with a Create model, slowly, on the processor.".to_string(),
        _ => format!(
            "The recommended {} models need a graphics card, and Pinhole didn't find one it can use. Only small models such as Stable Diffusion 1.5 run on the processor, slowly.",
            role_label(role).to_lowercase()
        ),
    }
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
        note: None,
        replaces_installed: false,
    }
}

fn component_files(
    registry: &Registry,
    family: &Family,
    hw: &HwContext,
    index: &InstalledIndex,
) -> Vec<FileToGet> {
    families::missing_components(registry, family, hw, index, true)
        .into_iter()
        .map(|(id, c)| component_file(id, c))
        .collect()
}

fn component_file(id: String, c: &pinhole_registry::Component) -> FileToGet {
    FileToGet {
        url: c.url.clone(),
        file_name: c.file.clone(),
        sha256: normalize_sha(&c.sha256),
        size_bytes: mb_to_bytes(c.size_mb),
        kind: families::component_model_kind(&c.kind),
        friendly_name: families::component_label(c),
        family: None,
        component_id: Some(id),
        dtype: None,
    }
}

/// The parts an installed model of `family` still needs before it can run
/// (the ones its Installed row lists), as downloads.
pub fn parts_to_run(
    registry: &Registry,
    family: &Family,
    hw: &HwContext,
    index: &InstalledIndex,
) -> Vec<FileToGet> {
    families::missing_to_run(registry, family, hw, index)
        .into_iter()
        .map(|(id, c)| component_file(id, c))
        .collect()
}

/// Required components (not the optional preview decoder) are all installed.
fn required_complete(
    registry: &Registry,
    family: &Family,
    hw: &HwContext,
    index: &InstalledIndex,
) -> bool {
    families::missing_components(registry, family, hw, index, false).is_empty()
}

/// Shown on a pick that is a smaller version of the model, chosen so it fits.
pub const SMALLER_NOTE: &str =
    "Smaller version, picked so it fits your graphics card. Pictures keep their size; fine detail is a little softer. It runs at about the same speed.";
/// Shown when the installed version is a tight fit and a smaller one fits.
pub const TIGHT_INSTALLED_NOTE: &str = "The version you have is a tight fit for your graphics card: it runs slower and can run out of memory. This smaller version fits: fine detail is a little softer, and it runs faster and more reliably.";

/// Shown when the installed version is too big for the card and a smaller one fits.
pub const TOO_BIG_INSTALLED_NOTE: &str = "The version you have is probably too big for your graphics card: it can fail to load or run out of memory. This smaller version fits: fine detail is a little softer, and it runs faster and more reliably.";

/// Registry option a file was downloaded from (by SHA-256, else file name).
fn option_of<'a>(options: &'a [QuantOption], file: &InstalledFile) -> Option<&'a QuantOption> {
    let name = file.rel_path.rsplit('/').next().unwrap_or(&file.rel_path);
    options.iter().find(|o| {
        o.sha256
            .as_deref()
            .is_some_and(|h| h.eq_ignore_ascii_case(&file.sha256))
            || o.file.eq_ignore_ascii_case(name)
    })
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
    let options = fam
        .download
        .as_ref()
        .map(families::quant_options)
        .unwrap_or_default();
    let prefer = registry.hardware_profile(hw.vram_gb).prefer_quant.clone();
    let need_of = |o: &QuantOption| -> VramNeed {
        o.vram
            .as_ref()
            .or(fam.vram_gb.as_ref())
            .map(families::need_from)
            .unwrap_or_else(|| families::estimate_need(registry, fam, hw, index, o.size_bytes))
    };
    // `min_quant`: only versions at least this good, and only when one Fits.
    let floor = cand.min_quant.as_deref().map(families::quant_rank);
    let good_enough = |quant: &str| floor.is_none_or(|f| families::quant_rank(quant) <= f);
    let eligible: Vec<QuantOption> = options
        .iter()
        .filter(|o| good_enough(&o.quant))
        .cloned()
        .collect();
    let chosen = families::choose_quant(&eligible, prefer.as_deref(), |o| {
        families::need_and_fit(registry, fam, hw, index, need_of(o), o.size_bytes)
    })
    .filter(|(_, _, f)| floor.is_none() || *f == Fit::Fits);

    // Installed versions of this family, best fit first.
    let mut installed: Vec<(&InstalledFile, VramNeed, Fit)> = index
        .models()
        .filter(|m| m.family.as_deref() == Some(family_id))
        .map(|m| {
            let (n, f) = families::need_and_fit(
                registry,
                fam,
                hw,
                index,
                families::installed_need(registry, fam, m, hw, index),
                m.size_bytes,
            );
            (m, n, f)
        })
        .filter(|(m, _, _)| good_enough(&installed_quant(m)))
        .collect();
    installed.sort_by_key(|(_, _, f)| fit_rank(*f));
    let mut note = None;
    if let Some((m, need, fit)) = installed.first().cloned() {
        // A tight installed version, and a smaller registry version that Fits isn't installed yet:
        // offer that one instead (the installed file stays; both show in the model picker).
        let smaller = chosen.as_ref().filter(|(o, _, f)| {
            fit != Fit::Fits
                && *f == Fit::Fits
                && !installed
                    .iter()
                    .any(|(i, _, _)| option_of(&options, i).is_some_and(|io| io.file == o.file))
        });
        if smaller.is_none() {
            if floor.is_some() && fit != Fit::Fits {
                return Err(Skip::Vram);
            }
            let complete = required_complete(registry, fam, hw, index);
            let quant = installed_quant(m);
            return Ok(PickPlan {
                pick: RecommendedPick {
                    role: role.into(),
                    role_label: role_label(role),
                    title: Some(title.clone()),
                    family_id: Some(family_id.into()),
                    good_at: good_at(role),
                    download_bytes: comp_bytes,
                    vram: Some(need),
                    fit: Some(fit),
                    installed: complete,
                    quant: Some(families::quant_of_file(&quant)).filter(|q| q != "unknown"),
                    license_note: fam.license_note.clone(),
                    unavailable_reason: None,
                    note: None,
                    replaces_installed: false,
                },
                action: if comps.is_empty() {
                    PickAction::Nothing
                } else {
                    PickAction::Download {
                        label: title,
                        files: comps,
                    }
                },
            });
        }
        note = Some(
            if fit == Fit::TooBig {
                TOO_BIG_INSTALLED_NOTE
            } else {
                TIGHT_INSTALLED_NOTE
            }
            .to_string(),
        );
    }

    if options.is_empty() {
        return Err(Skip::Broken);
    }
    let (opt, need, fit) = chosen.ok_or(Skip::Vram)?;
    if note.is_none() && options.first().is_some_and(|best| best.file != opt.file) {
        note = Some(SMALLER_NOTE.to_string());
    }
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
            fit: Some(fit),
            installed: false,
            quant: Some(opt.quant.clone()).filter(|q| q != "unknown"),
            license_note: fam.license_note.clone(),
            unavailable_reason: None,
            replaces_installed: matches!(
                note.as_deref(),
                Some(TIGHT_INSTALLED_NOTE | TOO_BIG_INSTALLED_NOTE)
            ),
            note,
        },
        action: PickAction::Download {
            label: title,
            files,
        },
    })
}

/// Weight type of an installed file: from its header, else its file name.
fn installed_quant(m: &InstalledFile) -> String {
    m.dtype
        .clone()
        .unwrap_or_else(|| families::quant_of_file(&m.rel_path))
}

fn fit_rank(f: Fit) -> u8 {
    match f {
        Fit::Fits => 0,
        Fit::Tight => 1,
        Fit::TooBig => 2,
    }
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
    let installed_file = index.models().find(|m| {
        m.civitai
            .as_ref()
            .is_some_and(|c| c.version_id == version_id)
    });
    let size = cand.size_mb.map(mb_to_bytes);
    let need_fit = match installed_file {
        Some(m) => Some(families::need_and_fit(
            registry,
            fam,
            hw,
            index,
            families::installed_need(registry, fam, m, hw, index),
            m.size_bytes,
        )),
        // Without a GPU the RAM need comes from the file size: unknown size → can't tell.
        None if hw.cpu_only() && size.is_none() => None,
        None => cand
            .vram_gb
            .as_ref()
            .or(fam.vram_gb.as_ref())
            .map(families::need_from)
            .or_else(|| size.map(|s| families::estimate_need(registry, fam, hw, index, s)))
            .map(|n| families::need_and_fit(registry, fam, hw, index, n, size.unwrap_or(0))),
    };
    if installed_file.is_none() && !need_fit.is_some_and(|(_, f)| f != Fit::TooBig) {
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
            download_bytes: if installed_file.is_some() {
                comp_bytes
            } else {
                size.unwrap_or(0) + comp_bytes
            },
            vram: need_fit.map(|(n, _)| n),
            fit: need_fit.map(|(_, f)| f),
            installed,
            quant: None,
            license_note: fam.license_note.clone(),
            unavailable_reason: None,
            note: None,
            replaces_installed: false,
        },
        action: if installed {
            PickAction::Nothing
        } else {
            PickAction::Civitai {
                version_id,
                family_id: family_id.into(),
            }
        },
    })
}

/// Default captioner files that are not installed (matched by hash or file name).
pub fn missing_captioner_files<'a>(
    registry: &'a Registry,
    index: &InstalledIndex,
) -> Vec<&'a CaptionerFile> {
    let Some(def) = registry.captioner().default.as_ref() else {
        return Vec::new();
    };
    [&def.model, &def.mmproj]
        .into_iter()
        .filter(|cf| {
            !index.files.iter().any(|f| {
                let name = f.rel_path.rsplit('/').next().unwrap_or(&f.rel_path);
                normalize_sha(&cf.sha256).is_some_and(|h| f.sha256.eq_ignore_ascii_case(&h))
                    || name.eq_ignore_ascii_case(&cf.file)
            })
        })
        .collect()
}

/// The edit model's Qwen2.5-VL encoder + vision projector are installed.
pub fn captioner_reuse_available(registry: &Registry, index: &InstalledIndex) -> bool {
    let ids = &registry.captioner().prefer_reuse;
    !ids.is_empty()
        && ids.iter().all(|id| {
            registry
                .component(id)
                .is_some_and(|c| families::installed_component(index, id, c).is_some())
        })
}

fn captioner_pick(
    registry: &Registry,
    index: &InstalledIndex,
    role: &str,
    cand: &RecommendedCandidate,
) -> Result<PickPlan, Skip> {
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
        note: None,
        replaces_installed: false,
    };
    match cand.captioner.as_deref() {
        Some("reuse") if captioner_reuse_available(registry, index) => Ok(PickPlan {
            pick: base("Image describer (uses your edit model)", 0, true),
            action: PickAction::Nothing,
        }),
        Some("reuse") => Err(Skip::NotVerified),
        Some("default") => {
            registry.captioner().default.as_ref().ok_or(Skip::Broken)?;
            let missing = missing_captioner_files(registry, index);
            let bytes = missing.iter().map(|f| mb_to_bytes(f.size_mb)).sum();
            let installed = missing.is_empty();
            Ok(PickPlan {
                pick: base("Image describer", bytes, installed),
                action: if installed {
                    PickAction::Nothing
                } else {
                    PickAction::Captioner
                },
            })
        }
        _ => Err(Skip::Broken),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{component, hw, hw_cpu, index, model, registry, with_civitai};

    const MB: u64 = 1_000_000;

    /// An installed Qwen-Image 2.1 file with its real size (its need is estimated from it).
    fn q21(id: &str, file: &str, size_bytes: u64) -> InstalledFile {
        let mut f = model(id, "qwen_image_21", ModelKind::Diffusion, file);
        f.size_bytes = size_bytes;
        f
    }

    fn picks(vram: f32, idx: &InstalledIndex) -> Vec<PickPlan> {
        recommend(&registry(), idx, &hw(vram))
    }

    fn role<'a>(p: &'a [PickPlan], role: &str) -> &'a PickPlan {
        p.iter().find(|p| p.pick.role == role).unwrap()
    }

    fn summary(p: &PickPlan) -> (Option<&str>, Option<&str>, Option<Fit>) {
        (
            p.pick.family_id.as_deref(),
            p.pick.quant.as_deref(),
            p.pick.fit,
        )
    }

    #[test]
    fn roles_in_order() {
        let p = picks(16.0, &index(vec![]));
        let roles: Vec<&str> = p.iter().map(|p| p.pick.role.as_str()).collect();
        assert_eq!(
            roles,
            ["realistic", "realistic_fast", "anime", "edit", "describe"]
        );
        assert_eq!(p[0].pick.role_label, "Realistic");
        assert_eq!(p[1].pick.role_label, "Realistic", "second Realistic card");
        assert!(p[0].pick.good_at.is_some() && p[1].pick.good_at.is_some());
        // The fast second Realistic card only appears when it isn't already the Realistic pick.
        let roles: Vec<String> = picks(8.0, &index(vec![]))
            .into_iter()
            .map(|p| p.pick.role)
            .collect();
        assert_eq!(roles, ["realistic", "anime", "edit", "describe"]);
    }

    #[test]
    fn six_gb() {
        let p = picks(6.0, &index(vec![]));
        assert_eq!(
            summary(role(&p, "realistic")),
            (Some("z_image_turbo"), Some("q4_k"), Some(Fit::Tight))
        );
        assert_eq!(
            summary(role(&p, "anime")),
            (Some("sdxl_illustrious"), None, Some(Fit::Tight))
        );
        assert_eq!(
            summary(role(&p, "edit")),
            (Some("qwen_image_21"), Some("q4_k"), Some(Fit::Tight)),
            "the 7B Qwen-Image 2.1 in 4-bit is the smallest edit model"
        );
        // z_image q4_k + flux_ae + Qwen3-4B Q4_K_M (the 8 GB bf16 encoder only from 20 GB)
        assert_eq!(
            role(&p, "realistic").pick.download_bytes,
            (3864 + 335 + 2497) * MB
        );
    }

    #[test]
    fn eight_gb() {
        let p = picks(8.0, &index(vec![]));
        let r = role(&p, "realistic");
        // Q8 (12 GB comfortable) and Q4 (8 GB) are both Tight: the one closest to fitting.
        assert_eq!(
            summary(r),
            (Some("z_image_turbo"), Some("q4_k"), Some(Fit::Tight))
        );
        // Q4 model + FLUX VAE + Qwen3-4B Q4_K_M text encoder (below 10 GB).
        assert_eq!(r.pick.download_bytes, (3864 + 335 + 2497) * MB);
        assert_eq!(r.pick.note.as_deref(), Some(SMALLER_NOTE));
        assert_eq!(r.pick.title.as_deref(), Some("Z-Image Turbo"));
        assert_eq!(r.pick.license_note.as_deref(), Some("Apache 2.0"));
        match &r.action {
            PickAction::Download { files, .. } => {
                assert_eq!(files[0].file_name, "z_image_turbo-Q4_K.gguf");
                assert_eq!(files[0].kind, ModelKind::Diffusion);
                assert_eq!(files[0].friendly_name, "Z-Image Turbo (Q4)");
                assert_eq!(
                    files[0].sha256.as_deref(),
                    Some("14b375ab4f226bc5378f68f37e899ef3c2242b8541e61e2bc1aff40976086fbd")
                );
                let comps: Vec<_> = files[1..]
                    .iter()
                    .map(|f| f.component_id.as_deref().unwrap())
                    .collect();
                assert_eq!(comps.len(), 2);
                assert!(comps.contains(&"flux_ae") && comps.contains(&"qwen3_4b_q4km"));
                assert!(files[1..].iter().all(|f| f.family.is_none()));
            }
            other => panic!("{other:?}"),
        }
        let a = role(&p, "anime");
        assert_eq!(
            a.action,
            PickAction::Civitai {
                version_id: 2940478,
                family_id: "sdxl_illustrious".into()
            }
        );
        assert_eq!(
            a.pick.download_bytes,
            (6939 + 335) * MB,
            "checkpoint + SDXL fp16-fix VAE"
        );
        assert_eq!(
            summary(role(&p, "edit")),
            (Some("qwen_image_21"), Some("q4_k"), Some(Fit::Tight))
        );
    }

    #[test]
    fn twelve_gb() {
        let p = picks(12.0, &index(vec![]));
        // Qwen-Image 2.1 Q6_K Fits (9.8 GB estimate); Q8_0 (11.3) would be Tight.
        assert_eq!(
            summary(role(&p, "realistic")),
            (Some("qwen_image_21"), Some("q6_k"), Some(Fit::Fits))
        );
        // Z-Image Q8 needs 12 GB to be comfortable (registry figure): the Q4 that Fits.
        assert_eq!(
            summary(role(&p, "realistic_fast")),
            (Some("z_image_turbo"), Some("q4_k"), Some(Fit::Fits))
        );
        assert_eq!(
            summary(role(&p, "anime")),
            (Some("sdxl_illustrious"), None, Some(Fit::Fits))
        );
        assert_eq!(
            summary(role(&p, "edit")),
            (Some("qwen_image_21"), Some("q6_k"), Some(Fit::Fits))
        );
    }

    #[test]
    fn sixteen_gb_runs_q8_qwen_image_21_for_create_and_edit() {
        // One download does both: Qwen-Image 2.1 Q8_0 + its VAE + the Qwen3-VL 8B Q4_K_M
        // text encoder (Q8 only from 20 GB) + the vision file edits need.
        let p = picks(16.0, &index(vec![]));
        let r = role(&p, "realistic");
        assert_eq!(
            summary(r),
            (Some("qwen_image_21"), Some("q8_0"), Some(Fit::Fits))
        );
        assert_eq!(r.pick.title.as_deref(), Some("Qwen-Image 2.1"));
        assert_eq!(
            r.pick.license_note.as_deref(),
            Some("Qwen Research License")
        );
        assert_eq!(r.pick.download_bytes, (7687 + 676 + 5028 + 1159) * MB);
        let v = r.pick.vram.unwrap();
        assert!(v.estimate);
        // 7.16 GiB weights + 0.63 VAE + 3.0 activations + 0.5 reserve.
        assert_eq!((v.gb, v.min_gb), (11.3, 7.1));
        assert_eq!(
            r.pick.note.as_deref(),
            Some(SMALLER_NOTE),
            "bf16 is the full version"
        );
        match &r.action {
            PickAction::Download { files, .. } => {
                assert_eq!(files[0].file_name, "qwen_image_2.1-Q8_0.gguf");
                assert_eq!(files[0].kind, ModelKind::Diffusion);
                assert_eq!(files[0].friendly_name, "Qwen-Image 2.1 (Q8)");
                assert_eq!(
                    files[0].sha256.as_deref(),
                    Some("f8b244b00937f0e444a40dbf7866460871b89b30142594973b6012d1b471dc0a")
                );
                assert!(files[0]
                    .url
                    .starts_with("https://huggingface.co/leejet/Qwen-Image-2.1-GGUF/"));
                let comps: Vec<_> = files[1..]
                    .iter()
                    .map(|f| f.component_id.as_deref().unwrap())
                    .collect();
                assert_eq!(comps.len(), 3);
                for c in ["qwen_image_21_vae", "qwen3vl_8b_q4km", "qwen3vl_8b_mmproj"] {
                    assert!(comps.contains(&c), "{c}");
                }
            }
            other => panic!("{other:?}"),
        }
        let e = role(&p, "edit");
        assert_eq!(summary(e), summary(r));
        assert_eq!(e.pick.download_bytes, r.pick.download_bytes);
        // Z-Image Turbo Q8_0 + the Q8_0 GGUF text encoder is the fast second card
        // (bf16 + bf16 ran out of VRAM on a real 16 GB card).
        let f = role(&p, "realistic_fast");
        assert_eq!(
            summary(f),
            (Some("z_image_turbo"), Some("q8_0"), Some(Fit::Fits))
        );
        assert_eq!(f.pick.download_bytes, (6577 + 335 + 4280) * MB);
        match &f.action {
            PickAction::Download { files, .. } => {
                assert_eq!(files[0].file_name, "z_image_turbo-Q8_0.gguf");
                assert!(files
                    .iter()
                    .any(|f| f.component_id.as_deref() == Some("qwen3_4b_q8")));
                assert!(
                    !files
                        .iter()
                        .any(|f| f.component_id.as_deref() == Some("qwen3_4b")),
                    "no bf16 encoder below 20 GB"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn twenty_four_gb() {
        let p = picks(24.0, &index(vec![]));
        let r = role(&p, "realistic");
        assert_eq!(
            summary(r),
            (Some("qwen_image_21"), Some("bf16"), Some(Fit::Fits))
        );
        assert_eq!(
            r.pick.download_bytes,
            (14230 + 676 + 8710 + 1159) * MB,
            "bf16 model + Q8 text encoder"
        );
        assert_eq!(r.pick.note, None, "full version, no note");
        assert_eq!(summary(role(&p, "edit")), summary(r));
        let f = role(&p, "realistic_fast");
        assert_eq!(
            summary(f),
            (Some("z_image_turbo"), Some("bf16"), Some(Fit::Fits))
        );
        assert_eq!(
            f.pick.download_bytes,
            (12310 + 335 + 8045) * MB,
            "bf16 model + bf16 encoder"
        );
    }

    #[test]
    fn picks_by_card() {
        // The table in config/models.yaml → recommended. Real cards report a little
        // under their size (a "16 GB" card is ~15.9 GiB).
        type Row = (
            f32,
            (&'static str, &'static str),
            Option<&'static str>,
            (&'static str, &'static str),
        );
        let rows: [Row; 8] = [
            (
                5.8,
                ("z_image_turbo", "q4_k"),
                None,
                ("qwen_image_21", "q4_k"),
            ),
            (
                7.8,
                ("z_image_turbo", "q4_k"),
                None,
                ("qwen_image_21", "q4_k"),
            ),
            (
                9.8,
                ("z_image_turbo", "q4_k"),
                None,
                ("qwen_image_21", "q4_k"),
            ),
            (
                11.8,
                ("qwen_image_21", "q6_k"),
                Some("q4_k"),
                ("qwen_image_21", "q6_k"),
            ),
            (
                15.8,
                ("qwen_image_21", "q8_0"),
                Some("q8_0"),
                ("qwen_image_21", "q8_0"),
            ),
            (
                19.8,
                ("qwen_image_21", "q8_0"),
                Some("q8_0"),
                ("qwen_image_21", "q8_0"),
            ),
            (
                23.8,
                ("qwen_image_21", "bf16"),
                Some("bf16"),
                ("qwen_image_21", "bf16"),
            ),
            (
                31.8,
                ("qwen_image_21", "bf16"),
                Some("bf16"),
                ("qwen_image_21", "bf16"),
            ),
        ];
        for (vram, (rf, rq), fast, (ef, eq)) in rows {
            let p = picks(vram, &index(vec![]));
            let r = role(&p, "realistic");
            assert_eq!(
                (r.pick.family_id.as_deref(), r.pick.quant.as_deref()),
                (Some(rf), Some(rq)),
                "{vram} GB realistic"
            );
            // Qwen-Image 2.1 is only picked when a Q6 or better version Fits.
            if rf == "qwen_image_21" {
                assert_eq!(r.pick.fit, Some(Fit::Fits), "{vram} GB");
            }
            let f = p.iter().find(|p| p.pick.role == "realistic_fast");
            assert_eq!(
                f.map(|f| (f.pick.family_id.as_deref(), f.pick.quant.as_deref())),
                fast.map(|q| (Some("z_image_turbo"), Some(q))),
                "{vram} GB fast card"
            );
            let e = role(&p, "edit");
            assert_eq!(
                (e.pick.family_id.as_deref(), e.pick.quant.as_deref()),
                (Some(ef), Some(eq)),
                "{vram} GB edit"
            );
            // Nothing recommended is too big for the card.
            assert!(
                p.iter().all(|p| p.pick.fit != Some(Fit::TooBig)),
                "{vram} GB"
            );
        }
        let p = picks(16.0, &index(vec![]));
        let f = role(&p, "realistic_fast");
        assert_eq!(f.pick.title.as_deref(), Some("Z-Image Turbo"));
        assert_eq!(f.pick.role_label, "Realistic");
        assert!(f.pick.good_at.as_deref().unwrap().contains("faster"));
        // No fast card without a GPU either (Z-Image is too big for the processor).
        assert!(picks(0.0, &index(vec![]))
            .iter()
            .all(|p| p.pick.role != "realistic_fast"));
    }

    #[test]
    fn qwen_image_21_needs_a_good_version_that_fits_for_create() {
        let reg = registry();
        // 10 GB: its Q6_K would be Tight → Z-Image Turbo, while Edit takes the Q4_K that Fits.
        let p = recommend(&reg, &index(vec![]), &hw(10.0));
        assert_eq!(
            role(&p, "realistic").pick.family_id.as_deref(),
            Some("z_image_turbo")
        );
        assert_eq!(
            summary(role(&p, "edit")),
            (Some("qwen_image_21"), Some("q4_k"), Some(Fit::Fits))
        );
        // An installed 4-bit copy (e.g. from Edit) is below the floor for Create:
        // on a 16 GB card Create still offers the Q8_0, Edit keeps using the Q4_K.
        let idx = index(vec![
            q21("q4", "qwen_image_2.1-Q4_K.gguf", 4_197_494_816),
            component(&reg, "qwen_image_21_vae"),
            component(&reg, "qwen3vl_8b_q4km"),
            component(&reg, "qwen3vl_8b_mmproj"),
        ]);
        let r = recommend_role(&reg, &idx, &hw(16.0), "realistic").unwrap();
        assert_eq!(
            summary(&r),
            (Some("qwen_image_21"), Some("q8_0"), Some(Fit::Fits))
        );
        assert!(!r.pick.installed);
        assert_eq!(r.pick.download_bytes, 7687 * MB, "parts are shared");
        let e = recommend_role(&reg, &idx, &hw(16.0), "edit").unwrap();
        assert!(e.pick.installed);
        assert_eq!(e.action, PickAction::Nothing);
        // Installed Q8_0 on a 10 GB card: Tight, so Create falls back to Z-Image.
        let idx = index(vec![q21("q8", "qwen_image_2.1-Q8_0.gguf", 7_687_155_744)]);
        let r = recommend_role(&reg, &idx, &hw(10.0), "realistic").unwrap();
        assert_eq!(r.pick.family_id.as_deref(), Some("z_image_turbo"));
    }

    #[test]
    fn cpu_only_gets_the_small_sd15_pick() {
        let p = picks(0.0, &index(vec![]));
        // Realistic: Z-Image and Juggernaut are too big for the processor → SD 1.5,
        // sized against RAM (Tight = "runs on the processor — slow").
        let r = role(&p, "realistic");
        assert_eq!(summary(r), (Some("sd15"), Some("fp16"), Some(Fit::Tight)));
        assert_eq!(
            r.pick.title.as_deref(),
            Some("Stable Diffusion 1.5 (small, runs on any computer)")
        );
        assert_eq!(r.pick.unavailable_reason, None);
        assert_eq!(
            r.pick.download_bytes,
            2133 * MB,
            "all-in-one: no components"
        );
        let v = r.pick.vram.unwrap();
        assert!(v.on_cpu && v.estimate, "{v:?}");
        assert_eq!(v.gb, 3.0, "1.99 GiB of weights + 1 GiB activations");
        match &r.action {
            PickAction::Download { label, files } => {
                assert_eq!(label, "Stable Diffusion 1.5 (small, runs on any computer)");
                assert_eq!(files.len(), 1);
                assert_eq!(files[0].file_name, "v1-5-pruned-emaonly-fp16.safetensors");
                assert_eq!(files[0].kind, ModelKind::Checkpoint);
                assert_eq!(files[0].family.as_deref(), Some("sd15"));
                assert_eq!(
                    files[0].sha256.as_deref(),
                    Some("e9476a13728cd75d8279f6ec8bad753a66a1957ca375a1464dc63b37db6e3916")
                );
            }
            other => panic!("{other:?}"),
        }
        // Anime / edit: nothing runs acceptably on the processor. No "Models tab" advice.
        for r in ["anime", "edit"] {
            let pick = &role(&p, r).pick;
            assert_eq!(pick.title, None, "{r}");
            let why = pick.unavailable_reason.as_deref().unwrap();
            assert!(why.contains("graphics card"), "{r}: {why}");
            assert!(!why.contains("Models tab"), "{r}: {why}");
            assert_eq!(role(&p, r).action, PickAction::Nothing);
        }
        assert!(role(&p, "edit")
            .pick
            .unavailable_reason
            .as_deref()
            .unwrap()
            .contains("Restyle"));
        // The small captioner still works on the CPU.
        let d = role(&p, "describe");
        assert_eq!(d.pick.title.as_deref(), Some("Image describer"));
        assert_eq!(d.pick.download_bytes, (1930 + 845) * MB);
        assert_eq!(d.action, PickAction::Captioner);
    }

    #[test]
    fn cpu_only_with_little_ram() {
        let reg = registry();
        // 3 GB of weights + activations + 4 GB spare: 6 GB of RAM is not enough, 8 GB (7.7 GiB) is.
        let r = recommend_role(&reg, &index(vec![]), &hw_cpu(6.0), "realistic").unwrap();
        assert_eq!(r.pick.title, None);
        assert_eq!(
            r.pick.unavailable_reason.as_deref(),
            Some("Pinhole didn't find a graphics card it can use, and this computer doesn't have enough memory to run even the small model on the processor. It needs at least 7 GB of RAM.")
        );
        let r = recommend_role(&reg, &index(vec![]), &hw_cpu(7.7), "realistic").unwrap();
        assert_eq!(summary(&r), (Some("sd15"), Some("fp16"), Some(Fit::Tight)));
        // RAM not known yet (detection still running): the size rule alone decides.
        let r = recommend_role(&reg, &index(vec![]), &hw_cpu(0.0), "realistic").unwrap();
        assert_eq!(r.pick.family_id.as_deref(), Some("sd15"));
    }

    #[test]
    fn small_gpus_fall_back_to_sd15() {
        // 4 GB: Z-Image Q4 needs 5, Juggernaut 6 → SD 1.5 (min 4) on the GPU.
        let p = picks(4.0, &index(vec![]));
        let r = role(&p, "realistic");
        assert_eq!(summary(r), (Some("sd15"), Some("fp16"), Some(Fit::Tight)));
        assert!(!r.pick.vram.unwrap().on_cpu);
        assert_eq!(
            r.pick.vram.unwrap().gb,
            6.0,
            "registry figure, not an estimate"
        );
        // 6 GB: the better picks come first.
        assert_eq!(
            role(&picks(6.0, &index(vec![])), "realistic")
                .pick
                .family_id
                .as_deref(),
            Some("z_image_turbo")
        );
    }

    #[test]
    fn too_little_vram_message() {
        let p = picks(3.0, &index(vec![]));
        let r = &role(&p, "realistic").pick;
        assert_eq!(r.title, None);
        assert_eq!(
            r.unavailable_reason.as_deref(),
            Some("None of the recommended models fit in 3 GB of graphics memory. Smaller ones are available when you browse CivitAI.")
        );
    }

    #[test]
    fn installed_sd15_on_cpu_is_complete_and_sized_against_ram() {
        let reg = registry();
        let mut f = model(
            "sd",
            "sd15",
            ModelKind::Checkpoint,
            "v1-5-pruned-emaonly-fp16.safetensors",
        );
        f.size_bytes = 2_132_696_762;
        let r = recommend_role(&reg, &index(vec![f]), &hw_cpu(16.0), "realistic").unwrap();
        assert!(r.pick.installed);
        assert_eq!(r.action, PickAction::Nothing);
        assert_eq!(r.pick.fit, Some(Fit::Tight));
        assert!(r.pick.vram.unwrap().on_cpu);
    }

    #[test]
    fn unverified_civitai_candidates_are_unavailable() {
        let yaml = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../config/models.yaml"
        ))
        .replace("civitai_version_id: 2940478", "civitai_version_id: TODO")
        .replace("civitai_version_id: 290640", "civitai_version_id: TODO");
        let reg = Registry::from_yaml(&yaml, None).unwrap();
        let p = recommend(&reg, &index(vec![]), &hw(16.0));
        let a = &role(&p, "anime").pick;
        assert_eq!(a.title, None);
        assert_eq!(a.unavailable_reason.as_deref(), Some("No anime model has been picked for Pinhole yet. You can browse CivitAI for one instead."));
    }

    #[test]
    fn shared_components_counted_once_and_only_when_missing() {
        let reg = registry();
        // Z-Image installed with its components → realistic is done.
        let idx = index(vec![
            model(
                "zit",
                "z_image_turbo",
                ModelKind::Diffusion,
                "z_image_turbo-Q8_0.gguf",
            ),
            component(&reg, "flux_ae"),
            component(&reg, "qwen3_4b_q4km"),
        ]);
        let p = recommend(&reg, &idx, &hw(8.0));
        let r = role(&p, "realistic");
        assert!(r.pick.installed);
        assert_eq!(r.pick.download_bytes, 0);
        assert_eq!(r.pick.quant.as_deref(), Some("q8_0"));
        assert_eq!(r.action, PickAction::Nothing);
        // Qwen-Image 2.1 with the old Qwen-Image VAE installed: its own VAE is still needed
        // (not interchangeable), and the text encoder + vision file are counted once.
        let idx = index(vec![
            component(&reg, "qwen_image_vae"),
            component(&reg, "qwen3vl_8b_q4km"),
        ]);
        let e = recommend_role(&reg, &idx, &hw(8.0), "edit").unwrap();
        assert_eq!(e.pick.download_bytes, (4197 + 676 + 1159) * MB);
        match &e.action {
            PickAction::Download { files, .. } => {
                assert!(files
                    .iter()
                    .all(|f| f.component_id.as_deref() != Some("qwen3vl_8b_q4km")));
                assert!(files
                    .iter()
                    .any(|f| f.component_id.as_deref() == Some("qwen_image_21_vae")));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn installed_model_with_missing_component_is_not_complete() {
        let reg = registry();
        let idx = index(vec![
            model(
                "zit",
                "z_image_turbo",
                ModelKind::Diffusion,
                "z_image_turbo_bf16.safetensors",
            ),
            component(&reg, "flux_ae"),
        ]);
        let r = recommend_role(&reg, &idx, &hw(24.0), "realistic_fast").unwrap();
        assert!(!r.pick.installed);
        assert_eq!(
            r.pick.download_bytes,
            8045 * MB,
            "only the bf16 text encoder at 24 GB"
        );
        assert_eq!(
            r.pick.vram.unwrap().gb,
            16.0,
            "registry figure for the bf16 file"
        );
        assert_eq!(r.pick.note, None);
        // An installed Q8 encoder is reused at 24 GB: no second text encoder download.
        let idx = index(vec![
            model(
                "zit",
                "z_image_turbo",
                ModelKind::Diffusion,
                "z_image_turbo_bf16.safetensors",
            ),
            component(&reg, "flux_ae"),
            component(&reg, "qwen3_4b_q8"),
        ]);
        let r = recommend_role(&reg, &idx, &hw(24.0), "realistic_fast").unwrap();
        assert!(r.pick.installed);
        assert_eq!(r.pick.download_bytes, 0);
    }

    #[test]
    fn tight_installed_version_offers_the_smaller_one_that_fits() {
        let reg = registry();
        // Qwen-Image 2.1 bf16 installed on a 16 GB card: Tight (17.4 GB). Q8_0 Fits → offered.
        let idx = index(vec![
            q21("q21", "qwen_image_2.1_bf16.safetensors", 14_230_280_616),
            component(&reg, "qwen_image_21_vae"),
            component(&reg, "qwen3vl_8b_q4km"),
            component(&reg, "qwen3vl_8b_mmproj"),
        ]);
        for r in ["realistic", "edit"] {
            let e = recommend_role(&reg, &idx, &hw(16.0), r).unwrap();
            assert_eq!(
                summary(&e),
                (Some("qwen_image_21"), Some("q8_0"), Some(Fit::Fits)),
                "{r}"
            );
            assert!(!e.pick.installed);
            assert_eq!(e.pick.note.as_deref(), Some(TIGHT_INSTALLED_NOTE));
            assert!(e.pick.replaces_installed);
            assert_eq!(
                e.pick.download_bytes,
                7687 * MB,
                "only the smaller model; parts are shared"
            );
        }
        // Once the smaller version is installed, the card shows it as done.
        let mut files = idx.files.clone();
        files.push(q21("q8", "qwen_image_2.1-Q8_0.gguf", 7_687_155_744));
        let e = recommend_role(&reg, &index(files), &hw(16.0), "edit").unwrap();
        assert!(e.pick.installed);
        assert_eq!(e.pick.fit, Some(Fit::Fits));
        assert_eq!(e.action, PickAction::Nothing);
        // On a 24 GB card the installed bf16 Fits: nothing to offer (the Q8 text encoder
        // is the one missing part there).
        let e = recommend_role(&reg, &idx, &hw(24.0), "edit").unwrap();
        assert_eq!(e.pick.quant.as_deref(), Some("bf16"));
        assert_eq!(e.pick.note, None);
    }

    #[test]
    fn civitai_pick_installed() {
        let reg = registry();
        let idx = index(vec![
            with_civitai(
                model(
                    "nova",
                    "sdxl_illustrious",
                    ModelKind::Checkpoint,
                    "novaAnimeXL_ilV190.safetensors",
                ),
                2940478,
            ),
            component(&reg, "sdxl_vae_fp16_fix"),
        ]);
        let a = recommend_role(&reg, &idx, &hw(8.0), "anime").unwrap();
        assert!(a.pick.installed);
        assert_eq!(a.pick.download_bytes, 0);
        assert_eq!(a.action, PickAction::Nothing);
    }

    #[test]
    fn captioner_reuse() {
        let reg = registry();
        let idx = index(vec![
            component(&reg, "qwen25_vl_7b_q8"),
            component(&reg, "qwen25_vl_7b_mmproj"),
        ]);
        assert!(captioner_reuse_available(&reg, &idx));
        let d = recommend_role(&reg, &idx, &hw(16.0), "describe").unwrap();
        assert!(d.pick.installed);
        assert_eq!(d.pick.download_bytes, 0);
        assert_eq!(d.action, PickAction::Nothing);
        // Default captioner installed by file name (engine area registers it).
        let mut f = component(&reg, "flux_ae");
        f.rel_path = "models/captioners/Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf".into();
        f.component_id = None;
        f.sha256 = crate::testkit::sha(1);
        let idx = index(vec![f]);
        let missing = missing_captioner_files(&reg, &idx);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].file, "mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf");
    }

    #[test]
    fn pick_json_matches_types_ts() {
        let p = picks(8.0, &index(vec![]));
        let v = serde_json::to_value(&role(&p, "realistic").pick).unwrap();
        for key in [
            "role",
            "roleLabel",
            "title",
            "familyId",
            "goodAt",
            "downloadBytes",
            "vram",
            "fit",
            "installed",
            "quant",
            "licenseNote",
            "unavailableReason",
        ] {
            assert!(v.get(key).is_some(), "{key}");
        }
        assert_eq!(v["fit"], "tight");
    }

    #[test]
    fn yaml_ids() {
        assert_eq!(yaml_id(Some(&serde_yaml::Value::from(12u64))), Some(12));
        assert_eq!(yaml_id(Some(&serde_yaml::Value::from("TODO"))), None);
        assert_eq!(yaml_id(Some(&serde_yaml::Value::from("34"))), Some(34));
        assert_eq!(yaml_id(None), None);
    }

    #[test]
    fn older_edit_models_are_no_longer_offered() {
        // Qwen Image Edit 2511 installed on 16 GB (as before Qwen-Image 2.1): the Edit card
        // now offers Qwen-Image 2.1, which runs there at Q8 instead of a 3-bit copy.
        let reg = registry();
        let idx = index(vec![
            model(
                "qe",
                "qwen_image_edit_2511",
                ModelKind::Diffusion,
                "qwen-image-edit-2511-Q3_K_M.gguf",
            ),
            component(&reg, "qwen_image_vae"),
            component(&reg, "qwen25_vl_7b_q8"),
            component(&reg, "qwen25_vl_7b_mmproj"),
        ]);
        let e = recommend_role(&reg, &idx, &hw(16.0), "edit").unwrap();
        assert_eq!(
            summary(&e),
            (Some("qwen_image_21"), Some("q8_0"), Some(Fit::Fits))
        );
        assert!(!e.pick.installed);
        assert!(!e.pick.replaces_installed);
        assert!(picks(16.0, &index(vec![]))
            .iter()
            .all(|p| !["edit_alt", "realistic_detail"].contains(&p.pick.role.as_str())));
    }
}
