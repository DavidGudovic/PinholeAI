//! IPC view models. Field-for-field mirrors of `src/lib/types.ts`
//! (serde camelCase). No prompt text anywhere in here.

use pinhole_registry::vram::{Fit, VramNeed};
use serde::{Deserialize, Serialize};

use crate::filters::{ContentMode, PriceMode};

// ------------------------------------------------------------------ catalog

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyLabel {
    pub key: String,
    pub label: String,
}

/// `{ label, api }` (sorts, periods).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyedLabel {
    pub label: String,
    pub api: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFilterOptions {
    pub looks: Vec<KeyLabel>,
    pub sorts: Vec<KeyedLabel>,
    pub periods: Vec<KeyedLabel>,
    pub content: Vec<KeyLabel>,
    pub price: Vec<KeyLabel>,
    pub default_content: ContentMode,
    pub default_price: PriceMode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogCard {
    pub model_id: u64,
    pub version_id: u64,
    pub name: String,
    pub version_name: String,
    /// CivitAI type: `Checkpoint` | `LORA`.
    #[serde(rename = "type")]
    pub kind: String,
    pub base_model: String,
    pub family_id: Option<String>,
    pub style_badge: Option<String>,
    pub creator: Option<String>,
    /// Fetch through `fetch_preview`; never an `<img src>`.
    pub preview_url: Option<String>,
    pub preview_is_video: bool,
    pub preview_nsfw: bool,
    pub model_nsfw: bool,
    pub thumbs_up_ratio: Option<f64>,
    pub download_count: u64,
    pub download_bytes: Option<u64>,
    pub vram: Option<VramNeed>,
    pub fit: Option<Fit>,
    pub early_access: bool,
    pub commercial_ok: bool,
    pub license_note: Option<String>,
    pub installed: bool,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowsePage {
    pub items: Vec<CatalogCard>,
    pub next_cursor: Option<String>,
    pub offline: bool,
    pub partial: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyChoice {
    pub family_id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanFile {
    pub name: String,
    pub size_bytes: u64,
    pub format: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanComponent {
    pub component_id: String,
    pub label: String,
    pub size_bytes: u64,
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallPlan {
    pub version_id: u64,
    pub model_name: String,
    pub version_name: String,
    pub main_file: PlanFile,
    pub family: Option<FamilyChoice>,
    pub family_candidates: Vec<FamilyChoice>,
    pub components: Vec<PlanComponent>,
    pub total_download_bytes: u64,
    pub free_disk_bytes: u64,
    pub enough_disk: bool,
    pub vram: Option<VramNeed>,
    pub fit: Option<Fit>,
    pub license_note: Option<String>,
    pub is_lora: bool,
    pub trained_words: Vec<String>,
    pub blocked_reason: Option<String>,
    pub needs_api_key: bool,
}

// ------------------------------------------------------------------ installed models

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModel {
    pub id: String,
    pub friendly_name: String,
    pub family_id: Option<String>,
    pub family_label: Option<String>,
    pub style_badge: Option<String>,
    pub modes: Vec<String>,
    pub is_edit_model: bool,
    pub size_bytes: u64,
    pub vram: Option<VramNeed>,
    pub fit: Option<Fit>,
    pub last_used: Option<i64>,
    pub missing_components: Vec<String>,
    pub license_note: Option<String>,
    pub civitai_model_id: Option<u64>,
    pub civitai_version_id: Option<u64>,
    pub base_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledLora {
    pub id: String,
    pub friendly_name: String,
    pub family_id: Option<String>,
    pub base_model: Option<String>,
    pub trained_words: Vec<String>,
    pub size_bytes: u64,
    pub civitai_version_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeedsChoice {
    pub token: String,
    pub file_name: String,
    pub candidates: Vec<FamilyChoice>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddFileResult {
    pub model: Option<InstalledModel>,
    pub lora: Option<InstalledLora>,
    pub needs_choice: Option<NeedsChoice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeleteReason {
    Model,
    OrphanComponent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFile {
    pub rel_path: String,
    pub size_bytes: u64,
    pub reason: DeleteReason,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePreview {
    pub model_id: String,
    pub files: Vec<DeleteFile>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedPick {
    /// `realistic` | `anime` | `edit` | `describe`
    pub role: String,
    pub role_label: String,
    pub title: Option<String>,
    pub family_id: Option<String>,
    pub good_at: Option<String>,
    /// Only what is missing (shared components counted once).
    pub download_bytes: u64,
    pub vram: Option<VramNeed>,
    pub fit: Option<Fit>,
    pub installed: bool,
    /// `bf16` | `q8_0` | `q4_k` for registry models.
    pub quant: Option<String>,
    pub license_note: Option<String>,
    pub unavailable_reason: Option<String>,
}

// ------------------------------------------------------------------ paste from CivitAI

/// A resource from pasted generation data (ids/hashes only — never prompt text).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PastedResource {
    /// `checkpoint` | `lora` | `embed` | `vae` | …
    #[serde(rename = "type")]
    pub kind: String,
    pub model_version_id: Option<u64>,
    pub model_name: Option<String>,
    pub model_version_name: Option<String>,
    /// AutoV2 (10 hex) or full SHA-256.
    pub hash: Option<String>,
    pub weight: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedResource {
    pub resource: PastedResource,
    pub installed_id: Option<String>,
    pub installable_version_id: Option<u64>,
    pub display_name: String,
    pub family_id: Option<String>,
    pub download_bytes: Option<u64>,
    pub fit: Option<Fit>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedResources {
    pub checkpoint: Option<ResolvedResource>,
    pub loras: Vec<ResolvedResource>,
    pub ignored: Vec<ResolvedResource>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_resource_from_ui_json() {
        let r: PastedResource =
            serde_json::from_str(r#"{"type":"lora","modelVersionId":123,"modelName":"x","modelVersionName":null,"hash":"ABCDEF0123","weight":0.8}"#)
                .unwrap();
        assert_eq!(r.kind, "lora");
        assert_eq!(r.model_version_id, Some(123));
        assert_eq!(r.weight, Some(0.8));
        let r: PastedResource = serde_json::from_str(r#"{"type":"checkpoint"}"#).unwrap();
        assert_eq!(r.hash, None);
    }

    #[test]
    fn serialises_camel_case() {
        let p = DeletePreview {
            model_id: "m".into(),
            files: vec![DeleteFile { rel_path: "models/vae/ae.safetensors".into(), size_bytes: 1, reason: DeleteReason::OrphanComponent }],
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["modelId"], "m");
        assert_eq!(v["files"][0]["relPath"], "models/vae/ae.safetensors");
        assert_eq!(v["files"][0]["reason"], "orphanComponent");
        let r = AddFileResult { model: None, lora: None, needs_choice: Some(NeedsChoice { token: "t".into(), file_name: "f".into(), candidates: vec![] }) };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["needsChoice"]["fileName"], "f");
    }
}
