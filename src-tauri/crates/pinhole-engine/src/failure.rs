//! Classify engine failures from the (already redacted) ring-buffer tail and
//! the exit status, so the core can say what to do next instead of dumping
//! engine output (CLAUDE.md UX rules).

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
    "failed to allocate",
    "allocation of size",
    "alloc params backend buffer failed",
    "std::bad_alloc",
    "not enough memory",
    "insufficient memory",
];
const DRIVER: &[&str] = &["driver version is insufficient", "cuda driver version", "unsupported ptx version", "no kernel image is available"];
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
const GLIBC: &[&str] = &["glibc_2.", "glibcxx_3.", "version `glibc", "version `glibcxx"];
const MISSING_LIB: &[&str] = &["error while loading shared libraries", "cannot open shared object file", ".dll was not found", "could not load library"];
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
    if has(MISSING_LIB) || matches!(exit_code, Some(STATUS_DLL_NOT_FOUND) | Some(STATUS_ENTRYPOINT_NOT_FOUND)) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_failures() {
        assert_eq!(classify("ggml_backend_cuda_buffer_type_alloc_buffer: allocating 9000 MiB on device 0: cudaMalloc failed: out of memory", None), Failure::OutOfMemory);
        assert_eq!(classify("ggml_vulkan: Device memory allocation of size 123 failed.\nvk::Device::allocateMemory: ErrorOutOfDeviceMemory", None), Failure::OutOfMemory);
        assert_eq!(classify("./sd-server: /lib/x86_64-linux-gnu/libc.so.6: version `GLIBC_2.38' not found", Some(1)), Failure::GlibcTooOld);
        assert_eq!(classify("sd-server: error while loading shared libraries: libvulkan.so.1: cannot open shared object file", Some(127)), Failure::MissingLibrary);
        assert_eq!(classify("", Some(-1073741515)), Failure::MissingLibrary);
        assert_eq!(classify("CUDA error: CUDA driver version is insufficient for CUDA runtime version", None), Failure::DriverTooOld);
        assert_eq!(classify("ggml_cuda_init: failed to initialize CUDA: no CUDA-capable device is detected", None), Failure::NoGpu);
        assert_eq!(classify("[ERROR] main.cpp:91  - new_sd_ctx_t failed", Some(1)), Failure::ModelLoad);
        assert_eq!(classify("something else", Some(1)), Failure::Unknown);
    }

}
