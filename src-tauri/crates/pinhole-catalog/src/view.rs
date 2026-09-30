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

/// One entry of the Tags multi-select.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagOption {
    pub key: String,
    pub label: String,
    /// Finds only what Safe mode hides (the NSFW tag): off while Safe mode is on.
    pub needs_safe_mode_off: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFilterOptions {
    pub looks: Vec<KeyLabel>,
    pub tags: Vec<TagOption>,
    pub sorts: Vec<KeyedLabel>,
    pub periods: Vec<KeyedLabel>,
    pub content: Vec<KeyLabel>,
    pub price: Vec<KeyLabel>,
    pub default_content: ContentMode,
    pub default_price: PriceMode,
    /// Opening sort / time (`api` values from `sorts` / `periods`).
    pub default_sort: String,
    pub default_period: String,
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
    /// The preview comes from a video; `preview_url` asks the CDN for a still frame.
    pub preview_is_video: bool,
    pub preview_nsfw: bool,
    pub model_nsfw: bool,
    pub thumbs_up_ratio: Option<f64>,
    pub download_count: u64,
    pub download_bytes: Option<u64>,
    pub vram: Option<VramNeed>,
    pub fit: Option<Fit>,
    pub early_access: bool,
    /// The creator asks for no adult content ("Safe images only" badge).
    pub sfw_only: bool,
    pub commercial_ok: bool,
    pub license_note: Option<String>,
    pub installed: bool,
    pub blocked_reason: Option<String>,
    /// Set when a smaller file of this version was picked so it fits the card
    /// (e.g. "Compact (FP8)"); the Install dialog explains it and offers the others.
    #[serde(default)]
    pub smaller_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowsePage {
    pub items: Vec<CatalogCard>,
    pub next_cursor: Option<String>,
    pub offline: bool,
    pub partial: bool,
    /// CivitAI models looked at for this page.
    #[serde(default)]
    pub checked: u32,
    /// …of which Safe mode hid (made for adults).
    #[serde(default)]
    pub hidden_by_content: u32,
    /// …of which Look, price, commercial use, kind or compatibility hid.
    #[serde(default)]
    pub hidden_by_filters: u32,
    /// …of which "Runs on my card" hid (too big for this machine).
    #[serde(default)]
    pub hidden_by_size: u32,
}

impl BrowsePage {
    /// Nothing can match: no request, no more pages.
    pub fn empty() -> Self {
        Self {
            offline: false,
            next_cursor: None,
            ..Self::offline(None)
        }
    }

    /// Offline mode: no request; the cursor is handed back unchanged.
    pub fn offline(cursor: Option<String>) -> Self {
        Self {
            items: Vec::new(),
            next_cursor: cursor,
            offline: true,
            partial: false,
            checked: 0,
            hidden_by_content: 0,
            hidden_by_filters: 0,
            hidden_by_size: 0,
        }
    }
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

/// One installable file of a CivitAI version (Install dialog size choice).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanFileOption {
    /// CivitAI file id: pass back to `plan_civitai_install` / `install_civitai`.
    pub file_id: u64,
    pub name: String,
    pub size_bytes: u64,
    /// "Full quality" | "Compact (FP8)" | "Compact (Q4)" | "Compact (8-bit)" …
    pub label: String,
    /// "Pictures can look grainy" (4-bit or smaller) and/or "Slower: part of it
    /// runs from system memory".
    #[serde(default)]
    pub note: Option<String>,
    pub vram: Option<VramNeed>,
    pub fit: Option<Fit>,
    pub selected: bool,
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
    /// Every installable file of the version (more than one = a size choice).
    #[serde(default)]
    pub file_options: Vec<PlanFileOption>,
    /// Label of the main file when a smaller one was picked so it fits the card.
    #[serde(default)]
    pub smaller_file: Option<String>,
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
    /// Instruction edits can take a second image (registry `multi_ref`).
    #[serde(default)]
    pub multi_ref: bool,
    pub size_bytes: u64,
    pub vram: Option<VramNeed>,
    pub fit: Option<Fit>,
    pub last_used: Option<i64>,
    pub missing_components: Vec<String>,
    pub license_note: Option<String>,
    pub civitai_model_id: Option<u64>,
    pub civitai_version_id: Option<u64>,
    pub base_model: Option<String>,
    /// `Q4`, `Q3`… when the file's weights are 4-bit or smaller (from its header).
    #[serde(default)]
    pub low_bit: Option<String>,
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
    pub civitai_model_id: Option<u64>,
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
    /// Plain-words note when the pick is a smaller version chosen so it fits
    /// (or replaces an installed version that is a tight fit).
    pub note: Option<String>,
    /// True when an installed version of this family is a tight fit and this smaller one fits.
    #[serde(default)]
    pub replaces_installed: bool,
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
            files: vec![DeleteFile {
                rel_path: "models/vae/ae.safetensors".into(),
                size_bytes: 1,
                reason: DeleteReason::OrphanComponent,
            }],
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["modelId"], "m");
        assert_eq!(v["files"][0]["relPath"], "models/vae/ae.safetensors");
        assert_eq!(v["files"][0]["reason"], "orphanComponent");
        let r = AddFileResult {
            model: None,
            lora: None,
            needs_choice: Some(NeedsChoice {
                token: "t".into(),
                file_name: "f".into(),
                candidates: vec![],
            }),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["needsChoice"]["fileName"], "f");
    }
}
