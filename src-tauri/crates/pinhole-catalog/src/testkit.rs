//! Test helpers: the shipped registry, hardware contexts and installed files.

use pinhole_registry::wiring::HwContext;
use pinhole_registry::Registry;
use pinhole_store::datadir::ModelKind;
use pinhole_store::installed::CivitaiRef;
use pinhole_store::{InstalledFile, InstalledIndex};

pub fn registry() -> Registry {
    Registry::from_yaml(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../config/models.yaml")), None).unwrap()
}

/// A PC with 16 GB of RAM and `vram_gb` of VRAM (0 = no GPU, CPU backend).
pub fn hw(vram_gb: f32) -> HwContext {
    HwContext { vram_gb, backend: if vram_gb > 0.0 { "cuda".into() } else { "cpu".into() }, ram_gb: 16.0 }
}

/// No usable GPU, `ram_gb` of system RAM (0 = not known yet).
pub fn hw_cpu(ram_gb: f32) -> HwContext {
    HwContext { vram_gb: 0.0, backend: "cpu".into(), ram_gb }
}

pub fn sha(seed: u8) -> String {
    format!("{seed:02x}").repeat(32)
}

pub fn model(id: &str, family: &str, kind: ModelKind, file: &str) -> InstalledFile {
    InstalledFile {
        id: id.into(),
        rel_path: format!("models/{}/{file}", kind.dir_name()),
        kind,
        sha256: sha(id.bytes().fold(0u8, |a, b| a.wrapping_add(b))),
        size_bytes: 6_500_000_000,
        family: Some(family.into()),
        component_id: None,
        friendly_name: id.into(),
        civitai: None,
        added_at: 0,
        last_used: None,
        observed_vram_gb: None,
        dtype: None,
    }
}

pub fn with_civitai(mut f: InstalledFile, version_id: u64) -> InstalledFile {
    f.civitai = Some(CivitaiRef {
        model_id: 1,
        version_id,
        model_name: Some("m".into()),
        version_name: None,
        base_model: Some("SDXL 1.0".into()),
        trained_words: vec![],
        license: None,
    });
    f
}

/// A registry component as an installed file (real hash from the registry).
pub fn component(reg: &Registry, id: &str) -> InstalledFile {
    let c = reg.component(id).unwrap_or_else(|| panic!("component {id}"));
    InstalledFile {
        id: format!("c-{id}"),
        rel_path: format!("models/{}/{}", crate::families::component_model_kind(&c.kind).dir_name(), c.file),
        kind: crate::families::component_model_kind(&c.kind),
        sha256: crate::families::normalize_sha(&c.sha256).unwrap_or_else(|| sha(9)),
        size_bytes: c.size_mb * 1_000_000,
        family: None,
        component_id: Some(id.into()),
        friendly_name: c.file.clone(),
        civitai: None,
        added_at: 0,
        last_used: None,
        observed_vram_gb: None,
        dtype: None,
    }
}

pub fn index(files: Vec<InstalledFile>) -> InstalledIndex {
    InstalledIndex { schema_version: 1, files, ..Default::default() }
}
