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
    assert_eq!(
        comps(&required_components(reg, fam("z_image_turbo"), &hw(16.0))),
        vec![("vae", "flux_ae"), ("llm", "qwen3_4b")]
    );
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
fn launch_args_z_image_turbo_16gb_cuda() {
    let f = files(
        "z_image_turbo",
        Layout::DiffusionOnly,
        "/models/diffusion/z_image_turbo_bf16.safetensors",
        &[
            ("llm", "/models/text_encoders/qwen_3_4b.safetensors"),
            ("vae", "/models/vae/ae.safetensors"),
        ],
    );
    let extras = LaunchExtras {
        lora_dir: Some(PathBuf::from("/data/models/loras")),
        ..Default::default()
    };
    assert_eq!(
        launch_args(shipped(), &f, &hw(16.0), &extras),
        vec![
            "--diffusion-model",
            "/models/diffusion/z_image_turbo_bf16.safetensors",
            "--vae",
            "/models/vae/ae.safetensors",
            "--llm",
            "/models/text_encoders/qwen_3_4b.safetensors",
            "--diffusion-fa",
            "--lora-model-dir",
            "/data/models/loras",
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
            "--vae-tiling"
        ]
    );
    let no_tiling = LaunchExtras {
        vae_tiling: Some(false),
        ..Default::default()
    };
    assert_eq!(
        launch_args(reg, &x, &hw(6.0), &no_tiling),
        vec!["--model", "/m/juggernaut.safetensors", "--vae", "/c/fix"]
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
        vec!["--model", "/m/sd15.safetensors", "--vae-tiling"]
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
