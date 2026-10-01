//! `config/engine.yaml`: pinned engine versions, per-platform archives and
//! backend selection with fallbacks.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::EngineError;

/// One downloadable archive (`.zip` or `.tar.gz`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveSpec {
    pub url: String,
    /// Lowercase hex, or `TODO` while unverified.
    pub sha256: String,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub size_mb: Option<u64>,
}

impl ArchiveSpec {
    /// Verified hash, or `None` for `TODO` / empty / malformed values.
    pub fn verified_sha256(&self) -> Option<String> {
        let s = self.sha256.trim().to_ascii_lowercase();
        (s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())).then_some(s)
    }

    /// Archive file name (last URL path segment, query stripped).
    pub fn file_name(&self) -> String {
        let no_query = self.url.split(['?', '#']).next().unwrap_or("");
        let name = no_query.rsplit('/').next().unwrap_or("").trim();
        if name.is_empty() {
            "engine-archive.zip".into()
        } else {
            name.to_string()
        }
    }

    /// Best-known size in bytes.
    pub fn bytes(&self) -> Option<u64> {
        self.size_bytes.or(self.size_mb.map(|mb| mb * 1_000_000))
    }
}

/// A build = main archive + extra archives, all unpacked into one folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildSpec {
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub size_mb: Option<u64>,
    #[serde(default)]
    pub extra: Vec<ArchiveSpec>,
    #[serde(default)]
    pub note: Option<String>,
    /// Linux: minimum glibc the binaries need (e.g. `2.38`).
    #[serde(default)]
    pub min_glibc: Option<String>,
}

impl BuildSpec {
    /// Main archive first, then extras.
    pub fn archives(&self) -> Vec<ArchiveSpec> {
        let mut v = vec![ArchiveSpec {
            url: self.url.clone(),
            sha256: self.sha256.clone(),
            size_bytes: self.size_bytes,
            size_mb: self.size_mb,
        }];
        v.extend(self.extra.iter().cloned());
        v
    }

    /// Total download size in bytes (0 when unknown).
    pub fn total_bytes(&self) -> u64 {
        self.archives().iter().filter_map(|a| a.bytes()).sum()
    }

    /// `true` when every URL is filled in (not `TODO`).
    pub fn has_urls(&self) -> bool {
        self.archives()
            .iter()
            .all(|a| a.url.starts_with("https://"))
    }

    /// File names of archives without a real pinned SHA-256 (`TODO` / missing).
    pub fn unverified_archives(&self) -> Vec<String> {
        self.archives()
            .iter()
            .filter(|a| a.verified_sha256().is_none())
            .map(|a| a.file_name())
            .collect()
    }
}

/// Release builds refuse to install an engine archive whose pinned SHA-256 is
/// `TODO` or missing; debug builds allow it while a new pin is being prepared.
pub const REQUIRE_PINNED_HASHES: bool = cfg!(not(debug_assertions));

/// `Err(file names)` when `require` is set and some archive isn't hash-pinned.
pub fn check_pinned(build: &BuildSpec, require: bool) -> Result<(), Vec<String>> {
    let missing = build.unverified_archives();
    if require && !missing.is_empty() {
        return Err(missing);
    }
    Ok(())
}

/// One engine (sd.cpp or llama.cpp).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnginePin {
    pub repo: String,
    pub version: String,
    #[serde(default)]
    pub commit: Option<String>,
    /// Executable base name without `.exe` (`sd-server`, `llama-server`).
    pub binary: String,
    #[serde(default)]
    pub launch_defaults: Vec<String>,
    /// Backend remaps applied before selection (`{ cuda: vulkan }`).
    #[serde(default)]
    pub backend_override: BTreeMap<String, String>,
    /// `<os>_<backend>` → build.
    #[serde(default)]
    pub builds: BTreeMap<String, BuildSpec>,
}

/// The whole `engine.yaml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineConfig {
    #[serde(default)]
    pub schema_version: u32,
    pub stable_diffusion_cpp: EnginePin,
    pub llama_cpp: EnginePin,
    /// GPU vendor → backend (`nvidia: cuda`, `none: cpu`).
    #[serde(default)]
    pub selection: BTreeMap<String, String>,
    /// backend → backends to try when there is no build for it.
    #[serde(default)]
    pub fallbacks: BTreeMap<String, Vec<String>>,
}

/// A build chosen for this machine.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedBuild {
    /// `windows_cuda`, `linux_vulkan`…
    pub key: String,
    /// Backend actually used (after override/fallback).
    pub backend: String,
    pub build: BuildSpec,
}

impl EngineConfig {
    pub fn load(path: &Path) -> Result<Self, EngineError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| EngineError::Config(format!("could not read {}: {e}", path.display())))?;
        Self::from_yaml(&text)
    }

    pub fn from_yaml(text: &str) -> Result<Self, EngineError> {
        let cfg: EngineConfig =
            serde_yaml::from_str(text).map_err(|e| EngineError::Config(e.to_string()))?;
        for (name, pin) in [
            ("stable_diffusion_cpp", &cfg.stable_diffusion_cpp),
            ("llama_cpp", &cfg.llama_cpp),
        ] {
            if pin.version.trim().is_empty() || pin.version == "TODO" {
                return Err(EngineError::Config(format!("{name}.version is not pinned")));
            }
            if pin.binary.trim().is_empty() || pin.binary.contains(['/', '\\']) {
                return Err(EngineError::Config(format!(
                    "{name}.binary must be a plain file name"
                )));
            }
        }
        Ok(cfg)
    }

    /// Backend for a GPU vendor key (`nvidia` | `amd` | `intel` | `none`).
    pub fn backend_for_vendor(&self, vendor: &str) -> String {
        self.selection.get(vendor).cloned().unwrap_or_else(|| {
            if vendor == "nvidia" {
                "cuda".into()
            } else if vendor == "none" {
                "cpu".into()
            } else {
                "vulkan".into()
            }
        })
    }

    /// Candidate backends in order: `backend` (after the pin's override), then fallbacks, then `cpu`.
    pub fn backend_candidates(&self, pin: &EnginePin, backend: &str) -> Vec<String> {
        let first = pin
            .backend_override
            .get(backend)
            .cloned()
            .unwrap_or_else(|| backend.to_string());
        let mut out = vec![first.clone()];
        let fallbacks =
            self.fallbacks
                .get(&first)
                .cloned()
                .unwrap_or_else(|| match first.as_str() {
                    "cuda" => vec!["vulkan".into(), "cpu".into()],
                    "vulkan" => vec!["cpu".into()],
                    _ => vec![],
                });
        for b in fallbacks
            .into_iter()
            .chain(std::iter::once("cpu".to_string()))
        {
            if !out.contains(&b) {
                out.push(b);
            }
        }
        out
    }

    /// Pick the build for `(os, backend)`: exact match, else the fallbacks.
    pub fn select_build(
        &self,
        pin: &EnginePin,
        os: &str,
        backend: &str,
    ) -> Result<SelectedBuild, EngineError> {
        for b in self.backend_candidates(pin, backend) {
            let key = format!("{os}_{b}");
            if let Some(build) = pin.builds.get(&key) {
                if build.has_urls() {
                    return Ok(SelectedBuild {
                        key,
                        backend: b,
                        build: build.clone(),
                    });
                }
            }
        }
        Err(EngineError::NoBuild {
            os: os.to_string(),
            backend: backend.to_string(),
        })
    }
}

/// `windows` | `linux` | `macos` | other `std::env::consts::OS`.
pub fn current_os() -> &'static str {
    std::env::consts::OS
}

/// Executable file name for this OS (`sd-server.exe` on Windows).
pub fn exe_name(binary: &str) -> String {
    if cfg!(windows) {
        format!("{binary}.exe")
    } else {
        binary.to_string()
    }
}

/// Linux: the running glibc version (`2.39`), from `gnu_get_libc_version`.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn glibc_version() -> Option<String> {
    extern "C" {
        fn gnu_get_libc_version() -> *const std::os::raw::c_char;
    }
    // SAFETY: returns a pointer to a static NUL-terminated string.
    let ptr = unsafe { gnu_get_libc_version() };
    if ptr.is_null() {
        return None;
    }
    let s = unsafe { std::ffi::CStr::from_ptr(ptr) };
    s.to_str().ok().map(|s| s.to_string())
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn glibc_version() -> Option<String> {
    None
}

/// `true` if `have` (e.g. `2.35`) is at least `need` (e.g. `2.38`).
pub fn version_at_least(have: &str, need: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parse(have), parse(need));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> EngineConfig {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../config/engine.yaml");
        EngineConfig::load(&path).expect("shipped engine.yaml parses")
    }

    #[test]
    fn shipped_yaml_parses_and_is_pinned() {
        let cfg = shipped();
        // The lock-down build from Pinhole's fork (engine/sd-cpp/): an upstream tag + `-pinhole<n>`.
        // A new version string also keeps an old upstream install from being reused.
        let version = &cfg.stable_diffusion_cpp.version;
        assert!(version.starts_with("master-") && version.contains("-pinhole"));
        assert_eq!(cfg.stable_diffusion_cpp.binary, "sd-server");
        assert_eq!(cfg.llama_cpp.binary, "llama-server");
        for key in [
            "windows_cuda",
            "windows_vulkan",
            "windows_cpu",
            "linux_cuda",
            "linux_vulkan",
            "linux_cpu",
        ] {
            let b = cfg
                .stable_diffusion_cpp
                .builds
                .get(key)
                .unwrap_or_else(|| panic!("sd build {key}"));
            assert!(
                b.url.starts_with(
                    "https://github.com/DavidGudovic/stable-diffusion.cpp/releases/download/"
                ),
                "{key}"
            );
            assert!(
                b.url.contains(&cfg.stable_diffusion_cpp.version),
                "{key} url matches version"
            );
            assert!(b.size_bytes.unwrap_or(0) > 1_000_000, "{key} size");
            let l = cfg
                .llama_cpp
                .builds
                .get(key)
                .unwrap_or_else(|| panic!("llama build {key}"));
            assert!(
                l.url.contains(&cfg.llama_cpp.version),
                "{key} llama url matches version"
            );
        }
        // Windows CUDA needs the CUDA runtime DLL archive in the same folder.
        let cuda = &cfg.stable_diffusion_cpp.builds["windows_cuda"];
        assert_eq!(cuda.extra.len(), 1);
        assert!(cuda.extra[0]
            .url
            .ends_with("cudart-sd-bin-win-cu12-x64.zip"));
        assert_eq!(cuda.archives().len(), 2);
        // Launch defaults never bind to anything but loopback.
        assert!(cfg
            .stable_diffusion_cpp
            .launch_defaults
            .windows(2)
            .any(|w| w == ["--listen-ip", "127.0.0.1"]));
        assert!(cfg
            .llama_cpp
            .launch_defaults
            .windows(2)
            .any(|w| w == ["--host", "127.0.0.1"]));
    }

    #[test]
    fn every_hash_is_hex_or_todo() {
        let cfg = shipped();
        for pin in [&cfg.stable_diffusion_cpp, &cfg.llama_cpp] {
            for (key, b) in &pin.builds {
                for a in b.archives() {
                    assert!(
                        a.sha256 == "TODO" || a.verified_sha256().is_some(),
                        "{key}: {}",
                        a.sha256
                    );
                }
            }
        }
    }

    #[test]
    fn selection_and_fallbacks() {
        let cfg = shipped();
        let sd = &cfg.stable_diffusion_cpp;
        assert_eq!(cfg.backend_for_vendor("nvidia"), "cuda");
        assert_eq!(cfg.backend_for_vendor("amd"), "vulkan");
        assert_eq!(cfg.backend_for_vendor("none"), "cpu");

        let s = cfg.select_build(sd, "windows", "cuda").unwrap();
        assert_eq!(
            (s.key.as_str(), s.backend.as_str()),
            ("windows_cuda", "cuda")
        );
        // Pinhole's fork builds Linux CUDA (RTX 30+; app.rs picks Vulkan for older cards).
        let s = cfg.select_build(sd, "linux", "cuda").unwrap();
        assert_eq!((s.key.as_str(), s.backend.as_str()), ("linux_cuda", "cuda"));
        assert_eq!(s.build.extra.len(), 1, "Linux CUDA runtime zip");
        let s = cfg.select_build(sd, "linux", "cpu").unwrap();
        assert_eq!(s.key, "linux_cpu");
        assert!(cfg.select_build(sd, "macos", "cpu").is_err());

        // llama.cpp: CUDA is remapped to Vulkan (no second CUDA runtime download).
        let l = cfg.select_build(&cfg.llama_cpp, "windows", "cuda").unwrap();
        assert_eq!(l.key, "windows_vulkan");
    }

    #[test]
    fn fallback_skips_todo_urls() {
        let yaml = r#"
stable_diffusion_cpp:
  repo: r
  version: v1
  binary: sd-server
  builds:
    linux_vulkan: { url: TODO, sha256: TODO }
    linux_cpu: { url: "https://github.com/x/y/releases/download/v1/a.zip", sha256: TODO, size_mb: 3 }
llama_cpp:
  repo: r
  version: b1
  binary: llama-server
"#;
        let cfg = EngineConfig::from_yaml(yaml).unwrap();
        let s = cfg
            .select_build(&cfg.stable_diffusion_cpp, "linux", "vulkan")
            .unwrap();
        assert_eq!(s.key, "linux_cpu");
        assert_eq!(s.build.archives()[0].verified_sha256(), None);
        assert_eq!(s.build.archives()[0].file_name(), "a.zip");
        assert_eq!(s.build.total_bytes(), 3_000_000);
        // Release builds refuse the TODO hash; debug builds let it through.
        assert_eq!(check_pinned(&s.build, true), Err(vec!["a.zip".to_string()]));
        assert_eq!(check_pinned(&s.build, false), Ok(()));
    }

    #[test]
    fn shipped_builds_are_all_hash_pinned() {
        let cfg = shipped();
        for pin in [&cfg.stable_diffusion_cpp, &cfg.llama_cpp] {
            for (key, b) in &pin.builds {
                assert_eq!(
                    check_pinned(b, true),
                    Ok(()),
                    "{key} has a TODO sha256 (release builds would refuse it)"
                );
            }
        }
    }

    #[test]
    fn rejects_unpinned_or_bad_binary() {
        let bad = "stable_diffusion_cpp: { repo: r, version: TODO, binary: sd-server }\nllama_cpp: { repo: r, version: b1, binary: llama-server }\n";
        assert!(EngineConfig::from_yaml(bad).is_err());
        let bad = "stable_diffusion_cpp: { repo: r, version: v, binary: ../sd }\nllama_cpp: { repo: r, version: b1, binary: llama-server }\n";
        assert!(EngineConfig::from_yaml(bad).is_err());
    }

    #[test]
    fn version_compare() {
        assert!(version_at_least("2.39", "2.38"));
        assert!(version_at_least("2.38", "2.38"));
        assert!(!version_at_least("2.35", "2.38"));
        assert!(version_at_least("3.0", "2.38"));
    }
}
