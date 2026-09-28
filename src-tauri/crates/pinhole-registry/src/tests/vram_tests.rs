use super::*;
use crate::vram::*;
use crate::wiring::{gpu_resident_components, HwContext};

const GIB: u64 = 1024 * 1024 * 1024;

fn need(gb: f32, min_gb: f32) -> VramNeed {
    VramNeed {
        gb,
        min_gb,
        estimate: false,
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
    let n = estimate(reg, sdxl, 13 * GIB / 2, 5 * GIB / 16);
    assert!(n.estimate);
    assert!(approx(n.gb, 9.4), "{n:?}"); // 9.3125 rounded up
    assert!(approx(n.min_gb, 5.8), "{n:?}"); // 3.25 + 2 + 0.5 = 5.75 rounded up
    assert_eq!(fit(&n, 12.0), Fit::Fits);
    assert_eq!(fit(&n, 8.0), Fit::Tight);
    assert_eq!(fit(&n, 4.0), Fit::TooBig);

    // Flux Q4 (≈6.4 GiB) with clip_l + t5 fp8 + VAE on the GPU.
    let flux = reg.family("flux1_dev").unwrap();
    let n = estimate(
        reg,
        flux,
        6_931_817_760,
        335_304_388 + 246_144_152 + 4_893_934_904,
    );
    assert!(n.gb > n.min_gb);
    assert_eq!(fit(&n, 8.0), Fit::Tight);
    assert_eq!(fit(&n, 16.0), Fit::Fits);
    assert_eq!(
        estimate(reg, flux, 0, 0).min_gb,
        estimate(reg, flux, 0, 0).gb
    );
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
            estimate: false
        })
    );
    assert_eq!(
        registry_need(z, Some("q4_k")),
        Some(VramNeed {
            gb: 8.0,
            min_gb: 5.0,
            estimate: false
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
