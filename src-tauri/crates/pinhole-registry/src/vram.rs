//! VRAM needed + Fits / Tight / Too big (SPEC §6.2).

use serde::{Deserialize, Serialize};

use crate::{Family, Registry};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Fit {
    Fits,
    Tight,
    TooBig,
}

/// "Needs ~X GB VRAM".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VramNeed {
    /// Comfortable amount (registry `recommended`, or the estimate).
    pub gb: f32,
    /// Minimum with CPU offload / VAE tiling.
    pub min_gb: f32,
    /// `true` when computed from file sizes rather than measured.
    pub estimate: bool,
}

/// Fits: `need.gb <= vram - 1`; Tight: `need.min_gb <= vram`; else TooBig.
/// `vram_gb == 0` (CPU only) → TooBig for everything except explicitly tiny models.
pub fn fit(need: &VramNeed, vram_gb: f32) -> Fit {
    let _ = (need, vram_gb);
    todo!("registry agent")
}

/// Estimate for unknown files: weights + GPU-resident components + `activation_gb`.
pub fn estimate(registry: &Registry, family: &Family, main_file_bytes: u64, component_bytes_on_gpu: u64) -> VramNeed {
    let _ = (registry, family, main_file_bytes, component_bytes_on_gpu);
    todo!("registry agent")
}
