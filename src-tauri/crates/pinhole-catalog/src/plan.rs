//! CivitAI install plans (SPEC §5.4 "Install" steps 1–4) and the file list an
//! install downloads: safe model file + missing components, family resolved.

use pinhole_registry::vram;
use pinhole_registry::wiring::HwContext;
use pinhole_registry::Registry;
use pinhole_store::datadir::ModelKind;
use pinhole_store::installed::CivitaiRef;
use pinhole_store::InstalledIndex;

use crate::api::{Model, ModelVersion};
use crate::families::{self, mb_to_bytes, FamilyResolution};
use crate::filters::CatalogFilters;
use crate::local;
use crate::recommend::FileToGet;
use crate::select;
use crate::view::{FamilyChoice, InstallPlan, PlanComponent, PlanFile};

/// Kept free on top of the download so the disk isn't filled to the brim.
pub const DISK_MARGIN_BYTES: u64 = 512 * 1_000_000;

/// Registry, hardware and installed files the plan is computed against.
pub struct PlanEnv<'a> {
    pub registry: &'a Registry,
    pub index: &'a InstalledIndex,
    pub hw: &'a HwContext,
    pub filters: &'a CatalogFilters,
}

/// CivitAI type of the version's model (`Checkpoint`, `LORA`…).
pub fn version_kind(version: &ModelVersion, model: Option<&Model>) -> String {
    version.model.as_ref().map(|m| m.kind.clone()).filter(|k| !k.is_empty()).or_else(|| model.map(|m| m.kind.clone())).unwrap_or_default()
}

pub fn model_name(version: &ModelVersion, model: Option<&Model>) -> String {
    version
        .model
        .as_ref()
        .map(|m| m.name.clone())
        .filter(|n| !n.is_empty())
        .or_else(|| model.map(|m| m.name.clone()))
        .unwrap_or_else(|| format!("CivitAI model {}", version.model_id))
}

pub fn friendly_name(version: &ModelVersion, model: Option<&Model>) -> String {
    let m = model_name(version, model);
    if version.name.trim().is_empty() {
        m
    } else {
        format!("{m} · {}", version.name.trim())
    }
}

fn license_note(env: &PlanEnv, family_id: Option<&str>, model: Option<&Model>) -> Option<String> {
    family_id
        .and_then(|id| env.registry.family(id))
        .and_then(|f| f.license_note.clone())
        .or_else(|| model.filter(|m| !m.allow_commercial_use.allows("Image")).map(|_| "The creator doesn't allow commercial use.".to_string()))
}

/// The install plan shown before downloading.
pub fn build_plan(
    env: &PlanEnv,
    version: &ModelVersion,
    model: Option<&Model>,
    free_disk_bytes: u64,
    needs_api_key: bool,
) -> InstallPlan {
    let kind = version_kind(version, model);
    let is_lora = env.filters.is_lora_type(&kind);
    let picked = select::select_file(&version.files, &env.filters.allowed_file_formats);
    let mut blocked_reason = picked.as_ref().err().cloned();
    let main_file = match &picked {
        Ok(f) => PlanFile { name: f.name.clone(), size_bytes: f.size_bytes(), format: select::file_format(f) },
        Err(_) => version
            .files
            .first()
            .map(|f| PlanFile { name: f.name.clone(), size_bytes: f.size_bytes(), format: select::file_format(f) })
            .unwrap_or(PlanFile { name: String::new(), size_bytes: 0, format: String::new() }),
    };
    let sha = picked.as_ref().ok().and_then(|f| f.sha256());
    let main_installed = sha.as_deref().is_some_and(|h| env.index.find_by_sha(h).is_some());

    let (family, candidates): (Option<FamilyChoice>, Vec<FamilyChoice>) =
        match families::resolve_family(env.registry, sha.as_deref(), Some(&version.base_model), None) {
            FamilyResolution::Resolved(id) => (families::family_choice(env.registry, &id), Vec::new()),
            FamilyResolution::Ambiguous(ids) => (None, ids.iter().filter_map(|id| families::family_choice(env.registry, id)).collect()),
            FamilyResolution::Unsupported(base) => {
                if blocked_reason.is_none() {
                    blocked_reason = Some(families::unsupported_message(base.as_deref()));
                }
                (None, Vec::new())
            }
        };
    if model.is_some_and(|m| m.is_unavailable()) || version.model.as_ref().is_some_and(|m| {
        matches!(m.mode.as_deref().map(str::to_ascii_lowercase).as_deref(), Some("archived") | Some("takendown"))
    }) {
        blocked_reason = Some("This model was archived by its creator and can't be downloaded.".into());
    }

    // Components for the resolved family (or the first candidate, as an estimate).
    let comp_family = family.as_ref().map(|f| f.family_id.clone()).or_else(|| candidates.first().map(|c| c.family_id.clone()));
    let mut components = Vec::new();
    if !is_lora {
        if let Some(fam) = comp_family.as_deref().and_then(|id| env.registry.family(id)) {
            for rc in families::wanted_components(env.registry, fam, env.hw, true) {
                if let Some(c) = env.registry.component(&rc.component_id) {
                    components.push(PlanComponent {
                        component_id: rc.component_id.clone(),
                        label: families::component_label(c),
                        size_bytes: mb_to_bytes(c.size_mb),
                        installed: families::installed_component(env.index, &rc.component_id, c).is_some(),
                    });
                }
            }
        }
    }
    let total_download_bytes = if main_installed { 0 } else { main_file.size_bytes }
        + components.iter().filter(|c| !c.installed).map(|c| c.size_bytes).sum::<u64>();

    let need = match (&family, is_lora) {
        (Some(f), false) => env.registry.family(&f.family_id).map(|fam| families::family_need(env.registry, fam, env.hw, main_file.size_bytes)),
        _ => None,
    };
    let family_id = family.as_ref().map(|f| f.family_id.clone());
    InstallPlan {
        version_id: version.id,
        model_name: model_name(version, model),
        version_name: version.name.clone(),
        main_file,
        family,
        family_candidates: candidates,
        components,
        total_download_bytes,
        free_disk_bytes,
        enough_disk: free_disk_bytes >= total_download_bytes.saturating_add(DISK_MARGIN_BYTES),
        vram: need,
        fit: need.map(|n| vram::fit(&n, env.hw.vram_gb)),
        license_note: license_note(env, family_id.as_deref(), model),
        is_lora,
        trained_words: version.trained_words.clone(),
        blocked_reason,
        needs_api_key,
    }
}

/// What `install_civitai` downloads: the safe model file (unless already
/// installed) + missing components (LoRAs need none).
#[derive(Debug, Clone)]
pub struct CivitaiInstall {
    pub label: String,
    /// Main file first.
    pub files: Vec<FileToGet>,
    pub civitai: CivitaiRef,
    pub main_installed: bool,
}

/// `family_id` must already be decided (resolved, or picked by the user).
pub fn civitai_install_files(
    env: &PlanEnv,
    version: &ModelVersion,
    model: Option<&Model>,
    family_id: Option<&str>,
) -> Result<CivitaiInstall, String> {
    let file = select::select_file(&version.files, &env.filters.allowed_file_formats)?;
    let kind = version_kind(version, model);
    let is_lora = env.filters.is_lora_type(&kind);
    let family = family_id.and_then(|id| env.registry.family(id));
    if !is_lora && family.is_none() {
        return Err("Pick which kind of model this is first.".into());
    }
    let ext = local::allowed_extension(std::path::Path::new(&file.name)).ok_or_else(|| {
        "This model isn't available as a .safetensors or .gguf file, so Pinhole can't install it.".to_string()
    })?;
    let sha = file.sha256();
    let main_installed = sha.as_deref().is_some_and(|h| env.index.find_by_sha(h).is_some());
    let label = friendly_name(version, model);
    let mut files = Vec::new();
    if !main_installed {
        files.push(FileToGet {
            url: file.download_url.clone(),
            file_name: local::sanitize_file_name(&file.name, ext),
            sha256: sha.clone(),
            size_bytes: file.size_bytes(),
            kind: if is_lora { ModelKind::Lora } else { family.map(families::main_model_kind).unwrap_or(ModelKind::Checkpoint) },
            friendly_name: label.clone(),
            family: family_id.map(str::to_string),
            component_id: None,
            dtype: file
                .metadata
                .fp
                .clone()
                .map(|s| s.to_ascii_lowercase())
                .or_else(|| Some(families::quant_of_file(&file.name)).filter(|q| q != "unknown")),
        });
    }
    if let (false, Some(fam)) = (is_lora, family) {
        for (id, c) in families::missing_components(env.registry, fam, env.hw, env.index, true) {
            files.push(FileToGet {
                url: c.url.clone(),
                file_name: c.file.clone(),
                sha256: families::normalize_sha(&c.sha256),
                size_bytes: mb_to_bytes(c.size_mb),
                kind: families::component_model_kind(&c.kind),
                friendly_name: families::component_label(c),
                family: None,
                component_id: Some(id),
                dtype: None,
            });
        }
    }
    let civitai = CivitaiRef {
        model_id: if version.model_id > 0 { version.model_id } else { model.map(|m| m.id).unwrap_or(0) },
        version_id: version.id,
        model_name: Some(model_name(version, model)),
        version_name: Some(version.name.clone()).filter(|n| !n.is_empty()),
        base_model: Some(version.base_model.clone()).filter(|b| !b.is_empty()),
        trained_words: version.trained_words.clone(),
        license: model.filter(|m| !m.allow_commercial_use.allows("Image")).map(|_| "No commercial use".to_string()),
    };
    Ok(CivitaiInstall { label, files, civitai, main_installed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::tests::filters;
    use crate::testkit::{component, hw, index, registry};
    use pinhole_registry::vram::Fit;

    fn jugg() -> (ModelVersion, Model) {
        (
            serde_json::from_str(include_str!("../tests/fixtures/model_version.json")).unwrap(),
            serde_json::from_str(include_str!("../tests/fixtures/model.json")).unwrap(),
        )
    }

    #[test]
    fn checkpoint_plan() {
        let reg = registry();
        let f = filters();
        let idx = index(vec![]);
        let h = hw(8.0);
        let env = PlanEnv { registry: &reg, index: &idx, hw: &h, filters: &f };
        let (v, m) = jugg();
        let p = build_plan(&env, &v, Some(&m), 100_000_000_000, false);
        assert_eq!(p.version_id, 1759168);
        assert_eq!(p.model_name, "Juggernaut XL");
        assert_eq!(p.version_name, "Ragnarok");
        assert_eq!(p.main_file.name, "juggernautXL_ragnarok.safetensors");
        assert_eq!(p.main_file.format, "SafeTensor");
        assert_eq!(p.family.as_ref().unwrap().family_id, "sdxl", "known hash");
        assert!(p.family_candidates.is_empty());
        assert_eq!(p.components.len(), 1);
        assert_eq!(p.components[0].component_id, "sdxl_vae_fp16_fix");
        assert!(!p.components[0].installed);
        assert_eq!(p.total_download_bytes, 7_105_349_736 + 335_000_000);
        assert!(p.enough_disk);
        assert_eq!(p.vram.unwrap().gb, 10.0);
        assert_eq!(p.fit, Some(Fit::Tight));
        assert_eq!(p.license_note.as_deref(), Some("The creator doesn't allow commercial use."));
        assert!(!p.is_lora);
        assert_eq!(p.blocked_reason, None);
        assert!(!p.needs_api_key);
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["mainFile"]["sizeBytes"], 7_105_349_736u64);
        assert_eq!(json["components"][0]["componentId"], "sdxl_vae_fp16_fix");

        // Component already installed (matched by SHA-256) + little disk.
        let mut vae = component(&reg, "sdxl_vae_fp16_fix");
        vae.component_id = None;
        let idx = index(vec![vae]);
        let env = PlanEnv { registry: &reg, index: &idx, hw: &h, filters: &f };
        let p = build_plan(&env, &v, Some(&m), 7_000_000_000, true);
        assert!(p.components[0].installed);
        assert_eq!(p.total_download_bytes, 7_105_349_736);
        assert!(!p.enough_disk);
        assert!(p.needs_api_key);
    }

    #[test]
    fn ambiguous_and_unsupported_plans() {
        let reg = registry();
        let f = filters();
        let idx = index(vec![]);
        let h = hw(16.0);
        let env = PlanEnv { registry: &reg, index: &idx, hw: &h, filters: &f };
        let (mut v, _) = jugg();
        v.base_model = "Qwen".into();
        v.files[0].hashes.clear();
        let p = build_plan(&env, &v, None, u64::MAX, false);
        assert_eq!(p.family, None);
        let ids: Vec<&str> = p.family_candidates.iter().map(|c| c.family_id.as_str()).collect();
        assert_eq!(ids, ["qwen_image", "qwen_image_edit_2511"]);
        assert!(!p.components.is_empty(), "estimated with the first candidate");
        assert_eq!(p.blocked_reason, None);

        v.base_model = "SD 3.5 Large".into();
        let p = build_plan(&env, &v, None, u64::MAX, false);
        assert_eq!(p.blocked_reason.as_deref(), Some("Pinhole can't run SD 3.5 Large models yet."));

        v.base_model = "Other".into();
        let p = build_plan(&env, &v, None, u64::MAX, false);
        assert!(p.family_candidates.len() > 5, "Other → ask among every family");
        assert_eq!(p.blocked_reason, None);
    }

    #[test]
    fn lora_plan_and_install_files() {
        let reg = registry();
        let f = filters();
        let idx = index(vec![]);
        let h = hw(8.0);
        let env = PlanEnv { registry: &reg, index: &idx, hw: &h, filters: &f };
        let v: ModelVersion = serde_json::from_str(include_str!("../tests/fixtures/by_hash_lora.json")).unwrap();
        let p = build_plan(&env, &v, None, u64::MAX, false);
        assert!(p.is_lora);
        assert_eq!(p.family.as_ref().unwrap().family_id, "sd15");
        assert!(p.components.is_empty());
        assert_eq!(p.trained_words, ["watercolor"]);
        assert_eq!(p.vram, None);
        assert_eq!(p.model_name, "Soft Watercolor");

        let inst = civitai_install_files(&env, &v, None, Some("sd15")).unwrap();
        assert_eq!(inst.files.len(), 1);
        assert_eq!(inst.files[0].kind, ModelKind::Lora);
        assert_eq!(inst.civitai.trained_words, ["watercolor"]);
        assert_eq!(inst.civitai.model_id, 81234);
        assert_eq!(inst.civitai.base_model.as_deref(), Some("SD 1.5"));
    }

    #[test]
    fn checkpoint_install_files() {
        let reg = registry();
        let f = filters();
        let idx = index(vec![]);
        let h = hw(8.0);
        let env = PlanEnv { registry: &reg, index: &idx, hw: &h, filters: &f };
        let (v, m) = jugg();
        let inst = civitai_install_files(&env, &v, Some(&m), Some("sdxl")).unwrap();
        assert_eq!(inst.label, "Juggernaut XL · Ragnarok");
        assert_eq!(inst.files.len(), 2);
        let main = &inst.files[0];
        assert_eq!(main.kind, ModelKind::Checkpoint);
        assert_eq!(main.file_name, "juggernautXL_ragnarok.safetensors");
        assert_eq!(main.url, "https://civitai.com/api/download/models/1759168");
        assert_eq!(main.sha256.as_deref(), Some("dd08fa32f98d05a2443ca1419e46df1575a0811f6e3b246d9dd47ff20f5eb66a"));
        assert_eq!(main.family.as_deref(), Some("sdxl"));
        assert_eq!(main.dtype.as_deref(), Some("fp16"));
        assert_eq!(inst.files[1].component_id.as_deref(), Some("sdxl_vae_fp16_fix"));
        assert_eq!(inst.civitai.license.as_deref(), Some("No commercial use"));
        assert!(civitai_install_files(&env, &v, Some(&m), None).is_err(), "family must be decided");

        // Already installed main file → only missing components.
        let mut have = crate::testkit::model("j", "sdxl", ModelKind::Checkpoint, "j.safetensors");
        have.sha256 = "dd08fa32f98d05a2443ca1419e46df1575a0811f6e3b246d9dd47ff20f5eb66a".into();
        let idx = index(vec![have, component(&reg, "sdxl_vae_fp16_fix")]);
        let env = PlanEnv { registry: &reg, index: &idx, hw: &h, filters: &f };
        let inst = civitai_install_files(&env, &v, Some(&m), Some("sdxl")).unwrap();
        assert!(inst.main_installed);
        assert!(inst.files.is_empty());
    }

    #[test]
    fn blocked_files_refuse_install() {
        let reg = registry();
        let f = filters();
        let idx = index(vec![]);
        let h = hw(8.0);
        let env = PlanEnv { registry: &reg, index: &idx, hw: &h, filters: &f };
        let (mut v, m) = jugg();
        v.files[0].name = "juggernaut.ckpt".into();
        v.files[0].metadata.format = Some("PickleTensor".into());
        let p = build_plan(&env, &v, Some(&m), u64::MAX, false);
        assert!(p.blocked_reason.unwrap().contains(".ckpt"));
        assert!(civitai_install_files(&env, &v, Some(&m), Some("sdxl")).unwrap_err().contains(".ckpt"));
    }
}
