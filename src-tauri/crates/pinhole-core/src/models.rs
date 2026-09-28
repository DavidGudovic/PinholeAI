//! Installed models, add-a-file, delete, recommended picks, install flows,
//! CivitAI resource resolution (paste). OWNER: catalog agent.

use pinhole_net::download::DownloadedFile;
use pinhole_store::datadir::ModelKind;
use pinhole_store::installed::{CivitaiRef, InstalledFile};

use crate::{AppCore, CoreResult};

#[derive(Default)]
pub struct ModelsState {}

/// What a finished download is, for registering it in `installed.json`.
#[derive(Debug, Clone)]
pub struct Registration {
    pub kind: ModelKind,
    pub friendly_name: String,
    pub family: Option<String>,
    pub component_id: Option<String>,
    pub civitai: Option<CivitaiRef>,
    pub dtype: Option<String>,
}

/// Add a downloaded (already verified, already in `Data/models/...`) file to
/// `installed.json`, save it, and emit `ModelsChanged`. Shared by every install
/// flow (catalog, recommended, captioner, upscaler).
pub fn register_download(core: &AppCore, file: &DownloadedFile, reg: Registration) -> CoreResult<InstalledFile> {
    let _ = (core, file, reg);
    todo!("catalog agent")
}
