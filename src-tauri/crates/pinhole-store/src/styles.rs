//! Styles (SPEC §7): built-ins in `config/styles/` (read-only) + user styles in
//! `Data/styles/<slug>.yaml`. The ONLY user text Pinhole stores, and only when
//! the user explicitly clicks "Save as style".

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{DataDir, StoreError};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Style {
    /// Slug; `builtin:<slug>` for shipped styles.
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub positive: String,
    #[serde(default)]
    pub negative: Option<String>,
    /// Families it's written for; empty = all.
    #[serde(default)]
    pub families: Vec<String>,
    /// Thumbnail file name inside `Data/styles/` (PNG), user-picked only.
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub builtin: bool,
}

/// Built-ins (sorted by name) followed by user styles (sorted by name).
pub fn list(builtin_dir: &Path, dir: &DataDir) -> Result<Vec<Style>, StoreError> {
    let _ = (builtin_dir, dir);
    todo!("store agent")
}

/// Create or update a user style. Empty `id` → new slug from name (deduplicated).
pub fn save(dir: &DataDir, style: Style) -> Result<Style, StoreError> {
    let _ = (dir, style);
    todo!("store agent")
}

pub fn delete(dir: &DataDir, id: &str) -> Result<(), StoreError> {
    let _ = (dir, id);
    todo!("store agent")
}

/// Look up any style (built-in or user) by id.
pub fn get(builtin_dir: &Path, dir: &DataDir, id: &str) -> Result<Style, StoreError> {
    let _ = (builtin_dir, dir, id);
    todo!("store agent")
}
