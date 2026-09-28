//! App info, settings, hardware. OWNER: store agent.
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pinhole_hardware::{default_backend, GpuInfo, HardwareInfo};
use pinhole_registry::wiring::HwContext;
use pinhole_store::Settings;
use serde::{Deserialize, Serialize};

use crate::{AppCore, CoreError, CoreEvent, CoreResult};

/// Mirrors `AppInfo` in `src/lib/types.ts`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub data_dir: String,
    pub portable: bool,
    /// `windows` | `linux` | `macos`
    pub os: String,
}

/// Mirrors `HardwareView` in `src/lib/types.ts`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HardwareView {
    /// `None` while detection is still running.
    pub detected: Option<HardwareInfo>,
    /// Effective VRAM after Settings overrides (0 = CPU only).
    pub vram_gb: f32,
    /// Selected GPU (after override) or `None` for CPU.
    pub gpu: Option<GpuInfo>,
    /// `cuda` | `vulkan` | `cpu`
    pub backend: String,
    /// Hardware profile name from the registry: `low` | `mid` | `high` | `ultra`.
    pub tier: String,
}

/// Detected hardware with the Settings overrides applied.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveHardware {
    pub gpu: Option<GpuInfo>,
    pub vram_gb: f32,
    /// `cuda` | `vulkan` | `cpu`
    pub backend: String,
}

pub fn app_info(core: &AppCore) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: core.data.root.display().to_string(),
        portable: core.data.portable,
        os: std::env::consts::OS.to_string(),
    }
}

pub fn get_settings(core: &AppCore) -> Settings {
    core.settings.read().clone()
}

/// Save `settings.yaml` (values normalised) and apply side effects: the
/// Offline flag takes effect immediately for every network call; GPU / VRAM /
/// backend overrides are read live by [`hw_context`]. Returns what was stored.
pub fn set_settings(core: &AppCore, settings: Settings) -> CoreResult<Settings> {
    let settings = settings.normalized();
    // Hold the write lock across save + apply so concurrent calls can't leave
    // memory and settings.yaml disagreeing.
    let mut current = core.settings.write();
    pinhole_store::settings::save(&core.data, &settings).map_err(crate::library::store_err)?;
    core.offline.set(settings.offline);
    *current = settings.clone();
    Ok(settings)
}

/// Pure: apply Settings overrides to detected hardware.
///
/// * GPU: `cpu` (or engine backend `cpu`) → none; `gpu:<index>` → that GPU if
///   it exists; otherwise (and for `auto`) the best GPU.
/// * VRAM: CPU-only → 0; else the manual override, else the GPU's VRAM, else 0.
/// * Backend: the explicit engine-backend setting, else by GPU vendor
///   ([`default_backend`]: NVIDIA → cuda, AMD/Intel → vulkan, none → cpu).
///
/// Before detection finishes (`detected == None`) the automatic parts are
/// "no GPU": vram 0 (unless overridden), backend `cpu` (unless overridden).
pub fn effective_hardware(settings: &Settings, detected: Option<&HardwareInfo>) -> EffectiveHardware {
    let explicit_backend = match settings.engine_backend.as_str() {
        b @ ("cuda" | "vulkan" | "cpu") => Some(b),
        _ => None,
    };
    let cpu_only = settings.force_cpu() || explicit_backend == Some("cpu");
    if cpu_only {
        return EffectiveHardware { gpu: None, vram_gb: 0.0, backend: "cpu".into() };
    }
    let gpu = detected
        .and_then(|hw| settings.gpu_index().and_then(|i| hw.gpu(i)).or_else(|| hw.best_gpu()))
        .cloned();
    let vram_gb = settings
        .vram_override_gb
        .filter(|v| v.is_finite() && *v > 0.0)
        .or_else(|| gpu.as_ref().map(|g| g.vram_gb))
        .unwrap_or(0.0);
    let backend = explicit_backend.unwrap_or_else(|| default_backend(gpu.as_ref())).to_string();
    EffectiveHardware { gpu, vram_gb, backend }
}

/// Current effective hardware (Settings + detection so far).
pub fn effective(core: &AppCore) -> EffectiveHardware {
    let settings = core.settings.read().clone();
    let detected = core.hardware.read().clone();
    effective_hardware(&settings, detected.as_ref())
}

/// What the Settings sheet / first-run screen shows (`get_hardware`).
pub fn hardware_view(core: &AppCore) -> HardwareView {
    let settings = core.settings.read().clone();
    let detected = core.hardware.read().clone();
    let eff = effective_hardware(&settings, detected.as_ref());
    let tier = core.registry().hardware_profile(eff.vram_gb).name.clone();
    HardwareView { detected, vram_gb: eff.vram_gb, gpu: eff.gpu, backend: eff.backend, tier }
}

/// Detect hardware in the background, store it, emit `HardwareReady`.
/// Uses the current Tokio runtime when there is one, else a plain thread.
pub fn start_hardware_detection(core: &Arc<AppCore>) {
    let core = core.clone();
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn(async move {
                let info = tokio::task::spawn_blocking(pinhole_hardware::detect).await.ok();
                finish_detection(&core, info);
            });
        }
        Err(_) => {
            std::thread::spawn(move || {
                let info = std::panic::catch_unwind(pinhole_hardware::detect).ok();
                finish_detection(&core, info);
            });
        }
    }
}

fn finish_detection(core: &AppCore, info: Option<HardwareInfo>) {
    // `detect` never fails by contract; if it panicked anyway, report "no GPU"
    // so the UI isn't left waiting (the user can still set a VRAM override).
    let info = info.unwrap_or_else(|| HardwareInfo {
        gpus: Vec::new(),
        ram_gb: 0.0,
        cpu_threads: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        os: std::env::consts::OS.to_string(),
    });
    *core.hardware.write() = Some(info);
    core.emit(CoreEvent::HardwareReady);
}

/// Wait (up to `timeout`) for background detection to finish. Returns whether
/// hardware info is available. For flows that must not guess (engine install).
pub async fn wait_for_hardware(core: &AppCore, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if core.hardware.read().is_some() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Effective hardware for wiring/VRAM decisions: Settings overrides applied
/// (GPU pick / force CPU / VRAM override / engine backend). Used by the engine
/// and catalog areas. Before detection finishes: vram 0, backend "cpu".
pub fn hw_context(core: &AppCore) -> HwContext {
    let eff = effective(core);
    HwContext { vram_gb: eff.vram_gb, backend: eff.backend }
}

/// The Data folder ("Open Data folder" in Settings).
pub fn data_folder(core: &AppCore) -> CoreResult<PathBuf> {
    ensure_dir(core.data.root.clone())
}

/// `Data/outputs/` (created if missing so the file manager can open it).
pub fn outputs_folder(core: &AppCore) -> CoreResult<PathBuf> {
    ensure_dir(core.data.outputs())
}

fn ensure_dir(dir: PathBuf) -> CoreResult<PathBuf> {
    std::fs::create_dir_all(&dir)
        .map_err(|e| CoreError::new("io", format!("Couldn't create the folder {}.", dir.display())).with_details(e.to_string()))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pinhole_hardware::Vendor;

    fn gpu(index: usize, vendor: Vendor, vram_gb: f32) -> GpuInfo {
        GpuInfo { index, vendor, name: format!("gpu{index}"), vram_gb }
    }

    fn hw() -> HardwareInfo {
        HardwareInfo {
            gpus: vec![gpu(0, Vendor::Nvidia, 15.9), gpu(1, Vendor::Intel, 0.0), gpu(2, Vendor::Amd, 8.0)],
            ram_gb: 32.0,
            cpu_threads: 16,
            os: "linux".into(),
        }
    }

    fn settings(gpu: &str, vram: Option<f32>, backend: &str) -> Settings {
        Settings { gpu: gpu.into(), vram_override_gb: vram, engine_backend: backend.into(), ..Settings::default() }
    }

    #[test]
    fn auto_picks_best_gpu() {
        let e = effective_hardware(&settings("auto", None, "auto"), Some(&hw()));
        assert_eq!(e.gpu.as_ref().unwrap().index, 0);
        assert_eq!(e.vram_gb, 15.9);
        assert_eq!(e.backend, "cuda");
    }

    #[test]
    fn picked_gpu_and_overrides() {
        let e = effective_hardware(&settings("gpu:2", None, "auto"), Some(&hw()));
        assert_eq!((e.gpu.as_ref().unwrap().index, e.vram_gb, e.backend.as_str()), (2, 8.0, "vulkan"));
        let e = effective_hardware(&settings("gpu:1", Some(6.0), "auto"), Some(&hw()));
        assert_eq!((e.gpu.as_ref().unwrap().vendor, e.vram_gb, e.backend.as_str()), (Vendor::Intel, 6.0, "vulkan"));
        // Unknown index → best GPU.
        let e = effective_hardware(&settings("gpu:9", None, "auto"), Some(&hw()));
        assert_eq!(e.gpu.as_ref().unwrap().index, 0);
        // Explicit backend wins.
        let e = effective_hardware(&settings("auto", None, "vulkan"), Some(&hw()));
        assert_eq!((e.vram_gb, e.backend.as_str()), (15.9, "vulkan"));
    }

    #[test]
    fn cpu_means_no_gpu_and_zero_vram() {
        for s in [settings("cpu", Some(12.0), "auto"), settings("auto", Some(12.0), "cpu"), settings("cpu", None, "cuda")] {
            let e = effective_hardware(&s, Some(&hw()));
            assert_eq!(e, EffectiveHardware { gpu: None, vram_gb: 0.0, backend: "cpu".into() });
        }
    }

    #[test]
    fn before_detection() {
        let e = effective_hardware(&settings("auto", None, "auto"), None);
        assert_eq!(e, EffectiveHardware { gpu: None, vram_gb: 0.0, backend: "cpu".into() });
        let e = effective_hardware(&settings("auto", Some(16.0), "cuda"), None);
        assert_eq!((e.vram_gb, e.backend.as_str()), (16.0, "cuda"));
    }

    #[test]
    fn no_gpus_detected() {
        let none = HardwareInfo { gpus: vec![], ram_gb: 8.0, cpu_threads: 4, os: "linux".into() };
        let e = effective_hardware(&settings("auto", None, "auto"), Some(&none));
        assert_eq!(e, EffectiveHardware { gpu: None, vram_gb: 0.0, backend: "cpu".into() });
    }

    #[test]
    fn view_json_shape_matches_types_ts() {
        let v = HardwareView { detected: Some(hw()), vram_gb: 15.9, gpu: Some(gpu(0, Vendor::Nvidia, 15.9)), backend: "cuda".into(), tier: "high".into() };
        let j = serde_json::to_value(&v).unwrap();
        for k in ["detected", "vramGb", "gpu", "backend", "tier"] {
            assert!(j.get(k).is_some(), "{k}");
        }
        for k in ["gpus", "ramGb", "cpuThreads", "os"] {
            assert!(j["detected"].get(k).is_some(), "{k}");
        }
        let empty = HardwareView { detected: None, vram_gb: 0.0, gpu: None, backend: "cpu".into(), tier: "low".into() };
        let j = serde_json::to_value(&empty).unwrap();
        assert!(j["detected"].is_null() && j["gpu"].is_null());
        let info = AppInfo { version: "0.1.0".into(), data_dir: "/d".into(), portable: false, os: "linux".into() };
        let j = serde_json::to_value(&info).unwrap();
        for k in ["version", "dataDir", "portable", "os"] {
            assert!(j.get(k).is_some(), "{k}");
        }
    }
}
