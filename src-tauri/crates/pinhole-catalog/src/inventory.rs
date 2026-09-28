//! Installed models: view models for the Installed list / model picker,
//! missing components, and what a delete removes (model + orphaned components).

use std::collections::BTreeSet;

use pinhole_registry::vram;
use pinhole_registry::wiring::HwContext;
use pinhole_registry::Registry;
use pinhole_store::datadir::ModelKind;
use pinhole_store::{InstalledFile, InstalledIndex};

use crate::families;
use crate::view::{DeleteFile, DeletePreview, DeleteReason, InstalledLora, InstalledModel};

/// Friendly badge for installed models: the recommended role the family heads
/// (`realistic` → "Realistic", `anime` → "Anime"), only for files installed from
/// the registry (CivitAI finetunes of a family can be any style).
pub fn registry_style_badge(registry: &Registry, file: &InstalledFile) -> Option<String> {
    if file.civitai.is_some() {
        return None;
    }
    let family = file.family.as_deref()?;
    for (role, label) in [("realistic", "Realistic"), ("anime", "Anime")] {
        let heads = registry
            .recommended()
            .get(role)
            .is_some_and(|c| c.iter().any(|c| c.source.as_deref() == Some("registry") && c.family.as_deref() == Some(family)));
        if heads {
            return Some(label.to_string());
        }
    }
    None
}

pub fn installed_model_view(registry: &Registry, index: &InstalledIndex, file: &InstalledFile, hw: &HwContext) -> InstalledModel {
    let family = file.family.as_deref().and_then(|id| registry.family(id));
    let (vram, fit, missing, modes, is_edit, family_label, license) = match family {
        Some(f) => {
            let need = families::installed_need(registry, f, file, hw);
            let missing: Vec<String> =
                families::missing_components(registry, f, hw, index, false).into_iter().map(|(_, c)| families::component_label(c)).collect();
            let is_edit = f.role.as_deref() == Some("edit") || f.modes.iter().any(|m| m == "edit");
            (Some(need), Some(vram::fit(&need, hw.vram_gb)), missing, f.modes.clone(), is_edit, Some(f.label.clone()), f.license_note.clone())
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
        trained_words: file.civitai.as_ref().map(|c| c.trained_words.clone()).unwrap_or_default(),
        size_bytes: file.size_bytes,
        civitai_version_id: file.civitai.as_ref().map(|c| c.version_id),
    }
}

/// Installed component files that only `model_id` needs: components its family
/// can use that no *other* installed model's family can use.
pub fn orphaned_components<'a>(registry: &Registry, index: &'a InstalledIndex, model_id: &str) -> Vec<&'a InstalledFile> {
    let Some(target) = index.get(model_id) else { return Vec::new() };
    if !matches!(target.kind, ModelKind::Checkpoint | ModelKind::Diffusion) {
        return Vec::new();
    }
    let Some(family) = target.family.as_deref().and_then(|f| registry.family(f)) else { return Vec::new() };
    let ours = families::family_component_ids(family);
    let still_used: BTreeSet<String> = index
        .models()
        .filter(|m| m.id != target.id)
        .filter_map(|m| m.family.as_deref().and_then(|f| registry.family(f)))
        .flat_map(families::family_component_ids)
        .collect();
    let mut out: Vec<&InstalledFile> = Vec::new();
    for id in ours.difference(&still_used) {
        let Some(comp) = registry.component(id) else { continue };
        for f in index.files.iter().filter(|f| {
            f.component_id.as_deref() == Some(id.as_str())
                || (f.component_id.is_none() && families::normalize_sha(&comp.sha256).is_some_and(|h| f.sha256.eq_ignore_ascii_case(&h)))
        }) {
            if !out.iter().any(|o| o.id == f.id) {
                out.push(f);
            }
        }
    }
    out
}

/// What "Delete" removes for `model_id` (any installed file id).
pub fn delete_preview(registry: &Registry, index: &InstalledIndex, model_id: &str) -> Option<DeletePreview> {
    let target = index.get(model_id)?;
    let mut files =
        vec![DeleteFile { rel_path: target.rel_path.clone(), size_bytes: target.size_bytes, reason: DeleteReason::Model }];
    files.extend(orphaned_components(registry, index, model_id).into_iter().map(|f| DeleteFile {
        rel_path: f.rel_path.clone(),
        size_bytes: f.size_bytes,
        reason: DeleteReason::OrphanComponent,
    }));
    Some(DeletePreview { model_id: model_id.to_string(), files })
}

/// `true` if `rel_path` stays inside the Data folder (relative, no `..`).
pub fn is_safe_rel_path(rel_path: &str) -> bool {
    let p = std::path::Path::new(rel_path);
    !rel_path.is_empty()
        && !rel_path.starts_with('/')
        && !rel_path.starts_with('\\')
        && !p.is_absolute()
        && !rel_path.contains(':')
        && p.components().all(|c| matches!(c, std::path::Component::Normal(_)))
        && !rel_path.split(['/', '\\']).any(|s| s == "..")
}

#[cfg(test)]
mod tests {
    use super::*;

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
