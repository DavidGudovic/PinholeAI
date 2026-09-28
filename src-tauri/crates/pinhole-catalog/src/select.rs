//! Safe file selection for CivitAI installs (CLAUDE.md "Security rules for
//! downloads", SPEC §5.4 Install step 1):
//! * only `SafeTensor` (`.safetensors`) and `GGUF` (`.gguf`) — never
//!   `PickleTensor` / `.ckpt` / `.pt` / `.pth` / `.bin`;
//! * `pickleScanResult` and `virusScanResult` must both be `Success`;
//! * prefer the primary file, fp16/bf16 over fp8 over fp32, pruned over full.
//!
//! The format rules are hard-coded: `catalog-filters.yaml` can narrow them but
//! never widen them.

use crate::api::ModelFile;

/// The only formats Pinhole will ever download from CivitAI.
pub const HARD_ALLOWED_FORMATS: &[&str] = &["SafeTensor", "GGUF"];

/// Extensions that can carry pickled (executable) code. Always refused.
const PICKLE_EXTENSIONS: &[&str] = &["ckpt", "pt", "pth", "bin", "pkl", "pickle"];

/// Safetensors precisions stable-diffusion.cpp cannot load (bitsandbytes NF4,
/// SVDQuant / ComfyUI int4, NVFP4, MXFP4…). GGUF quants are all fine. `int8`
/// is allowed: the pinned engine runs ComfyUI `int8_tensorwise` (+ convrot)
/// files (docs/int8_convrot.md), which is what CivitAI's int8 Krea 2 /
/// Mage-Flow / Qwen 2.1 files are; any other int8 layout is refused after the
/// download by the header check (`pinhole_registry::detect::unsupported_weights`).
const UNSUPPORTED_SAFETENSORS_FP: &[&str] = &["nf4", "int4", "fp4", "nvfp4", "svdq", "mxfp4"];

/// CivitAI file types that hold model weights.
const WEIGHT_FILE_TYPES: &[&str] = &["model", "pruned model"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Verdict {
    // Ordered by how useful the message is when nothing is installable.
    ScanPending,
    ScanFailed,
    UnsupportedQuant,
    Pickle,
    Unsupported,
    Ok,
}

fn ext_of(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// Normalised format: `SafeTensor` | `GGUF` | `PickleTensor` | other (as reported).
pub fn file_format(f: &ModelFile) -> String {
    if let Some(fmt) = f.metadata.format.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return match fmt.to_ascii_lowercase().as_str() {
            "safetensor" | "safetensors" => "SafeTensor".into(),
            "gguf" => "GGUF".into(),
            "pickletensor" => "PickleTensor".into(),
            _ => fmt.to_string(),
        };
    }
    match ext_of(&f.name).as_str() {
        "safetensors" => "SafeTensor".into(),
        "gguf" => "GGUF".into(),
        e if PICKLE_EXTENSIONS.contains(&e) => "PickleTensor".into(),
        _ => "Other".into(),
    }
}

fn is_weight_file(f: &ModelFile) -> bool {
    f.kind.trim().is_empty() || WEIGHT_FILE_TYPES.iter().any(|t| f.kind.eq_ignore_ascii_case(t))
}

fn verdict(f: &ModelFile, allowed_formats: &[String]) -> Verdict {
    let ext = ext_of(&f.name);
    let format = file_format(f);
    if PICKLE_EXTENSIONS.contains(&ext.as_str()) || format == "PickleTensor" {
        return Verdict::Pickle;
    }
    let hard_ok = HARD_ALLOWED_FORMATS.iter().any(|a| a.eq_ignore_ascii_case(&format));
    let yaml_ok = allowed_formats.iter().any(|a| a.eq_ignore_ascii_case(&format));
    let ext_ok = match format.as_str() {
        "SafeTensor" => ext == "safetensors",
        "GGUF" => ext == "gguf",
        _ => false,
    };
    if !(hard_ok && yaml_ok && ext_ok) {
        return Verdict::Unsupported;
    }
    let scan_ok = |s: &str| s.trim().eq_ignore_ascii_case("success");
    let scan_pending = |s: &str| s.trim().is_empty() || s.trim().eq_ignore_ascii_case("pending");
    if !(scan_ok(&f.pickle_scan_result) && scan_ok(&f.virus_scan_result)) {
        let failed = [&f.pickle_scan_result, &f.virus_scan_result].iter().any(|s| !scan_ok(s) && !scan_pending(s));
        return if failed { Verdict::ScanFailed } else { Verdict::ScanPending };
    }
    if format == "SafeTensor" {
        if let Some(fp) = f.metadata.fp.as_deref() {
            if UNSUPPORTED_SAFETENSORS_FP.iter().any(|u| fp.trim().eq_ignore_ascii_case(u)) {
                return Verdict::UnsupportedQuant;
            }
        }
    }
    Verdict::Ok
}

fn fp_rank(f: &ModelFile) -> u8 {
    match f.metadata.fp.as_deref().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("fp16") | Some("bf16") => 0,
        Some("fp8") => 1,
        Some("fp32") => 3,
        _ => 2,
    }
}

fn size_rank(f: &ModelFile) -> u8 {
    match f.metadata.size.as_deref().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("pruned") => 0,
        Some("full") => 2,
        _ => 1,
    }
}

fn reason(v: Verdict) -> &'static str {
    match v {
        Verdict::ScanPending => "CivitAI hasn't finished its safety scan of this file yet. Try again later.",
        Verdict::ScanFailed => "CivitAI's safety scan flagged this file, so Pinhole won't install it.",
        Verdict::UnsupportedQuant => {
            "This model only comes in a compressed format (such as NF4 or INT4) that Pinhole can't run. Look for a version with fp16, bf16, fp8, int8 or GGUF files."
        }
        Verdict::Pickle => {
            "This model is only available as an old-style .ckpt/.pt file, which can hide harmful code. Pinhole only installs .safetensors and .gguf files."
        }
        Verdict::Unsupported | Verdict::Ok => "This model isn't available as a .safetensors or .gguf file, so Pinhole can't install it.",
    }
}

pub const NO_FILE_REASON: &str = "This version has no model file to download.";

/// A safe file CivitAI lists without a SHA-256: the download couldn't be verified.
pub const NO_HASH_REASON: &str =
    "CivitAI doesn't list a checksum for this file, so Pinhole can't verify the download. Try another version of this model.";

/// Pick the file to install, or a plain-language reason why none is safe.
///
/// Ranking among safe files: files without a SHA-256 last (they can't be
/// verified, and installs refuse them — see [`NO_HASH_REASON`]), then fp32 files
/// (a half-precision copy is the same model at half the download), then the
/// primary file, then fp16/bf16 → fp8 → unknown → fp32, then pruned → unknown →
/// full, then the smaller file.
pub fn select_file<'a>(files: &'a [ModelFile], allowed_formats: &[String]) -> Result<&'a ModelFile, String> {
    let weights: Vec<&ModelFile> = files.iter().filter(|f| is_weight_file(f)).collect();
    if weights.is_empty() {
        return Err(NO_FILE_REASON.into());
    }
    let mut ok: Vec<&ModelFile> = Vec::new();
    let mut best_problem: Option<Verdict> = None;
    for f in weights {
        match verdict(f, allowed_formats) {
            Verdict::Ok => ok.push(f),
            v => best_problem = Some(best_problem.map_or(v, |b| b.min(v))),
        }
    }
    ok.into_iter()
        .min_by_key(|f| (f.sha256().is_none(), fp_rank(f) == 3, !f.primary, fp_rank(f), size_rank(f), f.size_bytes()))
        .ok_or_else(|| reason(best_problem.unwrap_or(Verdict::Unsupported)).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::FileMetadata;

    fn allowed() -> Vec<String> {
        vec!["SafeTensor".into(), "GGUF".into()]
    }

    fn file(name: &str, format: Option<&str>, fp: Option<&str>, size: Option<&str>, primary: bool, kb: f64) -> ModelFile {
        ModelFile {
            id: kb as u64,
            name: name.into(),
            size_kb: kb,
            kind: "Model".into(),
            pickle_scan_result: "Success".into(),
            virus_scan_result: "Success".into(),
            metadata: FileMetadata { format: format.map(Into::into), fp: fp.map(Into::into), size: size.map(Into::into) },
            primary,
            download_url: format!("https://civitai.com/api/download/models/1?f={name}"),
            ..Default::default()
        }
    }

    #[test]
    fn pickle_only_is_blocked() {
        let files = vec![file("a.ckpt", Some("PickleTensor"), Some("fp16"), Some("pruned"), true, 2e6)];
        let err = select_file(&files, &allowed()).unwrap_err();
        assert!(err.contains(".ckpt"), "{err}");
        // A .pt file mislabelled as SafeTensor is still refused.
        let files = vec![file("a.pt", Some("SafeTensor"), None, None, true, 1.0)];
        assert!(select_file(&files, &allowed()).unwrap_err().contains(".ckpt/.pt"));
        let files = vec![file("a.pth", None, None, None, true, 1.0)];
        assert!(select_file(&files, &allowed()).is_err());
    }

    #[test]
    fn safe_alternative_beats_pickle_primary() {
        let files = vec![
            file("a.ckpt", Some("PickleTensor"), Some("fp16"), Some("pruned"), true, 2e6),
            file("a.safetensors", Some("SafeTensor"), Some("fp16"), Some("pruned"), false, 2e6),
        ];
        assert_eq!(select_file(&files, &allowed()).unwrap().name, "a.safetensors");
    }

    #[test]
    fn failed_or_pending_scans_are_blocked() {
        let mut f = file("a.safetensors", Some("SafeTensor"), Some("fp16"), None, true, 1.0);
        f.virus_scan_result = "Danger".into();
        let err = select_file(std::slice::from_ref(&f), &allowed()).unwrap_err();
        assert!(err.contains("flagged"), "{err}");
        f.virus_scan_result = "Success".into();
        f.pickle_scan_result = "Error".into();
        assert!(select_file(std::slice::from_ref(&f), &allowed()).unwrap_err().contains("flagged"));
        f.pickle_scan_result = "Pending".into();
        assert!(select_file(std::slice::from_ref(&f), &allowed()).unwrap_err().contains("hasn't finished"));
        f.pickle_scan_result = String::new();
        assert!(select_file(std::slice::from_ref(&f), &allowed()).is_err(), "missing scan result is not Success");
        // A scanned alternative is fine.
        let good = file("b.safetensors", Some("SafeTensor"), Some("fp16"), None, false, 1.0);
        assert_eq!(select_file(&[f, good], &allowed()).unwrap().name, "b.safetensors");
    }

    #[test]
    fn gguf_accepted() {
        let files = vec![file("m-Q4_K_M.gguf", Some("GGUF"), None, None, true, 7e6)];
        assert_eq!(select_file(&files, &allowed()).unwrap().name, "m-Q4_K_M.gguf");
        // GGUF with a nonsense "nf4" fp label is still fine (GGUF quants all load).
        let files = vec![file("m.gguf", Some("GGUF"), Some("nf4"), Some("pruned"), true, 7e6)];
        assert!(select_file(&files, &allowed()).is_ok());
        // Unless the YAML narrowed the list.
        assert!(select_file(&files, &["SafeTensor".to_string()]).is_err());
    }

    #[test]
    fn file_with_sha256_preferred() {
        let mut hashed = file("b.safetensors", Some("SafeTensor"), Some("fp16"), Some("pruned"), false, 2e6);
        hashed.hashes.insert("SHA256".into(), "AB".repeat(32));
        let bare = file("a.safetensors", Some("SafeTensor"), Some("fp16"), Some("pruned"), true, 2e6);
        assert_eq!(select_file(&[bare.clone(), hashed], &allowed()).unwrap().name, "b.safetensors");
        // Still selectable when it's the only one (plans/installs then refuse it).
        assert_eq!(select_file(&[bare], &allowed()).unwrap().name, "a.safetensors");
    }

    #[test]
    fn primary_preferred() {
        let files = vec![
            file("b.safetensors", Some("SafeTensor"), Some("fp16"), Some("pruned"), false, 2e6),
            file("a.safetensors", Some("SafeTensor"), Some("fp16"), Some("full"), true, 2.1e6),
        ];
        assert_eq!(select_file(&files, &allowed()).unwrap().name, "a.safetensors");
    }

    #[test]
    fn half_precision_pruned_preferred_over_fp32_full() {
        let files = vec![
            file("full32.safetensors", Some("SafeTensor"), Some("fp32"), Some("full"), true, 13e6),
            file("full16.safetensors", Some("SafeTensor"), Some("fp16"), Some("full"), false, 6.8e6),
            file("pruned16.safetensors", Some("SafeTensor"), Some("fp16"), Some("pruned"), false, 6.7e6),
            file("pruned8.safetensors", Some("SafeTensor"), Some("fp8"), Some("pruned"), false, 4e6),
        ];
        assert_eq!(select_file(&files, &allowed()).unwrap().name, "pruned16.safetensors");
        // fp32 is still used when it's all there is.
        assert_eq!(select_file(&files[..1], &allowed()).unwrap().name, "full32.safetensors");
    }

    #[test]
    fn unsupported_safetensors_quants() {
        let files = vec![file("m_nf4.safetensors", Some("SafeTensor"), Some("nf4"), Some("pruned"), true, 3e6)];
        assert!(select_file(&files, &allowed()).unwrap_err().contains("NF4"));
        let files = vec![
            file("m_int4.safetensors", Some("SafeTensor"), Some("int4"), None, true, 6e6),
            file("m_bf16.safetensors", Some("SafeTensor"), Some("bf16"), None, false, 12e6),
        ];
        assert_eq!(select_file(&files, &allowed()).unwrap().name, "m_bf16.safetensors");
        // CivitAI Krea 2 uploads: int8 (ComfyUI int8_tensorwise, runs) over int4 / nvfp4 (don't).
        let files = vec![
            file("k_int4.safetensors", Some("SafeTensor"), Some("int4"), None, true, 7e6),
            file("k_nvfp4.safetensors", Some("SafeTensor"), Some("nvfp4"), None, false, 8e6),
            file("k_int8.safetensors", Some("SafeTensor"), Some("int8"), None, false, 13e6),
        ];
        assert_eq!(select_file(&files, &allowed()).unwrap().name, "k_int8.safetensors");
        let err = select_file(&files[..2], &allowed()).unwrap_err();
        assert!(err.contains("INT4") && err.contains("int8"), "{err}");
        // Otherwise equal: fp8 is preferred over int8.
        let files = vec![
            file("k_int8.safetensors", Some("SafeTensor"), Some("int8"), None, false, 13e6),
            file("k_fp8.safetensors", Some("SafeTensor"), Some("fp8"), None, false, 13e6),
        ];
        assert_eq!(select_file(&files, &allowed()).unwrap().name, "k_fp8.safetensors");
    }

    #[test]
    fn ignores_non_weight_files() {
        let mut zip = file("training_data.zip", Some("Other"), None, None, true, 4.0);
        zip.kind = "Training Data".into();
        assert_eq!(select_file(std::slice::from_ref(&zip), &allowed()).unwrap_err(), NO_FILE_REASON);
        let mut vae = file("sdxl_vae.safetensors", Some("SafeTensor"), None, None, false, 3e5);
        vae.kind = "VAE".into();
        let model = file("m.safetensors", Some("SafeTensor"), Some("fp16"), None, false, 6e6);
        assert_eq!(select_file(&[zip, vae, model], &allowed()).unwrap().name, "m.safetensors");
        let diffusers = file("m.zip", Some("Diffusers"), None, None, true, 1.0);
        assert!(select_file(&[diffusers], &allowed()).unwrap_err().contains(".safetensors or .gguf"));
    }

    #[test]
    fn fixture_selection() {
        let page: crate::api::ModelsPage = serde_json::from_str(include_str!("../tests/fixtures/models_page.json")).unwrap();
        let pick = |id: u64| {
            let m = page.items.iter().find(|m| m.id == id).unwrap();
            select_file(&m.model_versions[0].files, &allowed()).map(|f| f.name.clone())
        };
        assert_eq!(pick(139562).unwrap(), "realvisxlV50_v50Bakedvae_full_fp16.safetensors");
        assert!(pick(5000).unwrap_err().contains(".ckpt"));
        assert_eq!(pick(618692).unwrap(), "fluxFusion_Q4_K_M.gguf", "primary GGUF");
        assert!(pick(777001).unwrap_err().contains("scan"));
    }
}
