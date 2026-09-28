use std::collections::BTreeMap;
use std::io::Write;

use super::*;
use crate::detect::{
    self, parse_header_bytes, read_header, DetectError, FileFormat, HeaderInfo, MAX_HEADER_BYTES,
};
use crate::Layout;

// ----------------------------------------------------------------------------- builders

fn st_dtype_size(dtype: &str) -> u64 {
    match dtype {
        "F64" | "I64" | "U64" => 8,
        "F32" | "I32" | "U32" => 4,
        "F16" | "BF16" | "I16" | "U16" => 2,
        _ => 1,
    }
}

/// Safetensors bytes (header only) + total file size. Shapes in PyTorch order.
fn safetensors(tensors: &[(&str, &str, &[u64])], metadata: &[(&str, &str)]) -> (Vec<u8>, u64) {
    let mut obj = serde_json::Map::new();
    let mut offset = 0u64;
    for (name, dtype, shape) in tensors {
        let n: u64 = shape.iter().product::<u64>() * st_dtype_size(dtype);
        obj.insert(
            (*name).into(),
            serde_json::json!({ "dtype": dtype, "shape": shape, "data_offsets": [offset, offset + n] }),
        );
        offset += n;
    }
    if !metadata.is_empty() {
        let m: serde_json::Map<String, serde_json::Value> = metadata
            .iter()
            .map(|(k, v)| ((*k).to_string(), serde_json::Value::from(*v)))
            .collect();
        obj.insert("__metadata__".into(), serde_json::Value::Object(m));
    }
    let header = serde_json::to_vec(&serde_json::Value::Object(obj)).unwrap();
    let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
    bytes.extend_from_slice(&header);
    let size = bytes.len() as u64 + offset;
    (bytes, size)
}

fn st_header(tensors: &[(&str, &str, &[u64])]) -> HeaderInfo {
    let (b, size) = safetensors(tensors, &[]);
    parse_header_bytes(&b, size).expect("valid synthetic safetensors")
}

enum GVal {
    U32(u32),
    I64(i64),
    F32(f32),
    Bool(bool),
    Str(&'static str),
    StrArr(Vec<&'static str>),
    U32Arr(Vec<u32>),
    NestedArr,
}

fn gstr(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u64).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn ggml_bytes(ne: &[u64], ttype: u32) -> u64 {
    let (_, blck, size) = detect::ggml_type_info(ttype).unwrap();
    ne.iter().product::<u64>().div_ceil(blck) * size
}

/// GGUF v3 bytes (header only) + total file size. `ne` in ggml order.
fn gguf(kvs: &[(&str, GVal)], tensors: &[(&str, &[u64], u32)]) -> (Vec<u8>, u64) {
    let mut out = b"GGUF".to_vec();
    out.extend_from_slice(&3u32.to_le_bytes());
    out.extend_from_slice(&(tensors.len() as u64).to_le_bytes());
    out.extend_from_slice(&(kvs.len() as u64).to_le_bytes());
    for (k, v) in kvs {
        gstr(&mut out, k);
        match v {
            GVal::U32(x) => {
                out.extend_from_slice(&4u32.to_le_bytes());
                out.extend_from_slice(&x.to_le_bytes());
            }
            GVal::I64(x) => {
                out.extend_from_slice(&11u32.to_le_bytes());
                out.extend_from_slice(&x.to_le_bytes());
            }
            GVal::F32(x) => {
                out.extend_from_slice(&6u32.to_le_bytes());
                out.extend_from_slice(&x.to_le_bytes());
            }
            GVal::Bool(x) => {
                out.extend_from_slice(&7u32.to_le_bytes());
                out.push(*x as u8);
            }
            GVal::Str(s) => {
                out.extend_from_slice(&8u32.to_le_bytes());
                gstr(&mut out, s);
            }
            GVal::StrArr(items) => {
                out.extend_from_slice(&9u32.to_le_bytes());
                out.extend_from_slice(&8u32.to_le_bytes());
                out.extend_from_slice(&(items.len() as u64).to_le_bytes());
                for s in items {
                    gstr(&mut out, s);
                }
            }
            GVal::U32Arr(items) => {
                out.extend_from_slice(&9u32.to_le_bytes());
                out.extend_from_slice(&4u32.to_le_bytes());
                out.extend_from_slice(&(items.len() as u64).to_le_bytes());
                for x in items {
                    out.extend_from_slice(&x.to_le_bytes());
                }
            }
            GVal::NestedArr => {
                // array of 2 arrays of u8
                out.extend_from_slice(&9u32.to_le_bytes());
                out.extend_from_slice(&9u32.to_le_bytes());
                out.extend_from_slice(&2u64.to_le_bytes());
                for _ in 0..2 {
                    out.extend_from_slice(&0u32.to_le_bytes());
                    out.extend_from_slice(&3u64.to_le_bytes());
                    out.extend_from_slice(&[1, 2, 3]);
                }
            }
        }
    }
    let mut offset = 0u64;
    let mut end = 0u64;
    for (name, ne, ttype) in tensors {
        gstr(&mut out, name);
        out.extend_from_slice(&(ne.len() as u32).to_le_bytes());
        for d in *ne {
            out.extend_from_slice(&d.to_le_bytes());
        }
        out.extend_from_slice(&ttype.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        let bytes = ggml_bytes(ne, *ttype);
        end = offset + bytes;
        offset += bytes.div_ceil(32) * 32;
    }
    let data_start = (out.len() as u64).div_ceil(32) * 32;
    // File ends right after the last tensor (no trailing padding).
    (out, data_start + end)
}

fn candidates(h: &HeaderInfo) -> Vec<String> {
    detect::detect(shipped(), h).candidates
}

// ----------------------------------------------------------------------------- family fixtures

#[test]
fn sd15_checkpoint() {
    let h = st_header(&[
        (
            "model.diffusion_model.input_blocks.0.0.weight",
            "F16",
            &[320, 4, 3, 3],
        ),
        (
            "model.diffusion_model.middle_block.1.norm.weight",
            "F16",
            &[1280],
        ),
        (
            "cond_stage_model.transformer.text_model.embeddings.token_embedding.weight",
            "F16",
            &[49408, 768],
        ),
        (
            "first_stage_model.decoder.conv_in.weight",
            "F32",
            &[512, 4, 3, 3],
        ),
    ]);
    let d = detect::detect(shipped(), &h);
    assert_eq!(d.candidates, vec!["sd15", "sd15_fast"]);
    assert_eq!(d.layout, Layout::AllInOne);
    assert!(d.has_vae && d.has_text_encoders && !d.is_lora);
    assert_eq!(d.is_component, None);
    assert_eq!(d.dtype, "f16");

    // SD 2.x: OpenCLIP (1024) text encoder → not ours.
    let sd2 = st_header(&[
        (
            "model.diffusion_model.input_blocks.0.0.weight",
            "F16",
            &[320, 4, 3, 3],
        ),
        (
            "cond_stage_model.model.token_embedding.weight",
            "F16",
            &[49408, 1024],
        ),
    ]);
    assert!(candidates(&sd2).is_empty());
    // A CLIP-L token embedding of the wrong width is rejected by tensor_ne0.
    let odd = st_header(&[
        (
            "model.diffusion_model.input_blocks.0.0.weight",
            "F16",
            &[320, 4, 3, 3],
        ),
        (
            "cond_stage_model.transformer.text_model.embeddings.token_embedding.weight",
            "F16",
            &[49408, 1024],
        ),
    ]);
    assert!(candidates(&odd).is_empty());
}

#[test]
fn sd1_tiny_unets_are_not_sd15() {
    // get_sd_version(): an SD1 UNet without the second middle-block ResNet is
    // SDXS-512 (no output_blocks.7.1 either) or an SD1 tiny UNet — not SD 1.5.
    let base: Vec<(&str, &str, &[u64])> = vec![
        (
            "model.diffusion_model.input_blocks.0.0.weight",
            "F16",
            &[320, 4, 3, 3],
        ),
        (
            "cond_stage_model.transformer.text_model.embeddings.token_embedding.weight",
            "F16",
            &[49408, 768],
        ),
    ];
    let sdxs = st_header(&base);
    assert!(candidates(&sdxs).is_empty(), "SDXS-512");
    let mut tiny = base.clone();
    tiny.push((
        "model.diffusion_model.output_blocks.7.1.norm.weight",
        "F16",
        &[640],
    ));
    assert!(candidates(&st_header(&tiny)).is_empty(), "SD1 tiny UNet");

    // With middle_block.1 it is SD 1.5 again — also in diffusers naming.
    let mut full = base.clone();
    full.push((
        "model.diffusion_model.middle_block.1.proj_in.weight",
        "F16",
        &[1280, 1280, 1, 1],
    ));
    assert_eq!(candidates(&st_header(&full)), vec!["sd15", "sd15_fast"]);
    let diffusers = st_header(&[
        (
            "unet.down_blocks.0.resnets.0.conv1.weight",
            "F16",
            &[320, 320, 3, 3],
        ),
        (
            "unet.mid_block.resnets.1.conv1.weight",
            "F16",
            &[1280, 1280, 3, 3],
        ),
        (
            "te.text_model.embeddings.token_embedding.weight",
            "F16",
            &[49408, 768],
        ),
    ]);
    assert_eq!(candidates(&diffusers), vec!["sd15", "sd15_fast"]);
    let diffusers_tiny = st_header(&[
        (
            "unet.down_blocks.0.resnets.0.conv1.weight",
            "F16",
            &[320, 320, 3, 3],
        ),
        (
            "unet.mid_block.resnets.0.conv1.weight",
            "F16",
            &[1280, 1280, 3, 3],
        ),
        (
            "te.text_model.embeddings.token_embedding.weight",
            "F16",
            &[49408, 768],
        ),
    ]);
    assert!(candidates(&diffusers_tiny).is_empty());
}

#[test]
fn sdxl_checkpoint_lists_finetune_families_after_base() {
    let h = st_header(&[
        (
            "model.diffusion_model.input_blocks.0.0.weight",
            "F16",
            &[320, 4, 3, 3],
        ),
        (
            "model.diffusion_model.middle_block.1.norm.weight",
            "F16",
            &[1280],
        ),
        (
            "conditioner.embedders.0.transformer.text_model.embeddings.token_embedding.weight",
            "F16",
            &[49408, 768],
        ),
        (
            "conditioner.embedders.1.model.token_embedding.weight",
            "F16",
            &[49408, 1280],
        ),
        (
            "first_stage_model.encoder.conv_in.weight",
            "F16",
            &[128, 3, 3, 3],
        ),
    ]);
    let d = detect::detect(shipped(), &h);
    assert_eq!(
        d.candidates,
        vec!["sdxl", "sdxl_pony", "sdxl_illustrious", "sdxl_fast"]
    );
    assert_eq!(d.layout, Layout::AllInOne);
    // SVD shares the UNet naming.
    let mut names = h.tensor_names.clone();
    names.push("model.diffusion_model.input_blocks.8.0.time_mixer.mix_factor".into());
    let svd = HeaderInfo {
        tensor_names: names,
        tensors: vec![],
        ..h
    };
    assert!(candidates(&svd).is_empty());
}

#[test]
fn flux_variants() {
    let dev: &[(&str, &str, &[u64])] = &[
        ("double_blocks.0.img_attn.qkv.weight", "BF16", &[9216, 3072]),
        ("single_blocks.0.linear1.weight", "BF16", &[21504, 3072]),
        ("img_in.weight", "BF16", &[3072, 64]),
        ("txt_in.weight", "BF16", &[3072, 4096]),
        ("guidance_in.in_layer.weight", "BF16", &[3072, 256]),
    ];
    let d = detect::detect(shipped(), &st_header(dev));
    assert_eq!(d.candidates, vec!["flux1_dev", "flux1_kontext"]);
    assert_eq!(d.layout, Layout::DiffusionOnly);
    assert!(!d.has_vae && !d.has_text_encoders);
    assert_eq!(d.dtype, "bf16");

    // schnell: no guidance embedder.
    let schnell: Vec<_> = dev
        .iter()
        .filter(|t| !t.0.starts_with("guidance_in"))
        .cloned()
        .collect();
    assert_eq!(candidates(&st_header(&schnell)), vec!["flux1_schnell"]);

    // Fill (img_in 384), LongCat (txt_in 3584), Chroma, FLUX.2 → not FLUX.1.
    let with = |name: &'static str, shape: &'static [u64]| {
        let mut v: Vec<(&str, &str, &[u64])> =
            dev.iter().filter(|t| t.0 != name).cloned().collect();
        v.push((name, "BF16", shape));
        v
    };
    assert!(candidates(&st_header(&with("img_in.weight", &[3072, 384]))).is_empty());
    assert!(candidates(&st_header(&with("txt_in.weight", &[3072, 3584]))).is_empty());
    assert!(candidates(&st_header(&with(
        "distilled_guidance_layer.in_proj.weight",
        &[5120, 64]
    )))
    .is_empty());
    assert!(candidates(&st_header(&with(
        "double_stream_modulation_img.lin.weight",
        &[18432, 3072]
    )))
    .is_empty());

    // Diffusers naming (converted by the loader) is recognised too.
    let diffusers = st_header(&[
        (
            "transformer_blocks.0.attn.to_q.weight",
            "BF16",
            &[3072, 3072],
        ),
        (
            "single_transformer_blocks.0.attn.to_k.weight",
            "BF16",
            &[3072, 3072],
        ),
        (
            "time_text_embed.guidance_embedder.linear_1.weight",
            "BF16",
            &[3072, 256],
        ),
        ("x_embedder.weight", "BF16", &[3072, 64]),
    ]);
    assert_eq!(candidates(&diffusers), vec!["flux1_dev", "flux1_kontext"]);

    // ComfyUI all-in-one checkpoint.
    let aio = st_header(&[
        (
            "model.diffusion_model.double_blocks.0.img_attn.qkv.weight",
            "F8_E4M3",
            &[9216, 3072],
        ),
        ("model.diffusion_model.img_in.weight", "BF16", &[3072, 64]),
        (
            "model.diffusion_model.guidance_in.in_layer.weight",
            "BF16",
            &[3072, 256],
        ),
        (
            "text_encoders.clip_l.transformer.text_model.embeddings.token_embedding.weight",
            "F16",
            &[1000, 768],
        ),
        ("vae.decoder.conv_in.weight", "F16", &[512, 16, 3, 3]),
    ]);
    let d = detect::detect(shipped(), &aio);
    assert_eq!(d.candidates, vec!["flux1_dev", "flux1_kontext"]);
    assert_eq!(d.layout, Layout::AllInOne);
    assert!(d.has_vae && d.has_text_encoders);
    assert_eq!(d.dtype, "f8_e4m3");
}

#[test]
fn z_image_and_ming() {
    let z: &[(&str, &str, &[u64])] = &[
        ("cap_embedder.0.weight", "BF16", &[2560]),
        ("layers.0.attention.qkv.weight", "BF16", &[11520, 3840]),
        ("x_embedder.weight", "BF16", &[3840, 64]),
    ];
    assert_eq!(
        candidates(&st_header(z)),
        vec!["z_image_turbo", "z_image_base"]
    );
    let mut ming = z.to_vec();
    ming.push((
        "text_encoders.llm.connector.layers.0.self_attn.q_proj.weight",
        "BF16",
        &[8, 8],
    ));
    assert!(candidates(&st_header(&ming)).is_empty());
    // Prefixed (all-in-one style) names still match: rules strip `model.diffusion_model.`.
    let prefixed = st_header(&[(
        "model.diffusion_model.cap_embedder.0.weight",
        "BF16",
        &[2560],
    )]);
    assert_eq!(candidates(&prefixed), vec!["z_image_turbo", "z_image_base"]);
}

#[test]
fn qwen_image_vs_edit() {
    let q: &[(&str, &str, &[u64])] = &[
        ("img_in.weight", "BF16", &[3072, 64]),
        ("txt_in.weight", "BF16", &[3072, 3584]),
        (
            "transformer_blocks.0.attn.norm_added_q.weight",
            "BF16",
            &[128],
        ),
        (
            "transformer_blocks.0.img_mlp.net.0.proj.weight",
            "BF16",
            &[12288, 3072],
        ),
        (
            "transformer_blocks.0.img_mod.1.weight",
            "BF16",
            &[18432, 3072],
        ),
    ];
    assert_eq!(
        candidates(&st_header(q)),
        vec!["qwen_image", "qwen_image_edit_2511"]
    );

    let mut edit = q.to_vec();
    edit.push(("__index_timestep_zero__", "BF16", &[0]));
    assert_eq!(candidates(&st_header(&edit)), vec!["qwen_image_edit_2511"]);

    let mut layered = q.to_vec();
    layered.push((
        "time_text_embed.addition_t_embedding.weight",
        "BF16",
        &[3072, 2],
    ));
    assert!(candidates(&st_header(&layered)).is_empty());

    const MAGE_IMG_IN: &[u64] = &[3072, 128];
    let mage: Vec<_> = q
        .iter()
        .map(|t| {
            if t.0 == "img_in.weight" {
                ("img_in.weight", "BF16", MAGE_IMG_IN)
            } else {
                *t
            }
        })
        .collect();
    assert!(candidates(&st_header(&mage)).is_empty());

    let mut lens = q.to_vec();
    lens.push(("transformer_blocks.0.img_mlp.w1.weight", "BF16", &[8, 8]));
    assert!(candidates(&st_header(&lens)).is_empty());
}

#[test]
fn gguf_qwen_edit_quant() {
    let (bytes, size) = gguf(
        &[
            ("general.architecture", GVal::Str("qwen_image")),
            ("general.alignment", GVal::U32(32)),
            ("general.file_type", GVal::U32(15)),
            ("general.quantized_by", GVal::Str("unsloth")),
            ("x.count", GVal::I64(-5)),
            ("x.scale", GVal::F32(1.5)),
            ("x.flag", GVal::Bool(true)),
            ("tokenizer.tokens", GVal::StrArr(vec!["a", "b", "c"])),
            ("x.ids", GVal::U32Arr(vec![1, 2, 3, 4])),
            ("x.nested", GVal::NestedArr),
        ],
        &[
            ("img_in.weight", &[64, 3072], 0),
            ("transformer_blocks.0.img_mod.1.weight", &[3072, 18432], 12),
            (
                "transformer_blocks.0.img_mlp.net.0.proj.weight",
                &[3072, 12288],
                14,
            ),
            ("__index_timestep_zero__", &[1], 0),
        ],
    );
    let h = parse_header_bytes(&bytes, size).unwrap();
    assert_eq!(h.format, FileFormat::Gguf);
    assert_eq!(h.dtype, "q4_k");
    let expect = ggml_bytes(&[64, 3072], 0)
        + ggml_bytes(&[3072, 18432], 12)
        + ggml_bytes(&[3072, 12288], 14)
        + 4;
    assert_eq!(h.tensor_bytes, expect);
    assert_eq!(ggml_bytes(&[3072, 18432], 12), 3072 * 18432 / 256 * 144);
    assert_eq!(h.metadata["general.architecture"], "qwen_image");
    assert_eq!(h.metadata["x.count"], "-5");
    assert_eq!(h.metadata["x.scale"], "1.5");
    assert_eq!(h.metadata["x.flag"], "true");
    assert_eq!(h.metadata["tokenizer.tokens"], "[string; 3]");
    assert_eq!(h.metadata["x.ids"], "[u32; 4]");
    assert_eq!(h.metadata["x.nested"], "[array; 2]");
    assert_eq!(h.tensors[0].ne, vec![64, 3072]);
    assert_eq!(h.tensors[1].dtype, "q4_k");
    assert_eq!(candidates(&h), vec!["qwen_image_edit_2511"]);

    // Same file through read_header (only the header is read).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("q.gguf");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(&bytes).unwrap();
    f.set_len(size).unwrap();
    drop(f);
    let from_file = read_header(&path).unwrap();
    assert_eq!(from_file.tensor_names, h.tensor_names);
    assert_eq!(from_file.tensor_bytes, h.tensor_bytes);
    assert_eq!(from_file.file_size, size);
}

#[test]
fn gguf_z_image_and_companions() {
    let (b, size) = gguf(
        &[],
        &[
            ("cap_embedder.0.weight", &[2560], 0),
            ("layers.0.attention.qkv.weight", &[3840, 11520], 8),
        ],
    );
    let h = parse_header_bytes(&b, size).unwrap();
    assert_eq!(h.dtype, "q8_0");
    assert_eq!(candidates(&h), vec!["z_image_turbo", "z_image_base"]);

    let (b, size) = gguf(
        &[
            ("general.architecture", GVal::Str("clip")),
            ("clip.has_vision_encoder", GVal::Bool(true)),
        ],
        &[
            ("v.blk.0.attn_q.weight", &[1280, 1280], 8),
            ("mm.0.weight", &[1280, 5120], 8),
        ],
    );
    let d = detect::detect(shipped(), &parse_header_bytes(&b, size).unwrap());
    assert_eq!(d.is_component.as_deref(), Some("llm_vision"));
    assert!(d.candidates.is_empty());

    let (b, size) = gguf(
        &[("general.architecture", GVal::Str("qwen2vl"))],
        &[
            ("token_embd.weight", &[3584, 152064], 8),
            ("blk.0.attn_q.weight", &[3584, 3584], 8),
        ],
    );
    let d = detect::detect(shipped(), &parse_header_bytes(&b, size).unwrap());
    assert_eq!(d.is_component.as_deref(), Some("llm"));
    assert!(d.has_text_encoders);
}

#[test]
fn loras_are_recognised_and_get_no_family() {
    let kohya = st_header(&[
        (
            "lora_unet_input_blocks_1_1_proj_in.lora_down.weight",
            "F16",
            &[8, 320],
        ),
        (
            "lora_unet_input_blocks_1_1_proj_in.lora_up.weight",
            "F16",
            &[320, 8],
        ),
        ("lora_unet_input_blocks_1_1_proj_in.alpha", "F16", &[]),
        (
            "lora_te_text_model_encoder_layers_0_mlp_fc1.lora_down.weight",
            "F16",
            &[8, 768],
        ),
    ]);
    let d = detect::detect(shipped(), &kohya);
    assert!(d.is_lora);
    assert!(d.candidates.is_empty() && d.is_component.is_none());
    assert!(!d.has_vae && !d.has_text_encoders);

    // A PEFT Flux LoRA would otherwise look like schnell (no guidance_in).
    let peft = st_header(&[
        (
            "transformer.single_transformer_blocks.0.attn.to_k.lora_A.weight",
            "BF16",
            &[16, 3072],
        ),
        (
            "transformer.single_transformer_blocks.0.attn.to_k.lora_B.weight",
            "BF16",
            &[3072, 16],
        ),
    ]);
    let d = detect::detect(shipped(), &peft);
    assert!(d.is_lora && d.candidates.is_empty());

    // A checkpoint with a stray alpha is not a LoRA.
    let h = st_header(&[
        ("cap_embedder.0.weight", "BF16", &[2560]),
        ("layers.0.attention.qkv.weight", "BF16", &[11520, 3840]),
        ("something.alpha", "F32", &[]),
    ]);
    assert!(!detect::detect(shipped(), &h).is_lora);
}

#[test]
fn standalone_components() {
    let vae = st_header(&[
        (
            "encoder.down.0.block.0.conv1.weight",
            "F32",
            &[128, 128, 3, 3],
        ),
        (
            "decoder.up.0.block.0.conv1.weight",
            "F32",
            &[4096, 4096, 3, 3],
        ),
        ("decoder.conv_out.weight", "F32", &[3, 128, 3, 3]),
    ]);
    let d = detect::detect(shipped(), &vae);
    assert_eq!(d.is_component.as_deref(), Some("vae"));
    assert!(d.has_vae && d.candidates.is_empty());

    let taesd = st_header(&[
        ("encoder.layers.0.weight", "F16", &[64, 3, 3, 3]),
        ("decoder.layers.0.weight", "F16", &[64, 4, 3, 3]),
    ]);
    assert_eq!(
        detect::detect(shipped(), &taesd).is_component.as_deref(),
        Some("taesd")
    );

    let clip_l = st_header(&[
        (
            "text_model.embeddings.token_embedding.weight",
            "F16",
            &[49408, 768],
        ),
        (
            "text_model.encoder.layers.11.mlp.fc1.weight",
            "F16",
            &[3072, 768],
        ),
    ]);
    let d = detect::detect(shipped(), &clip_l);
    assert_eq!(d.is_component.as_deref(), Some("clip_l"));
    assert!(d.has_text_encoders && d.candidates.is_empty());

    let clip_g = st_header(&[
        (
            "text_model.embeddings.token_embedding.weight",
            "F16",
            &[49408, 1280],
        ),
        (
            "text_model.encoder.layers.31.mlp.fc1.weight",
            "F16",
            &[5120, 1280],
        ),
    ]);
    assert_eq!(
        detect::detect(shipped(), &clip_g).is_component.as_deref(),
        Some("clip_g")
    );

    let t5 = st_header(&[
        (
            "encoder.block.0.layer.0.SelfAttention.q.weight",
            "F16",
            &[4096, 4096],
        ),
        ("shared.weight", "F16", &[32128, 4096]),
    ]);
    assert_eq!(
        detect::detect(shipped(), &t5).is_component.as_deref(),
        Some("t5xxl")
    );

    let llm = st_header(&[
        ("model.embed_tokens.weight", "BF16", &[151936, 2560]),
        (
            "model.layers.0.self_attn.q_proj.weight",
            "BF16",
            &[4096, 2560],
        ),
    ]);
    assert_eq!(
        detect::detect(shipped(), &llm).is_component.as_deref(),
        Some("llm")
    );

    let unknown = st_header(&[("foo.bar", "F16", &[2, 2])]);
    let d = detect::detect(shipped(), &unknown);
    assert!(d.candidates.is_empty() && d.is_component.is_none() && !d.is_lora);
}

// ----------------------------------------------------------------------------- parsing edge cases

#[test]
fn safetensors_metadata_dtype_and_names() {
    let (b, size) = safetensors(
        &[
            ("a.weight", "F16", &[4, 4]),
            ("b.weight", "BF16", &[64, 64]),
            ("c.weight", "F8_E4M3", &[2]),
        ],
        &[("format", "pt"), ("modelspec.architecture", "flux-1-dev")],
    );
    let h = parse_header_bytes(&b, size).unwrap();
    assert_eq!(h.format, FileFormat::Safetensors);
    assert_eq!(h.tensor_names, vec!["a.weight", "b.weight", "c.weight"]);
    assert_eq!(h.dtype, "bf16");
    assert_eq!(h.tensor_bytes, 32 + 64 * 64 * 2 + 2);
    assert_eq!(h.metadata["modelspec.architecture"], "flux-1-dev");
    assert!(!h.tensor_names.iter().any(|n| n == "__metadata__"));
    assert_eq!(h.tensors[1].ne, vec![64, 64]);
    // File size 0 = unknown: data range checks are skipped.
    assert!(parse_header_bytes(&b, 0).is_ok());
}

fn malformed(r: Result<HeaderInfo, DetectError>) -> bool {
    matches!(r, Err(DetectError::Malformed(_)))
}

#[test]
fn safetensors_rejects_bad_headers() {
    // Declared header over the cap.
    let mut huge = (MAX_HEADER_BYTES + 1).to_le_bytes().to_vec();
    huge.extend_from_slice(b"{}");
    assert!(malformed(parse_header_bytes(&huge, 10 * MAX_HEADER_BYTES)));
    // Header longer than the file.
    let mut long = 1000u64.to_le_bytes().to_vec();
    long.extend_from_slice(b"{\"a\":1}");
    assert!(malformed(parse_header_bytes(&long, 100)));
    // Header longer than the bytes we were given.
    assert!(malformed(parse_header_bytes(&long, 0)));

    let with_json = |json: &str| {
        let mut v = (json.len() as u64).to_le_bytes().to_vec();
        v.extend_from_slice(json.as_bytes());
        let size = v.len() as u64 + 16;
        parse_header_bytes(&v, size)
    };
    assert!(malformed(with_json("{\"a\": ")));
    assert!(malformed(with_json("{\"a\": 1}")));
    assert!(malformed(with_json(
        "{\"a\": {\"dtype\": \"F16\", \"shape\": [2], \"data_offsets\": [8, 4]}}"
    )));
    assert!(malformed(with_json(
        "{\"a\": {\"dtype\": \"F16\", \"shape\": [2], \"data_offsets\": [0, 400]}}"
    )));
    assert!(malformed(with_json(
        "{\"a\": {\"dtype\": \"F16\", \"shape\": [-2], \"data_offsets\": [0, 4]}}"
    )));
    assert!(malformed(with_json(
        "{\"a\": {\"shape\": [2], \"data_offsets\": [0, 4]}}"
    )));
    assert!(malformed(with_json(
        "{\"a\": {\"dtype\": \"F16\", \"shape\": [4294967296, 4294967296, 4294967296], \"data_offsets\": [0, 4]}}"
    )));
    // Deeply nested JSON hits serde_json's recursion limit instead of the stack.
    let deep = format!("{{\"a\": {}{}}}", "[".repeat(10_000), "]".repeat(10_000));
    assert!(malformed(with_json(&deep)));
    assert!(
        with_json("{\"a\": {\"dtype\": \"F16\", \"shape\": [2], \"data_offsets\": [0, 4]}}")
            .is_ok()
    );

    // Not safetensors at all.
    assert!(matches!(
        parse_header_bytes(b"PK\x03\x04\x14\x00\x00\x00\x08\x00", 100),
        Err(DetectError::UnknownFormat)
    ));
    assert!(matches!(
        parse_header_bytes(b"\x02\x00\x00\x00\x00\x00\x00\x00[]", 10),
        Err(DetectError::UnknownFormat)
    ));
    assert!(matches!(
        parse_header_bytes(b"abc", 3),
        Err(DetectError::UnknownFormat)
    ));
}

#[test]
fn gguf_rejects_bad_headers() {
    let (good, size) = gguf(
        &[("general.architecture", GVal::Str("x"))],
        &[("t", &[32], 8)],
    );
    assert!(parse_header_bytes(&good, size).is_ok());

    // Truncated anywhere → Malformed, never a panic.
    for cut in 5..good.len() {
        assert!(
            malformed(parse_header_bytes(&good[..cut], 0)),
            "cut at {cut}"
        );
    }
    // Unsupported versions (v1, big-endian v3).
    for v in [1u32, 0x0300_0000] {
        let mut b = good.clone();
        b[4..8].copy_from_slice(&v.to_le_bytes());
        assert!(malformed(parse_header_bytes(&b, size)));
    }
    // Absurd tensor / kv counts.
    let mut b = good.clone();
    b[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(malformed(parse_header_bytes(&b, size)));
    let mut b = good.clone();
    b[16..24].copy_from_slice(&(u64::MAX / 2).to_le_bytes());
    assert!(malformed(parse_header_bytes(&b, size)));
    // Key length larger than the file.
    let mut b = good.clone();
    b[24..32].copy_from_slice(&(1u64 << 40).to_le_bytes());
    assert!(malformed(parse_header_bytes(&b, size)));
    // Tensor data past the end of the file (truncated download).
    assert!(malformed(parse_header_bytes(&good, size - 1)));
    // Unknown value type.
    let (mut b, size) = gguf(&[("k", GVal::U32(1))], &[]);
    let pos = 24 + 8 + 1; // magic+ver+counts, key len, key "k"
    b[pos..pos + 4].copy_from_slice(&99u32.to_le_bytes());
    assert!(malformed(parse_header_bytes(&b, size)));
    // Too many dimensions.
    let (b, size) = gguf(&[], &[("t", &[1; 17], 0)]);
    assert!(malformed(parse_header_bytes(&b, size)));
    // A huge string array claim is caught before looping.
    let mut arr = b"GGUF".to_vec();
    arr.extend_from_slice(&3u32.to_le_bytes());
    arr.extend_from_slice(&0u64.to_le_bytes());
    arr.extend_from_slice(&1u64.to_le_bytes());
    gstr(&mut arr, "k");
    arr.extend_from_slice(&9u32.to_le_bytes());
    arr.extend_from_slice(&8u32.to_le_bytes());
    arr.extend_from_slice(&(u64::MAX / 4).to_le_bytes());
    assert!(malformed(parse_header_bytes(&arr, arr.len() as u64)));
}

#[test]
fn unknown_ggml_types_are_tolerated() {
    let (b, size) = gguf(&[], &[("cap_embedder.0.weight", &[8], 0)]);
    // Patch the tensor type to an id newer than our table (offset: after name, n_dims, dims).
    let mut b = b;
    let type_pos = b.len() - 12;
    b[type_pos..type_pos + 4].copy_from_slice(&200u32.to_le_bytes());
    let h = parse_header_bytes(&b, size).unwrap();
    assert_eq!(h.tensors[0].dtype, "type_200");
    assert_eq!(h.tensor_bytes, 0);
}

#[test]
fn read_header_reads_only_the_header() {
    let dir = tempfile::tempdir().unwrap();
    let (bytes, size) = safetensors(
        &[
            ("cap_embedder.0.weight", "BF16", &[2560]),
            ("big.weight", "BF16", &[4096, 4096]),
        ],
        &[],
    );
    let path = dir.path().join("m.safetensors");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(&bytes).unwrap();
    // Sparse data region: the header parser must not need to read it.
    f.set_len(size).unwrap();
    drop(f);
    let h = read_header(&path).unwrap();
    assert_eq!(h.file_size, size);
    assert_eq!(h.tensor_bytes, 2560 * 2 + 4096 * 4096 * 2);
    assert_eq!(candidates(&h), vec!["z_image_turbo", "z_image_base"]);

    // A file cut inside the data region is rejected (offsets past EOF).
    let cut = dir.path().join("cut.safetensors");
    std::fs::write(&cut, &bytes).unwrap();
    assert!(malformed(read_header(&cut)));

    let tiny = dir.path().join("tiny.bin");
    std::fs::write(&tiny, b"abc").unwrap();
    assert!(matches!(
        read_header(&tiny),
        Err(DetectError::UnknownFormat)
    ));
    let zip = dir.path().join("x.ckpt");
    std::fs::write(&zip, b"PK\x03\x04\x14\x00\x00\x00\x08\x00rest").unwrap();
    assert!(matches!(read_header(&zip), Err(DetectError::UnknownFormat)));
    let huge = dir.path().join("huge.safetensors");
    let mut v = (MAX_HEADER_BYTES * 2).to_le_bytes().to_vec();
    v.extend_from_slice(b"{\"a\":1}");
    std::fs::write(&huge, &v).unwrap();
    assert!(malformed(read_header(&huge)));
    assert!(matches!(
        read_header(&dir.path().join("missing.safetensors")),
        Err(DetectError::Io(_))
    ));
}

#[test]
fn rules_without_shapes_still_match_by_name() {
    // Callers may build a HeaderInfo from names only; shape rules then pass.
    let h = HeaderInfo {
        format: FileFormat::Safetensors,
        tensor_names: vec![
            "double_blocks.0.x".into(),
            "img_in.weight".into(),
            "guidance_in.in_layer.weight".into(),
        ],
        dtype: "bf16".into(),
        file_size: 0,
        tensor_bytes: 0,
        metadata: BTreeMap::new(),
        tensors: vec![],
    };
    assert_eq!(candidates(&h), vec!["flux1_dev", "flux1_kontext"]);
}

#[test]
fn anchored_patterns() {
    use crate::DetectRules;
    let h = st_header(&[
        ("model.diffusion_model.img_in.weight", "F16", &[4, 64]),
        ("foo.img_in.weight.bar", "F16", &[1]),
    ]);
    let rule = |p: &str| DetectRules {
        any_tensor: vec![p.into()],
        ..Default::default()
    };
    assert!(detect::rules_match(&rule("^img_in.weight$"), &h)); // prefix stripped
    assert!(detect::rules_match(
        &rule("^model.diffusion_model.img_in"),
        &h
    )); // raw name
    assert!(detect::rules_match(&rule("weight.bar$"), &h));
    assert!(!detect::rules_match(&rule("^weight"), &h));
    assert!(!detect::rules_match(&DetectRules::default(), &h)); // no positive rule
}
