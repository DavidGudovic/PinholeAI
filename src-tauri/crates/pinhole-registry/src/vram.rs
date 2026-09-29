//! VRAM needed + Fits / Tight / Too big (SPEC §6.2).
//!
//! Units: GB here means GiB (2³⁰ bytes), the unit GPU tools report
//! ("16 GB" cards show ~15.9 GiB total).

use serde::{Deserialize, Serialize};

use crate::wiring::HwContext;
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
    /// No usable GPU: `gb` / `min_gb` are system RAM the model needs on the
    /// processor ([`cpu_need`]), and the badge is judged against RAM ([`fit_cpu`]).
    #[serde(default)]
    pub on_cpu: bool,
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
/// Without a usable GPU: system RAM kept free for the OS and other apps.
pub const CPU_SPARE_RAM_GB: f32 = 4.0;
/// Without a usable GPU: models whose weights (main file + every component)
/// are at most this big still run on the processor in acceptable time; bigger
/// ones only when the family is `cpu_friendly` (SD 1.5).
pub const CPU_MAX_WEIGHTS_GB: f32 = 4.0;

const EPS: f32 = 1e-3;
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Fits: `need.gb <= vram - 1`; Tight: `need.min_gb <= vram`; else TooBig.
/// `vram_gb == 0` (CPU only) → TooBig for everything except explicitly tiny models
/// (`need.gb <= CPU_TINY_GB`, which are Tight: they run, slowly). Callers that
/// know the family and file sizes use [`need_and_fit`], which sizes CPU-only
/// machines against system RAM instead ([`fit_cpu`]).
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

/// Compute memory for reading the prompt (text encoder activations and the
/// engine's graph for them), on top of the encoder weights.
pub const TEXT_ENCODER_COMPUTE_GB: f32 = 1.0;

/// Estimate for unknown files, from the two stages of a generation.
///
/// ```text
/// diffusion stage = main + other_components + activation_gb + 0.5
/// prompt stage    = text_encoders + TEXT_ENCODER_COMPUTE_GB + 0.5
/// gb              = max(diffusion stage, prompt stage)
/// min_gb          = MIN_RESIDENT_SHARE·main + activation_gb + 0.5   (auto-fit streams the rest)
/// ```
/// * `main` = diffusion / checkpoint file size;
/// * `other_components` = GPU-resident components that are not text encoders
///   (the VAE), `text_encoders` = GPU text encoders (see
///   [`crate::wiring::gpu_resident_components`]);
/// * `activation_gb` = the family's compute buffers at its default resolution;
/// * 0.5 GB = the engine's device scratch reserve.
///
/// Text encoders are not added to the diffusion stage: they run once per
/// image, before sampling. sd-server's auto-fit (on by default, pinned engine
/// docs/backend.md "Automatic placement") keeps the diffusion weights on the
/// GPU first and parks the text encoders in RAM when they don't fit next to
/// them, releasing their GPU copy after the prompt is read; Pinhole moves them
/// to the processor if reading the prompt still runs out of memory
/// (`--backend te=cpu`, docs/ARCHITECTURE.md). Adding them made every Qwen
/// model read ~8 GB too big.
///
/// Both values are rounded up to 0.1 GB and `min_gb ≤ gb`.
pub fn estimate(
    registry: &Registry,
    family: &Family,
    main_file_bytes: u64,
    other_component_bytes: u64,
    text_encoder_bytes: u64,
) -> VramNeed {
    let _ = registry;
    let main = gib(main_file_bytes);
    let act = family.activation_gb.max(0.0);
    let diffusion = main + gib(other_component_bytes) + act + ENGINE_RESERVE_GB;
    let prompt = if text_encoder_bytes > 0 {
        gib(text_encoder_bytes) + TEXT_ENCODER_COMPUTE_GB + ENGINE_RESERVE_GB
    } else {
        0.0
    };
    let gb = ceil_tenth(diffusion.max(prompt));
    let min_gb = ceil_tenth(MIN_RESIDENT_SHARE * main + act + ENGINE_RESERVE_GB).min(gb);
    VramNeed {
        gb,
        min_gb,
        estimate: true,
        on_cpu: false,
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
        on_cpu: false,
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

/// RAM a model needs on the processor (no usable GPU): every weight file —
/// main file and components all live in RAM — plus the family's
/// `activation_gb`. Rounded up to 0.1 GB; `on_cpu = true`, `min_gb == gb`.
pub fn cpu_need(family: &Family, weights_bytes: u64) -> VramNeed {
    let gb = ceil_tenth(gib(weights_bytes) + family.activation_gb.max(0.0));
    VramNeed {
        gb,
        min_gb: gb,
        estimate: true,
        on_cpu: true,
    }
}

/// Fit without a usable GPU: **Tight** ("runs on the processor — slow") when
/// the model is small enough to be worth running there — its family is
/// `cpu_friendly` or its weights are at most [`CPU_MAX_WEIGHTS_GB`] — and
/// weights + activation ([`cpu_need`]) leave at least [`CPU_SPARE_RAM_GB`] of
/// system RAM free; else **TooBig**. Never Fits. `ram_gb <= 0` (RAM not
/// known yet) skips the RAM check.
pub fn fit_cpu(family: &Family, weights_bytes: u64, ram_gb: f32) -> Fit {
    let small = family.cpu_friendly || gib(weights_bytes) <= CPU_MAX_WEIGHTS_GB + EPS;
    let need = cpu_need(family, weights_bytes).gb;
    let ram_known = ram_gb.is_finite() && ram_gb > 0.0;
    let ram_ok = !ram_known || need + CPU_SPARE_RAM_GB <= ram_gb + EPS;
    if small && ram_ok {
        Fit::Tight
    } else {
        Fit::TooBig
    }
}

/// "Needs ~X GB" + badge on this machine. With a GPU: `need` against VRAM
/// ([`fit`]). Without one ([`HwContext::cpu_only`]): the RAM need of
/// `weights_bytes` (main file + every component the family loads) against
/// system RAM ([`cpu_need`], [`fit_cpu`]).
pub fn need_and_fit(
    family: &Family,
    need: VramNeed,
    weights_bytes: u64,
    hw: &HwContext,
) -> (VramNeed, Fit) {
    if hw.cpu_only() {
        (
            cpu_need(family, weights_bytes),
            fit_cpu(family, weights_bytes, hw.ram_gb),
        )
    } else {
        (need, fit(&need, hw.vram_gb))
    }
}
