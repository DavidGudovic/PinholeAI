use super::*;
use crate::vram::*;
use crate::wiring::{gpu_resident_components, HwContext};

const GIB: u64 = 1024 * 1024 * 1024;

fn need(gb: f32, min_gb: f32) -> VramNeed {
    VramNeed {
        gb,
        min_gb,
        estimate: false,
        on_cpu: false,
    }
}

#[test]
fn fit_boundaries() {
    let n = need(7.0, 5.0);
    assert_eq!(fit(&n, 8.0), Fit::Fits); // 7 ≤ 8 − 1
    assert_eq!(fit(&n, 7.99), Fit::Tight);
    assert_eq!(fit(&n, 5.0), Fit::Tight); // min ≤ vram
    assert_eq!(fit(&n, 4.99), Fit::TooBig);
    assert_eq!(fit(&n, 0.0), Fit::TooBig); // CPU only
    assert_eq!(fit(&n, -1.0), Fit::TooBig);
    assert_eq!(fit(&n, f32::NAN), Fit::TooBig);
    assert_eq!(fit(&need(0.8, 0.5), 0.0), Fit::Tight); // tiny model on CPU
    assert_eq!(fit(&need(16.0, 12.0), 16.0), Fit::Tight);
    assert_eq!(fit(&need(15.0, 12.0), 16.0), Fit::Fits);
}

#[test]
fn estimate_formula() {
    let reg = shipped();
    let sdxl = reg.family("sdxl").unwrap();
    // 6.5 GiB checkpoint + 0.3125 GiB VAE override + 2.0 activations + 0.5 reserve.
    let n = estimate(reg, sdxl, 13 * GIB / 2, 5 * GIB / 16, 0);
    assert!(n.estimate);
    assert!(approx(n.gb, 9.4), "{n:?}"); // 9.3125 rounded up
    assert!(approx(n.min_gb, 5.8), "{n:?}"); // 3.25 + 2 + 0.5 = 5.75 rounded up
    assert_eq!(fit(&n, 12.0), Fit::Fits);
    assert_eq!(fit(&n, 8.0), Fit::Tight);
    assert_eq!(fit(&n, 4.0), Fit::TooBig);

    // Flux Q4 (≈6.4 GiB) with a VAE, clip_l + t5 fp8 on the GPU.
    let flux = reg.family("flux1_dev").unwrap();
    let n = estimate(
        reg,
        flux,
        6_931_817_760,
        335_304_388,
        246_144_152 + 4_893_934_904,
    );
    assert!(n.gb > n.min_gb);
    assert_eq!(fit(&n, 8.0), Fit::Tight);
    assert_eq!(fit(&n, 16.0), Fit::Fits);
    assert_eq!(
        estimate(reg, flux, 0, 0, 0).min_gb,
        estimate(reg, flux, 0, 0, 0).gb
    );
}

/// Regression: the text encoder is not on the GPU during sampling, so it is
/// not added to the diffusion weights (Qwen models read ~8 GB too big).
#[test]
fn text_encoders_are_a_separate_stage() {
    let reg = shipped();
    let qwen = reg.family("qwen_image").unwrap();
    // CivitAI's usual Qwen-Image file: fp8, 19,951,792 KiB.
    let fp8 = 19_951_792 * 1024;
    let vae = 254_000_000;
    let te = 8_099_000_000; // Qwen2.5-VL 7B Q8
    let n = estimate(reg, qwen, fp8, vae, te);
    // 19.03 GiB + 0.24 VAE + 3.0 activations + 0.5 = 22.8, not 30.3 with the encoder.
    assert!(approx(n.gb, 22.8), "{n:?}");
    assert_eq!(fit(&n, 16.0), Fit::Tight);
    assert_eq!(fit(&n, 24.0), Fit::Fits);
    // A small diffusion model with a big encoder is sized by the prompt stage.
    let n = estimate(reg, qwen, GIB, 0, 8 * GIB);
    assert!(approx(n.gb, 9.5), "{n:?}"); // 8 + 1 + 0.5
}

#[test]
fn registry_values_are_not_estimates() {
    let reg = shipped();
    let z = reg.family("z_image_turbo").unwrap();
    assert_eq!(
        registry_need(z, None),
        Some(VramNeed {
            gb: 16.0,
            min_gb: 12.0,
            estimate: false,
            on_cpu: false,
        })
    );
    assert_eq!(
        registry_need(z, Some("q4_k")),
        Some(VramNeed {
            gb: 8.0,
            min_gb: 5.0,
            estimate: false,
            on_cpu: false,
        })
    );
    assert_eq!(registry_need(z, Some("nope")), registry_need(z, None));
    assert_eq!(
        registry_need(reg.family("sdxl").unwrap(), None).map(|n| n.gb),
        Some(10.0)
    );
    assert_eq!(registry_need(reg.family("flux1_dev").unwrap(), None), None);
}

#[test]
fn gpu_resident_components_follow_te_on_cpu_flags() {
    let hw = HwContext {
        vram_gb: 8.0,
        backend: "cuda".into(),
        ram_gb: 32.0,
    };
    let reg = shipped();
    assert_eq!(
        gpu_resident_components(reg, reg.family("flux1_dev").unwrap(), &hw).len(),
        3
    );
    let reg = with_overrides("families: { flux1_dev: { flags: [\"--clip-on-cpu\"] }, z_image_turbo: { flags: [\"--backend\", \"te=cpu\"] } }\n");
    let only_vae = |id: &str| {
        gpu_resident_components(&reg, reg.family(id).unwrap(), &hw)
            .iter()
            .map(|c| c.kind.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(only_vae("flux1_dev"), vec!["vae"]);
    assert_eq!(only_vae("z_image_turbo"), vec!["vae"]);
}

#[test]
fn cpu_fit_uses_system_ram_for_small_models() {
    let reg = shipped();
    let sd15 = reg.family("sd15").unwrap();
    let sdxl = reg.family("sdxl").unwrap();
    assert!(sd15.cpu_friendly && reg.family("sd15_fast").unwrap().cpu_friendly);
    assert!(!sdxl.cpu_friendly);
    // SD 1.5 fp16 (2 132 696 762 bytes ≈ 1.99 GiB) + 1.0 GiB activations.
    let bytes = 2_132_696_762;
    let n = cpu_need(sd15, bytes);
    assert!(n.on_cpu && n.estimate);
    assert!(approx(n.gb, 3.0), "{n:?}");
    assert_eq!(n.min_gb, n.gb);
    // Needs 3.0 + 4 GB spare: an "8 GB" PC (≈7.7 GiB) is enough, 6 GB is not.
    assert_eq!(fit_cpu(sd15, bytes, 7.7), Fit::Tight);
    assert_eq!(fit_cpu(sd15, bytes, 7.0), Fit::Tight);
    assert_eq!(fit_cpu(sd15, bytes, 6.0), Fit::TooBig);
    assert_eq!(
        fit_cpu(sd15, bytes, 0.0),
        Fit::Tight,
        "RAM unknown → size rule only"
    );
    // A full-precision SD 1.5 (7.7 GB) is still offered: the family is cpu_friendly.
    assert_eq!(fit_cpu(sd15, 7_700_000_000, 32.0), Fit::Tight);
    // Big families: too slow on the processor, however much RAM there is.
    assert_eq!(
        fit_cpu(sdxl, 7_105_000_000 + 335_000_000, 64.0),
        Fit::TooBig
    );
    // …unless the whole model is small (≤ 4 GiB of weights).
    assert_eq!(fit_cpu(sdxl, 3 * GIB, 64.0), Fit::Tight);
    assert_eq!(fit_cpu(sdxl, 5 * GIB, 64.0), Fit::TooBig);
    let z = reg.family("z_image_turbo").unwrap();
    assert_eq!(
        fit_cpu(z, 3_864_000_000 + 335_000_000 + 8_045_000_000, 64.0),
        Fit::TooBig
    );
}

#[test]
fn need_and_fit_picks_vram_or_ram() {
    let reg = shipped();
    let sd15 = reg.family("sd15").unwrap();
    let gpu_need = registry_need(sd15, None).unwrap();
    let hw = |vram_gb: f32, backend: &str| HwContext {
        vram_gb,
        backend: backend.into(),
        ram_gb: 16.0,
    };
    // GPU: the registry figure against VRAM.
    let (n, f) = need_and_fit(sd15, gpu_need, 2 * GIB, &hw(8.0, "cuda"));
    assert_eq!((n, f), (gpu_need, Fit::Fits));
    assert_eq!(
        need_and_fit(sd15, gpu_need, 2 * GIB, &hw(4.0, "vulkan")).1,
        Fit::Tight
    );
    // CPU (backend cpu, or no known VRAM): RAM need against system RAM.
    for h in [hw(0.0, "cpu"), hw(0.0, "cuda"), hw(8.0, "cpu")] {
        assert!(h.cpu_only());
        let (n, f) = need_and_fit(sd15, gpu_need, 2 * GIB, &h);
        assert!(n.on_cpu, "{n:?}");
        assert!(approx(n.gb, 3.0), "{n:?}");
        assert_eq!(f, Fit::Tight);
    }
    assert!(!hw(8.0, "cuda").cpu_only());
}
