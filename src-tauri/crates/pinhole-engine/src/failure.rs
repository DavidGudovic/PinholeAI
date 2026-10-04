//! Classify engine failures from the (already redacted) ring-buffer tail and
//! the exit status, so the core can say what to do next instead of dumping
//! engine output.

/// What went wrong, in terms the UI can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// GPU (or system) memory ran out.
    OutOfMemory,
    /// NVIDIA driver too old for the CUDA build.
    DriverTooOld,
    /// No usable GPU for this backend (CUDA init failed, no Vulkan device).
    NoGpu,
    /// Linux: the engine needs a newer glibc (Ubuntu 24.04+).
    GlibcTooOld,
    /// A shared library / DLL is missing (Vulkan loader, VC++ runtime…).
    MissingLibrary,
    /// The model (or a component) could not be loaded — damaged / unsupported file.
    ModelLoad,
    /// Anything else.
    Unknown,
}

const OOM: &[&str] = &[
    "out of memory",
    "outofdevicememory",
    "out_of_device_memory",
    "outofhostmemory",
    "cudamalloc failed",
    "cudaerrormemoryallocation",
    "cublas_status_alloc_failed",
    "failed to allocate",
    "allocation of size",
    "alloc params backend buffer failed",
    "alloc compute params backend buffer failed",
    "std::bad_alloc",
    "not enough memory",
    "insufficient memory",
    // sd.cpp model manager / graph runner (auto-fit budgets, src/model_manager.cpp,
    // src/core/ggml_runner.cpp): the runner could not get room for its workspace
    // (these segment phases all set GGML_STATUS_ALLOC_FAILED).
    "cannot make enough memory available",
    "failed during workspace preparation",
    "failed during workspace capacity check",
    "failed during allocated capacity check",
    "failed during workspace allocation",
];
/// The prompt encoder failed. On its own this is NOT a memory failure (a broken
/// or mismatched text encoder fails the same way); with a memory line in the
/// same output it says the text encoder is the stage that ran out.
const ENCODE_FAILED: &[&str] = &[
    "prompt encoding failed",
    "failed to encode prompt",
    "failed to encode negative prompt",
    "failed to encode image guidance prompt",
];
/// Lines that say which stage of `generate_image` failed (src/pipeline/image.cpp).
const STAGE_DIFFUSION: &[&str] = &[
    "sampling for image",
    "sampling failed",
    "no latent images generated",
];
const STAGE_VAE: &[&str] = &[
    "decode_first_stage failed",
    "encode_first_stage failed",
    "failed to encode init image",
    "failed to encode masked init image",
    "failed to encode reference image",
    "failed to encode control image",
    "no decoded images",
];
const DRIVER: &[&str] = &[
    "driver version is insufficient",
    "cuda driver version",
    "unsupported ptx version",
    "no kernel image is available",
];
const NO_GPU: &[&str] = &[
    "no cuda-capable device",
    "failed to initialize cuda",
    "ggml_cuda_init: failed",
    "no vulkan devices",
    "ggml_vulkan: no devices",
    "vkcreateinstance failed",
    "errorincompatibledriver",
    "error_incompatible_driver",
    "no devices found",
];
const GLIBC: &[&str] = &[
    "glibc_2.",
    "glibcxx_3.",
    "version `glibc",
    "version `glibcxx",
];
const MISSING_LIB: &[&str] = &[
    "error while loading shared libraries",
    "cannot open shared object file",
    ".dll was not found",
    "could not load library",
];
const MODEL: &[&str] = &[
    "new_sd_ctx_t failed",
    "load tensors from model loader failed",
    "get sd version from file failed",
    "init model loader from file failed",
    "unknown model",
    "invalid safetensor",
    "failed to load model",
    "error loading model",
    "unsupported model",
];

/// Windows `STATUS_DLL_NOT_FOUND` (0xC0000135) as an i32 exit code.
const STATUS_DLL_NOT_FOUND: i32 = 0xC000_0135_u32 as i32;
/// Windows `STATUS_ENTRYPOINT_NOT_FOUND` (0xC0000139).
const STATUS_ENTRYPOINT_NOT_FOUND: i32 = 0xC000_0139_u32 as i32;

/// Classify from log lines (redacted) and an optional exit code.
pub fn classify(log_tail: &str, exit_code: Option<i32>) -> Failure {
    let l = log_tail.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| l.contains(n));
    if has(GLIBC) {
        return Failure::GlibcTooOld;
    }
    if has(DRIVER) {
        return Failure::DriverTooOld;
    }
    if has(OOM) {
        return Failure::OutOfMemory;
    }
    if has(MISSING_LIB)
        || matches!(
            exit_code,
            Some(STATUS_DLL_NOT_FOUND) | Some(STATUS_ENTRYPOINT_NOT_FOUND)
        )
    {
        return Failure::MissingLibrary;
    }
    if has(NO_GPU) {
        return Failure::NoGpu;
    }
    if has(MODEL) {
        return Failure::ModelLoad;
    }
    Failure::Unknown
}

/// Which part of a generation failed (for the automatic memory fallbacks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Reading the prompt (text encoder / conditioner).
    TextEncoder,
    /// Denoising (the diffusion model).
    Diffusion,
    /// VAE encode / decode.
    Vae,
    /// The output doesn't say.
    Unknown,
}

/// The stage named by the last stage line in `log` (redacted engine output of
/// one job, oldest first).
pub fn failed_stage(log: &str) -> Stage {
    for line in log.lines().rev() {
        let l = line.to_lowercase();
        let has = |needles: &[&str]| needles.iter().any(|n| l.contains(n));
        if has(ENCODE_FAILED) {
            return Stage::TextEncoder;
        }
        if has(STAGE_VAE) {
            return Stage::Vae;
        }
        if has(STAGE_DIFFUSION) {
            return Stage::Diffusion;
        }
    }
    Stage::Unknown
}

/// `Some(stage)` when the engine output of a failed job shows the graphics card
/// (or the computer) ran out of memory.
pub fn memory_failure(log: &str) -> Option<Stage> {
    (classify(log, None) == Failure::OutOfMemory).then(|| failed_stage(log))
}

/// The memory plan sd-server printed while starting (auto-fit: free VRAM and
/// RAM, and where each part's weights went), from its output lines. Needs
/// `--log-level info`; empty when auto-fit didn't run (e.g. `--offload-to-cpu`).
pub fn memory_plan(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| l.contains("backend_fit.cpp"))
        .map(|l| l.trim_end().to_string())
        .collect()
}

/// Why a GPU-backend engine ended up without the graphics card (see [`missed_gpu`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissedGpu {
    /// The card was found but had no free memory: auto-fit put everything on the processor.
    NoFreeMemory,
    /// The card wasn't found (e.g. Linux without NVIDIA's Vulkan driver): integrated
    /// graphics or the processor does the work.
    NotFound,
}

/// After a GPU-backend launch, from the auto-fit lines of its log: did the engine
/// end up without the graphics card? `expect_nvidia`: the machine has an NVIDIA
/// card, so a plan without one (or with no dedicated GPU at all) means it was
/// missed. Machines with only integrated graphics also report "no GPU devices",
/// so without `expect_nvidia` that alone isn't a problem. No auto-fit lines: `None`.
pub fn missed_gpu(lines: &[String], expect_nvidia: bool) -> Option<MissedGpu> {
    let fit: Vec<&String> = lines
        .iter()
        .filter(|l| l.contains("backend_fit.cpp"))
        .collect();
    // Device lines: "<name> <description> free N MiB, budget N MiB" (the RAM line says "params budget").
    let devices: Vec<&&String> = fit.iter().filter(|l| l.contains("MiB, budget")).collect();
    let budget_mib = |l: &str| -> Option<i64> {
        l.rsplit("budget")
            .next()?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    };
    let all_full = !devices.is_empty() && devices.iter().all(|l| budget_mib(l) == Some(0));
    if all_full
        || fit
            .iter()
            .any(|l| l.contains("no GPU memory budget available"))
    {
        return Some(MissedGpu::NoFreeMemory);
    }
    // The chosen backends: `auto-fit: --backend "diffusion=CUDA0,te=cpu,..."`, sometimes
    // followed by `--params-backend "diffusion=CPU"`. That second list only says where the
    // weights wait between uses (the model runs on the card in parts), so only the first counts.
    let diffusion_on_cpu = fit.iter().any(|l| {
        backend_spec(l).is_some_and(|spec| {
            spec.split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("diffusion=cpu"))
        })
    });
    if !expect_nvidia {
        return None;
    }
    if diffusion_on_cpu || fit.iter().any(|l| l.contains("auto-fit: no GPU devices")) {
        return Some(MissedGpu::NotFound);
    }
    // CUDA device names start with "CUDA"; not every NVIDIA card has "NVIDIA" in its name.
    let is_nvidia = |l: &str| {
        let l = l.to_ascii_lowercase();
        [
            "cuda", "nvidia", "geforce", "quadro", "tesla", "titan", "rtx ",
        ]
        .iter()
        .any(|k| l.contains(k))
    };
    (!devices.is_empty() && !devices.iter().any(|l| is_nvidia(l))).then_some(MissedGpu::NotFound)
}

/// The quoted list after `auto-fit: --backend` in an auto-fit line.
fn backend_spec(line: &str) -> Option<&str> {
    let rest = line.split_once("auto-fit: --backend \"")?.1;
    Some(rest.split_once('"').map_or(rest, |(spec, _)| spec))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_failures() {
        assert_eq!(classify("ggml_backend_cuda_buffer_type_alloc_buffer: allocating 9000 MiB on device 0: cudaMalloc failed: out of memory", None), Failure::OutOfMemory);
        assert_eq!(classify("ggml_vulkan: Device memory allocation of size 123 failed.\nvk::Device::allocateMemory: ErrorOutOfDeviceMemory", None), Failure::OutOfMemory);
        assert_eq!(
            classify(
                "./sd-server: /lib/x86_64-linux-gnu/libc.so.6: version `GLIBC_2.38' not found",
                Some(1)
            ),
            Failure::GlibcTooOld
        );
        assert_eq!(classify("sd-server: error while loading shared libraries: libvulkan.so.1: cannot open shared object file", Some(127)), Failure::MissingLibrary);
        assert_eq!(classify("", Some(-1073741515)), Failure::MissingLibrary);
        assert_eq!(
            classify(
                "CUDA error: CUDA driver version is insufficient for CUDA runtime version",
                None
            ),
            Failure::DriverTooOld
        );
        assert_eq!(
            classify(
                "ggml_cuda_init: failed to initialize CUDA: no CUDA-capable device is detected",
                None
            ),
            Failure::NoGpu
        );
        assert_eq!(
            classify("[ERROR] main.cpp:91  - new_sd_ctx_t failed", Some(1)),
            Failure::ModelLoad
        );
        assert_eq!(classify("something else", Some(1)), Failure::Unknown);
    }

    /// The real report: Windows, RTX 5070 Ti 16 GB, Z-Image Turbo bf16 + Qwen3-4B,
    /// ~9 GB of VRAM held by another program (master-929-3f8527a).
    const TE_OOM_LOG: &str = "generate_image returned no results
ggml_cuda_init: found 1 CUDA devices (Total VRAM: 16275 MiB): Device 0: NVIDIA GeForce RTX 5070 Ti, compute capability 12.0, VMM: yes
[WARN] model_manager.cpp:1753 - model manager memory on CUDA0: reported free 0.00 MB / total 16275.44 MB, tracked weights 7480.09 MB / other runtime 0.00 MB / current runtime 0.00 MB
[WARN] model_manager.cpp:1919 - model manager cannot make enough memory available on CUDA0: need 518.58 MB device / 6.58 MB budget, available 0.00 MB device / 7044.91 MB budget
[ERROR] ggml_runner.cpp:899 - qwen3 segment 1/1 (graph) failed during workspace capacity check
[ERROR] conditioner.hpp:2224 - LLM prompt encoding failed
[ERROR] image.cpp:448 - failed to encode prompt";

    #[test]
    fn text_encoder_memory_failure_from_the_field() {
        assert_eq!(classify(TE_OOM_LOG, None), Failure::OutOfMemory);
        assert_eq!(memory_failure(TE_OOM_LOG), Some(Stage::TextEncoder));
        // Each memory line on its own counts…
        for line in TE_OOM_LOG.lines().skip(3).take(2) {
            assert_eq!(classify(line, None), Failure::OutOfMemory, "{line}");
        }
        // …but "failed to encode prompt" alone is not memory (broken text encoder file).
        for line in TE_OOM_LOG.lines().skip(5) {
            assert_eq!(classify(line, None), Failure::Unknown, "{line}");
            assert_eq!(memory_failure(line), None, "{line}");
        }
        // The CUDA banner alone is not a failure.
        assert_eq!(
            classify(TE_OOM_LOG.lines().nth(1).unwrap(), None),
            Failure::Unknown
        );
        assert_eq!(memory_failure("generate_image returned no results"), None);
    }

    #[test]
    fn stages_follow_the_last_stage_line() {
        let diffusion = "[WARN] model_manager.cpp:1919 - model manager cannot make enough memory available on CUDA0: need 900.00 MB device\n\
                         [ERROR] ggml_runner.cpp:899 - z_image segment 3/8 (blocks) failed during workspace capacity check\n\
                         [ERROR] image.cpp:904 - sampling for image 1/1 failed after 3.20s";
        assert_eq!(memory_failure(diffusion), Some(Stage::Diffusion));
        let vae = "ggml_backend_cuda_buffer_type_alloc_buffer: allocating 3000 MiB on device 0: cudaMalloc failed: out of memory\n\
                   [ERROR] image.cpp:614 - decode_first_stage failed for latent 1";
        assert_eq!(memory_failure(vae), Some(Stage::Vae));
        assert_eq!(
            memory_failure("CUDA error: out of memory\n  current device: 0"),
            Some(Stage::Unknown)
        );
        // A later stage line wins over an earlier one (a retried job's output).
        assert_eq!(
            failed_stage("failed to encode prompt\nsampling for image 1/1 failed"),
            Stage::Diffusion
        );
        // Not memory: the model file is broken.
        assert_eq!(
            memory_failure("[ERROR] main.cpp:91  - new_sd_ctx_t failed"),
            None
        );
        // Upstream's padded level tags; a graph runner that couldn't prepare its workspace.
        let vae = "[ERROR  ] ggml_runner.cpp:898  - vae segment 1/1 (graph) failed during workspace preparation
                   [ERROR  ] image.cpp:614  - decode_first_stage failed for latent 1";
        assert_eq!(memory_failure(vae), Some(Stage::Vae));
        // A segment that failed for another reason isn't reported as memory by itself.
        assert_eq!(classify("[ERROR  ] ggml_runner.cpp:898  - z_image segment 2/9 (blocks) failed during input binding", None), Failure::Unknown);
    }

    #[test]
    fn memory_plan_keeps_only_the_auto_fit_lines() {
        let log: Vec<String> = [
            "[INFO ] diffusion_engine.cpp:732  - loading diffusion model from 'C:/Pinhole/Data/models/q.gguf'",
            "[INFO ] backend_fit.cpp:323  - auto-fit plan:",
            "[INFO ] backend_fit.cpp:326  -     CUDA0        NVIDIA GeForce RTX 5070 Ti       free  15010 MiB, budget  14498 MiB",
            "[INFO ] backend_fit.cpp:333  -     RAM          free  24000 MiB, params budget  21600 MiB",
            "[INFO ] backend_fit.cpp:346  -     DiT          params  12630 MiB, compute reserve  2048 MiB -> compute CUDA0, params CPU",
            "[INFO ] backend_fit.cpp:474  - auto-fit: --backend \"diffusion=CUDA0\" --params-backend \"diffusion=CPU\"",
            "[INFO ] main.cpp:149  - listening on: http://127.0.0.1:5000",
        ]
        .map(String::from)
        .to_vec();
        let plan = memory_plan(&log);
        assert_eq!(plan.len(), 5);
        assert!(plan[0].ends_with("auto-fit plan:"));
        assert!(memory_plan(&log[..1]).is_empty());
    }

    #[test]
    fn notices_when_the_engine_is_not_on_the_graphics_card() {
        let log = |lines: &[&str]| lines.iter().map(|l| l.to_string()).collect::<Vec<_>>();
        let on_card = log(&[
            "[INFO ] backend_fit.cpp:323  - auto-fit plan:",
            "[INFO ] backend_fit.cpp:326  -     Vulkan0      NVIDIA GeForce RTX 5070 Ti       free  15010 MiB, budget  14498 MiB",
            "[INFO ] backend_fit.cpp:333  -     RAM          free  24000 MiB, params budget  21600 MiB",
            "[INFO ] backend_fit.cpp:474  - auto-fit: --backend \"diffusion=Vulkan0,te=Vulkan0,vae=Vulkan0\"",
        ]);
        assert_eq!(missed_gpu(&on_card, true), None);
        // A working card without "NVIDIA" in its name.
        let quadro = log(&["[INFO ] backend_fit.cpp:326  -     CUDA0        Quadro RTX 5000                  free  15010 MiB, budget  14498 MiB"]);
        assert_eq!(missed_gpu(&quadro, true), None);
        let igpu_only = log(&[
            "[INFO ] backend_fit.cpp:326  -     Vulkan0      AMD Radeon Graphics (RADV RAPHAEL_MENDOCINO) free   2048 MiB, budget   1536 MiB",
            "[INFO ] backend_fit.cpp:333  -     RAM          free  24000 MiB, params budget  21600 MiB",
        ]);
        assert_eq!(missed_gpu(&igpu_only, true), Some(MissedGpu::NotFound));
        assert_eq!(
            missed_gpu(&igpu_only, false),
            None,
            "an AMD machine is fine on its AMD card"
        );
        let no_dgpu = log(&[
            "[WARN ] backend_fit.cpp:444  - auto-fit: no GPU devices; using the default backend",
        ]);
        assert_eq!(missed_gpu(&no_dgpu, true), Some(MissedGpu::NotFound));
        assert_eq!(
            missed_gpu(&no_dgpu, false),
            None,
            "a laptop with only integrated graphics uses it"
        );
        // Text encoder on the processor (explicit plan) and no card found: diffusion on the processor too.
        let te_cpu = log(&[
            "[INFO ] backend_fit.cpp:474  - auto-fit: --backend \"te=cpu,diffusion=cpu,vae=cpu\"",
        ]);
        assert_eq!(missed_gpu(&te_cpu, true), Some(MissedGpu::NotFound));
        // The card is there but full: auto-fit puts diffusion on the processor.
        let full_card = log(&[
            "[INFO ] backend_fit.cpp:326  -     CUDA0        NVIDIA GeForce RTX 5070 Ti       free    300 MiB, budget      0 MiB",
            "[INFO ] backend_fit.cpp:474  - auto-fit: --backend \"te=cpu,diffusion=CPU,vae=CPU\"",
        ]);
        assert_eq!(missed_gpu(&full_card, true), Some(MissedGpu::NoFreeMemory));
        let full = log(&[
            "[WARN ] backend_fit.cpp:446  - auto-fit: no GPU memory budget available; using CPU",
        ]);
        assert_eq!(missed_gpu(&full, true), Some(MissedGpu::NoFreeMemory));
        assert_eq!(missed_gpu(&full, false), Some(MissedGpu::NoFreeMemory));
        assert_eq!(missed_gpu(&[], true), None);
        // The model runs on the card in parts with its weights in system memory: on the card.
        let in_parts = log(&[
            "[INFO ] backend_fit.cpp:326  -     CUDA0        NVIDIA GeForce RTX 5070 Ti       free  15010 MiB, budget  10914 MiB",
            "[INFO ] backend_fit.cpp:333  -     RAM          free  24000 MiB, params budget  21600 MiB",
            "[INFO ] backend_fit.cpp:474  - auto-fit: --backend \"diffusion=CUDA0,te=CUDA0,vae=CUDA0\" --params-backend \"diffusion=CPU\"",
        ]);
        assert_eq!(missed_gpu(&in_parts, true), None);
        let vulkan_in_parts = log(&[
            "[INFO ] backend_fit.cpp:326  -     Vulkan0      NVIDIA GeForce RTX 5070 Ti       free  15010 MiB, budget  10914 MiB",
            "[INFO ] backend_fit.cpp:474  - auto-fit: --backend \"te=cpu,diffusion=Vulkan0,vae=Vulkan0\" --params-backend \"diffusion=CPU,te=CPU\"",
        ]);
        assert_eq!(missed_gpu(&vulkan_in_parts, true), None);
    }
}
