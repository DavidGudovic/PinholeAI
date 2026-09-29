//! Installed models: view models for the Installed list / model picker,
//! missing components, and what a delete removes (model + orphaned components).

use std::collections::BTreeSet;

use pinhole_registry::wiring::HwContext;
use pinhole_registry::Registry;
use pinhole_store::datadir::ModelKind;
use pinhole_store::{InstalledFile, InstalledIndex};

use crate::families;
use crate::view::{DeleteFile, DeletePreview, DeleteReason, InstalledLora, InstalledModel};

/// Friendly badge for installed models: the recommended role the family heads
/// (`realistic` → "Realistic", `anime` → "Anime"), only for files installed from
/// the registry (CivitAI finetunes of a family can be any style). "Heads" = the
/// role's first registry candidate: a last-resort fallback (SD 1.5, whose
/// finetunes come in every style) gets no badge.
pub fn registry_style_badge(registry: &Registry, file: &InstalledFile) -> Option<String> {
    if file.civitai.is_some() {
        return None;
    }
    let family = file.family.as_deref()?;
    for (role, label) in [
        ("realistic", "Realistic"),
        ("realistic_detail", "Realistic"),
        ("anime", "Anime"),
    ] {
        let heads = registry
            .recommended()
            .get(role)
            .and_then(|c| c.iter().find(|c| c.source.as_deref() == Some("registry")))
            .is_some_and(|c| c.family.as_deref() == Some(family));
        if heads {
            return Some(label.to_string());
        }
    }
    None
}

pub fn installed_model_view(
    registry: &Registry,
    index: &InstalledIndex,
    file: &InstalledFile,
    hw: &HwContext,
) -> InstalledModel {
    let family = file.family.as_deref().and_then(|id| registry.family(id));
    let (vram, fit, missing, modes, is_edit, family_label, license) = match family {
        Some(f) => {
            let (need, fit) = families::need_and_fit(
                registry,
                f,
                hw,
                index,
                families::installed_need(registry, f, file, hw, index),
                file.size_bytes,
            );
            let missing: Vec<String> = families::missing_to_run(registry, f, hw, index)
                .into_iter()
                .map(|(_, c)| families::component_label(c))
                .collect();
            let is_edit = f.role.as_deref() == Some("edit") || f.modes.iter().any(|m| m == "edit");
            (
                Some(need),
                Some(fit),
                missing,
                f.modes.clone(),
                is_edit,
                Some(f.label.clone()),
                f.license_note.clone(),
            )
        }
        None => (None, None, Vec::new(), Vec::new(), false, None, None),
    };
    InstalledModel {
        id: file.id.clone(),
        friendly_name: file.friendly_name.clone(),
        family_id: file.family.clone(),
        family_label,
        style_badge: registry_style_badge(registry, file),
        modes,
        is_edit_model: is_edit,
        size_bytes: file.size_bytes,
        vram,
        fit,
        last_used: file.last_used,
        missing_components: missing,
        license_note: license.or_else(|| file.civitai.as_ref().and_then(|c| c.license.clone())),
        civitai_model_id: file.civitai.as_ref().map(|c| c.model_id),
        civitai_version_id: file.civitai.as_ref().map(|c| c.version_id),
        base_model: file.civitai.as_ref().and_then(|c| c.base_model.clone()),
    }
}

pub fn installed_lora_view(file: &InstalledFile) -> InstalledLora {
    InstalledLora {
        id: file.id.clone(),
        friendly_name: file.friendly_name.clone(),
        family_id: file.family.clone(),
        base_model: file.civitai.as_ref().and_then(|c| c.base_model.clone()),
        trained_words: file
            .civitai
            .as_ref()
            .map(|c| c.trained_words.clone())
            .unwrap_or_default(),
        size_bytes: file.size_bytes,
        civitai_model_id: file.civitai.as_ref().map(|c| c.model_id),
        civitai_version_id: file.civitai.as_ref().map(|c| c.version_id),
    }
}

/// Installed component files that only `model_id` needs: components its family
/// can use that no *other* installed model's family can use.
pub fn orphaned_components<'a>(
    registry: &Registry,
    index: &'a InstalledIndex,
    model_id: &str,
) -> Vec<&'a InstalledFile> {
    let Some(target) = index.get(model_id) else {
        return Vec::new();
    };
    if !matches!(target.kind, ModelKind::Checkpoint | ModelKind::Diffusion) {
        return Vec::new();
    }
    let Some(family) = target.family.as_deref().and_then(|f| registry.family(f)) else {
        return Vec::new();
    };
    let ours = families::family_component_ids(family);
    let still_used: BTreeSet<String> = index
        .models()
        .filter(|m| m.id != target.id)
        .filter_map(|m| m.family.as_deref().and_then(|f| registry.family(f)))
        .flat_map(families::family_component_ids)
        .collect();
    let mut out: Vec<&InstalledFile> = Vec::new();
    for id in ours.difference(&still_used) {
        let Some(comp) = registry.component(id) else {
            continue;
        };
        for f in index.files.iter().filter(|f| {
            f.component_id.as_deref() == Some(id.as_str())
                || (f.component_id.is_none()
                    && families::normalize_sha(&comp.sha256)
                        .is_some_and(|h| f.sha256.eq_ignore_ascii_case(&h)))
        }) {
            if !out.iter().any(|o| o.id == f.id) {
                out.push(f);
            }
        }
    }
    out
}

/// What "Delete" removes for `model_id` (any installed file id).
pub fn delete_preview(
    registry: &Registry,
    index: &InstalledIndex,
    model_id: &str,
) -> Option<DeletePreview> {
    let target = index.get(model_id)?;
    let mut files = vec![DeleteFile {
        rel_path: target.rel_path.clone(),
        size_bytes: target.size_bytes,
        reason: DeleteReason::Model,
    }];
    files.extend(
        orphaned_components(registry, index, model_id)
            .into_iter()
            .map(|f| DeleteFile {
                rel_path: f.rel_path.clone(),
                size_bytes: f.size_bytes,
                reason: DeleteReason::OrphanComponent,
            }),
    );
    Some(DeletePreview {
        model_id: model_id.to_string(),
        files,
    })
}

/// `true` if `rel_path` stays inside the Data folder (relative, no `..`).
pub fn is_safe_rel_path(rel_path: &str) -> bool {
    let p = std::path::Path::new(rel_path);
    !rel_path.is_empty()
        && !rel_path.starts_with('/')
        && !rel_path.starts_with('\\')
        && !p.is_absolute()
        && !rel_path.contains(':')
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
        && !rel_path.split(['/', '\\']).any(|s| s == "..")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{component, hw, hw_cpu, index, model, registry, with_civitai};
    use pinhole_registry::vram::Fit;

    fn ids(v: Vec<&InstalledFile>) -> Vec<String> {
        let mut out: Vec<String> = v
            .into_iter()
            .map(|f| f.component_id.clone().unwrap_or_else(|| f.id.clone()))
            .collect();
        out.sort();
        out
    }

    #[test]
    fn orphans_skip_components_other_models_use() {
        let reg = registry();
        let idx = index(vec![
            model("zit", "z_image_turbo", ModelKind::Diffusion, "z.gguf"),
            model("kontext", "flux1_kontext", ModelKind::Diffusion, "k.gguf"),
            component(&reg, "flux_ae"),
            component(&reg, "qwen3_4b"),
            component(&reg, "clip_l"),
            component(&reg, "t5xxl_fp8"),
        ]);
        assert_eq!(
            ids(orphaned_components(&reg, &idx, "zit")),
            ["qwen3_4b"],
            "the FLUX VAE is shared with Kontext"
        );
        assert_eq!(
            ids(orphaned_components(&reg, &idx, "kontext")),
            ["clip_l", "t5xxl_fp8"]
        );
        let p = delete_preview(&reg, &idx, "kontext").unwrap();
        assert_eq!(p.files[0].reason, DeleteReason::Model);
        assert_eq!(p.files[0].rel_path, "models/diffusion/k.gguf");
        assert_eq!(p.files.len(), 3);
        assert!(p.files[1..]
            .iter()
            .all(|f| f.reason == DeleteReason::OrphanComponent));
    }

    #[test]
    fn vram_dependent_choices_count_as_used() {
        let reg = registry();
        // FLUX.1 dev can use t5xxl_fp8 (below 16 GB) → Kontext's encoder stays.
        let idx = index(vec![
            model("dev", "flux1_dev", ModelKind::Diffusion, "dev.gguf"),
            model("kontext", "flux1_kontext", ModelKind::Diffusion, "k.gguf"),
            component(&reg, "flux_ae"),
            component(&reg, "clip_l"),
            component(&reg, "t5xxl_fp8"),
            component(&reg, "t5xxl_fp16"),
        ]);
        assert!(orphaned_components(&reg, &idx, "kontext").is_empty());
        assert_eq!(ids(orphaned_components(&reg, &idx, "dev")), ["t5xxl_fp16"]);
    }

    #[test]
    fn two_models_of_one_family_share_everything() {
        let reg = registry();
        let idx = index(vec![
            model("a", "sdxl", ModelKind::Checkpoint, "a.safetensors"),
            model("b", "sdxl_pony", ModelKind::Checkpoint, "b.safetensors"),
            component(&reg, "sdxl_vae_fp16_fix"),
        ]);
        assert!(orphaned_components(&reg, &idx, "a").is_empty());
        let idx = index(vec![
            model("a", "sdxl", ModelKind::Checkpoint, "a.safetensors"),
            component(&reg, "sdxl_vae_fp16_fix"),
        ]);
        assert_eq!(
            ids(orphaned_components(&reg, &idx, "a")),
            ["sdxl_vae_fp16_fix"]
        );
    }

    #[test]
    fn loras_and_components_delete_alone() {
        let reg = registry();
        let idx = index(vec![
            model("l", "sdxl", ModelKind::Lora, "l.safetensors"),
            component(&reg, "sdxl_vae_fp16_fix"),
        ]);
        assert_eq!(delete_preview(&reg, &idx, "l").unwrap().files.len(), 1);
        assert!(delete_preview(&reg, &idx, "missing").is_none());
    }

    #[test]
    fn installed_other_encoder_option_counts_as_complete() {
        // Regression: bf16 Qwen3-4B installed before 16 GB moved to Q8 → no "Get".
        let reg = registry();
        let zit = model(
            "zit",
            "z_image_turbo",
            ModelKind::Diffusion,
            "z_image_turbo_bf16.safetensors",
        );
        let idx = index(vec![
            zit.clone(),
            component(&reg, "flux_ae"),
            component(&reg, "qwen3_4b"),
        ]);
        assert!(installed_model_view(&reg, &idx, &zit, &hw(16.0))
            .missing_components
            .is_empty());
        let idx = index(vec![zit.clone(), component(&reg, "flux_ae")]);
        let v = installed_model_view(&reg, &idx, &zit, &hw(16.0));
        assert!(
            v.missing_components[0].contains("Qwen3-4B-Q8_0.gguf"),
            "{:?}",
            v.missing_components
        );
    }

    #[test]
    fn installed_model_view_fields() {
        let reg = registry();
        let mut zit = model(
            "zit",
            "z_image_turbo",
            ModelKind::Diffusion,
            "z_image_turbo-Q8_0.gguf",
        );
        zit.last_used = Some(42);
        let idx = index(vec![zit.clone(), component(&reg, "flux_ae")]);
        let v = installed_model_view(&reg, &idx, &zit, &hw(8.0));
        assert_eq!(v.family_label.as_deref(), Some("Z-Image Turbo"));
        assert_eq!(v.style_badge.as_deref(), Some("Realistic"));
        assert_eq!(v.modes, ["txt2img", "img2img"]);
        assert!(!v.is_edit_model);
        assert_eq!(v.vram.unwrap().gb, 12.0, "registry figure for the Q8 file");
        assert_eq!(v.fit, Some(Fit::Tight));
        assert_eq!(v.missing_components.len(), 1);
        // 8 GB: the Q4_K_M GGUF text encoder (bf16 only from 20 GB).
        assert!(
            v.missing_components[0].contains("Qwen3-4B-Q4_K_M.gguf"),
            "{:?}",
            v.missing_components
        );
        assert_eq!(v.license_note.as_deref(), Some("Apache 2.0"));
        assert_eq!(v.last_used, Some(42));

        // Measured VRAM wins.
        let mut measured = zit.clone();
        measured.observed_vram_gb = Some(9.5);
        let v = installed_model_view(&reg, &idx, &measured, &hw(8.0));
        assert_eq!(v.vram.unwrap().gb, 9.5);
        assert!(!v.vram.unwrap().estimate);

        // Edit family + CivitAI ids.
        let q = with_civitai(
            model("q", "qwen_image_edit_2511", ModelKind::Diffusion, "q.gguf"),
            7,
        );
        let v = installed_model_view(&reg, &index(vec![q.clone()]), &q, &hw(16.0));
        assert!(v.is_edit_model);
        assert_eq!(v.civitai_version_id, Some(7));
        assert_eq!(v.base_model.as_deref(), Some("SDXL 1.0"));
        assert_eq!(v.style_badge, None, "no badge guessed for CivitAI files");

        // No usable GPU: sized against RAM. SD 1.5 runs (slowly), Z-Image does not.
        let v = installed_model_view(&reg, &idx, &zit, &hw_cpu(32.0));
        assert_eq!(v.fit, Some(Fit::TooBig));
        assert!(v.vram.unwrap().on_cpu);
        let mut sd = model(
            "sd",
            "sd15",
            ModelKind::Checkpoint,
            "dreamshaper_8.safetensors",
        );
        sd.size_bytes = 2_132_696_762;
        let v = installed_model_view(&reg, &index(vec![sd.clone()]), &sd, &hw_cpu(8.0));
        assert_eq!(v.fit, Some(Fit::Tight));
        assert_eq!(v.vram.unwrap().gb, 3.0);
        assert_eq!(
            v.style_badge, None,
            "SD 1.5 is only a fallback pick, not the Realistic model"
        );
        assert_eq!(
            installed_model_view(&reg, &index(vec![sd.clone()]), &sd, &hw(8.0)).fit,
            Some(Fit::Fits)
        );

        // Unknown family → estimate-free, empty.
        let mut u = model("u", "nope", ModelKind::Checkpoint, "u.safetensors");
        u.family = None;
        let v = installed_model_view(&reg, &index(vec![u.clone()]), &u, &hw(8.0));
        assert_eq!((v.vram, v.fit, v.family_label), (None, None, None));

        // Flux dev (no registry figure) → estimate from file size.
        let d = model("dev", "flux1_dev", ModelKind::Diffusion, "dev.gguf");
        let v = installed_model_view(&reg, &index(vec![d.clone()]), &d, &hw(12.0));
        assert!(v.vram.unwrap().estimate);
        assert!(v.vram.unwrap().gb > 6.5);
        let json = serde_json::to_value(&v).unwrap();
        assert!(json.get("missingComponents").is_some() && json.get("isEditModel").is_some());
    }

    #[test]
    fn lora_view() {
        let mut l = with_civitai(model("l", "sdxl_pony", ModelKind::Lora, "l.safetensors"), 5);
        l.civitai.as_mut().unwrap().trained_words = vec!["pnkstyle".into()];
        let v = installed_lora_view(&l);
        assert_eq!(v.trained_words, ["pnkstyle"]);
        assert_eq!(v.civitai_version_id, Some(5));
        assert_eq!(v.family_id.as_deref(), Some("sdxl_pony"));
    }

    #[test]
    fn rel_path_safety() {
        assert!(is_safe_rel_path("models/vae/ae.safetensors"));
        assert!(!is_safe_rel_path("../outside.safetensors"));
        assert!(!is_safe_rel_path("models/../../x"));
        assert!(!is_safe_rel_path("/etc/passwd"));
        assert!(!is_safe_rel_path("C:/Windows/x"));
        assert!(!is_safe_rel_path("models\\..\\..\\x"));
        assert!(!is_safe_rel_path(""));
    }
}
