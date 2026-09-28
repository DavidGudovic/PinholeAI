//! VRAM needed + Fits / Tight / Too big (SPEC §6.2).
//!
//! Units: GB here means GiB (2³⁰ bytes), the unit GPU tools report
//! ("16 GB" cards show ~15.9 GiB total).

use serde::{Deserialize, Serialize};

use crate::{Family, Registry, VramGb};

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

/// Headroom kept free for the desktop / other apps (SPEC §6.2: `X ≤ VRAM − 1 GB`).
pub const HEADROOM_GB: f32 = 1.0;
/// On a CPU-only machine only models needing at most this much count as
/// runnable ("Tight"); everything else is "Too big".
pub const CPU_TINY_GB: f32 = 1.0;
/// Scratch memory the engine keeps free on the device (docs/backend.md: 512 MiB).
pub const ENGINE_RESERVE_GB: f32 = 0.5;
/// Share of the main weights that must stay GPU-resident for acceptable speed
/// when the rest is streamed from RAM by auto-fit / graph-cut execution.
pub const MIN_RESIDENT_SHARE: f32 = 0.5;

const EPS: f32 = 1e-3;
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Fits: `need.gb <= vram - 1`; Tight: `need.min_gb <= vram`; else TooBig.
/// `vram_gb == 0` (CPU only) → TooBig for everything except explicitly tiny models
/// (`need.gb <= CPU_TINY_GB`, which are Tight: they run, slowly).
pub fn fit(need: &VramNeed, vram_gb: f32) -> Fit {
    // NaN counts as "no GPU".
    if vram_gb.is_nan() || vram_gb <= 0.0 {
        return if need.gb <= CPU_TINY_GB + EPS {
            Fit::Tight
        } else {
            Fit::TooBig
        };
    }
    if need.gb <= vram_gb - HEADROOM_GB + EPS {
        Fit::Fits
    } else if need.min_gb <= vram_gb + EPS {
        Fit::Tight
    } else {
        Fit::TooBig
    }
}

fn gib(bytes: u64) -> f32 {
    (bytes as f64 / GIB) as f32
}

/// Round up to one decimal (so "~X GB" never under-states).
fn ceil_tenth(v: f32) -> f32 {
    ((v * 10.0 - 1e-4).ceil() / 10.0).max(0.0)
}

/// Estimate for unknown files: weights + GPU-resident components + `activation_gb`.
///
/// ```text
/// gb     = main + components_on_gpu + activation_gb + 0.5          (everything resident)
/// min_gb = MIN_RESIDENT_SHARE·main + activation_gb + 0.5           (auto-fit streams the rest)
/// ```
/// * `main` = diffusion / checkpoint file size, `components_on_gpu` = VAE and
///   text encoders that stay on the GPU (see [`crate::wiring::gpu_resident_components`]);
/// * `activation_gb` = the family's compute buffers at its default resolution;
/// * 0.5 GB = the engine's device scratch reserve;
/// * at the minimum, text encoders / VAE are parked in RAM and staged on
///   demand (sd.cpp auto-fit), and at least half of the main weights must stay
///   resident — streaming more than that is "will not run acceptably".
///
/// Both values are rounded up to 0.1 GB and `min_gb ≤ gb`.
pub fn estimate(
    registry: &Registry,
    family: &Family,
    main_file_bytes: u64,
    component_bytes_on_gpu: u64,
) -> VramNeed {
    let _ = registry;
    let main = gib(main_file_bytes);
    let comps = gib(component_bytes_on_gpu);
    let act = family.activation_gb.max(0.0);
    let gb = ceil_tenth(main + comps + act + ENGINE_RESERVE_GB);
    let min_gb = ceil_tenth(MIN_RESIDENT_SHARE * main + act + ENGINE_RESERVE_GB).min(gb);
    VramNeed {
        gb,
        min_gb,
        estimate: true,
    }
}

/// Measured / curated need from the registry (SPEC §6.2 item 1): the quant's
/// `vram_gb` (`alt_quants.<quant>`), else the family download's, else the
/// family's. `quant = None` = the main download.
pub fn registry_need(family: &Family, quant: Option<&str>) -> Option<VramNeed> {
    let from = |v: &VramGb| VramNeed {
        gb: v.recommended,
        min_gb: v.min.min(v.recommended),
        estimate: false,
    };
    let dl = family.download.as_ref();
    quant
        .and_then(|q| {
            dl.and_then(|d| d.alt_quants.get(q))
                .and_then(|a| a.vram_gb.as_ref())
        })
        .or_else(|| dl.and_then(|d| d.vram_gb.as_ref()))
        .or(family.vram_gb.as_ref())
        .map(from)
}
