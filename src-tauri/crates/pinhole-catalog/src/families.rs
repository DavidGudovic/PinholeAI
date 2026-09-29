//! Registry-facing helpers shared by the catalog, recommended picks and the
//! installed-models view: family resolution (SPEC §6 order), architecture
//! compatibility (for LoRAs), components per family, VRAM needs and quant
//! choice. Model knowledge stays in `config/models.yaml`; this only reads it.

use std::collections::BTreeSet;

use pinhole_registry::vram::{self, Fit, VramNeed};
use pinhole_registry::wiring::{self, HwContext, RequiredComponent};
use pinhole_registry::{Component, ComponentChoice, DownloadSpec, Family, Registry, VramGb};
use pinhole_store::datadir::ModelKind;
use pinhole_store::{InstalledFile, InstalledIndex};

use crate::view::FamilyChoice;

/// Registry sizes (`size_mb`) are decimal megabytes.
pub const MB: u64 = 1_000_000;

pub fn mb_to_bytes(mb: u64) -> u64 {
    mb.saturating_mul(MB)
}

/// Lowercase 64-hex SHA-256, or `None` for `TODO` / malformed values.
pub fn normalize_sha(s: &str) -> Option<String> {
    let s = s.trim().to_ascii_lowercase();
    (s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())).then_some(s)
}

pub fn family_choice(registry: &Registry, id: &str) -> Option<FamilyChoice> {
    registry.family(id).map(|f| FamilyChoice {
        family_id: id.to_string(),
        label: f.label.clone(),
    })
}

// ------------------------------------------------------------------ family resolution

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FamilyResolution {
    Resolved(String),
    /// Ask the user (candidate ids, best first).
    Ambiguous(Vec<String>),
    /// Nothing in the registry runs this. Carries the CivitAI base model, if any.
    Unsupported(Option<String>),
}

/// CivitAI uses this for "unknown architecture".
pub const OTHER_BASE_MODEL: &str = "Other";

/// SPEC §6: known hash → CivitAI `baseModel` → header sniffing → ask.
/// `header_candidates` narrows an ambiguous base-model match.
pub fn resolve_family(
    registry: &Registry,
    sha256: Option<&str>,
    base_model: Option<&str>,
    header_candidates: Option<&[String]>,
) -> FamilyResolution {
    if let Some(known) = sha256
        .and_then(normalize_sha)
        .and_then(|h| registry.known_file(&h).map(|k| k.family.clone()))
    {
        if registry.family(&known).is_some() {
            return FamilyResolution::Resolved(known);
        }
    }
    let header: Vec<String> = header_candidates
        .unwrap_or(&[])
        .iter()
        .filter(|id| registry.family(id).is_some())
        .cloned()
        .collect();
    let base = base_model.map(str::trim).filter(|b| !b.is_empty());
    if let Some(base) = base.filter(|b| !b.eq_ignore_ascii_case(OTHER_BASE_MODEL)) {
        let by_base: Vec<String> = registry
            .families_for_base_model(base)
            .iter()
            .map(|f| family_id(registry, f))
            .collect();
        match by_base.len() {
            1 => return FamilyResolution::Resolved(by_base[0].clone()),
            n if n > 1 => {
                let narrowed: Vec<String> = by_base
                    .iter()
                    .filter(|id| header.contains(id))
                    .cloned()
                    .collect();
                return match narrowed.len() {
                    1 => FamilyResolution::Resolved(narrowed[0].clone()),
                    0 => FamilyResolution::Ambiguous(by_base),
                    _ => FamilyResolution::Ambiguous(narrowed),
                };
            }
            _ if header.is_empty() => return FamilyResolution::Unsupported(Some(base.to_string())),
            _ => {}
        }
    }
    match header.len() {
        1 => return FamilyResolution::Resolved(header[0].clone()),
        n if n > 1 => return FamilyResolution::Ambiguous(header),
        _ => {}
    }
    if base.is_some_and(|b| b.eq_ignore_ascii_case(OTHER_BASE_MODEL)) {
        return FamilyResolution::Ambiguous(
            registry
                .families()
                .map(|f| family_id(registry, f))
                .collect(),
        );
    }
    FamilyResolution::Unsupported(base.map(str::to_string))
}

/// A family's id. The registry fills `Family::id` when it resolves families.
pub fn family_id(_registry: &Registry, f: &Family) -> String {
    f.id.clone()
}

pub fn unsupported_message(base: Option<&str>) -> String {
    match base {
        Some(b) => format!("Pinhole can't run {b} models yet."),
        None => "Pinhole doesn't recognise this kind of model.".to_string(),
    }
}

/// Architecture root of a family: follow `detect.same_as`, then `inherits`,
/// to the family that defines the tensor layout (Pony / Illustrious → SDXL,
/// Kontext / schnell → FLUX.1 dev, Qwen Image Edit → Qwen-Image).
pub fn arch_root(registry: &Registry, family_id: &str) -> String {
    let mut id = family_id.to_string();
    for _ in 0..16 {
        let Some(f) = registry.family(&id) else { break };
        match f.detect.same_as.clone().or_else(|| f.inherits.clone()) {
            Some(next) if next != id && registry.family(&next).is_some() => id = next,
            _ => break,
        }
    }
    id
}

/// Same tensor layout: a LoRA for one works with the other.
pub fn same_architecture(registry: &Registry, a: &str, b: &str) -> bool {
    a == b || arch_root(registry, a) == arch_root(registry, b)
}

// ------------------------------------------------------------------ components

/// Plain-language label for a component kind.
pub fn component_kind_label(kind: &str) -> &'static str {
    match kind {
        "vae" => "Image decoder (VAE)",
        "clip_l" | "clip_g" | "t5xxl" | "llm" => "Text encoder",
        "llm_vision" => "Vision encoder",
        "taesd" => "Live preview",
        "upscaler" => "Upscaler",
        _ => "Model part",
    }
}

pub fn component_label(c: &Component) -> String {
    format!("{} · {}", component_kind_label(&c.kind), c.file)
}

/// `Data/models/<kind>/` folder for a component kind.
pub fn component_model_kind(kind: &str) -> ModelKind {
    match kind {
        "vae" => ModelKind::Vae,
        "taesd" => ModelKind::Taesd,
        "upscaler" => ModelKind::Upscaler,
        _ => ModelKind::TextEncoder,
    }
}

/// Main-file folder for a family's layout.
pub fn main_model_kind(family: &Family) -> ModelKind {
    match family.layout {
        pinhole_registry::Layout::AllInOne => ModelKind::Checkpoint,
        pinhole_registry::Layout::DiffusionOnly => ModelKind::Diffusion,
    }
}

/// Every component id a family could use (all VRAM-dependent choices, TAESD).
/// Used for "is this component still needed by someone" when deleting.
pub fn family_component_ids(family: &Family) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for choice in family.components.values() {
        match choice {
            ComponentChoice::Fixed(id) => {
                out.insert(id.clone());
            }
            ComponentChoice::ByVram(map) => out.extend(map.values().cloned()),
        }
    }
    if let Some(t) = &family.taesd {
        out.insert(t.clone());
    }
    out
}

/// Installed file for a component: by component id, or by its verified SHA-256.
pub fn installed_component<'a>(
    index: &'a InstalledIndex,
    id: &str,
    c: &Component,
) -> Option<&'a InstalledFile> {
    index
        .find_component(id)
        .or_else(|| normalize_sha(&c.sha256).and_then(|h| index.find_by_sha(&h)))
}

/// Components to install for `family` on this hardware: required ones plus,
/// when `with_optional`, the tiny live-preview decoder. Deduplicated. When
/// the pick for this VRAM is not installed but another option of the same
/// choice is (another quant of the text encoder), that one is used instead of
/// downloading a second copy.
pub fn wanted_components(
    registry: &Registry,
    family: &Family,
    hw: &HwContext,
    index: &InstalledIndex,
    with_optional: bool,
) -> Vec<RequiredComponent> {
    let installed = |id: &str| {
        registry
            .component(id)
            .is_some_and(|c| installed_component(index, id, c).is_some())
    };
    let mut out: Vec<RequiredComponent> = Vec::new();
    let mut add = |rc: RequiredComponent| {
        if !out.iter().any(|o| o.component_id == rc.component_id) {
            out.push(rc);
        }
    };
    wiring::required_components_with(registry, family, hw, &installed)
        .into_iter()
        .for_each(&mut add);
    if with_optional {
        wiring::optional_components(registry, family)
            .into_iter()
            .for_each(&mut add);
    }
    out
}

/// Wanted components that are not installed yet.
pub fn missing_components<'a>(
    registry: &'a Registry,
    family: &Family,
    hw: &HwContext,
    index: &InstalledIndex,
    with_optional: bool,
) -> Vec<(String, &'a Component)> {
    wanted_components(registry, family, hw, index, with_optional)
        .into_iter()
        .filter_map(|rc| {
            registry
                .component(&rc.component_id)
                .map(|c| (rc.component_id, c))
        })
        .filter(|(id, c)| installed_component(index, id, c).is_none())
        .collect()
}

/// Components an installed model still lacks before it can run: like
/// [`missing_components`] (required only), but another installed option of a
/// VRAM-dependent choice counts (`wiring::required_components_with`).
pub fn missing_to_run<'a>(
    registry: &'a Registry,
    family: &Family,
    hw: &HwContext,
    index: &InstalledIndex,
) -> Vec<(String, &'a Component)> {
    let installed = |id: &str| {
        registry
            .component(id)
            .is_some_and(|c| installed_component(index, id, c).is_some())
    };
    wiring::required_components_with(registry, family, hw, &installed)
        .into_iter()
        .filter_map(|rc| {
            registry
                .component(&rc.component_id)
                .map(|c| (rc.component_id, c))
        })
        .filter(|(id, c)| installed_component(index, id, c).is_none())
        .collect()
}

/// Kinds read only while encoding the prompt (a separate stage, see `vram::estimate`).
pub fn is_text_encoder_kind(kind: &str) -> bool {
    matches!(kind, "clip_l" | "clip_g" | "t5xxl" | "llm" | "llm_vision")
}

/// Whether a registry component is installed (by id or verified SHA-256).
fn installed_pred<'a>(
    registry: &'a Registry,
    index: &'a InstalledIndex,
) -> impl Fn(&str) -> bool + 'a {
    move |id: &str| {
        registry
            .component(id)
            .is_some_and(|c| installed_component(index, id, c).is_some())
    }
}

/// Bytes of the components that stay on the GPU (registry
/// `wiring::gpu_resident_components`), for `vram::estimate`:
/// `(other components such as the VAE, text encoders)`.
pub fn gpu_component_bytes(
    registry: &Registry,
    family: &Family,
    hw: &HwContext,
    index: &InstalledIndex,
) -> (u64, u64) {
    wiring::gpu_resident_components_with(registry, family, hw, &installed_pred(registry, index))
        .iter()
        .filter_map(|rc| registry.component(&rc.component_id))
        .fold((0, 0), |(other, te), c| {
            let b = mb_to_bytes(c.size_mb);
            if is_text_encoder_kind(&c.kind) {
                (other, te + b)
            } else {
                (other + b, te)
            }
        })
}

/// [`vram::estimate`] for a main file of `family` on this hardware.
pub fn estimate_need(
    registry: &Registry,
    family: &Family,
    hw: &HwContext,
    index: &InstalledIndex,
    main_bytes: u64,
) -> VramNeed {
    let (other, te) = gpu_component_bytes(registry, family, hw, index);
    vram::estimate(registry, family, main_bytes, other, te)
}

// ------------------------------------------------------------------ VRAM

pub fn need_from(v: &VramGb) -> VramNeed {
    VramNeed {
        gb: v.recommended,
        min_gb: v.min.min(v.recommended),
        estimate: false,
        on_cpu: false,
    }
}

/// Registry figure for the family when it has one (measured), else an
/// estimate from the main file size (SPEC §6.2).
pub fn family_need(
    registry: &Registry,
    family: &Family,
    hw: &HwContext,
    index: &InstalledIndex,
    main_bytes: u64,
) -> VramNeed {
    match &family.vram_gb {
        Some(v) => need_from(v),
        None => estimate_need(registry, family, hw, index, main_bytes),
    }
}

/// Bytes a model keeps in RAM when it runs on the processor: the main file
/// plus every required component (VAE, text encoders) for this hardware.
pub fn cpu_weight_bytes(
    registry: &Registry,
    family: &Family,
    hw: &HwContext,
    index: &InstalledIndex,
    main_bytes: u64,
) -> u64 {
    wiring::required_components_with(registry, family, hw, &installed_pred(registry, index))
        .iter()
        .filter_map(|rc| registry.component(&rc.component_id))
        .fold(main_bytes, |sum, c| {
            sum.saturating_add(mb_to_bytes(c.size_mb))
        })
}

/// "Needs ~X GB" + Fits / Tight / Too big on this machine (SPEC §6.2): the GPU
/// need against VRAM, or — without a usable GPU ([`HwContext::cpu_only`]) —
/// the RAM for the main file + components against system RAM
/// ([`vram::fit_cpu`]: Tight = "runs on the processor — slow").
pub fn need_and_fit(
    registry: &Registry,
    family: &Family,
    hw: &HwContext,
    index: &InstalledIndex,
    need: VramNeed,
    main_bytes: u64,
) -> (VramNeed, Fit) {
    let weights = if hw.cpu_only() {
        cpu_weight_bytes(registry, family, hw, index, main_bytes)
    } else {
        main_bytes
    };
    vram::need_and_fit(family, need, weights, hw)
}

// ------------------------------------------------------------------ quants

/// One downloadable variant of a registry model (main `download` or an `alt_quants` entry).
#[derive(Debug, Clone, PartialEq)]
pub struct QuantOption {
    /// Normalised quant (`bf16`, `q8_0`, `q4_k`…).
    pub quant: String,
    pub file: String,
    pub url: String,
    pub sha256: Option<String>,
    pub size_bytes: u64,
    pub vram: Option<VramGb>,
}

/// Normalised quant/precision from a file name or quant key.
pub fn quant_of_file(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    let table: &[(&[&str], &str)] = &[
        (&["bf16"], "bf16"),
        (&["fp16", "f16"], "fp16"),
        (&["fp8", "f8_e4m3", "f8_e5m2", "e4m3fn"], "fp8"),
        (&["q8_0", "q8"], "q8_0"),
        (&["q6_k"], "q6_k"),
        (&["q5_k"], "q5_k"),
        (&["q5_0", "q5_1"], "q5_0"),
        (&["q4_k"], "q4_k"),
        (&["q4_0", "q4_1"], "q4_0"),
        (&["q3_k"], "q3_k"),
        (&["q2_k"], "q2_k"),
        (&["fp32", "f32"], "fp32"),
    ];
    table
        .iter()
        .find(|(pats, _)| pats.iter().any(|p| n.contains(p)))
        .map(|(_, q)| q.to_string())
        .unwrap_or_else(|| "unknown".into())
}

/// Lower = higher quality. Unknown files are treated as full precision.
pub fn quant_rank(q: &str) -> u32 {
    match quant_of_file(q).as_str() {
        "fp32" => 0,
        "bf16" | "fp16" | "unknown" => 1,
        "fp8" => 2,
        "q8_0" => 3,
        "q6_k" => 4,
        "q5_k" => 5,
        "q5_0" => 6,
        "q4_k" => 7,
        "q4_0" => 8,
        "q3_k" => 9,
        _ => 10,
    }
}

/// Short suffix for friendly names (`Q8`, `Q4`, `FP8`), none for full precision.
pub fn quant_suffix(q: &str) -> Option<&'static str> {
    match quant_of_file(q).as_str() {
        "fp8" => Some("FP8"),
        "q8_0" => Some("Q8"),
        "q6_k" => Some("Q6"),
        "q5_k" | "q5_0" => Some("Q5"),
        "q4_k" | "q4_0" => Some("Q4"),
        "q3_k" => Some("Q3"),
        "q2_k" => Some("Q2"),
        _ => None,
    }
}

pub fn file_name_from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// Main download + `alt_quants`, best quality first.
pub fn quant_options(spec: &DownloadSpec) -> Vec<QuantOption> {
    let mut out = vec![QuantOption {
        quant: quant_of_file(&spec.file),
        file: spec.file.clone(),
        url: spec.url.clone(),
        sha256: normalize_sha(&spec.sha256),
        size_bytes: mb_to_bytes(spec.size_mb),
        vram: spec.vram_gb,
    }];
    for (key, q) in &spec.alt_quants {
        out.push(QuantOption {
            quant: quant_of_file(key),
            file: q.file.clone().unwrap_or_else(|| file_name_from_url(&q.url)),
            url: q.url.clone(),
            sha256: normalize_sha(&q.sha256),
            size_bytes: mb_to_bytes(q.size_mb),
            vram: q.vram_gb,
        });
    }
    out.sort_by(|a, b| {
        quant_rank(&a.quant)
            .cmp(&quant_rank(&b.quant))
            .then(b.size_bytes.cmp(&a.size_bytes))
    });
    out
}

/// SPEC §6.1: the best quant that **Fits**, starting at the hardware tier's
/// `prefer_quant` (bf16 → Q8 → Q4); if none Fits, the Tight one that is
/// closest to fitting (lowest need: least of the model streamed from RAM).
/// A recommended pick must run well on the card, so a smaller version that
/// Fits beats a bigger one that is Tight. `fit_of` sizes one option on this
/// machine (see [`need_and_fit`]).
pub fn choose_quant<'a>(
    options: &'a [QuantOption],
    prefer: Option<&str>,
    fit_of: impl Fn(&QuantOption) -> (VramNeed, Fit),
) -> Option<(&'a QuantOption, VramNeed, Fit)> {
    let start = prefer.map(quant_rank).unwrap_or(0);
    let preferred = options.iter().filter(|o| quant_rank(&o.quant) >= start);
    let rest = options.iter().filter(|o| quant_rank(&o.quant) < start);
    let sized: Vec<(&QuantOption, VramNeed, Fit)> = preferred
        .chain(rest)
        .map(|o| (o, fit_of(o)))
        .map(|(o, (n, f))| (o, n, f))
        .collect();
    sized
        .iter()
        .find(|(_, _, f)| *f == Fit::Fits)
        .cloned()
        .or_else(|| {
            // min_by keeps the first of equal needs (preference order).
            sized
                .iter()
                .filter(|(_, _, f)| *f == Fit::Tight)
                .min_by(|a, b| a.1.gb.total_cmp(&b.1.gb))
                .cloned()
        })
}

/// VRAM need of an installed main model: measured peak → registry download
/// spec for this exact file → family figure → estimate from file size.
pub fn installed_need(
    registry: &Registry,
    family: &Family,
    file: &InstalledFile,
    hw: &HwContext,
    index: &InstalledIndex,
) -> VramNeed {
    if let Some(o) = file.observed_vram_gb.filter(|o| o.is_finite() && *o > 0.0) {
        return VramNeed {
            gb: o,
            min_gb: o,
            estimate: false,
            on_cpu: false,
        };
    }
    if let Some(spec) = &family.download {
        let name = file.rel_path.rsplit('/').next().unwrap_or(&file.rel_path);
        let hit = quant_options(spec).into_iter().find(|o| {
            o.sha256
                .as_deref()
                .is_some_and(|h| h.eq_ignore_ascii_case(&file.sha256))
                || o.file.eq_ignore_ascii_case(name)
        });
        if let Some(v) = hit.and_then(|o| o.vram) {
            return need_from(&v);
        }
    }
    family_need(registry, family, hw, index, file.size_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quant_names() {
        assert_eq!(quant_of_file("z_image_turbo_bf16.safetensors"), "bf16");
        assert_eq!(quant_of_file("qwen-image-edit-2511-Q4_K_M.gguf"), "q4_k");
        assert_eq!(quant_of_file("z_image_turbo-Q8_0.gguf"), "q8_0");
        assert_eq!(quant_of_file("q8_0"), "q8_0");
        assert_eq!(quant_of_file("t5xxl_fp8_e4m3fn.safetensors"), "fp8");
        assert_eq!(quant_of_file("model.safetensors"), "unknown");
        assert!(quant_rank("bf16") < quant_rank("q8_0"));
        assert!(quant_rank("q8_0") < quant_rank("q4_k"));
        assert_eq!(quant_suffix("q4_k"), Some("Q4"));
        assert_eq!(quant_suffix("bf16"), None);
        assert_eq!(
            file_name_from_url("https://huggingface.co/a/b/resolve/main/x-Q4_K.gguf?download=true"),
            "x-Q4_K.gguf"
        );
    }

    #[test]
    fn sha_normalisation() {
        assert_eq!(normalize_sha("TODO"), None);
        assert_eq!(normalize_sha(&"AB".repeat(32)), Some("ab".repeat(32)));
        assert_eq!(normalize_sha("abc"), None);
    }

    #[test]
    fn labels_are_plain() {
        assert_eq!(component_kind_label("t5xxl"), "Text encoder");
        assert_eq!(component_model_kind("llm_vision"), ModelKind::TextEncoder);
        assert_eq!(component_model_kind("vae"), ModelKind::Vae);
    }
}
