use super::*;
use crate::{ComponentChoice, Layout, RegistryError};

#[test]
fn shipped_registry_loads_and_validates() {
    let reg = shipped();
    let problems = reg.validate();
    assert!(
        problems.is_empty(),
        "models.yaml problems:\n{}",
        problems.join("\n")
    );

    for id in [
        "sd15",
        "sd15_fast",
        "sdxl",
        "sdxl_pony",
        "sdxl_illustrious",
        "sdxl_fast",
        "flux1_dev",
        "flux1_schnell",
        "z_image_turbo",
        "z_image_base",
        "qwen_image",
        "qwen_image_edit_2511",
        "flux1_kontext",
        "qwen_image_21",
        "flux2_klein_4b",
        "flux2_klein_4b_base",
        "flux2_klein_9b",
        "flux2_klein_9b_base",
        "flux2_dev",
        "krea2_turbo",
        "krea2_raw",
        "anima",
        "anima_turbo",
        "chroma",
        "sd3",
        "sd35_turbo",
        "hidream_o1",
        "hidream_o1_dev",
        "ernie_image",
        "ernie_image_turbo",
        "mage_flow",
        "mage_flow_turbo",
    ] {
        let f = reg.family(id).unwrap_or_else(|| panic!("family {id}"));
        assert_eq!(f.id, id);
    }
    // YAML order is kept.
    let order: Vec<&str> = reg.families_in_order().map(|f| f.id.as_str()).collect();
    assert_eq!(order.first(), Some(&"sd15"));
    assert!(order.iter().position(|i| *i == "sdxl") < order.iter().position(|i| *i == "sdxl_pony"));

    // Anchors (`*shapes_1024`) resolved.
    assert_eq!(
        reg.family("sdxl").unwrap().dials.shape["portrait"],
        [832, 1216]
    );
    assert_eq!(reg.family("sd15").unwrap().dials.shape["wide"], [896, 512]);

    // Every shipped Hugging Face file is pinned; only the github .pth upscaler is pending.
    for (id, c) in reg.components() {
        if c.url.contains("huggingface.co") {
            assert_eq!(c.sha256.len(), 64, "component {id} sha256");
        }
    }
    let sd15 = &reg.test_models()["sd15"];
    assert_eq!(sd15.family.as_deref(), Some("sd15"));
    assert_eq!(sd15.file, "v1-5-pruned-emaonly-fp16.safetensors");
    assert_eq!(sd15.sha256.len(), 64);
    assert!(!reg.engine_features().taesd_preview);
    assert_eq!(
        reg.style_template("natural"),
        Some("{prompt}. Style: {style}")
    );
    assert_eq!(
        reg.recommended()["edit"][0].family.as_deref(),
        Some("qwen_image_edit_2511")
    );
    assert!(reg.captioner().default.is_some());
}

#[test]
fn inherits_merges_parent_dials_with_own_defaults() {
    let reg = shipped();
    let sdxl = reg.family("sdxl").unwrap();
    let pony = reg.family("sdxl_pony").unwrap();
    assert_eq!(pony.inherits.as_deref(), Some("sdxl"));
    assert_eq!(pony.label, "SDXL · Pony");
    // Inherited from sdxl.
    let q = pony.dials.quality.as_ref().unwrap();
    assert_eq!((q.fast, q.balanced, q.best), (20, 30, 40));
    assert_eq!(pony.dials.shape, sdxl.dials.shape);
    assert_eq!(pony.layout, Layout::AllInOne);
    assert_eq!(pony.taesd.as_deref(), Some("taesdxl"));
    assert!(
        matches!(&pony.components["vae_override"], ComponentChoice::Fixed(c) if c == "sdxl_vae_fp16_fix")
    );
    assert!(!pony.dials.hires_at_best.as_ref().unwrap().enabled);
    assert_eq!(pony.style_template, "tags");
    // Own values.
    assert_eq!(pony.defaults.sampler.as_deref(), Some("euler_a"));
    assert_eq!(pony.defaults.scheduler.as_deref(), Some("discrete"));
    assert_eq!(pony.defaults.clip_skip, Some(2));
    assert!(pony
        .defaults
        .auto_prompt_prefix
        .as_deref()
        .unwrap()
        .starts_with("score_9"));
    assert_eq!(pony.dials.cfg_range, Some([4.0, 8.0]));
    assert_eq!(pony.civitai_base_models, vec!["Pony".to_string()]);
    assert!(sdxl.defaults.auto_prompt_prefix.is_none());

    // `download` is not inherited; nulls in the child clear parent values.
    let base = reg.family("z_image_base").unwrap();
    assert!(reg.family("z_image_turbo").unwrap().download.is_some());
    assert!(base.download.is_none());
    assert_eq!(base.dials.cfg_fixed, None);
    assert_eq!(base.dials.cfg_range, Some([3.0, 7.0]));
    assert_eq!(base.flags, vec!["--diffusion-fa".to_string()]);
    assert!(base.uses_negative_prompt);
    let schnell = reg.family("flux1_schnell").unwrap();
    assert_eq!(schnell.defaults.guidance, None);
    assert_eq!(schnell.defaults.sampler.as_deref(), Some("euler"));
    assert_eq!(schnell.license_note.as_deref(), Some("Apache 2.0"));
}

#[test]
fn multi_level_inherits_and_errors() {
    let yaml = r#"
style_templates: { tags: "{prompt}, {style}" }
families:
  a: { label: A, style_template: tags, layout: all_in_one, activation_gb: 1.0, dials: { quality: { fast: 1, balanced: 2, best: 3 } } }
  b: { inherits: a, label: B, dials: { cfg_default: 5.0 } }
  c: { inherits: b, label: C, dials: { quality: { best: 9 } } }
"#;
    let reg = Registry::from_yaml(yaml, None).unwrap();
    let c = reg.family("c").unwrap();
    assert_eq!(c.label, "C");
    assert_eq!(c.activation_gb, 1.0);
    assert_eq!(c.dials.cfg_default, Some(5.0));
    let q = c.dials.quality.as_ref().unwrap();
    assert_eq!((q.fast, q.balanced, q.best), (1, 2, 9));

    let cycle = r#"
families:
  a: { inherits: c, label: A, style_template: tags, layout: all_in_one }
  b: { inherits: a, label: B }
  c: { inherits: b, label: C }
"#;
    let err = Registry::from_yaml(cycle, None).unwrap_err();
    assert!(
        matches!(err, RegistryError::Invalid(ref m) if m.contains("cycle")),
        "{err}"
    );

    let unknown =
        "families:\n  a: { inherits: nope, label: A, style_template: tags, layout: all_in_one }\n";
    assert!(
        matches!(Registry::from_yaml(unknown, None), Err(RegistryError::BadInherit(a, p)) if a == "a" && p == "nope")
    );

    assert!(matches!(
        Registry::from_yaml("families: [1, 2]", None),
        Err(RegistryError::Yaml(_))
    ));
    assert!(
        matches!(Registry::from_yaml("families: {a: {label: A}}", None), Err(RegistryError::Yaml(m)) if m.contains("`a`"))
    );
    assert!(matches!(
        Registry::from_yaml(": : :", None),
        Err(RegistryError::Yaml(_))
    ));
}

#[test]
fn yaml_merge_keys_and_anchors() {
    let yaml = r#"
style_templates: { tags: "{prompt}, {style}" }
common: &common { style_template: tags, layout: all_in_one, activation_gb: 2.5 }
families:
  a:
    <<: *common
    label: A
    dials: { quality: &q { fast: 1, balanced: 2, best: 3 } }
  b:
    <<: *common
    label: B
    activation_gb: 4.0
    dials: { quality: *q }
"#;
    let reg = Registry::from_yaml(yaml, None).unwrap();
    assert_eq!(reg.family("a").unwrap().activation_gb, 2.5);
    assert_eq!(reg.family("b").unwrap().activation_gb, 4.0); // explicit key beats `<<`
    assert_eq!(
        reg.family("b")
            .unwrap()
            .dials
            .quality
            .as_ref()
            .unwrap()
            .best,
        3
    );
}

#[test]
fn overrides_deep_merge_user_wins() {
    let reg = with_overrides(
        r#"
components:
  my_vae: { kind: vae, file: my.safetensors, url: "https://huggingface.co/x/y/resolve/main/my.safetensors", sha256: TODO, size_mb: 1 }
families:
  sdxl:
    dials: { cfg_default: 7.5, quality: { best: 50 } }
    flags: ["--vae-conv-direct"]
  my_finetune:
    inherits: sdxl
    label: "My SDXL"
    civitai_base_models: ["My Base"]
    components: { vae_override: my_vae }
hardware_profiles:
  - { name: only, max_vram_gb: 999, flags: [] }
"#,
    );
    let sdxl = reg.family("sdxl").unwrap();
    assert_eq!(sdxl.dials.cfg_default, Some(7.5));
    let q = sdxl.dials.quality.as_ref().unwrap();
    assert_eq!((q.fast, q.balanced, q.best), (20, 30, 50)); // maps merge
    assert_eq!(sdxl.flags, vec!["--vae-conv-direct".to_string()]); // sequences replace
    assert_eq!(sdxl.label, "SDXL"); // untouched keys stay
                                    // Children see the overridden parent.
    assert_eq!(
        reg.family("sdxl_pony")
            .unwrap()
            .dials
            .quality
            .as_ref()
            .unwrap()
            .best,
        50
    );
    let mine = reg.family("my_finetune").unwrap();
    assert_eq!(mine.dials.cfg_default, Some(7.5));
    assert!(matches!(&mine.components["vae_override"], ComponentChoice::Fixed(c) if c == "my_vae"));
    assert_eq!(reg.families_for_base_model("my base")[0].id, "my_finetune");
    assert_eq!(reg.families_in_order().last().unwrap().id, "my_finetune");
    assert!(reg.component("my_vae").is_some() && reg.component("flux_ae").is_some());
    assert_eq!(reg.hardware_profiles().len(), 1); // lists replace
    assert!(reg.validate().is_empty(), "{:?}", reg.validate());

    // `null` removes a family / component.
    let removed =
        with_overrides("families: { sdxl_fast: null }\ncomponents: { realesrgan_x4: null }\n");
    assert!(removed.family("sdxl_fast").is_none() && removed.component("realesrgan_x4").is_none());
    assert!(removed.families_for_base_model("SDXL Lightning").is_empty());
    assert!(!removed.families_in_order().any(|f| f.id == "sdxl_fast"));

    // Empty / comment-only overrides are fine; a scalar top level is not.
    assert!(Registry::from_yaml(&shipped_yaml(), Some("# nothing\n")).is_ok());
    assert!(Registry::from_yaml(&shipped_yaml(), Some("42")).is_err());
}

#[test]
fn load_from_disk_with_and_without_overrides() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("models.yaml"), shipped_yaml()).unwrap();
    let missing = dir.path().join("overrides.yaml");
    assert!(Registry::load(dir.path(), Some(&missing)).is_ok());
    std::fs::write(&missing, "families: { sd15: { label: \"Mine\" } }\n").unwrap();
    assert_eq!(
        Registry::load(dir.path(), Some(&missing))
            .unwrap()
            .family("sd15")
            .unwrap()
            .label,
        "Mine"
    );
    let empty = tempfile::tempdir().unwrap();
    assert!(matches!(
        Registry::load(empty.path(), None),
        Err(RegistryError::Io { .. })
    ));
}

#[test]
fn same_as_copies_rules_but_keeps_family_distinct() {
    let reg = shipped();
    let dev = reg.family("flux1_dev").unwrap();
    let kontext = reg.family("flux1_kontext").unwrap();
    assert_eq!(kontext.detect.same_as.as_deref(), Some("flux1_dev"));
    assert_eq!(kontext.detect.any_tensor, dev.detect.any_tensor);
    assert_eq!(kontext.detect.none_tensor, dev.detect.none_tensor);
    assert_eq!(kontext.detect.all_of_any, dev.detect.all_of_any);
    assert_eq!(kontext.detect.tensor_ne0, dev.detect.tensor_ne0);
    assert!(kontext
        .detect
        .ambiguous_with
        .contains(&"flux1_dev".to_string()));
    assert!(dev
        .detect
        .ambiguous_with
        .contains(&"flux1_kontext".to_string()));
    assert!(!kontext
        .detect
        .ambiguous_with
        .contains(&"flux1_kontext".to_string()));
    // Kontext keeps its own everything else.
    assert_eq!(kontext.role.as_deref(), Some("edit"));
    assert_eq!(kontext.defaults.guidance, Some(2.5));

    // decisive_tensor is family-specific.
    let qi = reg.family("qwen_image").unwrap();
    let qe = reg.family("qwen_image_edit_2511").unwrap();
    assert!(qi.detect.decisive_tensor.is_empty());
    assert_eq!(
        qe.detect.decisive_tensor,
        vec!["__index_timestep_zero__".to_string()]
    );
    assert_eq!(qe.detect.any_tensor, qi.detect.any_tensor);

    // Symmetric ambiguity for the SDXL finetunes.
    let sdxl = reg.family("sdxl").unwrap();
    for f in ["sdxl_pony", "sdxl_illustrious", "sdxl_fast"] {
        assert!(sdxl.detect.ambiguous_with.contains(&f.to_string()), "{f}");
    }

    let bad = "families:\n  a: { label: A, style_template: tags, layout: all_in_one, detect: { same_as: zz } }\n";
    assert!(
        matches!(Registry::from_yaml(bad, None), Err(RegistryError::Invalid(m)) if m.contains("zz"))
    );
    let cyc = "families:\n  a: { label: A, style_template: tags, layout: all_in_one, detect: { same_as: b } }\n  b: { label: B, style_template: tags, layout: all_in_one, detect: { same_as: a } }\n";
    assert!(
        matches!(Registry::from_yaml(cyc, None), Err(RegistryError::Invalid(m)) if m.contains("cycle"))
    );
}

#[test]
fn base_model_lookups() {
    let reg = shipped();
    let ids = |b: &str| {
        reg.families_for_base_model(b)
            .iter()
            .map(|f| f.id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids("pony"), vec!["sdxl_pony"]);
    assert_eq!(ids(" Flux.1 D "), vec!["flux1_dev"]);
    assert_eq!(ids("QWEN"), vec!["qwen_image", "qwen_image_edit_2511"]);
    assert_eq!(ids("NoobAI"), vec!["sdxl_illustrious"]);
    assert_eq!(ids("SDXL Lightning"), vec!["sdxl_fast"]);
    assert!(ids("Pony V7").is_empty());
    assert!(ids("").is_empty());
    // Families added 2026-09-28: exact strings from CivitAI's /api/v1/enums.
    for (base, want) in [
        ("Krea 2", vec!["krea2_turbo"]),
        ("Anima", vec!["anima"]),
        ("Flux.1 Krea", vec!["flux1_dev"]),
        ("Flux.2 D", vec!["flux2_dev"]),
        ("Flux.2 Klein 4B", vec!["flux2_klein_4b"]),
        ("Flux.2 Klein 4B-base", vec!["flux2_klein_4b_base"]),
        ("Flux.2 Klein 9B", vec!["flux2_klein_9b"]),
        ("Flux.2 Klein 9B-base", vec!["flux2_klein_9b_base"]),
        ("Chroma", vec!["chroma"]),
        ("Qwen 2.1", vec!["qwen_image_21"]),
        ("SD 3", vec!["sd3"]),
        ("SD 3.5", vec!["sd3"]),
        ("SD 3.5 Large", vec!["sd3"]),
        ("SD 3.5 Medium", vec!["sd3"]),
        ("SD 3.5 Large Turbo", vec!["sd35_turbo"]),
        ("HiDream-O1", vec!["hidream_o1"]),
        ("Ernie", vec!["ernie_image"]),
        ("MageFlow", vec!["mage_flow"]),
        ("SD 1.4", vec!["sd15"]),
        ("SDXL 0.9", vec!["sdxl"]),
    ] {
        assert_eq!(ids(base), want, "{base}");
    }
    // Not runnable by the pinned engine (video-only, API-only or no architecture).
    for base in [
        "MiniMax H3",
        "Qwen 2",
        "Qwen 3",
        "HiDream",
        "Lumina",
        "AuraFlow",
        "PixArt E",
    ] {
        assert!(ids(base).is_empty(), "{base}");
    }

    let all = reg.all_civitai_base_models();
    let mut sorted = all.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(all, sorted);
    assert_eq!(all.iter().filter(|b| *b == "Qwen").count(), 1);
    for b in [
        "SD 1.5",
        "SDXL 1.0",
        "Pony",
        "Illustrious",
        "Flux.1 Kontext",
        "ZImageTurbo",
        "ZImageBase",
    ] {
        assert!(all.contains(&b.to_string()), "{b}");
    }
}

#[test]
fn known_files_and_hardware_profiles() {
    let reg = shipped();
    let k = reg
        .known_file("2407613050B809FFDFF18A4AC99AF83EA6B95443ECEBDF80E064A79C825574A6")
        .unwrap();
    assert_eq!(k.family, "z_image_turbo");
    assert_eq!(
        reg.known_file("e9476a13728cd75d8279f6ec8bad753a66a1957ca375a1464dc63b37db6e3916")
            .unwrap()
            .family,
        "sd15"
    );
    assert!(reg.known_file("TODO").is_none());
    assert!(reg.known_file("").is_none());
    assert!(reg.known_file("00").is_none());

    assert_eq!(reg.hardware_profile(0.0).name, "low");
    assert_eq!(reg.hardware_profile(7.9).name, "low");
    assert_eq!(reg.hardware_profile(8.0).name, "mid");
    assert_eq!(reg.hardware_profile(12.0).name, "mid");
    assert_eq!(reg.hardware_profile(16.0).name, "high");
    assert_eq!(reg.hardware_profile(24.0).name, "ultra");
    // 13–20 GB prefers Q8 (bf16 Z-Image + encoder did not fit a 16 GB card).
    assert_eq!(
        reg.hardware_profile(16.0).prefer_quant.as_deref(),
        Some("q8_0")
    );
    assert_eq!(
        reg.hardware_profile(24.0).prefer_quant.as_deref(),
        Some("bf16")
    );
    assert_eq!(reg.hardware_profile(5000.0).name, "ultra");
    for p in reg.hardware_profiles() {
        assert!(
            !p.flags.iter().any(|f| f == "--offload-to-cpu"),
            "auto-fit handles offload"
        );
    }

    let unsorted =
        "hardware_profiles:\n  - { name: b, max_vram_gb: 20 }\n  - { name: a, max_vram_gb: 8 }\n";
    let reg = Registry::from_yaml(unsorted, None).unwrap();
    assert_eq!(reg.hardware_profile(6.0).name, "a");
    let none = Registry::from_yaml("{}", None).unwrap();
    assert!(none.hardware_profile(8.0).flags.is_empty());
}

#[test]
fn validate_reports_dangling_references() {
    let yaml = r#"
style_templates: { tags: "{prompt}, {style}" }
families:
  a:
    label: A
    style_template: nope
    layout: all_in_one
    components: { vae: missing }
    detect: { ambiguous_with: [ghost] }
    defaults: { sampler: "DPM++ 2M Karras", scheduler: normalish }
    flags: ["--not-a-flag"]
    dials: { shape: { square: [1000, 1000] } }
"#;
    let reg = Registry::from_yaml(yaml, None).unwrap();
    let p = reg.validate().join("\n");
    let whole = "style_templates: { tags: \"{prompt}, {style}\" }\nfamilies:\n  w: { label: W, style_template: tags, layout: diffusion_only, detect: { any_tensor: [x], whole_checkpoint: true }, dials: { shape: { square: [64, 64] }, quality: { fast: 1, balanced: 1, best: 1 }, cfg_fixed: 1.0 } }\n";
    let reg_w = Registry::from_yaml(whole, None).unwrap();
    assert!(
        reg_w.validate().join("\n").contains("whole_checkpoint"),
        "{:?}",
        reg_w.validate()
    );
    // A registry recommendation needs the family's own download.
    let rec =
        format!("{whole}recommended:\n  realistic:\n    - {{ family: w, source: registry }}\n");
    let reg_r = Registry::from_yaml(&rec, None).unwrap();
    assert!(
        reg_r.validate().join("\n").contains("no `download`"),
        "{:?}",
        reg_r.validate()
    );
    for needle in [
        "missing",
        "nope",
        "ghost",
        "DPM++ 2M Karras",
        "normalish",
        "--not-a-flag",
        "1000x1000",
        "positive rule",
    ] {
        assert!(p.contains(needle), "expected `{needle}` in:\n{p}");
    }
}

#[test]
fn krea2_turbo_is_the_second_realistic_download() {
    let reg = shipped();
    assert_eq!(
        reg.recommended()["realistic"][0].family.as_deref(),
        Some("z_image_turbo"),
        "Z-Image Turbo stays the first Realistic pick"
    );
    let k = &reg.recommended()["realistic_detail"][0];
    assert_eq!(k.family.as_deref(), Some("krea2_turbo"));
    assert_eq!(k.source.as_deref(), Some("registry"));
    // Ungated GGUF mirror named by docs/krea2.md (krea/Krea-2-* are gated).
    let d = reg
        .family("krea2_turbo")
        .unwrap()
        .download
        .as_ref()
        .unwrap();
    assert_eq!(d.file, "Krea-2-Turbo-Q8_0.gguf");
    assert!(d
        .url
        .starts_with("https://huggingface.co/realrebelai/KREA-2_GGUFs/resolve/main/TURBO/"));
    assert_eq!(d.vram_gb.unwrap().min, 20.0, "Q8_0 only from 20 GB");
    let q5 = &d.alt_quants["q5_k"];
    assert_eq!(q5.file.as_deref(), Some("Krea-2-Turbo-Q5_K_S.gguf"));
    assert_eq!(q5.vram_gb.unwrap().min, 12.0, "offered from 12 GB");
    // Both files are also known hashes of Krea 2 Turbo.
    for sha in [&d.sha256, &q5.sha256] {
        assert_eq!(reg.known_file(sha).unwrap().family, "krea2_turbo");
    }
    assert!(
        reg.family("krea2_raw").unwrap().download.is_none(),
        "`download` is not inherited"
    );
}
