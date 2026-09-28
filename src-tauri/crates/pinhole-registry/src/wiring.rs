//! Wiring (SPEC §6): family + installed files + hardware → `sd-server` launch
//! arguments and concrete request parameters. Pure functions, no I/O.
//!
//! Flag names are the ones in stable-diffusion.cpp `examples/common/common.cpp`
//! (`SDContextParams::get_options`, `SDGenerationParams::get_options`) and
//! `examples/server/runtime.cpp` for the pinned engine.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{ComponentChoice, Family, Layout, Registry};

/// Valid `sample_method` names (`sample_method_to_str`, src/stable-diffusion.cpp).
pub const SAMPLERS: &[&str] = &[
    "euler",
    "euler_a",
    "heun",
    "dpm2",
    "dpm++2s_a",
    "dpm++2m",
    "dpm++2mv2",
    "ipndm",
    "ipndm_v",
    "lcm",
    "ddim_trailing",
    "tcd",
    "res_multistep",
    "res_2s",
    "er_sde",
    "euler_cfg_pp",
    "euler_a_cfg_pp",
    "euler_ge",
    "dpm++2m_sde",
    "dpm++2m_sde_bt",
    "lms",
];

/// Valid `scheduler` names (`scheduler_to_str`, src/stable-diffusion.cpp;
/// `normal` is accepted as an alias of `discrete`).
pub const SCHEDULERS: &[&str] = &[
    "discrete",
    "karras",
    "exponential",
    "ays",
    "gits",
    "sgm_uniform",
    "simple",
    "smoothstep",
    "kl_optimal",
    "lcm",
    "bong_tangent",
    "ltx2",
    "logit_normal",
    "flux2",
    "flux",
    "beta",
    "llada_image",
];

/// Boolean (value-less) sd-server options.
const BOOL_FLAGS: &[&str] = &[
    // SDContextParams bool_options
    "--disable-prefetch",
    "--disable-segmented-compute",
    "--eager-load",
    "--force-sdxl-vae-conv-scale",
    "--offload-to-cpu",
    "--mmap",
    "--control-net-cpu",
    "--clip-on-cpu",
    "--vae-on-cpu",
    "--fa",
    "--diffusion-fa",
    "--sage-attn",
    "--diffusion-conv-direct",
    "--vae-conv-direct",
    // SDGenerationParams bool_options (server defaults)
    "--increase-ref-index",
    "--circular",
    "--circularx",
    "--circulary",
    "--disable-image-metadata",
    "--vae-tiling",
    "--temporal-tiling",
    "--hires",
    // server / logging
    "--color",
    "--verbose",
    "--list-devices",
];

/// Options that take one value.
const VALUE_FLAGS: &[&str] = &[
    // SDContextParams string / int / manual options
    "--model",
    "--clip_l",
    "--clip_g",
    "--clip_vision",
    "--t5xxl",
    "--llm",
    "--tokenizer",
    "--llm_vision",
    "--qwen2vl",
    "--qwen2vl_vision",
    "--diffusion-model",
    "--high-noise-diffusion-model",
    "--uncond-diffusion-model",
    "--embeddings-connectors",
    "--vae",
    "--vae-format",
    "--audio-vae",
    "--audio-encoder",
    "--taesd",
    "--tae",
    "--control-net",
    "--ip-adapter",
    "--motion-module",
    "--embd-dir",
    "--lora-model-dir",
    "--hires-upscalers-dir",
    "--tensor-type-rules",
    "--model-args",
    "--photo-maker",
    "--pulid-weights",
    "--upscale-model",
    "--backend",
    "--params-backend",
    "--split-mode",
    "--rpc-servers",
    "--max-vram",
    "--threads",
    "--conditioning-cache-size",
    "--linear-scale",
    "--attn-scale",
    "--auto-fit",
    "--type",
    "--rng",
    "--sampler-rng",
    "--prediction",
    "--lora-apply-mode",
    // SDGenerationParams value options that make sense as server defaults
    "--vae-tile-size",
    "--vae-relative-tile-size",
    "--vae-tile-overlap",
    "--cache-mode",
    "--cache-option",
    "--image-preprocess",
    "--ref-image-args",
    "--extra-sample-args",
    "--extra-tiling-args",
    // server
    "--listen-ip",
    "--listen-port",
    "--serve-html-path",
    "--log-level",
];

/// Value options whose values are comma-separated lists: repeated occurrences
/// are merged instead of the last one silently winning.
const LIST_FLAGS: &[&str] = &[
    "--model-args",
    "--backend",
    "--params-backend",
    "--tensor-type-rules",
    "--split-mode",
    "--rpc-servers",
];

/// `true` for a flag the pinned sd-server understands (used by `Registry::validate`).
pub fn is_known_flag(flag: &str) -> bool {
    BOOL_FLAGS.contains(&flag) || VALUE_FLAGS.contains(&flag)
}

/// Component kind → sd-server flag, in the order they are emitted.
const COMPONENT_FLAGS: &[(&str, &str)] = &[
    ("vae", "--vae"),
    ("clip_l", "--clip_l"),
    ("clip_g", "--clip_g"),
    ("t5xxl", "--t5xxl"),
    ("llm", "--llm"),
    ("llm_vision", "--llm_vision"),
    ("clip_vision", "--clip_vision"),
];

fn kind_rank(kind: &str) -> usize {
    COMPONENT_FLAGS
        .iter()
        .position(|(k, _)| *k == kind)
        .unwrap_or(COMPONENT_FLAGS.len())
}

/// Files that make up one runnable model set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelFiles {
    pub family_id: String,
    /// Checkpoint (all-in-one) or diffusion-only file.
    pub main: PathBuf,
    pub layout: Layout,
    /// component kind (`vae`, `clip_l`, `clip_g`, `t5xxl`, `llm`, `llm_vision`, `taesd`) → path.
    pub components: BTreeMap<String, PathBuf>,
}

/// Hardware facts wiring needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HwContext {
    /// Effective VRAM (detected or user override). 0 for CPU-only.
    pub vram_gb: f32,
    /// `cuda` | `vulkan` | `cpu`
    pub backend: String,
    /// System RAM in GB (0 = unknown, e.g. before hardware detection finishes).
    /// Used for Fits / Too big when there is no usable GPU ([`HwContext::cpu_only`]).
    #[serde(default)]
    pub ram_gb: f32,
}

impl HwContext {
    /// No usable GPU: the engine runs on the processor (backend `cpu`, or no
    /// known VRAM), so models are sized against system RAM
    /// ([`crate::vram::fit_cpu`]) instead of VRAM.
    pub fn cpu_only(&self) -> bool {
        self.backend == "cpu" || self.vram_gb.is_nan() || self.vram_gb <= 0.0
    }
}

/// Extra directories passed at launch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LaunchExtras {
    pub lora_dir: Option<PathBuf>,
    pub upscalers_dir: Option<PathBuf>,
    /// Force VAE tiling on (Fine-tune override). `Some(false)` removes the
    /// profile's `--vae-tiling`. Prefer the per-request value from
    /// [`ResolvedParams::vae_tiling`]: changing launch args restarts the engine.
    pub vae_tiling: Option<bool>,
    /// Enable TAESD preview decoding if the family has one installed.
    /// Only honoured when `engine_features.taesd_preview` is true (see models.yaml).
    pub use_taesd: bool,
}

/// A component the family needs: `(kind, component_id)`, VRAM choice resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredComponent {
    pub kind: String,
    pub component_id: String,
}

/// Resolve a [`ComponentChoice`] for this much VRAM. `ByVram` keys are
/// `vram_gte_<GB>` (the highest threshold ≤ `vram_gb` wins) plus `else`
/// (or `default`) as the fallback.
pub fn resolve_choice(choice: &ComponentChoice, vram_gb: f32) -> Option<String> {
    match choice {
        ComponentChoice::Fixed(id) => Some(id.clone()),
        ComponentChoice::ByVram(map) => {
            let mut best: Option<(f32, &String)> = None;
            for (key, id) in map {
                let Some(t) = key
                    .strip_prefix("vram_gte_")
                    .and_then(|t| t.parse::<f32>().ok())
                else {
                    continue;
                };
                if vram_gb >= t && best.is_none_or(|(b, _)| t > b) {
                    best = Some((t, id));
                }
            }
            best.map(|(_, id)| id.clone())
                .or_else(|| map.get("else").or_else(|| map.get("default")).cloned())
        }
    }
}

/// Components required to run `family` on this hardware (VRAM-dependent choices
/// resolved; `vae_override` included for all-in-one families; TAESD excluded —
/// see [`optional_components`]).
pub fn required_components(
    registry: &Registry,
    family: &Family,
    hw: &HwContext,
) -> Vec<RequiredComponent> {
    let _ = registry;
    let mut out: Vec<RequiredComponent> = Vec::new();
    for (key, choice) in &family.components {
        let kind = match key.as_str() {
            // An explicit `vae` entry wins over `vae_override`.
            "vae_override" if family.components.contains_key("vae") => continue,
            "vae_override" => "vae",
            "taesd" => continue,
            k => k,
        };
        if let Some(id) = resolve_choice(choice, hw.vram_gb) {
            if !id.is_empty() {
                out.push(RequiredComponent {
                    kind: kind.to_owned(),
                    component_id: id,
                });
            }
        }
    }
    out.sort_by(|a, b| {
        kind_rank(&a.kind)
            .cmp(&kind_rank(&b.kind))
            .then_with(|| a.kind.cmp(&b.kind))
    });
    out
}

/// Components whose weights stay on the GPU in the comfortable ("Fits") case,
/// for [`crate::vram::estimate`]: the VAE always; text encoders unless the
/// family pins them to the CPU (`--clip-on-cpu` or `--backend …te=cpu`).
pub fn gpu_resident_components(
    registry: &Registry,
    family: &Family,
    hw: &HwContext,
) -> Vec<RequiredComponent> {
    let te_on_cpu = family.flags.iter().any(|f| f == "--clip-on-cpu")
        || family.flags.windows(2).any(|w| {
            w[0] == "--backend"
                && w[1].split(',').any(|a| {
                    let a = a.trim().to_ascii_lowercase();
                    a == "cpu" || a == "te=cpu" || a == "clip=cpu"
                })
        });
    required_components(registry, family, hw)
        .into_iter()
        .filter(|c| {
            !(te_on_cpu && matches!(c.kind.as_str(), "clip_l" | "clip_g" | "t5xxl" | "llm"))
        })
        .collect()
}

/// Nice-to-have components (TAESD live preview). Empty while the pinned
/// engine cannot use TAESD for previews only (`engine_features.taesd_preview`).
pub fn optional_components(registry: &Registry, family: &Family) -> Vec<RequiredComponent> {
    if !registry.engine_features().taesd_preview {
        return Vec::new();
    }
    family
        .taesd
        .iter()
        .filter(|id| !id.is_empty())
        .map(|id| RequiredComponent {
            kind: "taesd".into(),
            component_id: id.clone(),
        })
        .collect()
}

/// An ordered list of sd-server options with de-duplication.
#[derive(Default)]
struct ArgList {
    units: Vec<(String, Option<String>)>,
}

impl ArgList {
    fn set(&mut self, flag: &str, value: Option<String>) {
        if let Some(existing) = self.units.iter_mut().find(|(f, _)| f == flag) {
            match (&mut existing.1, value) {
                (Some(old), Some(new)) if LIST_FLAGS.contains(&flag) => {
                    for item in new.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                        if !old.split(',').any(|o| o.trim() == item) {
                            old.push(',');
                            old.push_str(item);
                        }
                    }
                }
                (slot, new) => {
                    if new.is_some() {
                        *slot = new; // later source wins for single-value options
                    }
                }
            }
            return;
        }
        self.units.push((flag.to_owned(), value));
    }

    fn remove(&mut self, flag: &str) {
        self.units.retain(|(f, _)| f != flag);
    }

    /// Add raw registry flags (`["--model-args", "a=b", "--diffusion-fa"]`).
    fn extend_raw(&mut self, flags: &[String]) {
        let mut i = 0;
        while i < flags.len() {
            let flag = flags[i].as_str();
            let takes_value = !BOOL_FLAGS.contains(&flag)
                && flag.starts_with('-')
                && flags.get(i + 1).is_some_and(|next| !next.starts_with("--"));
            if takes_value {
                self.set(flag, Some(flags[i + 1].clone()));
                i += 2;
            } else {
                self.set(flag, None);
                i += 1;
            }
        }
    }

    fn into_vec(self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.units.len() * 2);
        for (f, v) in self.units {
            out.push(f);
            if let Some(v) = v {
                out.push(v);
            }
        }
        out
    }
}

fn path_arg(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Build `sd-server` arguments for a model set (without `--listen-ip/--listen-port`
/// and `--log-level`, which the engine manager adds). Includes `--diffusion-model`
/// or `--model`, component flags (`--vae`, `--clip_l`, `--t5xxl`, `--llm`,
/// `--llm_vision`…), family `flags`, hardware-profile flags and extras.
///
/// Order: main file, components (vae, clip_l, clip_g, t5xxl, llm, llm_vision),
/// family flags, hardware-profile flags, `--lora-model-dir`, `--hires-upscalers-dir`.
/// Repeated options are de-duplicated; list options (`--model-args`, `--backend`…)
/// are comma-merged.
pub fn launch_args(
    registry: &Registry,
    files: &ModelFiles,
    hw: &HwContext,
    extras: &LaunchExtras,
) -> Vec<String> {
    let mut args = ArgList::default();
    let main_flag = match files.layout {
        Layout::AllInOne => "--model",
        Layout::DiffusionOnly => "--diffusion-model",
    };
    args.set(main_flag, Some(path_arg(&files.main)));

    let mut comps: Vec<(&String, &PathBuf)> = files.components.iter().collect();
    comps.sort_by_key(|(k, _)| kind_rank(k));
    for (kind, path) in comps {
        if let Some((_, flag)) = COMPONENT_FLAGS.iter().find(|(k, _)| k == kind) {
            args.set(flag, Some(path_arg(path)));
        }
    }
    if extras.use_taesd && registry.engine_features().taesd_preview {
        if let Some(p) = files.components.get("taesd") {
            args.set("--taesd", Some(path_arg(p)));
        }
    }

    if let Some(family) = registry.family(&files.family_id) {
        args.extend_raw(&family.flags);
    }
    args.extend_raw(&registry.hardware_profile(hw.vram_gb).flags);
    match extras.vae_tiling {
        Some(true) => args.set("--vae-tiling", None),
        Some(false) => args.remove("--vae-tiling"),
        None => {}
    }

    if let Some(dir) = &extras.lora_dir {
        args.set("--lora-model-dir", Some(path_arg(dir)));
    }
    if let Some(dir) = &extras.upscalers_dir {
        args.set("--hires-upscalers-dir", Some(path_arg(dir)));
    }
    args.into_vec()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Shape {
    Square,
    Portrait,
    Landscape,
    Wide,
}

impl Shape {
    /// Key in `dials.shape`.
    pub fn key(self) -> &'static str {
        match self {
            Shape::Square => "square",
            Shape::Portrait => "portrait",
            Shape::Landscape => "landscape",
            Shape::Wide => "wide",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Quality {
    Fast,
    Balanced,
    Best,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GenMode {
    Txt2img,
    Img2img,
    Edit,
}

/// Fine-tune drawer overrides. `None` = use the registry default.
/// PRIVACY: `negative_prompt` is user text — never persist or log this struct.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FineTune {
    pub sampler: Option<String>,
    pub scheduler: Option<String>,
    pub steps: Option<u32>,
    pub cfg: Option<f32>,
    pub guidance: Option<f32>,
    pub seed: Option<i64>,
    pub flow_shift: Option<f32>,
    pub clip_skip: Option<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub hires: Option<bool>,
    pub hires_scale: Option<f32>,
    pub hires_denoise: Option<f32>,
    pub vae_tiling: Option<bool>,
    pub negative_prompt: Option<String>,
    /// Add the family's `auto_prompt_prefix` (e.g. Pony score tags). Default true.
    pub auto_prompt_prefix: Option<bool>,
}

/// Dial positions from the simple UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dials {
    pub shape: Shape,
    pub quality: Quality,
    /// "Stick to prompt" / "Stay close to original", 0.0 (loose) ..= 1.0 (strict).
    pub stick: f32,
    /// 1 | 2 | 4
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HiresParams {
    pub scale: f32,
    pub denoising_strength: f32,
    /// Second-pass steps; `0` = reuse `steps` (sd-server `hires.steps`).
    pub steps: u32,
}

/// Concrete parameters for one `img_gen` request (prompt text excluded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedParams {
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg: f32,
    pub guidance: Option<f32>,
    pub sampler: Option<String>,
    pub scheduler: Option<String>,
    pub clip_skip: Option<i32>,
    pub flow_shift: Option<f32>,
    pub hires: Option<HiresParams>,
    pub vae_tiling: bool,
    pub batch_count: u32,
}

/// Fallback when a family has no `dials.quality`: sd.cpp's default step count.
const DEFAULT_STEPS: u32 = 20;
/// sd.cpp's default `txt_cfg`.
const DEFAULT_CFG: f32 = 7.0;
const DEFAULT_HIRES_SCALE: f32 = 1.5;
const DEFAULT_HIRES_DENOISE: f32 = 0.45;
const MAX_SIDE: u32 = 4096;

/// Width/height multiple for this family (`dials.size_multiple`, default 64).
pub fn size_multiple(family: &Family) -> u32 {
    family.dials.size_multiple.filter(|m| *m > 0).unwrap_or(64)
}

/// Round `v` to the nearest multiple of `m` (at least `m`, at most 4096).
pub fn round_to_multiple(v: u32, m: u32) -> u32 {
    let m = m.max(1);
    let r = (v.saturating_add(m / 2) / m) * m;
    r.clamp(m, (MAX_SIDE / m).max(1) * m)
}

/// `[w, h]` for a shape (falls back to square, then 1024², when the family lacks it).
pub fn shape_size(family: &Family, shape: Shape) -> [u32; 2] {
    family
        .dials
        .shape
        .get(shape.key())
        .or_else(|| family.dials.shape.get("square"))
        .copied()
        .unwrap_or([1024, 1024])
}

/// What the stick dial drives in `mode`: `cfg` or `guidance`.
fn stick_target(family: &Family, edit: bool) -> &str {
    let d = &family.dials;
    let t = if edit {
        d.stay_close_maps_to
            .as_deref()
            .or(d.stick_to_prompt_maps_to.as_deref())
    } else {
        d.stick_to_prompt_maps_to.as_deref()
    };
    match t {
        Some("guidance") => "guidance",
        _ => "cfg",
    }
}

fn lerp([lo, hi]: [f32; 2], t: f32) -> f32 {
    lo + (hi - lo) * t
}

/// Engine value → dial position (inverse of the mapping in [`resolve_params`]).
fn position([lo, hi]: [f32; 2], value: f32, inverted: bool) -> f32 {
    if (hi - lo).abs() < f32::EPSILON {
        return 0.5;
    }
    let p = ((value - lo) / (hi - lo)).clamp(0.0, 1.0);
    if inverted {
        1.0 - p
    } else {
        p
    }
}

/// Map dials + fine-tune overrides to concrete parameters for `family`.
///
/// * size: `dials.shape[shape]` or the Fine-tune width/height, rounded to
///   `dials.size_multiple`;
/// * steps: `dials.quality` (Fine-tune `steps` wins);
/// * "Stick to prompt" (txt2img/img2img) maps `stick` 0..1 linearly onto
///   `cfg_range`, or onto `guidance_range` when `stick_to_prompt_maps_to: guidance`;
///   `cfg_fixed` pins CFG;
/// * "Stay close to original" (edit) uses `stay_close_maps_to` and is
///   **inverted**: strict (1.0) = the low end of the range, because more
///   CFG / guidance pushes the edit further away from the source image;
/// * hires fix at Best when `hires_at_best.enabled` (txt2img only; Fine-tune
///   `hires` can force it on/off);
/// * `vae_tiling` from the hardware profile flags unless overridden;
/// * `batch_count` = `dials.count`.
pub fn resolve_params(
    registry: &Registry,
    family: &Family,
    dials: &Dials,
    fine: &FineTune,
    mode: GenMode,
    hw: &HwContext,
) -> ResolvedParams {
    let d = &family.dials;
    let multiple = size_multiple(family);
    let [sw, sh] = shape_size(family, dials.shape);
    let width = round_to_multiple(fine.width.filter(|w| *w > 0).unwrap_or(sw), multiple);
    let height = round_to_multiple(fine.height.filter(|h| *h > 0).unwrap_or(sh), multiple);

    let quality_steps = d.quality.as_ref().map(|q| match dials.quality {
        Quality::Fast => q.fast,
        Quality::Balanced => q.balanced,
        Quality::Best => q.best,
    });
    let steps = fine
        .steps
        .filter(|s| *s > 0)
        .or(quality_steps)
        .unwrap_or(DEFAULT_STEPS)
        .max(1);

    let edit = mode == GenMode::Edit;
    let stick = if dials.stick.is_finite() {
        dials.stick.clamp(0.0, 1.0)
    } else {
        0.5
    };
    let t = if edit { 1.0 - stick } else { stick };
    let target = stick_target(family, edit);

    let cfg = fine.cfg.filter(|c| c.is_finite()).unwrap_or_else(|| {
        match (d.cfg_fixed, target, d.cfg_range) {
            (Some(fixed), _, _) => fixed,
            (None, "cfg", Some(range)) => lerp(range, t),
            _ => d.cfg_default.unwrap_or(DEFAULT_CFG),
        }
    });
    let guidance = fine
        .guidance
        .filter(|g| g.is_finite())
        .or(match (target, d.guidance_range) {
            ("guidance", Some(range)) => Some(lerp(range, t)),
            _ => family.defaults.guidance,
        });

    let hires = {
        let cfg_h = d.hires_at_best.as_ref();
        let auto = cfg_h.is_some_and(|h| h.enabled) && dials.quality == Quality::Best;
        let on = mode == GenMode::Txt2img && fine.hires.unwrap_or(auto);
        on.then(|| HiresParams {
            scale: fine
                .hires_scale
                .filter(|s| s.is_finite() && *s > 1.0)
                .or(cfg_h.and_then(|h| h.scale))
                .unwrap_or(DEFAULT_HIRES_SCALE),
            denoising_strength: fine
                .hires_denoise
                .filter(|s| s.is_finite())
                .or(cfg_h.and_then(|h| h.denoising_strength))
                .unwrap_or(DEFAULT_HIRES_DENOISE)
                .clamp(0.0, 1.0),
            steps: cfg_h.and_then(|h| h.steps).unwrap_or(0),
        })
    };

    let vae_tiling = fine.vae_tiling.unwrap_or_else(|| {
        registry
            .hardware_profile(hw.vram_gb)
            .flags
            .iter()
            .any(|f| f == "--vae-tiling")
    });

    ResolvedParams {
        width,
        height,
        steps,
        cfg,
        guidance,
        sampler: fine
            .sampler
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| family.defaults.sampler.clone()),
        scheduler: fine
            .scheduler
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| family.defaults.scheduler.clone()),
        clip_skip: fine.clip_skip.or(family.defaults.clip_skip),
        flow_shift: fine
            .flow_shift
            .filter(|f| f.is_finite())
            .or(family.defaults.flow_shift),
        hires,
        vae_tiling,
        batch_count: dials.count.max(1),
    }
}

/// Everything the UI needs to render dials + Fine-tune defaults for a family.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyUi {
    pub family_id: String,
    pub label: String,
    pub modes: Vec<String>,
    /// shape name → [w, h]
    pub shapes: BTreeMap<String, [u32; 2]>,
    pub quality_steps: [u32; 3],
    /// Show the "Stick to prompt" dial.
    pub show_stick: bool,
    /// What the dial maps to: `cfg` | `guidance`.
    pub stick_maps_to: String,
    /// Engine-unit range the dial spans (`cfg_range` or `guidance_range`).
    pub stick_range: [f32; 2],
    /// Default dial POSITION, 0.0..=1.0 (use it as `Dials.stick`). It maps back
    /// to `default_cfg` / `default_guidance`; for edit families the dial is
    /// inverted ("Stay close to original": 1.0 = low end of `stick_range`).
    pub stick_default: f32,
    pub uses_negative_prompt: bool,
    pub default_negative_prompt: Option<String>,
    pub default_sampler: Option<String>,
    pub default_scheduler: Option<String>,
    pub default_clip_skip: Option<i32>,
    pub default_flow_shift: Option<f32>,
    pub default_cfg: f32,
    pub default_guidance: Option<f32>,
    pub auto_prompt_prefix: Option<String>,
    pub hires_at_best: bool,
    pub license_note: Option<String>,
    pub is_edit_family: bool,
}

/// `true` for instruction-edit families (`role: edit` or mode `edit`).
pub fn is_edit_family(family: &Family) -> bool {
    family.role.as_deref() == Some("edit") || family.modes.iter().any(|m| m == "edit")
}

pub fn family_ui(registry: &Registry, family: &Family) -> FamilyUi {
    let _ = registry;
    let d = &family.dials;
    let edit = is_edit_family(family);
    let target = stick_target(family, edit);
    let default_cfg = d
        .cfg_fixed
        .or(d.cfg_default)
        .unwrap_or_else(|| d.cfg_range.map_or(DEFAULT_CFG, |r| lerp(r, 0.5)));

    let (show_stick, range, default_value) = match target {
        "guidance" => match d.guidance_range {
            Some(r) => (
                true,
                r,
                family.defaults.guidance.unwrap_or_else(|| lerp(r, 0.5)),
            ),
            None => (false, [default_cfg, default_cfg], default_cfg),
        },
        _ => match (d.cfg_fixed, d.cfg_range) {
            (None, Some(r)) => (true, r, d.cfg_default.unwrap_or_else(|| lerp(r, 0.5))),
            _ => (false, [default_cfg, default_cfg], default_cfg),
        },
    };
    let stick_default = if show_stick {
        position(range, default_value, edit)
    } else {
        0.5
    };

    let quality_steps = d
        .quality
        .as_ref()
        .map_or([DEFAULT_STEPS; 3], |q| [q.fast, q.balanced, q.best]);
    let mut shapes = d.shape.clone();
    if shapes.is_empty() {
        shapes.insert("square".into(), [1024, 1024]);
    }

    FamilyUi {
        family_id: family.id.clone(),
        label: family.label.clone(),
        modes: family.modes.clone(),
        shapes,
        quality_steps,
        show_stick,
        stick_maps_to: target.to_owned(),
        stick_range: range,
        stick_default,
        uses_negative_prompt: family.uses_negative_prompt,
        default_negative_prompt: family
            .defaults
            .negative_prompt
            .clone()
            .filter(|_| family.uses_negative_prompt),
        default_sampler: family.defaults.sampler.clone(),
        default_scheduler: family.defaults.scheduler.clone(),
        default_clip_skip: family.defaults.clip_skip,
        default_flow_shift: family.defaults.flow_shift,
        default_cfg,
        default_guidance: family.defaults.guidance,
        auto_prompt_prefix: family.defaults.auto_prompt_prefix.clone(),
        hires_at_best: d.hires_at_best.as_ref().is_some_and(|h| h.enabled),
        license_note: family.license_note.clone(),
        is_edit_family: edit,
    }
}
