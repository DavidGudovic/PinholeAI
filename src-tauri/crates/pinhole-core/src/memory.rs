//! Out of memory (docs/ARCHITECTURE.md §4): before sd-server starts
//! ([`crate::engine`]), leftover engines under `Data/engine/` are killed, an idle
//! describe engine is stopped and (NVIDIA) graphics memory used by other programs
//! is measured. A job that
//! runs out of memory is retried with each memory-saving choice at most once:
//! reading the prompt → the text encoder moves to the processor
//! (`--backend te=cpu`, remembered per model for the app session; Settings
//! `textEncoderOnCpu` can force it on or off); decoding / unknown stage →
//! `--vae-tiling` if it isn't on yet; then (right away when denoising runs
//! out) more of the card is kept free (`--max-vram -4`, every GPU launch has
//! `-2`, see [`VRAM_RESERVE_GIB`]); then the weights stay in system memory and
//! are streamed to the card (`--offload-to-cpu`, only while that engine stays
//! loaded). Otherwise the error is `vram` with a
//! message that says what to do next (never the generic "couldn't make this
//! image").

use pinhole_engine::failure::Stage;
use pinhole_hardware::OtherGpuUse;

use crate::engine::EngineFlags;
use crate::{AppCore, CoreError};

/// The graphics card ran out of memory (no numbers about other programs).
pub const VRAM_MESSAGE: &str = "Your graphics card ran out of memory. Close other programs that use the graphics card (games, other AI apps) and try again, or pick the smaller version of this model in Models.";

/// System memory ran out (CPU engine, or the text encoder already on the processor).
pub const RAM_MESSAGE: &str = "Your computer ran out of memory. Close other programs and try again, or pick the smaller version of this model in Models.";

/// Reading the prompt ran out of graphics memory while Settings keeps the text encoder on the card.
pub const TE_ON_GPU_MESSAGE: &str = "Your graphics card ran out of memory while reading your prompt. In Settings → Engine, set “Read the prompt on the processor” to Automatic or On, or close other programs that use the graphics card and try again.";

pub(crate) const TE_RETRY_NOTE: &str = "Your graphics card ran out of memory while reading your prompt — trying again with that step on the processor (a bit slower).";

pub(crate) const TILING_RETRY_NOTE: &str =
    "Your graphics card ran out of memory — trying once more with memory-saving settings.";

pub(crate) const OFFLOAD_RETRY_NOTE: &str = "Your graphics card ran out of memory — trying again with the model kept in system memory and sent to the card as needed (slower).";

pub(crate) const MORE_ROOM_RETRY_NOTE: &str = "Your graphics card ran out of memory. Trying again with the model loaded onto the card in parts (slower).";

const RETRY_NOTES: &[&str] = &[
    TE_RETRY_NOTE,
    TILING_RETRY_NOTE,
    MORE_ROOM_RETRY_NOTE,
    OFFLOAD_RETRY_NOTE,
];

const MORE_ROOM_NOTE: &str = "This model keeps more of the graphics card free and is sent to it in parts, because the card ran out of memory earlier. Pictures take a bit longer.";

/// Graphics memory (GiB) a GPU launch keeps free on top of the engine's own
/// estimate (`--max-vram -2` on a 12 GB+ card, see [`vram_reserves`]). sd.cpp budgets weights + working memory + 0.5 GiB
/// (backend_fit.cpp, model_manager.cpp `check_capacity`) and runs the whole
/// model in one piece when that fits the free memory it measured. On Windows /
/// CUDA the real use was ~0.85 GiB higher (a Krea 2 edit on a 16 GB card: 12.5 GB
/// of weights staged, then 1.6 GB free for a 1.9 GB workspace), and the job
/// failed with every weight held for that one piece, so nothing could be
/// evicted. With a budget below free memory, a model that would only just fit
/// runs in parts instead (ggml_runner.cpp segmented execution): the card caches
/// what fits and the rest streams in, like Forge's reserved inference memory.
pub(crate) const VRAM_RESERVE_GIB: u8 = 2;

/// The reserve after running out of memory anyway on a 16 GB+ card
/// ([`MORE_ROOM_RETRY_NOTE`]).
pub(crate) const MORE_ROOM_RESERVE_GIB: u8 = 4;

/// (reserve at launch, reserve for the "more room" retry) in GiB for a card
/// with `vram_gb` (0 = unknown). Smaller cards keep less free: the reserve
/// comes out of what auto-fit may keep on the card (free − reserve instead of
/// free − 0.5 GiB). The retry stays at a quarter of the card at most, because
/// sd.cpp treats a reserve at or above the free memory as no limit at all
/// (ggml_graph_cut.cpp `resolve_auto_max_vram_bytes`). Below 8 GB the launch
/// keeps the engine's own default. The buckets start half a GB below the
/// nominal size, because cards report a little less (16303 MiB → 15.9 GB).
pub(crate) fn vram_reserves(vram_gb: f32) -> (u8, u8) {
    if !(vram_gb.is_finite() && vram_gb > 0.0) {
        (1, 2)
    } else if vram_gb >= 15.5 {
        (VRAM_RESERVE_GIB, MORE_ROOM_RESERVE_GIB)
    } else if vram_gb >= 11.5 {
        (VRAM_RESERVE_GIB, 3)
    } else if vram_gb >= 7.5 {
        (1, 2)
    } else if vram_gb >= 4.0 {
        (0, 1)
    } else {
        (0, 0)
    }
}

const OFFLOAD_NOTE: &str = "This model is kept in system memory and sent to the graphics card as needed, because the card ran out of memory. Pictures take longer until the model is next loaded.";

/// System memory kept free when deciding whether a model fits there (OS, other apps).
const OFFLOAD_SPARE_RAM_GB: f64 = 2.0;

const TE_ON_CPU_NOTE: &str = "Your prompt is read on the processor for this model because the graphics card ran out of memory earlier. You can change this in Settings → Engine (“Read the prompt on the processor”).";

const TILING_ON_NOTE: &str = "This model finishes pictures in smaller pieces to save memory, because it ran out of memory earlier (a bit slower). You can turn this off in Fine-tune.";

/// Memory-saving launch choices made automatically for one model (RAM only,
/// kept for the rest of the app session).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MemFallback {
    /// `--backend te=cpu`: the text encoder runs on the processor.
    pub te_on_cpu: bool,
    /// `--vae-tiling`
    pub vae_tiling: bool,
    /// `--offload-to-cpu`: every weight lives in system memory (mapped from the
    /// file, `--mmap`) and the card only caches what fits, so the engine can
    /// make room for its working memory. Never remembered for the session: it
    /// only sticks while the engine that needed it stays loaded.
    pub offload: bool,
    /// `--max-vram -N`: graphics memory (GiB) the engine keeps free beyond its
    /// own estimate. 0 = don't pass it (CPU engine, small card). In the
    /// remembered choices: only set when a retry raised it.
    pub vram_reserve_gib: u8,
    /// The reserve a "more room" retry raises it to; 0 = no such retry.
    /// See [`vram_reserves`].
    pub more_room_gib: u8,
}

/// Settings `textEncoderOnCpu`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TeChoice {
    Auto,
    On,
    Off,
}

impl TeChoice {
    pub(crate) fn current(core: &AppCore) -> Self {
        match core.settings.read().text_encoder_on_cpu.as_str() {
            "on" => TeChoice::On,
            "off" => TeChoice::Off,
            _ => TeChoice::Auto,
        }
    }
}

/// sd.cpp module names for the text encoder (`parse_backend_module`,
/// src/core/ggml_extend_backend.cpp; case-insensitive, `-`/`_` ignored).
const TE_MODULES: &[&str] = &[
    "te",
    "clip",
    "text",
    "textencoder",
    "textencoders",
    "conditioner",
    "cond",
    "llm",
    "t5",
    "t5xxl",
];

fn is_te_module(key: &str) -> bool {
    let k: String = key
        .trim()
        .chars()
        .filter(|c| *c != '-' && *c != '_')
        .collect::<String>()
        .to_ascii_lowercase();
    TE_MODULES.contains(&k.as_str())
}

/// Does sd-server run the text encoder on the CPU with these args?
/// (`--backend` entries in order, a later one wins; `--clip-on-cpu` is
/// prepended by sd.cpp so any explicit entry overrides it.)
pub(crate) fn text_encoder_on_cpu(args: &[String]) -> bool {
    let (mut default, mut te) = (None::<String>, None::<String>);
    for w in args.windows(2).filter(|w| w[0] == "--backend") {
        for part in w[1].split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part.split_once('=') {
                None => default = Some(part.to_string()),
                Some((k, v))
                    if matches!(
                        k.trim().to_ascii_lowercase().as_str(),
                        "all" | "default" | "*"
                    ) =>
                {
                    default = Some(v.trim().to_string())
                }
                Some((k, v)) if is_te_module(k) => te = Some(v.trim().to_string()),
                Some(_) => {}
            }
        }
    }
    match te {
        Some(b) => b.eq_ignore_ascii_case("cpu"),
        None => {
            args.iter().any(|a| a == "--clip-on-cpu")
                || default.is_some_and(|b| b.eq_ignore_ascii_case("cpu"))
        }
    }
}

/// Run the text encoder on the processor: every `--backend` list is merged
/// into one, other text-encoder entries dropped and `te=cpu` added last (sd.cpp
/// joins repeated `--backend` values with `,`; a later module entry wins).
pub(crate) fn with_text_encoder_on_cpu(args: &mut Vec<String>) {
    let mut parts: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(args.len() + 2);
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--backend" && i + 1 < args.len() {
            let keep = args[i + 1].split(',').map(str::trim).filter(|p| {
                !p.is_empty() && !p.split_once('=').is_some_and(|(k, _)| is_te_module(k))
            });
            parts.extend(keep.map(String::from));
            i += 2;
        } else {
            out.push(args[i].clone());
            i += 1;
        }
    }
    parts.push("te=cpu".into());
    out.extend(["--backend".into(), parts.join(",")]);
    *args = out;
}

/// This model's memory choices: Settings `textEncoderOnCpu` on / off, or (auto)
/// what an out-of-memory retry chose earlier this session. GPU backends only.
pub(crate) fn memory_choices(
    core: &AppCore,
    model_id: &str,
    gpu_backend: bool,
    vram_gb: f32,
) -> MemFallback {
    let mut fb = core
        .gen
        .mem_fallback
        .lock()
        .get(model_id)
        .copied()
        .unwrap_or_default();
    fb.te_on_cpu = gpu_backend
        && match TeChoice::current(core) {
            TeChoice::On => true,
            TeChoice::Off => false,
            TeChoice::Auto => fb.te_on_cpu,
        };
    let (reserve, more_room) = if gpu_backend {
        vram_reserves(vram_gb)
    } else {
        (0, 0)
    };
    fb.vram_reserve_gib = if gpu_backend {
        fb.vram_reserve_gib.max(reserve)
    } else {
        0
    };
    fb.more_room_gib = more_room;
    fb
}

/// Remember the automatic choices `learned` by a retry that then succeeded
/// for `model_id` (RAM only, app session).
pub(crate) fn remember_memory_choices(core: &AppCore, model_id: &str, learned: MemFallback) {
    if learned == MemFallback::default() {
        return;
    }
    let mut remembered = core.gen.mem_fallback.lock();
    let entry = remembered.entry(model_id.to_string()).or_default();
    entry.te_on_cpu |= learned.te_on_cpu;
    entry.vae_tiling |= learned.vae_tiling;
    entry.vram_reserve_gib = entry.vram_reserve_gib.max(learned.vram_reserve_gib);
}

/// Weights in system memory stick while this model runs with the same
/// settings (see `GenState::offloaded`); anything else tries the card again.
pub(crate) fn with_remembered_offload(
    core: &AppCore,
    model_id: &str,
    wiring_args: &[String],
    fb: MemFallback,
    gpu_backend: bool,
) -> MemFallback {
    let offload = gpu_backend && {
        let with_offload = with_memory_choices(
            wiring_args,
            MemFallback {
                offload: true,
                ..fb
            },
        );
        core.gen
            .offloaded
            .lock()
            .as_ref()
            .is_some_and(|(id, a)| id == model_id && *a == with_offload)
    };
    MemFallback { offload, ..fb }
}

/// Wiring args with the memory choices applied.
pub(crate) fn with_memory_choices(wiring_args: &[String], fb: MemFallback) -> Vec<String> {
    let mut args = wiring_args.to_vec();
    if fb.te_on_cpu && !text_encoder_on_cpu(&args) {
        with_text_encoder_on_cpu(&mut args);
    }
    if fb.vae_tiling && !args.iter().any(|a| a == "--vae-tiling") {
        args.push("--vae-tiling".into());
    }
    if fb.offload && !args.iter().any(|a| a == "--offload-to-cpu") {
        args.push("--offload-to-cpu".into());
    }
    // A --max-vram from the registry (family / hardware profile) wins.
    if fb.vram_reserve_gib > 0 && !has_max_vram(&args) {
        args.extend(["--max-vram".into(), format!("-{}", fb.vram_reserve_gib)]);
    }
    args
}

fn has_max_vram(args: &[String]) -> bool {
    args.iter()
        .any(|a| a == "--max-vram" || a.starts_with("--max-vram="))
}

/// The value of the last `flag value` / `flag=value` in `args`.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let eq = format!("{flag}=");
    let mut found = None;
    for (i, a) in args.iter().enumerate() {
        if a == flag {
            found = args.get(i + 1).cloned();
        } else if let Some(v) = a.strip_prefix(&eq) {
            found = Some(v.to_string());
        }
    }
    found
}

/// Weight files these launch args load (main model + components), in GiB.
fn weights_gb(args: &[String]) -> f64 {
    let bytes: u64 = args
        .windows(2)
        .filter(|w| pinhole_registry::wiring::WEIGHT_FILE_FLAGS.contains(&w[0].as_str()))
        .filter_map(|w| std::fs::metadata(&w[1]).ok())
        .map(|m| m.len())
        .sum();
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}

/// Every weight fits in system memory with room to spare (unknown RAM: assume so).
pub(crate) fn offload_fits_ram(args: &[String], ram_gb: f32) -> bool {
    !(ram_gb.is_finite() && ram_gb > 0.0)
        || weights_gb(args) + OFFLOAD_SPARE_RAM_GB <= f64::from(ram_gb)
}

/// The next retry after running out of memory at `stage`, if any. Each
/// choice is made at most once, so a job is retried at most four times:
/// * reading the prompt → text encoder on the processor (Settings Automatic,
///   GPU backend, not there yet); with Settings Off → more room, then weights
///   to system memory;
/// * decoding (VAE) or unknown → VAE tiling (not on yet and allowed), then
///   more room, then weights to system memory;
/// * denoising → more room, then weights to system memory (tiling only as a
///   last resort: it only helps the VAE).
///
/// "More room" raises the graphics memory the engine keeps free (`--max-vram`,
/// e.g. from 2 to 4 GiB on a 16 GB card, see [`vram_reserves`]), so a model
/// that almost fits runs in parts and its weights give way to working memory.
/// Weights in system memory alone don't help: sd.cpp still runs the model in
/// one piece when its estimate fits, holding every weight on the card
/// (src/core/ggml_runner.cpp), which is why the `--max-vram` budget stays on.
///
/// Weights go to system memory only on a GPU backend and when they fit there
/// (`offload_ok`, see [`offload_fits_ram`]). Returns the new choices and the note.
pub(crate) fn next_memory_fallback(
    fb: MemFallback,
    stage: Stage,
    te: TeChoice,
    gpu_backend: bool,
    args: &[String],
    tiling_allowed: bool,
    offload_ok: bool,
) -> Option<(MemFallback, &'static str)> {
    let tiling = (tiling_allowed && !args.iter().any(|a| a == "--vae-tiling")).then_some((
        MemFallback {
            vae_tiling: true,
            ..fb
        },
        TILING_RETRY_NOTE,
    ));
    let offload = (gpu_backend && offload_ok && !args.iter().any(|a| a == "--offload-to-cpu"))
        .then_some((
            MemFallback {
                offload: true,
                ..fb
            },
            OFFLOAD_RETRY_NOTE,
        ));
    // Only when the budget in `args` is Pinhole's own (not a registry --max-vram).
    let own_reserve = flag_value(args, "--max-vram")
        == (fb.vram_reserve_gib > 0).then(|| format!("-{}", fb.vram_reserve_gib));
    let more_room = (gpu_backend && fb.vram_reserve_gib < fb.more_room_gib && own_reserve)
        .then_some((
            MemFallback {
                vram_reserve_gib: fb.more_room_gib,
                ..fb
            },
            MORE_ROOM_RETRY_NOTE,
        ))
        .or(offload);
    match stage {
        // No GPU, or the text encoder already on the processor: it's system memory.
        Stage::TextEncoder if !gpu_backend || text_encoder_on_cpu(args) => None,
        Stage::TextEncoder if te == TeChoice::Auto => Some((
            MemFallback {
                te_on_cpu: true,
                ..fb
            },
            TE_RETRY_NOTE,
        )),
        Stage::TextEncoder => more_room,
        Stage::Diffusion if gpu_backend && offload_ok => more_room,
        // Tiling as a last resort: the stage is read from the engine output.
        Stage::Diffusion => more_room.or(tiling),
        Stage::Vae | Stage::Unknown => tiling.or(more_room),
    }
}

/// `9 GB`, `8.9 GB` (MiB in, GiB out like the rest of the UI).
fn gb_text(mib: u64) -> String {
    let gb = (mib as f64 / 1024.0 * 10.0).round() / 10.0;
    if gb.fract() == 0.0 {
        format!("{gb:.0} GB")
    } else {
        format!("{gb:.1} GB")
    }
}

/// "Other programs are using 9 GB of your graphics memory: python.exe (8.9 GB)."
pub(crate) fn others_sentence(o: &OtherGpuUse) -> String {
    let mut names: Vec<(String, Option<u64>)> = Vec::new();
    for p in &o.processes {
        match names
            .iter_mut()
            .find(|(n, _)| n.eq_ignore_ascii_case(&p.name))
        {
            Some((_, used)) => {
                *used = match (*used, p.used_mib) {
                    (Some(a), Some(b)) => Some(a + b),
                    (a, b) => a.or(b),
                }
            }
            None => names.push((p.name.clone(), p.used_mib)),
        }
    }
    let listed: Vec<String> = names
        .iter()
        .take(3)
        .map(|(n, used)| match used {
            Some(m) if *m >= 100 => format!("{n} ({})", gb_text(*m)),
            _ => n.clone(),
        })
        .collect();
    let amount = gb_text(o.others_mib);
    if listed.is_empty() {
        format!("Other programs are using {amount} of your graphics memory.")
    } else {
        format!(
            "Other programs are using {amount} of your graphics memory: {}.",
            listed.join(", ")
        )
    }
}

/// How [`others_sentence`] starts (to replace an older note).
const OTHERS_PREFIX: &str = "Other programs are using ";

/// Shown while loading when other programs hold a lot of graphics memory.
pub(crate) fn others_note(o: &OtherGpuUse) -> String {
    format!(
        "{} If pictures fail, close them and try again.",
        others_sentence(o)
    )
}

/// Out of graphics memory: names other programs when the engine start saw them.
pub(crate) fn vram_message(core: &AppCore) -> String {
    match core.gen.gpu_others.lock().clone().filter(OtherGpuUse::is_significant) {
        Some(o) => format!(
            "Your graphics card ran out of memory. {} Close them and try again, or pick the smaller version of this model in Models.",
            others_sentence(&o)
        ),
        None => VRAM_MESSAGE.to_string(),
    }
}

/// Engine output of an out-of-memory job, after the memory plan this model's
/// last auto-fit launch printed (when it isn't in the output already).
pub(crate) fn with_memory_plan(
    core: &AppCore,
    model_id: &str,
    args: &[String],
    details: String,
) -> String {
    let plan = match core.gen.memory_plan.lock().as_ref() {
        Some((id, plan)) if id == model_id => plan.clone(),
        _ => return details,
    };
    if plan.iter().all(|l| details.contains(l.as_str())) {
        return details;
    }
    // An offloaded engine doesn't run auto-fit: the plan is the earlier attempt's.
    let title = if args.iter().any(|a| a == "--offload-to-cpu") {
        "Memory plan of the earlier attempt (before the weights moved to system memory):"
    } else {
        "Memory plan when the engine started:"
    };
    format!("{title}\n{}\n\n{details}", plan.join("\n"))
}

/// The final out-of-memory error for a job (after any retry).
pub(crate) fn memory_error(
    core: &AppCore,
    stage: Stage,
    args: &[String],
    gpu_backend: bool,
) -> CoreError {
    let msg = if !gpu_backend || (stage == Stage::TextEncoder && text_encoder_on_cpu(args)) {
        RAM_MESSAGE.to_string()
    } else if stage == Stage::TextEncoder && TeChoice::current(core) == TeChoice::Off {
        TE_ON_GPU_MESSAGE.to_string()
    } else {
        vram_message(core)
    };
    CoreError::new("vram", msg)
}

/// `EngineStatus.note` for the running engine.
pub(crate) fn engine_note(core: &AppCore, flags: &EngineFlags) -> Option<String> {
    if !flags.running {
        return None;
    }
    let mut notes: Vec<String> = core.gen.not_on_gpu.lock().iter().cloned().collect();
    let fb = flags
        .loaded_model_id
        .as_ref()
        .and_then(|id| core.gen.mem_fallback.lock().get(id).copied())
        .unwrap_or_default();
    if fb.te_on_cpu && TeChoice::current(core) == TeChoice::Auto {
        notes.push(TE_ON_CPU_NOTE.to_string());
    }
    if fb.vae_tiling {
        notes.push(TILING_ON_NOTE.to_string());
    }
    if fb.vram_reserve_gib > 0 && core.gen.offloaded.lock().is_none() {
        notes.push(MORE_ROOM_NOTE.to_string());
    }
    if core.gen.offloaded.lock().is_some() {
        notes.push(OFFLOAD_NOTE.to_string());
    }
    if let Some(o) = core
        .gen
        .gpu_others
        .lock()
        .clone()
        .filter(OtherGpuUse::is_significant)
    {
        notes.push(others_sentence(&o));
    }
    (!notes.is_empty()).then(|| notes.join(" "))
}

/// Make sure sd-server runs with exactly `wiring_args` (restart otherwise) and
/// is ready. Returns its base URL.
///
/// Before a launch: the old engine has fully exited (`stop` waits), leftover
/// engines under `Data/engine/` are killed, an idle describe engine is
/// stopped (GPU backends) and graphics memory used by other programs is
/// measured (NVIDIA) — a lot of it becomes a progress note.
/// Remember a launch with the weights in system memory (see `GenState::offloaded`).
pub(crate) fn note_offload(core: &AppCore, model_id: &str, wiring_args: &[String]) {
    *core.gen.offloaded.lock() = wiring_args
        .iter()
        .any(|a| a == "--offload-to-cpu")
        .then(|| (model_id.to_string(), wiring_args.to_vec()));
}

/// Note about other programs' graphics memory, measured at each engine start
/// (replaces the one from an earlier start of this job; shown first).
pub(crate) fn set_others_note(core: &AppCore, note: Option<String>) {
    let mut n = core.gen.job_note.lock();
    n.retain(|x| !x.starts_with(OTHERS_PREFIX));
    if let Some(note) = note {
        n.insert(0, note);
    }
}

/// Show `note` for the automatic retry that is starting (replaces the note of
/// an earlier retry of the same job; other notes stay).
pub(crate) fn set_retry_note(core: &AppCore, note: &str) {
    let mut n = core.gen.job_note.lock();
    n.retain(|x| !RETRY_NOTES.contains(&x.as_str()));
    n.push(note.to_string());
}

#[cfg(test)]
mod tests {
    use pinhole_engine::failure::memory_failure;

    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn text_encoder_moves_to_the_processor_in_backend_lists() {
        let mut a = v(&["--diffusion-model", "/d", "--diffusion-fa"]);
        assert!(!text_encoder_on_cpu(&a));
        with_text_encoder_on_cpu(&mut a);
        assert_eq!(
            a,
            v(&[
                "--diffusion-model",
                "/d",
                "--diffusion-fa",
                "--backend",
                "te=cpu"
            ])
        );
        assert!(text_encoder_on_cpu(&a));

        // Existing lists are merged; other text-encoder entries (any alias) dropped.
        let mut a = v(&[
            "--backend",
            "diffusion=cuda0,CLIP=cuda0,vae=cpu",
            "--vae",
            "/v",
            "--backend",
            "t5_xxl=cuda0",
        ]);
        with_text_encoder_on_cpu(&mut a);
        assert_eq!(
            a,
            v(&["--vae", "/v", "--backend", "diffusion=cuda0,vae=cpu,te=cpu"])
        );
        let mut a = v(&["--backend", "cuda0"]);
        with_text_encoder_on_cpu(&mut a);
        assert_eq!(a, v(&["--backend", "cuda0,te=cpu"]));

        // sd.cpp semantics: a later entry wins; --clip-on-cpu is prepended.
        assert!(text_encoder_on_cpu(&v(&["--backend", "cpu"])));
        assert!(text_encoder_on_cpu(&v(&[
            "--backend",
            "all=cpu,diffusion=cuda0"
        ])));
        assert!(!text_encoder_on_cpu(&v(&["--backend", "cpu,te=cuda0"])));
        assert!(text_encoder_on_cpu(&v(&["--clip-on-cpu"])));
        assert!(!text_encoder_on_cpu(&v(&[
            "--clip-on-cpu",
            "--backend",
            "llm=cuda0"
        ])));
        assert!(!text_encoder_on_cpu(&v(&["--backend", "vae=cpu"])));
    }

    #[test]
    fn one_memory_retry_per_stage() {
        let gpu = v(&["--diffusion-model", "/d"]);
        let none = MemFallback::default();
        let next = |fb, stage, te, args: &[String]| {
            next_memory_fallback(fb, stage, te, true, args, true, true)
        };
        // Reading the prompt: text encoder to the processor, and nothing after that.
        let (fb, note) = next(none, Stage::TextEncoder, TeChoice::Auto, &gpu).unwrap();
        assert_eq!(
            (fb, note),
            (
                MemFallback {
                    te_on_cpu: true,
                    ..none
                },
                TE_RETRY_NOTE
            )
        );
        let args = with_memory_choices(&gpu, fb);
        assert!(text_encoder_on_cpu(&args), "{args:?}");
        assert!(next(fb, Stage::TextEncoder, TeChoice::Auto, &args).is_none());
        // Settings keeps it on the card: the weights go to system memory instead.
        let (fb, note) = next(none, Stage::TextEncoder, TeChoice::Off, &gpu).unwrap();
        assert_eq!(
            (fb, note),
            (
                MemFallback {
                    offload: true,
                    ..none
                },
                OFFLOAD_RETRY_NOTE
            )
        );
        // Denoising: straight to system memory (tiling only helps the VAE), once.
        let (fb, note) = next(none, Stage::Diffusion, TeChoice::Auto, &gpu).unwrap();
        assert_eq!(
            (fb, note),
            (
                MemFallback {
                    offload: true,
                    ..none
                },
                OFFLOAD_RETRY_NOTE
            )
        );
        let args = with_memory_choices(&gpu, fb);
        assert_eq!(args.iter().filter(|a| *a == "--offload-to-cpu").count(), 1);
        assert!(next(fb, Stage::Diffusion, TeChoice::Auto, &args).is_none());
        // Decoding / unknown: VAE tiling once, then system memory once.
        for stage in [Stage::Vae, Stage::Unknown] {
            let (fb, note) = next(none, stage, TeChoice::Auto, &gpu).unwrap();
            assert_eq!(
                (fb.vae_tiling, fb.offload, note),
                (true, false, TILING_RETRY_NOTE)
            );
            let args = with_memory_choices(&gpu, fb);
            assert_eq!(args.iter().filter(|a| *a == "--vae-tiling").count(), 1);
            let (fb, note) = next(fb, stage, TeChoice::Auto, &args).unwrap();
            assert_eq!(
                (fb.vae_tiling, fb.offload, note),
                (true, true, OFFLOAD_RETRY_NOTE)
            );
            assert!(next(fb, stage, TeChoice::Auto, &with_memory_choices(&gpu, fb)).is_none());
            // Fine-tune turned tiling off: system memory right away.
            let (fb, _) =
                next_memory_fallback(none, stage, TeChoice::Auto, true, &gpu, false, true).unwrap();
            assert_eq!((fb.vae_tiling, fb.offload), (false, true));
        }
        // Not enough system memory for every weight: no offload, tiling as a last resort.
        let (fb, _) = next_memory_fallback(
            none,
            Stage::Diffusion,
            TeChoice::Auto,
            true,
            &gpu,
            true,
            false,
        )
        .unwrap();
        assert_eq!((fb.vae_tiling, fb.offload), (true, false));
        assert!(next_memory_fallback(
            fb,
            Stage::Diffusion,
            TeChoice::Auto,
            true,
            &with_memory_choices(&gpu, fb),
            true,
            false
        )
        .is_none());
        // No GPU: VAE tiling only, never for the prompt.
        assert!(next_memory_fallback(
            none,
            Stage::TextEncoder,
            TeChoice::Auto,
            false,
            &gpu,
            true,
            true
        )
        .is_none());
        let (fb, _) = next_memory_fallback(
            none,
            Stage::Diffusion,
            TeChoice::Auto,
            false,
            &gpu,
            true,
            true,
        )
        .unwrap();
        assert_eq!((fb.vae_tiling, fb.offload), (true, false));
        let (fb, _) =
            next_memory_fallback(none, Stage::Vae, TeChoice::Auto, false, &gpu, true, true)
                .unwrap();
        assert_eq!((fb.vae_tiling, fb.offload), (true, false));
    }

    /// Field report (Windows, RTX 5070 Ti 16 GB, Krea 2 edit at 864×1536): the
    /// weights were already in system memory, yet sd.cpp staged all 12.5 GB of
    /// them for a one-piece run and had 1.6 GB left for a 1.9 GB workspace. Every
    /// GPU launch keeps 2 GB free (`--max-vram -2`), and the retry keeps 4 GB
    /// free so the model runs in parts, instead of repeating the same launch.
    #[test]
    fn krea2_edit_workspace_failure_retries_with_more_room() {
        let log = "[WARN   ] model_manager.cpp:1753 - model manager memory on CUDA0: reported free 1644.26 MB / total 16275.44 MB, tracked weights 12535.73 MB / other runtime 0.00 MB / current runtime 0.00 MB\n\
                   [WARN   ] model_manager.cpp:1919 - model manager cannot make enough memory available on CUDA0: need 1861.73 MB device / 1349.73 MB budget, available 1644.26 MB device / unlimited budget\n\
                   [ERROR  ] ggml_runner.cpp:899  - krea2 segment 1/1 (graph) failed during workspace capacity check\n\
                   [ERROR  ] diffusion_engine.cpp:2594 - diffusion model compute failed\n\
                   [ERROR  ] diffusion_engine.cpp:2733 - Diffusion model sampling failed\n\
                   [ERROR  ] image.cpp:907  - sampling for image 1/1 failed after 3.73s";
        let stage = memory_failure(log).expect("out of memory");
        assert_eq!(stage, Stage::Diffusion);
        let wiring = v(&["--diffusion-model", "/krea2.safetensors"]);
        let (reserve, more_room) = vram_reserves(16.0);
        let fb = MemFallback {
            vram_reserve_gib: reserve,
            more_room_gib: more_room,
            ..Default::default()
        };
        let args = with_memory_choices(&wiring, fb);
        assert_eq!(flag_value(&args, "--max-vram").as_deref(), Some("-2"));
        let next = |fb, args: &[String]| {
            next_memory_fallback(fb, stage, TeChoice::Auto, true, args, true, true)
        };
        let (fb, note) = next(fb, &args).unwrap();
        assert_eq!(
            (fb.vram_reserve_gib, fb.offload, note),
            (MORE_ROOM_RESERVE_GIB, false, MORE_ROOM_RETRY_NOTE)
        );
        let args = with_memory_choices(&wiring, fb);
        assert_eq!(flag_value(&args, "--max-vram").as_deref(), Some("-4"));
        assert_eq!(args.iter().filter(|a| *a == "--max-vram").count(), 1);
        // Then system memory, keeping the 4 GB budget; then nothing is left.
        let (fb, note) = next(fb, &args).unwrap();
        assert_eq!(
            (fb.vram_reserve_gib, fb.offload, note),
            (MORE_ROOM_RESERVE_GIB, true, OFFLOAD_RETRY_NOTE)
        );
        let args = with_memory_choices(&wiring, fb);
        assert_eq!(flag_value(&args, "--max-vram").as_deref(), Some("-4"));
        assert!(next(fb, &args).is_none());

        // A --max-vram from the registry wins and isn't raised.
        let pinned = v(&["--diffusion-model", "/d", "--max-vram", "6"]);
        let (reserve, more_room) = vram_reserves(16.0);
        let fb = MemFallback {
            vram_reserve_gib: reserve,
            more_room_gib: more_room,
            ..Default::default()
        };
        let args = with_memory_choices(&pinned, fb);
        assert_eq!(flag_value(&args, "--max-vram").as_deref(), Some("6"));
        let (fb, note) = next(fb, &args).unwrap();
        assert_eq!((fb.offload, note), (true, OFFLOAD_RETRY_NOTE));
        // CPU engine: no budget at all.
        assert!(!with_memory_choices(&wiring, MemFallback::default())
            .iter()
            .any(|a| a == "--max-vram"));
    }

    /// The reserve scales with the card and the retry stays at a quarter of it
    /// (sd.cpp treats a reserve at or above the free memory as no limit).
    #[test]
    fn vram_reserve_scales_with_the_card() {
        assert_eq!(vram_reserves(16.0), (2, 4));
        assert_eq!(vram_reserves(15.9), (2, 4), "16 GB card as reported");
        assert_eq!(vram_reserves(15.6), (2, 4));
        assert_eq!(vram_reserves(12.0), (2, 3));
        assert_eq!(vram_reserves(11.9), (2, 3));
        assert_eq!(vram_reserves(8.0), (1, 2));
        assert_eq!(vram_reserves(7.9), (1, 2));
        assert_eq!(vram_reserves(6.0), (0, 1));
        assert_eq!(vram_reserves(4.0), (0, 1));
        assert_eq!(vram_reserves(2.0), (0, 0));
        assert_eq!(vram_reserves(0.0), (1, 2), "unknown card");
        for gb in [2.0_f32, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0] {
            let (r, m) = vram_reserves(gb);
            assert!(r <= m && f32::from(m) <= gb / 4.0, "{gb}");
        }
        // A 4 GB card launches without a budget; the retry adds one.
        let wiring = v(&["--diffusion-model", "/d"]);
        let fb = MemFallback {
            more_room_gib: 1,
            ..Default::default()
        };
        let args = with_memory_choices(&wiring, fb);
        assert!(!args.iter().any(|a| a == "--max-vram"));
        let (fb, note) = next_memory_fallback(
            fb,
            Stage::Diffusion,
            TeChoice::Auto,
            true,
            &args,
            true,
            true,
        )
        .unwrap();
        assert_eq!((fb.vram_reserve_gib, note), (1, MORE_ROOM_RETRY_NOTE));
        let args = with_memory_choices(&wiring, fb);
        assert_eq!(flag_value(&args, "--max-vram").as_deref(), Some("-1"));
    }

    #[test]
    fn offload_needs_room_in_system_memory() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("m.gguf");
        let t = dir.path().join("t.safetensors");
        std::fs::write(&f, vec![0u8; 3 << 20]).unwrap();
        std::fs::write(&t, vec![0u8; 1 << 20]).unwrap();
        let args = v(&[
            "--diffusion-model",
            f.to_str().unwrap(),
            "--t5xxl",
            t.to_str().unwrap(),
            "--listen-ip",
            "127.0.0.1",
        ]);
        assert!((weights_gb(&args) - 4.0 / 1024.0).abs() < 1e-9);
        assert!(offload_fits_ram(&args, 2.01));
        assert!(
            !offload_fits_ram(&args, 2.0),
            "4 MB of weights + 2 GB spare > 2 GB"
        );
        assert!(offload_fits_ram(&args, 0.0), "unknown RAM");
    }

    #[test]
    fn other_programs_are_named_with_their_memory() {
        let p = |pid, name: &str, mib| pinhole_hardware::GpuProcess {
            pid,
            name: name.into(),
            used_mib: mib,
        };
        let o = OtherGpuUse {
            gpu_index: 0,
            total_mib: 16275,
            others_mib: 9216,
            processes: vec![p(7, "python.exe", Some(9114))],
        };
        assert_eq!(
            others_sentence(&o),
            "Other programs are using 9 GB of your graphics memory: python.exe (8.9 GB)."
        );
        // WDDM: no per-process numbers; repeated names are listed once; at most three.
        let o = OtherGpuUse {
            others_mib: 10854,
            processes: vec![
                p(1, "python.exe", None),
                p(2, "python.exe", None),
                p(3, "obs64.exe", None),
                p(4, "a.exe", None),
                p(5, "b.exe", None),
            ],
            ..o
        };
        assert_eq!(others_sentence(&o), "Other programs are using 10.6 GB of your graphics memory: python.exe, obs64.exe, a.exe.");
        let o = OtherGpuUse {
            processes: vec![],
            ..o
        };
        assert_eq!(
            others_sentence(&o),
            "Other programs are using 10.6 GB of your graphics memory."
        );
        assert!(others_note(&o).ends_with("If pictures fail, close them and try again."));
    }

    /// The UI shows "Open Models" on a `vram` error only when its message points there
    /// (src/components/ErrorWithFix.tsx matches "in Models").
    #[test]
    fn vram_messages_point_to_models_except_the_settings_one() {
        for m in [VRAM_MESSAGE, RAM_MESSAGE] {
            assert!(m.contains("in Models"), "{m}");
        }
        assert!(!TE_ON_GPU_MESSAGE.contains("in Models"));
        // `vram_message` with other programs named: see tests/memory_tests.rs `out_of_memory_is_never_the_generic_message`.
    }
}
