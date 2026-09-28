//! GPU vendor + VRAM + RAM detection (SPEC §6.2). NVIDIA via `nvidia-smi`;
//! Windows DXGI adapter memory for AMD/Intel; Linux sysfs (`mem_info_vram_total`).
//! Manual override lives in Settings and is applied by the caller.
//!
//! All parsing is done by pure functions (`parse_nvidia_smi`, `gpu_from_dxgi`,
//! `gpu_from_sysfs`, `merge_gpus`) so it can be tested with fixtures.
//!
//! OWNER: store agent.

use std::path::Path;

use serde::{Deserialize, Serialize};

mod nvidia;
#[cfg(windows)]
mod dxgi;

pub use nvidia::{parse_compute_apps, parse_gpu_memory, parse_nvidia_smi, query_vram_usage, GpuMemory, GpuProcess, OtherGpuUse, VramUsage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Other,
}

impl Vendor {
    /// From a PCI vendor id.
    pub fn from_pci_id(id: u32) -> Vendor {
        match id {
            PCI_NVIDIA => Vendor::Nvidia,
            PCI_AMD => Vendor::Amd,
            PCI_INTEL => Vendor::Intel,
            _ => Vendor::Other,
        }
    }

    /// Tie-break order for [`HardwareInfo::best_gpu`].
    fn rank(self) -> u8 {
        match self {
            Vendor::Nvidia => 3,
            Vendor::Amd => 2,
            Vendor::Intel => 1,
            Vendor::Other => 0,
        }
    }
}

pub const PCI_NVIDIA: u32 = 0x10DE;
pub const PCI_AMD: u32 = 0x1002;
pub const PCI_INTEL: u32 = 0x8086;
/// Microsoft Basic Render Driver / remote display adapters.
const PCI_MICROSOFT: u32 = 0x1414;

/// Below this much "dedicated" memory an AMD/Intel adapter is treated as
/// integrated (UMA carve-out / stolen memory), i.e. `vram_gb = 0`.
const INTEGRATED_MAX_GB: f32 = 1.0;

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
    /// The GPU with the most VRAM (preferring NVIDIA on ties, then AMD, Intel;
    /// then the lower index).
    pub fn best_gpu(&self) -> Option<&GpuInfo> {
        self.gpus.iter().max_by(|a, b| {
            a.vram_gb
                .total_cmp(&b.vram_gb)
                .then_with(|| a.vendor.rank().cmp(&b.vendor.rank()))
                .then_with(|| b.index.cmp(&a.index))
        })
    }

    /// GPU by its `index` (Settings `gpu:<index>`).
    pub fn gpu(&self, index: usize) -> Option<&GpuInfo> {
        self.gpus.iter().find(|g| g.index == index)
    }
}

/// Detect hardware. Blocking (spawns `nvidia-smi`): call via `spawn_blocking`.
/// Never fails — unknowns become empty / 0.
///
/// GPU `index` is the position in `gpus`: NVIDIA GPUs first in `nvidia-smi`
/// order (so it matches the CUDA/`nvidia-smi` index), then the others.
pub fn detect() -> HardwareInfo {
    let nvidia = nvidia::query();
    let gpus = merge_gpus(nvidia, platform_gpus());
    let (ram_gb, cpu_threads) = memory_and_threads();
    HardwareInfo { gpus, ram_gb, cpu_threads, os: os_label() }
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

/// Combine `nvidia-smi` results with the platform's adapter list (DXGI / sysfs).
/// `nvidia-smi` numbers win for NVIDIA cards; platform NVIDIA entries are used
/// only when `nvidia-smi` found nothing. Indices are reassigned 0..n.
pub fn merge_gpus(mut nvidia: Vec<GpuInfo>, platform: Vec<GpuInfo>) -> Vec<GpuInfo> {
    nvidia.sort_by_key(|g| g.index);
    let (platform_nvidia, others): (Vec<_>, Vec<_>) = platform.into_iter().partition(|g| g.vendor == Vendor::Nvidia);
    let mut gpus = if nvidia.is_empty() { platform_nvidia } else { nvidia };
    gpus.extend(others);
    for (i, g) in gpus.iter_mut().enumerate() {
        g.index = i;
    }
    gpus
}

/// Bytes → GiB rounded to 0.1 (8188 MiB → 8.0, 16303 MiB → 15.9).
pub(crate) fn bytes_to_gb(bytes: u64) -> f32 {
    round1(bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

pub(crate) fn round1(x: f64) -> f32 {
    ((x * 10.0).round() / 10.0) as f32
}

/// AMD/Intel adapters with a tiny carve-out are integrated → 0.
fn dedicated_or_zero(vendor: Vendor, gb: f32) -> f32 {
    if matches!(vendor, Vendor::Amd | Vendor::Intel) && gb < INTEGRATED_MAX_GB {
        0.0
    } else {
        gb
    }
}

// ---------------------------------------------------------------- Windows DXGI

/// One `DXGI_ADAPTER_DESC1`, reduced to what we use.
#[derive(Debug, Clone, PartialEq)]
pub struct DxgiAdapter {
    pub vendor_id: u32,
    pub description: String,
    pub dedicated_video_memory: u64,
    /// `DXGI_ADAPTER_FLAG_SOFTWARE` set.
    pub software: bool,
}

/// Software adapters and Microsoft's render/remote-display adapters are skipped.
pub fn gpu_from_dxgi(a: &DxgiAdapter) -> Option<GpuInfo> {
    if a.software || a.vendor_id == PCI_MICROSOFT {
        return None;
    }
    let vendor = Vendor::from_pci_id(a.vendor_id);
    let vram_gb = dedicated_or_zero(vendor, bytes_to_gb(a.dedicated_video_memory));
    if vendor == Vendor::Other && vram_gb <= 0.0 {
        return None;
    }
    let name = a.description.trim();
    let name = if name.is_empty() { fallback_name(vendor, vram_gb) } else { name.to_string() };
    Some(GpuInfo { index: 0, vendor, name, vram_gb })
}

// ---------------------------------------------------------------- Linux sysfs

/// Contents of the sysfs files for one `/sys/class/drm/cardN`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SysfsCard {
    /// `device/vendor`, e.g. `0x1002`.
    pub vendor: String,
    /// `device/uevent` (for `DRIVER=` and `PCI_ID=`).
    pub uevent: Option<String>,
    /// `device/product_name` (amdgpu, often empty).
    pub product_name: Option<String>,
    /// First VRAM size file found (`device/mem_info_vram_total` on amdgpu;
    /// `lmem_total_bytes` / `device/lmem_total` on Intel discrete), in bytes.
    pub vram_total: Option<String>,
}

/// VRAM size files tried per card, relative to `/sys/class/drm/cardN`.
const SYSFS_VRAM_FILES: &[&str] = &[
    "device/mem_info_vram_total",
    "lmem_total_bytes",
    "device/lmem_total_bytes",
    "device/lmem_total",
];

/// Parse one sysfs card. Only NVIDIA / AMD / Intel cards are kept (virtual
/// display adapters are not useful for generation).
pub fn gpu_from_sysfs(card: &SysfsCard) -> Option<GpuInfo> {
    let vendor_id = parse_hex_u32(&card.vendor)?;
    let vendor = Vendor::from_pci_id(vendor_id);
    if vendor == Vendor::Other {
        return None;
    }
    let vram_bytes = card.vram_total.as_deref().and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0);
    let vram_gb = dedicated_or_zero(vendor, bytes_to_gb(vram_bytes));
    let pci_id = card.uevent.as_deref().and_then(|u| uevent_value(u, "PCI_ID"));
    let product = card.product_name.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let base = match product {
        Some(p) => p.to_string(),
        None => fallback_name(vendor, vram_gb),
    };
    let name = match pci_id {
        Some(id) if product.is_none() => format!("{base} ({id})"),
        _ => base,
    };
    Some(GpuInfo { index: 0, vendor, name, vram_gb })
}

/// Read every `cardN` under a drm class dir (normally `/sys/class/drm`).
pub fn scan_sysfs(drm_dir: &Path) -> Vec<GpuInfo> {
    let Ok(entries) = std::fs::read_dir(drm_dir) else { return Vec::new() };
    let mut cards: Vec<(u32, std::path::PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let n = name.strip_prefix("card")?;
            if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            Some((n.parse().ok()?, e.path()))
        })
        .collect();
    cards.sort_by_key(|(n, _)| *n);
    let read = |p: &Path| std::fs::read_to_string(p).ok();
    cards
        .into_iter()
        .filter_map(|(_, dir)| {
            let card = SysfsCard {
                vendor: read(&dir.join("device/vendor"))?,
                uevent: read(&dir.join("device/uevent")),
                product_name: read(&dir.join("device/product_name")),
                vram_total: SYSFS_VRAM_FILES.iter().find_map(|f| read(&dir.join(f))),
            };
            gpu_from_sysfs(&card)
        })
        .collect()
}

fn parse_hex_u32(s: &str) -> Option<u32> {
    let s = s.trim();
    let s = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    u32::from_str_radix(s, 16).ok()
}

fn uevent_value<'a>(uevent: &'a str, key: &str) -> Option<&'a str> {
    uevent.lines().find_map(|l| l.trim().strip_prefix(key)?.strip_prefix('=')).map(str::trim).filter(|v| !v.is_empty())
}

fn fallback_name(vendor: Vendor, vram_gb: f32) -> String {
    match vendor {
        Vendor::Nvidia => "NVIDIA GPU".into(),
        Vendor::Amd => "AMD Radeon GPU".into(),
        Vendor::Intel if vram_gb > 0.0 => "Intel Arc GPU".into(),
        Vendor::Intel => "Intel integrated graphics".into(),
        Vendor::Other => "GPU".into(),
    }
}

// ---------------------------------------------------------------- platform glue

#[cfg(windows)]
fn platform_gpus() -> Vec<GpuInfo> {
    dxgi::adapters().iter().filter_map(gpu_from_dxgi).collect()
}

#[cfg(target_os = "linux")]
fn platform_gpus() -> Vec<GpuInfo> {
    scan_sysfs(Path::new("/sys/class/drm"))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_gpus() -> Vec<GpuInfo> {
    Vec::new()
}

fn memory_and_threads() -> (f32, usize) {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let ram_gb = bytes_to_gb(sys.total_memory());
    sys.refresh_cpu_list(sysinfo::CpuRefreshKind::nothing());
    let threads = match sys.cpus().len() {
        0 => std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        n => n,
    };
    (ram_gb, threads)
}

/// "Windows 11 Pro 24H2", "Linux 24.04 Ubuntu"…; falls back to `windows` / `linux`.
fn os_label() -> String {
    sysinfo::System::long_os_version()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::env::consts::OS.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(index: usize, vendor: Vendor, vram_gb: f32) -> GpuInfo {
        GpuInfo { index, vendor, name: format!("{vendor:?} {index}"), vram_gb }
    }

    fn info(gpus: Vec<GpuInfo>) -> HardwareInfo {
        HardwareInfo { gpus, ram_gb: 32.0, cpu_threads: 16, os: "linux".into() }
    }

    #[test]
    fn best_gpu_prefers_vram_then_nvidia_then_index() {
        assert_eq!(info(vec![]).best_gpu(), None);
        let hw = info(vec![gpu(0, Vendor::Intel, 0.0), gpu(1, Vendor::Nvidia, 8.0), gpu(2, Vendor::Amd, 16.0)]);
        assert_eq!(hw.best_gpu().unwrap().index, 2);
        let hw = info(vec![gpu(0, Vendor::Amd, 16.0), gpu(1, Vendor::Nvidia, 16.0)]);
        assert_eq!(hw.best_gpu().unwrap().index, 1);
        let hw = info(vec![gpu(0, Vendor::Nvidia, 12.0), gpu(1, Vendor::Nvidia, 12.0)]);
        assert_eq!(hw.best_gpu().unwrap().index, 0);
        let hw = info(vec![gpu(0, Vendor::Intel, 0.0)]);
        assert_eq!(hw.best_gpu().unwrap().vendor, Vendor::Intel);
        assert_eq!(hw.gpu(0).unwrap().vendor, Vendor::Intel);
        assert!(hw.gpu(3).is_none());
    }

    #[test]
    fn backend_per_vendor() {
        assert_eq!(default_backend(None), "cpu");
        assert_eq!(default_backend(Some(&gpu(0, Vendor::Nvidia, 8.0))), "cuda");
        assert_eq!(default_backend(Some(&gpu(0, Vendor::Amd, 8.0))), "vulkan");
        assert_eq!(default_backend(Some(&gpu(0, Vendor::Intel, 0.0))), "vulkan");
        assert_eq!(default_backend(Some(&gpu(0, Vendor::Other, 4.0))), "cpu");
    }

    #[test]
    fn merge_prefers_nvidia_smi_and_reindexes() {
        let smi = vec![
            GpuInfo { index: 1, vendor: Vendor::Nvidia, name: "RTX 3060".into(), vram_gb: 12.0 },
            GpuInfo { index: 0, vendor: Vendor::Nvidia, name: "RTX 5070 Ti".into(), vram_gb: 15.9 },
        ];
        let platform = vec![
            GpuInfo { index: 0, vendor: Vendor::Intel, name: "Intel UHD".into(), vram_gb: 0.0 },
            GpuInfo { index: 0, vendor: Vendor::Nvidia, name: "NVIDIA GeForce RTX 5070 Ti".into(), vram_gb: 15.6 },
        ];
        let merged = merge_gpus(smi, platform.clone());
        let names: Vec<_> = merged.iter().map(|g| (g.index, g.name.as_str(), g.vram_gb)).collect();
        assert_eq!(names, vec![(0, "RTX 5070 Ti", 15.9), (1, "RTX 3060", 12.0), (2, "Intel UHD", 0.0)]);

        // nvidia-smi missing: platform NVIDIA entries are used, NVIDIA first.
        let merged = merge_gpus(vec![], platform);
        assert_eq!(merged[0].name, "NVIDIA GeForce RTX 5070 Ti");
        assert_eq!(merged[1].index, 1);
        assert!(merge_gpus(vec![], vec![]).is_empty());
    }

    #[test]
    fn dxgi_adapters() {
        let nvidia = DxgiAdapter {
            vendor_id: 0x10DE,
            description: "NVIDIA GeForce RTX 5070 Ti".into(),
            dedicated_video_memory: 16_607_346_688,
            software: false,
        };
        let g = gpu_from_dxgi(&nvidia).unwrap();
        assert_eq!((g.vendor, g.vram_gb), (Vendor::Nvidia, 15.5));

        let amd = DxgiAdapter {
            vendor_id: 0x1002,
            description: "AMD Radeon RX 7800 XT".into(),
            dedicated_video_memory: 16 * 1024 * 1024 * 1024 - 300 * 1024 * 1024,
            software: false,
        };
        assert_eq!(gpu_from_dxgi(&amd).unwrap().vram_gb, 15.7);

        let igpu = DxgiAdapter {
            vendor_id: 0x8086,
            description: "Intel(R) UHD Graphics 770".into(),
            dedicated_video_memory: 128 * 1024 * 1024,
            software: false,
        };
        let g = gpu_from_dxgi(&igpu).unwrap();
        assert_eq!((g.vendor, g.vram_gb, g.name.as_str()), (Vendor::Intel, 0.0, "Intel(R) UHD Graphics 770"));

        let arc = DxgiAdapter {
            vendor_id: 0x8086,
            description: "Intel(R) Arc(TM) A770 Graphics".into(),
            dedicated_video_memory: 16_225_624_064,
            software: false,
        };
        assert_eq!(gpu_from_dxgi(&arc).unwrap().vram_gb, 15.1);

        let basic = DxgiAdapter {
            vendor_id: 0x1414,
            description: "Microsoft Basic Render Driver".into(),
            dedicated_video_memory: 0,
            software: true,
        };
        assert_eq!(gpu_from_dxgi(&basic), None);
        let remote = DxgiAdapter { software: false, ..basic };
        assert_eq!(gpu_from_dxgi(&remote), None);

        let other = DxgiAdapter { vendor_id: 0x5143, description: " ".into(), dedicated_video_memory: 0, software: false };
        assert_eq!(gpu_from_dxgi(&other), None);
        let other = DxgiAdapter { dedicated_video_memory: 4 << 30, ..other };
        let g = gpu_from_dxgi(&other).unwrap();
        assert_eq!((g.vendor, g.name.as_str(), g.vram_gb), (Vendor::Other, "GPU", 4.0));
    }

    #[test]
    fn sysfs_cards() {
        let amd = SysfsCard {
            vendor: "0x1002\n".into(),
            uevent: Some("DRIVER=amdgpu\nPCI_CLASS=30000\nPCI_ID=1002:744C\nPCI_SUBSYS_ID=1EAE:7901\n".into()),
            product_name: Some("\n".into()),
            vram_total: Some("25753026560\n".into()),
        };
        let g = gpu_from_sysfs(&amd).unwrap();
        assert_eq!((g.vendor, g.vram_gb, g.name.as_str()), (Vendor::Amd, 24.0, "AMD Radeon GPU (1002:744C)"));

        let named = SysfsCard { product_name: Some("AMD Radeon RX 7900 XTX\n".into()), ..amd.clone() };
        assert_eq!(gpu_from_sysfs(&named).unwrap().name, "AMD Radeon RX 7900 XTX");

        let apu = SysfsCard { vram_total: Some("536870912".into()), ..amd.clone() };
        assert_eq!(gpu_from_sysfs(&apu).unwrap().vram_gb, 0.0);

        let intel = SysfsCard {
            vendor: "0x8086".into(),
            uevent: Some("DRIVER=i915\nPCI_ID=8086:A780\n".into()),
            product_name: None,
            vram_total: None,
        };
        let g = gpu_from_sysfs(&intel).unwrap();
        assert_eq!((g.vendor, g.vram_gb, g.name.as_str()), (Vendor::Intel, 0.0, "Intel integrated graphics (8086:A780)"));

        let arc = SysfsCard { vram_total: Some("17179869184".into()), ..intel };
        let g = gpu_from_sysfs(&arc).unwrap();
        assert_eq!((g.vram_gb, g.name.as_str()), (16.0, "Intel Arc GPU (8086:A780)"));

        let nvidia = SysfsCard { vendor: "0x10de".into(), uevent: Some("DRIVER=nvidia\nPCI_ID=10DE:2C05".into()), ..Default::default() };
        let g = gpu_from_sysfs(&nvidia).unwrap();
        assert_eq!((g.vendor, g.vram_gb), (Vendor::Nvidia, 0.0));

        let virtio = SysfsCard { vendor: "0x1af4".into(), ..Default::default() };
        assert_eq!(gpu_from_sysfs(&virtio), None);
        let garbage = SysfsCard { vendor: "zz".into(), ..Default::default() };
        assert_eq!(gpu_from_sysfs(&garbage), None);
    }

    #[test]
    fn sysfs_scan_fixture_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let drm = tmp.path();
        let card = |name: &str, files: &[(&str, &str)]| {
            let dir = drm.join(name);
            for (rel, content) in files {
                let p = dir.join(rel);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, content).unwrap();
            }
        };
        card("card1", &[("device/vendor", "0x1002\n"), ("device/mem_info_vram_total", "17163091968\n"), ("device/uevent", "PCI_ID=1002:73BF\n")]);
        card("card0", &[("device/vendor", "0x8086\n")]);
        card("card0-DP-1", &[("device/vendor", "0x1002\n")]); // connector, ignored
        card("card2", &[("device/vendor", "0x1234\n")]); // QEMU VGA, ignored
        card("card3", &[("uevent", "x")]); // no vendor file, ignored
        card("renderD128", &[("device/vendor", "0x1002\n")]);
        let gpus = scan_sysfs(drm);
        let got: Vec<_> = gpus.iter().map(|g| (g.vendor, g.vram_gb)).collect();
        assert_eq!(got, vec![(Vendor::Intel, 0.0), (Vendor::Amd, 16.0)]);
        assert!(scan_sysfs(&drm.join("missing")).is_empty());
    }

    #[test]
    fn detect_never_panics_and_is_sane() {
        let hw = detect();
        assert!(hw.cpu_threads >= 1);
        assert!(hw.ram_gb > 0.0);
        assert!(!hw.os.is_empty());
        for (i, g) in hw.gpus.iter().enumerate() {
            assert_eq!(g.index, i);
            assert!(g.vram_gb >= 0.0);
        }
    }

    #[test]
    fn serde_shape_matches_types_ts() {
        let hw = info(vec![gpu(0, Vendor::Nvidia, 16.0)]);
        let v = serde_json::to_value(&hw).unwrap();
        assert_eq!(v["gpus"][0]["vendor"], "nvidia");
        assert_eq!(v["gpus"][0]["vramGb"], 16.0);
        assert!(v.get("ramGb").is_some() && v.get("cpuThreads").is_some() && v.get("os").is_some());
    }
}
