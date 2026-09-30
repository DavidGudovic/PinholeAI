use std::collections::BTreeMap;
use std::path::PathBuf;

use super::*;
use crate::wiring::*;
use crate::{Family, Layout};

fn hw(vram: f32) -> HwContext {
    HwContext {
        vram_gb: vram,
        backend: if vram > 0.0 {
            "cuda".into()
        } else {
            "cpu".into()
        },
        ram_gb: 32.0,
    }
}

fn fam(id: &str) -> &'static Family {
    shipped()
        .family(id)
        .unwrap_or_else(|| panic!("family {id}"))
}

fn comps(v: &[RequiredComponent]) -> Vec<(&str, &str)> {
    v.iter()
        .map(|c| (c.kind.as_str(), c.component_id.as_str()))
        .collect()
}

fn files(family: &str, layout: Layout, main: &str, components: &[(&str, &str)]) -> ModelFiles {
    ModelFiles {
        family_id: family.into(),
        main: PathBuf::from(main),
        layout,
        components: components
            .iter()
            .map(|(k, p)| ((*k).to_string(), PathBuf::from(p)))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn dials(shape: Shape, quality: Quality, stick: f32, count: u32) -> Dials {
    Dials {
        shape,
        quality,
        stick,
        count,
    }
}

#[test]
fn required_components_resolve_vram_choices() {
    let reg = shipped();
    assert_eq!(
        comps(&required_components(reg, fam("flux1_dev"), &hw(8.0))),
        vec![
            ("vae", "flux_ae"),
            ("clip_l", "clip_l"),
            ("t5xxl", "t5xxl_fp8")
        ]
    );
    assert_eq!(
        comps(&required_components(reg, fam("flux1_dev"), &hw(16.0))),
        vec![
            ("vae", "flux_ae"),
            ("clip_l", "clip_l"),
            ("t5xxl", "t5xxl_fp16")
        ]
    );
    assert_eq!(
        comps(&required_components(reg, fam("flux1_dev"), &hw(15.99)))[2],
        ("t5xxl", "t5xxl_fp8")
    );
    assert_eq!(
        comps(&required_components(reg, fam("sdxl"), &hw(8.0))),
        vec![("vae", "sdxl_vae_fp16_fix")]
    );
    assert_eq!(
        comps(&required_components(reg, fam("sdxl_pony"), &hw(8.0))),
        vec![("vae", "sdxl_vae_fp16_fix")]
    );
    assert!(required_components(reg, fam("sd15"), &hw(8.0)).is_empty());
    // Z-Image text encoder by VRAM: bf16 only at 20 GB+ (a 16 GB card ran out
    // of VRAM encoding the prompt with bf16 + bf16), GGUF Q8 from 10 GB, Q4_K_M below.
    for (vram, te) in [
        (24.0, "qwen3_4b"),
        (20.0, "qwen3_4b"),
        (16.0, "qwen3_4b_q8"),
        (15.9, "qwen3_4b_q8"),
        (12.0, "qwen3_4b_q8"),
        (10.0, "qwen3_4b_q8"),
        (8.0, "qwen3_4b_q4km"),
        (0.0, "qwen3_4b_q4km"),
    ] {
        for f in ["z_image_turbo", "z_image_base", "flux2_klein_4b"] {
            let want = if f == "flux2_klein_4b" {
                "flux2_vae"
            } else {
                "flux_ae"
            };
            assert_eq!(
                comps(&required_components(reg, fam(f), &hw(vram))),
                vec![("vae", want), ("llm", te)],
                "{f} at {vram} GB"
            );
        }
    }
    assert_eq!(
        comps(&required_components(
            reg,
            fam("qwen_image_edit_2511"),
            &hw(16.0)
        )),
        vec![
            ("vae", "qwen_image_vae"),
            ("llm", "qwen25_vl_7b_q8"),
            ("llm_vision", "qwen25_vl_7b_mmproj")
        ]
    );
    assert_eq!(
        comps(&required_components(reg, fam("qwen_image"), &hw(16.0))),
        vec![("vae", "qwen_image_vae"), ("llm", "qwen25_vl_7b_q8")]
    );

    // Several thresholds: the highest one that fits wins.
    use crate::ComponentChoice;
    let choice = ComponentChoice::ByVram(
        [
            ("vram_gte_8", "mid"),
            ("vram_gte_16", "big"),
            ("else", "small"),
        ]
        .iter()
        .map(|(k, v)| ((*k).into(), (*v).into()))
        .collect(),
    );
    assert_eq!(resolve_choice(&choice, 4.0).as_deref(), Some("small"));
    assert_eq!(resolve_choice(&choice, 12.0).as_deref(), Some("mid"));
    assert_eq!(resolve_choice(&choice, 24.0).as_deref(), Some("big"));
}

#[test]
fn taesd_is_only_wired_when_the_engine_supports_previews() {
    let reg = shipped();
    assert!(optional_components(reg, fam("flux1_dev")).is_empty());
    let f = files(
        "z_image_turbo",
        Layout::DiffusionOnly,
        "/m/z.safetensors",
        &[("vae", "/c/ae"), ("llm", "/c/q"), ("taesd", "/c/taef1")],
    );
    let extras = LaunchExtras {
        use_taesd: true,
        ..Default::default()
    };
    assert!(!launch_args(reg, &f, &hw(16.0), &extras).contains(&"--taesd".to_string()));

    let on = with_overrides("engine_features: { taesd_preview: true }\n");
    assert_eq!(
        comps(&optional_components(&on, on.family("flux1_dev").unwrap())),
        vec![("taesd", "taef1")]
    );
    assert!(optional_components(&on, on.family("sd15").unwrap()).is_empty());
    let args = launch_args(&on, &f, &hw(16.0), &extras);
    let i = args.iter().position(|a| a == "--taesd").unwrap();
    assert_eq!(args[i + 1], "/c/taef1");
}

#[test]
fn installed_option_of_a_vram_choice_is_used() {
    // Regression: a 16 GB user who installed the bf16 Qwen3-4B before the tier
    // moved to Q8 must not be asked to download another text encoder.
    let reg = shipped();
    let llm = |installed: &[&str], vram: f32| {
        let is = |id: &str| installed.contains(&id);
        required_components_with(reg, fam("z_image_turbo"), &hw(vram), &is)
            .into_iter()
            .find(|c| c.kind == "llm")
            .map(|c| c.component_id)
            .unwrap()
    };
    assert_eq!(
        llm(&[], 16.0),
        "qwen3_4b_q8",
        "nothing installed: the pick for this VRAM"
    );
    assert_eq!(
        llm(&["qwen3_4b_q8", "qwen3_4b"], 16.0),
        "qwen3_4b_q8",
        "the pick wins when installed"
    );
    assert_eq!(
        llm(&["qwen3_4b"], 16.0),
        "qwen3_4b",
        "a larger installed option"
    );
    assert_eq!(
        llm(&["qwen3_4b", "qwen3_4b_q4km"], 16.0),
        "qwen3_4b_q4km",
        "closest smaller first"
    );
    assert_eq!(
        llm(&["qwen3_4b_q8"], 8.0),
        "qwen3_4b_q8",
        "closest larger when nothing smaller"
    );
    // Fixed components are unaffected.
    assert_eq!(
        comps(&required_components_with(
            reg,
            fam("z_image_turbo"),
            &hw(16.0),
            &|_| false
        )),
        comps(&required_components(reg, fam("z_image_turbo"), &hw(16.0)))
    );
}

#[test]
fn launch_args_z_image_turbo_16gb_cuda() {
    // Real-GPU report: bf16 Z-Image (12.3 GB) + the bf16 Qwen3-4B (8 GB) ran out
    // of VRAM on a 16 GB card. Below 20 GB: Q8_0 GGUF model + Q8_0 GGUF encoder.
    let reg = shipped();
    let hw16 = hw(16.0);
    let te: Vec<(String, String)> = required_components(reg, fam("z_image_turbo"), &hw16)
        .into_iter()
        .map(|c| (c.kind, c.component_id))
        .collect();
    assert_eq!(
        te,
        vec![
            ("vae".to_string(), "flux_ae".to_string()),
            ("llm".to_string(), "qwen3_4b_q8".to_string())
        ]
    );
    assert_eq!(
        reg.component("qwen3_4b_q8").unwrap().file,
        "Qwen3-4B-Q8_0.gguf"
    );
    assert_eq!(
        reg.hardware_profile(16.0).prefer_quant.as_deref(),
        Some("q8_0")
    );
    let f = files(
        "z_image_turbo",
        Layout::DiffusionOnly,
        "/models/diffusion/z_image_turbo-Q8_0.gguf",
        &[
            ("llm", "/models/text_encoders/Qwen3-4B-Q8_0.gguf"),
            ("vae", "/models/vae/ae.safetensors"),
        ],
    );
    let extras = LaunchExtras {
        lora_dir: Some(PathBuf::from("/data/models/loras")),
        ..Default::default()
    };
    assert_eq!(
        launch_args(reg, &f, &hw16, &extras),
        vec![
            "--diffusion-model",
            "/models/diffusion/z_image_turbo-Q8_0.gguf",
            "--vae",
            "/models/vae/ae.safetensors",
            "--llm",
            "/models/text_encoders/Qwen3-4B-Q8_0.gguf",
            "--diffusion-fa",
            "--lora-model-dir",
            "/data/models/loras",
        ]
    );
    // 24 GB: bf16 model + bf16 encoder, as before.
    assert_eq!(
        wired("z_image_turbo", "/m/z_image_turbo_bf16.safetensors", 24.0),
        vec![
            "--diffusion-model",
            "/m/z_image_turbo_bf16.safetensors",
            "--vae",
            "/c/flux_ae",
            "--llm",
            "/c/qwen3_4b",
            "--diffusion-fa",
        ]
    );
}

#[test]
fn launch_args_flux1_dev_8gb() {
    let f = files(
        "flux1_dev",
        Layout::DiffusionOnly,
        "/m/flux1-dev-q4_k.gguf",
        &[
            ("t5xxl", "/c/t5xxl_fp8_e4m3fn.safetensors"),
            ("clip_l", "/c/clip_l.safetensors"),
            ("vae", "/c/ae.safetensors"),
        ],
    );
    assert_eq!(
        launch_args(shipped(), &f, &hw(8.0), &LaunchExtras::default()),
        vec![
            "--diffusion-model",
            "/m/flux1-dev-q4_k.gguf",
            "--vae",
            "/c/ae.safetensors",
            "--clip_l",
            "/c/clip_l.safetensors",
            "--t5xxl",
            "/c/t5xxl_fp8_e4m3fn.safetensors",
            "--diffusion-fa",
            "--vae-tiling",
        ]
    );
}

#[test]
fn launch_args_other_families_and_overrides() {
    let reg = shipped();
    let q = files(
        "qwen_image_edit_2511",
        Layout::DiffusionOnly,
        "/m/q.gguf",
        &[("vae", "/c/qv"), ("llm", "/c/ql"), ("llm_vision", "/c/qm")],
    );
    let extras = LaunchExtras {
        upscalers_dir: Some(PathBuf::from("/u")),
        ..Default::default()
    };
    assert_eq!(
        launch_args(reg, &q, &hw(16.0), &extras),
        vec![
            "--diffusion-model",
            "/m/q.gguf",
            "--vae",
            "/c/qv",
            "--llm",
            "/c/ql",
            "--llm_vision",
            "/c/qm",
            "--diffusion-fa",
            "--model-args",
            "qwen_image_zero_cond_t=true",
            "--hires-upscalers-dir",
            "/u",
        ]
    );

    let x = files(
        "sdxl",
        Layout::AllInOne,
        "/m/juggernaut.safetensors",
        &[("vae", "/c/fix")],
    );
    assert_eq!(
        launch_args(reg, &x, &hw(6.0), &LaunchExtras::default()),
        vec![
            "--model",
            "/m/juggernaut.safetensors",
            "--vae",
            "/c/fix",
            "--diffusion-fa",
            "--vae-tiling"
        ]
    );
    let no_tiling = LaunchExtras {
        vae_tiling: Some(false),
        ..Default::default()
    };
    assert_eq!(
        launch_args(reg, &x, &hw(6.0), &no_tiling),
        vec![
            "--model",
            "/m/juggernaut.safetensors",
            "--vae",
            "/c/fix",
            "--diffusion-fa"
        ]
    );
    let tiling = LaunchExtras {
        vae_tiling: Some(true),
        ..Default::default()
    };
    assert_eq!(
        launch_args(reg, &x, &hw(24.0), &tiling)
            .last()
            .map(String::as_str),
        Some("--vae-tiling")
    );

    // CPU only → low tier.
    let s = files("sd15", Layout::AllInOne, "/m/sd15.safetensors", &[]);
    assert_eq!(
        launch_args(reg, &s, &hw(0.0), &LaunchExtras::default()),
        vec![
            "--model",
            "/m/sd15.safetensors",
            "--diffusion-fa",
            "--vae-tiling"
        ]
    );

    // Profile flags merge with family flags: bools de-dup, list options comma-merge.
    let over = with_overrides(
        "hardware_profiles:\n  - { name: all, max_vram_gb: 999, flags: [\"--diffusion-fa\", \"--model-args\", \"foo=1\", \"--max-vram\", \"-1\"] }\n",
    );
    assert_eq!(
        launch_args(&over, &q, &hw(16.0), &LaunchExtras::default()),
        vec![
            "--diffusion-model",
            "/m/q.gguf",
            "--vae",
            "/c/qv",
            "--llm",
            "/c/ql",
            "--llm_vision",
            "/c/qm",
            "--diffusion-fa",
            "--model-args",
            "qwen_image_zero_cond_t=true,foo=1",
            "--max-vram",
            "-1",
        ]
    );
    // Unknown component kinds get no flag; unknown families only get the files.
    let odd = files(
        "nope",
        Layout::DiffusionOnly,
        "/m/x",
        &[("upscaler", "/c/u")],
    );
    assert_eq!(
        launch_args(reg, &odd, &hw(16.0), &LaunchExtras::default()),
        vec!["--diffusion-model", "/m/x"]
    );
}

#[test]
fn every_emitted_flag_exists_in_the_engine() {
    let reg = shipped();
    for f in reg.families() {
        let mut components = BTreeMap::new();
        for c in required_components(reg, f, &hw(8.0)) {
            components.insert(c.kind, PathBuf::from("/c/x"));
        }
        let mf = ModelFiles {
            family_id: f.id.clone(),
            main: "/m".into(),
            layout: f.layout,
            components,
        };
        let extras = LaunchExtras {
            lora_dir: Some("/l".into()),
            upscalers_dir: Some("/u".into()),
            vae_tiling: Some(true),
            use_taesd: true,
        };
        for vram in [0.0, 6.0, 10.0, 16.0, 32.0] {
            for a in launch_args(reg, &mf, &hw(vram), &extras) {
                if a.starts_with("--") {
                    assert!(is_known_flag(&a), "{}: {a}", f.id);
                }
            }
        }
    }
}

#[test]
fn resolve_params_sdxl_defaults_and_stick() {
    let reg = shipped();
    let sdxl = fam("sdxl");
    let fine = FineTune::default();
    let p = resolve_params(
        reg,
        sdxl,
        &dials(Shape::Square, Quality::Balanced, 0.5, 2),
        &fine,
        GenMode::Txt2img,
        &hw(8.0),
    );
    assert_eq!(
        p,
        ResolvedParams {
            width: 1024,
            height: 1024,
            steps: 30,
            cfg: 6.0,
            guidance: None,
            sampler: Some("dpm++2m".into()),
            scheduler: Some("karras".into()),
            clip_skip: Some(1),
            flow_shift: None,
            hires: None,
            vae_tiling: true,
            batch_count: 2,
        }
    );
    let at = |stick: f32| {
        resolve_params(
            reg,
            sdxl,
            &dials(Shape::Portrait, Quality::Fast, stick, 1),
            &fine,
            GenMode::Txt2img,
            &hw(16.0),
        )
    };
    assert!(approx(at(0.0).cfg, 3.0));
    assert!(approx(at(1.0).cfg, 9.0));
    assert!(approx(at(7.0).cfg, 9.0)); // clamped
    assert!(approx(at(f32::NAN).cfg, 6.0));
    assert_eq!(
        (at(0.0).width, at(0.0).height, at(0.0).steps),
        (832, 1216, 20)
    );
    assert!(!at(0.0).vae_tiling);
    // Pony: own sampler / clip skip / range.
    let pony = resolve_params(
        reg,
        fam("sdxl_pony"),
        &dials(Shape::Wide, Quality::Best, 1.0, 4),
        &fine,
        GenMode::Txt2img,
        &hw(16.0),
    );
    assert_eq!((pony.width, pony.height, pony.steps), (1344, 768, 40));
    assert!(approx(pony.cfg, 8.0));
    assert_eq!(pony.sampler.as_deref(), Some("euler_a"));
    assert_eq!(pony.clip_skip, Some(2));
    assert_eq!(pony.batch_count, 4);
    assert!(pony.hires.is_none());
}

#[test]
fn resolve_params_hires_at_best() {
    let reg = shipped();
    let sd15 = fam("sd15");
    let fine = FineTune::default();
    let best = resolve_params(
        reg,
        sd15,
        &dials(Shape::Wide, Quality::Best, 0.5, 1),
        &fine,
        GenMode::Txt2img,
        &hw(8.0),
    );
    assert_eq!((best.width, best.height, best.steps), (896, 512, 35));
    assert_eq!(
        best.hires,
        Some(HiresParams {
            scale: 1.5,
            denoising_strength: 0.45,
            steps: 0
        })
    );
    assert!(resolve_params(
        reg,
        sd15,
        &dials(Shape::Square, Quality::Balanced, 0.5, 1),
        &fine,
        GenMode::Txt2img,
        &hw(8.0)
    )
    .hires
    .is_none());
    assert!(resolve_params(
        reg,
        sd15,
        &dials(Shape::Square, Quality::Best, 0.5, 1),
        &fine,
        GenMode::Img2img,
        &hw(8.0)
    )
    .hires
    .is_none());
    let off = FineTune {
        hires: Some(false),
        ..Default::default()
    };
    assert!(resolve_params(
        reg,
        sd15,
        &dials(Shape::Square, Quality::Best, 0.5, 1),
        &off,
        GenMode::Txt2img,
        &hw(8.0)
    )
    .hires
    .is_none());
    let forced = FineTune {
        hires: Some(true),
        hires_scale: Some(2.0),
        hires_denoise: Some(0.3),
        ..Default::default()
    };
    let p = resolve_params(
        reg,
        fam("sdxl"),
        &dials(Shape::Square, Quality::Fast, 0.5, 1),
        &forced,
        GenMode::Txt2img,
        &hw(8.0),
    );
    assert_eq!(
        p.hires,
        Some(HiresParams {
            scale: 2.0,
            denoising_strength: 0.3,
            steps: 0
        })
    );
    assert!(resolve_params(
        reg,
        fam("sd15_fast"),
        &dials(Shape::Square, Quality::Best, 0.5, 1),
        &fine,
        GenMode::Txt2img,
        &hw(8.0)
    )
    .hires
    .is_none());
}

#[test]
fn resolve_params_guidance_families_and_cfg_fixed() {
    let reg = shipped();
    let fine = FineTune::default();
    let flux = |stick: f32| {
        resolve_params(
            reg,
            fam("flux1_dev"),
            &dials(Shape::Square, Quality::Balanced, stick, 1),
            &fine,
            GenMode::Txt2img,
            &hw(16.0),
        )
    };
    assert!(approx(flux(0.0).guidance.unwrap(), 2.0));
    assert!(approx(flux(0.5).guidance.unwrap(), 3.5));
    assert!(approx(flux(1.0).guidance.unwrap(), 5.0));
    assert!(approx(flux(1.0).cfg, 1.0));
    assert_eq!(flux(0.5).scheduler.as_deref(), Some("flux"));
    assert_eq!(flux(0.5).steps, 24);

    let schnell = resolve_params(
        reg,
        fam("flux1_schnell"),
        &dials(Shape::Square, Quality::Fast, 1.0, 1),
        &fine,
        GenMode::Txt2img,
        &hw(16.0),
    );
    assert_eq!((schnell.steps, schnell.guidance), (2, None));
    assert!(approx(schnell.cfg, 1.0));

    for stick in [0.0, 1.0] {
        let z = resolve_params(
            reg,
            fam("z_image_turbo"),
            &dials(Shape::Square, Quality::Balanced, stick, 1),
            &fine,
            GenMode::Txt2img,
            &hw(16.0),
        );
        assert!(approx(z.cfg, 1.0));
        assert_eq!(
            (
                z.guidance,
                z.steps,
                z.sampler.as_deref(),
                z.scheduler.as_deref()
            ),
            (None, 8, Some("euler"), Some("simple"))
        );
    }
    let zb = resolve_params(
        reg,
        fam("z_image_base"),
        &dials(Shape::Square, Quality::Fast, 1.0, 1),
        &fine,
        GenMode::Txt2img,
        &hw(16.0),
    );
    assert!(approx(zb.cfg, 7.0));
    assert_eq!(zb.steps, 20);
}

#[test]
fn resolve_params_edit_stay_close_is_inverted() {
    let reg = shipped();
    let fine = FineTune::default();
    let q = |stick: f32| {
        resolve_params(
            reg,
            fam("qwen_image_edit_2511"),
            &dials(Shape::Square, Quality::Balanced, stick, 1),
            &fine,
            GenMode::Edit,
            &hw(16.0),
        )
    };
    assert!(approx(q(1.0).cfg, 1.5)); // stay very close → low CFG
    assert!(approx(q(0.0).cfg, 4.0));
    assert_eq!(q(0.5).flow_shift, Some(3.0));
    assert_eq!(q(0.5).guidance, None);
    assert_eq!((q(0.5).width, q(0.5).height), (1024, 1024));
    assert!(q(0.5).hires.is_none());

    let k = |stick: f32| {
        resolve_params(
            reg,
            fam("flux1_kontext"),
            &dials(Shape::Square, Quality::Best, stick, 1),
            &fine,
            GenMode::Edit,
            &hw(8.0),
        )
    };
    assert!(approx(k(1.0).guidance.unwrap(), 1.5));
    assert!(approx(k(0.0).guidance.unwrap(), 4.0));
    assert!(approx(k(0.3).cfg, 1.0));
    assert_eq!(k(0.3).steps, 30);
}

#[test]
fn resolve_params_fine_tune_overrides() {
    let reg = shipped();
    let fine = FineTune {
        sampler: Some("euler".into()),
        scheduler: Some("simple".into()),
        steps: Some(12),
        cfg: Some(4.2),
        guidance: Some(2.2),
        seed: Some(7),
        flow_shift: Some(2.0),
        clip_skip: Some(2),
        width: Some(1000),
        height: Some(700),
        vae_tiling: Some(false),
        ..Default::default()
    };
    let p = resolve_params(
        reg,
        fam("sdxl"),
        &dials(Shape::Square, Quality::Best, 0.0, 0),
        &fine,
        GenMode::Txt2img,
        &hw(6.0),
    );
    assert_eq!((p.width, p.height), (1024, 704)); // multiples of 64
    assert_eq!(p.steps, 12);
    assert!(approx(p.cfg, 4.2));
    assert_eq!(p.guidance, Some(2.2));
    assert_eq!(
        (p.sampler.as_deref(), p.scheduler.as_deref()),
        (Some("euler"), Some("simple"))
    );
    assert_eq!((p.clip_skip, p.flow_shift), (Some(2), Some(2.0)));
    assert!(!p.vae_tiling);
    assert_eq!(p.batch_count, 1);
    // cfg override beats cfg_fixed; DiT sizes round to 16.
    let f = resolve_params(
        reg,
        fam("flux1_dev"),
        &dials(Shape::Square, Quality::Best, 0.0, 1),
        &fine,
        GenMode::Txt2img,
        &hw(16.0),
    );
    assert_eq!((f.width, f.height), (1008, 704));
    assert!(approx(f.cfg, 4.2));
    // Tiny / huge sizes are clamped.
    let tiny = FineTune {
        width: Some(3),
        height: Some(100_000),
        ..Default::default()
    };
    let t = resolve_params(
        reg,
        fam("sd15"),
        &dials(Shape::Square, Quality::Fast, 0.5, 1),
        &tiny,
        GenMode::Txt2img,
        &hw(8.0),
    );
    assert_eq!((t.width, t.height), (64, 4096));
    assert_eq!(round_to_multiple(512, 64), 512);
    assert_eq!(round_to_multiple(543, 64), 512);
    assert_eq!(round_to_multiple(544, 64), 576);
}

#[test]
fn family_ui_fields() {
    let reg = shipped();
    let ui = family_ui(reg, fam("sdxl"));
    assert_eq!(ui.family_id, "sdxl");
    assert_eq!(ui.quality_steps, [20, 30, 40]);
    assert!(ui.show_stick && ui.stick_maps_to == "cfg");
    assert_eq!(ui.stick_range, [3.0, 9.0]);
    assert!(approx(ui.stick_default, 0.5));
    assert!(approx(ui.default_cfg, 6.0));
    assert!(ui.uses_negative_prompt && ui.default_negative_prompt.is_some());
    assert_eq!(ui.default_sampler.as_deref(), Some("dpm++2m"));
    assert!(!ui.hires_at_best && !ui.is_edit_family);
    assert_eq!(ui.shapes["square"], [1024, 1024]);

    let ui = family_ui(reg, fam("flux1_dev"));
    assert!(ui.show_stick && ui.stick_maps_to == "guidance");
    assert_eq!(ui.stick_range, [2.0, 5.0]);
    assert!(approx(ui.stick_default, 0.5));
    assert_eq!(ui.default_guidance, Some(3.5));
    assert!(approx(ui.default_cfg, 1.0));
    assert_eq!(ui.license_note.as_deref(), Some("Non-commercial license"));
    assert!(!ui.uses_negative_prompt && ui.default_negative_prompt.is_none());

    assert!(!family_ui(reg, fam("z_image_turbo")).show_stick);
    assert!(!family_ui(reg, fam("flux1_schnell")).show_stick);
    assert!(family_ui(reg, fam("sd15")).hires_at_best);
    assert!(family_ui(reg, fam("sdxl_pony"))
        .auto_prompt_prefix
        .is_some());

    let ui = family_ui(reg, fam("qwen_image_edit_2511"));
    assert!(ui.is_edit_family && ui.show_stick && ui.stick_maps_to == "cfg");
    assert!(approx(ui.stick_default, 0.6)); // inverted: 1 - (2.5 - 1.5) / 2.5
    assert_eq!(ui.default_flow_shift, Some(3.0));
    let ui = family_ui(reg, fam("flux1_kontext"));
    assert!(ui.is_edit_family && ui.stick_maps_to == "guidance");
    assert!(approx(ui.stick_default, 0.6));
}

/// The UI's default dial position must reproduce the registry defaults.
#[test]
fn stick_default_round_trips_for_every_family() {
    let reg = shipped();
    for f in reg.families() {
        let ui = family_ui(reg, f);
        let mode = if ui.is_edit_family {
            GenMode::Edit
        } else {
            GenMode::Txt2img
        };
        let p = resolve_params(
            reg,
            f,
            &dials(Shape::Square, Quality::Balanced, ui.stick_default, 1),
            &FineTune::default(),
            mode,
            &hw(16.0),
        );
        assert!(
            approx(p.cfg, ui.default_cfg),
            "{}: cfg {} vs {}",
            f.id,
            p.cfg,
            ui.default_cfg
        );
        if ui.show_stick && ui.stick_maps_to == "guidance" {
            assert!(
                approx(p.guidance.unwrap(), ui.default_guidance.unwrap()),
                "{}",
                f.id
            );
        }
        assert_eq!(ui.quality_steps[1], p.steps, "{}", f.id);
    }
}

/// Generators that can also edit (FLUX.2) stay Create models, are offered in
/// Edit, and their "Stay close" default reproduces the registry defaults.
#[test]
fn generators_that_edit_keep_create_dials_and_get_an_edit_default() {
    let reg = shipped();
    for id in ["flux2_klein_4b", "flux2_klein_9b_base", "flux2_dev"] {
        let f = fam(id);
        assert!(can_edit(f) && !is_edit_family(f), "{id}");
    }
    assert!(can_edit(fam("qwen_image_edit_2511")) && is_edit_family(fam("flux1_kontext")));
    assert!(!can_edit(fam("sdxl")) && !can_edit(fam("qwen_image")));

    // klein distilled: fixed CFG, no guidance → no dial to show.
    assert!(!family_ui(reg, fam("flux2_klein_4b")).stay_close_shown);
    let ui = family_ui(reg, fam("flux2_klein_9b_base"));
    assert!(ui.stay_close_shown && !ui.is_edit_family);
    assert!(approx(ui.stay_close_default, 1.0 - ui.stick_default));

    for f in reg.families().filter(|f| can_edit(f)) {
        let ui = family_ui(reg, f);
        let p = resolve_params(
            reg,
            f,
            &dials(Shape::Square, Quality::Balanced, ui.stay_close_default, 1),
            &FineTune::default(),
            GenMode::Edit,
            &hw(16.0),
        );
        assert!(approx(p.cfg, ui.default_cfg), "{}: cfg {}", f.id, p.cfg);
        if let (true, Some(g)) = (ui.stay_close_shown, f.defaults.guidance) {
            assert!(approx(p.guidance.unwrap(), g), "{}", f.id);
        }
    }
}

// ----------------------------------------------------------------------------- families added 2026-09-28

/// Required components for `family` at `vram`, wired to `/c/<component id>`.
fn wired(family: &str, main: &str, vram: f32) -> Vec<String> {
    let reg = shipped();
    let f = fam(family);
    let comps: Vec<(String, String)> = required_components(reg, f, &hw(vram))
        .into_iter()
        .map(|c| (c.kind, format!("/c/{}", c.component_id)))
        .collect();
    let comps: Vec<(&str, &str)> = comps
        .iter()
        .map(|(k, p)| (k.as_str(), p.as_str()))
        .collect();
    launch_args(
        reg,
        &files(family, f.layout, main, &comps),
        &hw(vram),
        &LaunchExtras::default(),
    )
}

#[test]
fn launch_args_krea2() {
    // docs/krea2.md: --diffusion-model + --llm Qwen3-VL-4B + --vae (Wan 2.1 layout) + --diffusion-fa.
    assert_eq!(
        wired(
            "krea2_turbo",
            "/m/krea2_turbo_int8_convrot.safetensors",
            16.0
        ),
        vec![
            "--diffusion-model",
            "/m/krea2_turbo_int8_convrot.safetensors",
            "--vae",
            "/c/qwen_image_vae",
            "--llm",
            "/c/qwen3vl_4b_q8",
            "--diffusion-fa",
        ]
    );
    // Small cards: Q4_K_M text encoder + VAE tiling from the low tier.
    assert_eq!(
        wired("krea2_raw", "/m/Krea-2-Base-Q4_K_M.gguf", 8.0),
        vec![
            "--diffusion-model",
            "/m/Krea-2-Base-Q4_K_M.gguf",
            "--vae",
            "/c/qwen_image_vae",
            "--llm",
            "/c/qwen3vl_4b_q4km",
            "--diffusion-fa",
            "--vae-tiling",
        ]
    );
    let reg = shipped();
    let p = resolve_params(
        reg,
        fam("krea2_turbo"),
        &dials(Shape::Square, Quality::Balanced, 0.5, 1),
        &FineTune::default(),
        GenMode::Txt2img,
        &hw(16.0),
    );
    assert_eq!((p.steps, p.cfg, p.width, p.height), (8, 1.0, 1024, 1024));
    assert_eq!(p.sampler.as_deref(), Some("euler"));
    assert_eq!(
        p.scheduler, None,
        "engine default + its Krea 2 flow shift (1.15)"
    );
    let raw = resolve_params(
        reg,
        fam("krea2_raw"),
        &dials(
            Shape::Square,
            Quality::Best,
            family_ui(reg, fam("krea2_raw")).stick_default,
            1,
        ),
        &FineTune::default(),
        GenMode::Txt2img,
        &hw(16.0),
    );
    assert_eq!(raw.steps, 52);
    assert!(approx(raw.cfg, 3.5));
}

#[test]
fn launch_args_anima() {
    assert_eq!(
        wired("anima", "/m/anima-base-v1.0.safetensors", 8.0),
        vec![
            "--diffusion-model",
            "/m/anima-base-v1.0.safetensors",
            "--vae",
            "/c/qwen_image_vae",
            "--llm",
            "/c/qwen3_06b_base",
            "--diffusion-fa",
            "--vae-tiling",
        ]
    );
    let reg = shipped();
    let ui = family_ui(reg, fam("anima"));
    assert_eq!(
        ui.auto_prompt_prefix.as_deref(),
        Some("masterpiece, best quality, score_7, safe, ")
    );
    assert!(ui.uses_negative_prompt && ui.default_negative_prompt.is_some());
    assert_eq!(
        reg.style_template(&fam("anima").style_template),
        Some("{prompt}, {style}")
    );
    let turbo = family_ui(reg, fam("anima_turbo"));
    assert!(!turbo.show_stick && !turbo.uses_negative_prompt);
    assert_eq!(turbo.quality_steps, [8, 10, 12]);
    assert!(approx(turbo.default_cfg, 1.0));
}

#[test]
fn launch_args_flux2_klein_and_dev() {
    assert_eq!(
        wired("flux2_klein_4b", "/m/flux-2-klein-4b-Q8_0.gguf", 16.0),
        vec![
            "--diffusion-model",
            "/m/flux-2-klein-4b-Q8_0.gguf",
            "--vae",
            "/c/flux2_vae",
            "--llm",
            "/c/qwen3_4b_q8",
            "--diffusion-fa",
        ]
    );
    assert_eq!(
        wired("flux2_klein_9b", "/m/flux-2-klein-9b-Q8_0.gguf", 24.0),
        vec![
            "--diffusion-model",
            "/m/flux-2-klein-9b-Q8_0.gguf",
            "--vae",
            "/c/flux2_vae",
            "--llm",
            "/c/qwen3_8b_q8",
            "--diffusion-fa",
        ]
    );
    assert_eq!(
        wired("flux2_klein_9b_base", "/m/k9b.safetensors", 16.0)[5],
        "/c/qwen3_8b_q4km"
    );
    assert_eq!(
        wired("flux2_dev", "/m/flux2-dev-Q4_K_M.gguf", 24.0),
        vec![
            "--diffusion-model",
            "/m/flux2-dev-Q4_K_M.gguf",
            "--vae",
            "/c/flux2_vae",
            "--llm",
            "/c/mistral_small_32_q4km",
            "--diffusion-fa",
        ]
    );
    let reg = shipped();
    let d = dials(Shape::Portrait, Quality::Balanced, 0.5, 2);
    let k = resolve_params(
        reg,
        fam("flux2_klein_4b"),
        &d,
        &FineTune::default(),
        GenMode::Txt2img,
        &hw(16.0),
    );
    assert_eq!(
        (k.steps, k.cfg, k.guidance, k.batch_count),
        (4, 1.0, None, 2)
    );
    assert_eq!((k.width, k.height), (832, 1216));
    let b = family_ui(reg, fam("flux2_klein_4b_base"));
    assert!(b.show_stick && b.stick_maps_to == "cfg" && approx(b.default_cfg, 4.0));
    let dev = family_ui(reg, fam("flux2_dev"));
    assert!(dev.show_stick && dev.stick_maps_to == "guidance");
    assert_eq!(dev.default_guidance, Some(4.0));
}

#[test]
fn launch_args_other_new_families() {
    // Qwen-Image 2.1: its own VAE, Qwen3-VL 8B, --fa (docs).
    assert_eq!(
        wired("qwen_image_21", "/m/qwen_image_2.1-Q4_K.gguf", 16.0),
        vec![
            "--diffusion-model",
            "/m/qwen_image_2.1-Q4_K.gguf",
            "--vae",
            "/c/qwen_image_21_vae",
            "--llm",
            "/c/qwen3vl_8b_q4km",
            "--fa",
        ]
    );
    let reg = shipped();
    // 32-pixel grid: 1000 → 992.
    let q = resolve_params(
        reg,
        fam("qwen_image_21"),
        &dials(Shape::Square, Quality::Balanced, 0.5, 1),
        &FineTune {
            width: Some(1000),
            height: Some(1000),
            ..Default::default()
        },
        GenMode::Txt2img,
        &hw(16.0),
    );
    assert_eq!((q.width, q.height), (992, 992));
    // SD 3.5: all-in-one checkpoint + CLIP-L, CLIP-G, T5 (fp8 below 24 GB).
    assert_eq!(
        wired("sd3", "/m/sd3.5_large.safetensors", 16.0),
        vec![
            "--model",
            "/m/sd3.5_large.safetensors",
            "--clip_l",
            "/c/clip_l",
            "--clip_g",
            "/c/clip_g",
            "--t5xxl",
            "/c/t5xxl_fp8",
            "--diffusion-fa",
        ]
    );
    // Chroma: T5 only, no CLIP-L.
    assert_eq!(
        wired("chroma", "/m/Chroma1-HD-Q8_0.gguf", 24.0),
        vec![
            "--diffusion-model",
            "/m/Chroma1-HD-Q8_0.gguf",
            "--vae",
            "/c/flux_ae",
            "--t5xxl",
            "/c/t5xxl_fp16",
            "--diffusion-fa",
        ]
    );
    // HiDream-O1: one file, --model, nothing else.
    assert_eq!(
        wired(
            "hidream_o1_dev",
            "/m/hidream_o1_image_dev_fp8_scaled.safetensors",
            24.0
        ),
        vec!["--model", "/m/hidream_o1_image_dev_fp8_scaled.safetensors"]
    );
    // ERNIE-Image and Mage-Flow.
    assert_eq!(
        wired("ernie_image_turbo", "/m/e.gguf", 12.0),
        vec![
            "--diffusion-model",
            "/m/e.gguf",
            "--vae",
            "/c/flux2_vae",
            "--llm",
            "/c/ministral3_3b_q8",
            "--diffusion-fa",
            "--vae-tiling",
        ]
    );
    assert_eq!(
        wired("mage_flow", "/m/mage_flow_int8_convrot.safetensors", 16.0),
        vec![
            "--diffusion-model",
            "/m/mage_flow_int8_convrot.safetensors",
            "--vae",
            "/c/mage_flow_vae",
            "--llm",
            "/c/qwen3vl_4b_q8",
            "--diffusion-fa",
        ]
    );
}

#[test]
fn minimax_h3_is_not_wired() {
    // The pinned engine only runs MiniMax-H3 through vid_gen (model.h
    // sd_version_supports_image_generation), so no family claims it.
    let reg = shipped();
    assert!(reg.families_for_base_model("MiniMax H3").is_empty());
    assert!(!reg
        .all_civitai_base_models()
        .contains(&"MiniMax H3".to_string()));
}

#[test]
fn weight_file_flags_cover_every_component_flag() {
    for f in WEIGHT_FILE_FLAGS {
        assert!(is_known_flag(f), "{f}");
    }
    for (_, f) in COMPONENT_FLAGS {
        assert!(WEIGHT_FILE_FLAGS.contains(f), "{f}");
    }
}

/// Hires ×2 of an SDXL copied example (896×1152 → 1792×2304) ran out of memory
/// encoding the upscaled picture: a big output tiles the VAE from the start;
/// Fine-tune "VAE tiling: Off" still wins, and a normal size stays untiled.
#[test]
fn big_hires_output_tiles_the_vae() {
    let reg = shipped();
    let at = |hires: Option<bool>, scale: Option<f32>, tiling: Option<bool>| {
        let fine = FineTune {
            width: Some(896),
            height: Some(1152),
            hires,
            hires_scale: scale,
            vae_tiling: tiling,
            ..Default::default()
        };
        resolve_params(
            reg,
            fam("sdxl"),
            &dials(Shape::Portrait, Quality::Balanced, 0.5, 0),
            &fine,
            GenMode::Txt2img,
            &hw(16.0),
        )
    };
    let big = at(Some(true), Some(2.0), None);
    assert!(big.hires.is_some() && big.vae_tiling);
    assert!(!at(Some(true), Some(2.0), Some(false)).vae_tiling);
    assert!(!at(Some(false), None, None).vae_tiling);
    assert!(!at(Some(true), Some(1.25), None).vae_tiling, "1.6 MP");
}
