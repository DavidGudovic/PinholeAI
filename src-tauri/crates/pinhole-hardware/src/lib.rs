//! GPU vendor + VRAM + RAM detection (SPEC §6.2). NVIDIA via `nvidia-smi`;
//! Windows DXGI adapter memory for AMD/Intel; Linux sysfs (`mem_info_vram_total`).
//! Manual override lives in Settings and is applied by the caller.
//!
//! OWNER: store agent.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    pub index: usize,
    pub vendor: Vendor,
    pub name: String,
    /// Dedicated VRAM in GB (0 if unknown / integrated).
    pub vram_gb: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HardwareInfo {
    pub gpus: Vec<GpuInfo>,
    pub ram_gb: f32,
    pub cpu_threads: usize,
    pub os: String,
}

impl HardwareInfo {
    /// The GPU with the most VRAM (preferring NVIDIA on ties).
    pub fn best_gpu(&self) -> Option<&GpuInfo> {
        todo!("store agent")
    }
}

/// Detect hardware. Blocking (spawns `nvidia-smi`): call via `spawn_blocking`.
/// Never fails — unknowns become empty / 0.
pub fn detect() -> HardwareInfo {
    todo!("store agent")
}

/// Engine backend for a vendor, per `config/engine.yaml → selection`:
/// nvidia → `cuda`, amd/intel → `vulkan`, none → `cpu`.
pub fn default_backend(gpu: Option<&GpuInfo>) -> &'static str {
    match gpu.map(|g| g.vendor) {
        Some(Vendor::Nvidia) => "cuda",
        Some(Vendor::Amd) | Some(Vendor::Intel) => "vulkan",
        _ => "cpu",
    }
}
