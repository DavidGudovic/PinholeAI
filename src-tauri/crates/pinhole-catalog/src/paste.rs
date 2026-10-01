//! "Paste from CivitAI": match pasted resources (ids and hashes only — the
//! prompt never reaches Rust) to installed files, or to installable CivitAI
//! versions, and flag LoRAs that don't fit the checkpoint.

use std::future::Future;

use futures_util::stream::{self, StreamExt};

use pinhole_net::NetError;
use pinhole_registry::wiring::HwContext;
use pinhole_registry::Registry;
use pinhole_store::{InstalledFile, InstalledIndex};

use crate::api::{CivitaiClient, ModelVersion};
use crate::families::{self, FamilyResolution};
use crate::filters::CatalogFilters;
use crate::local::hash_matches;
use crate::select;
use crate::view::{PastedResource, ResolvedResource, ResolvedResources};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Checkpoint,
    Lora,
    Other,
}

pub fn category(filters: &CatalogFilters, kind: &str) -> Category {
    let k = kind.trim().to_ascii_lowercase();
    match k.as_str() {
        "checkpoint" | "model" => Category::Checkpoint,
        "lora" | "lycoris" | "locon" | "dora" => Category::Lora,
        _ if filters.is_lora_type(kind) => Category::Lora,
        _ => Category::Other,
    }
}

/// Installed file for a pasted resource: CivitAI version id, else hash
/// (full SHA-256 or AutoV2), among models or LoRAs.
pub fn match_installed<'a>(
    index: &'a InstalledIndex,
    r: &PastedResource,
    cat: Category,
) -> Option<&'a InstalledFile> {
    let pool: Vec<&InstalledFile> = match cat {
        Category::Checkpoint => index.models().collect(),
        Category::Lora => index.loras().collect(),
        Category::Other => return None,
    };
    if let Some(vid) = r.model_version_id {
        if let Some(f) = pool
            .iter()
            .find(|f| f.civitai.as_ref().is_some_and(|c| c.version_id == vid))
        {
            return Some(f);
        }
    }
    let hash = r.hash.as_deref().map(str::trim).filter(|h| !h.is_empty())?;
    pool.into_iter().find(|f| hash_matches(&f.sha256, hash))
}

pub fn display_name(r: &PastedResource) -> String {
    let name = r
        .model_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let ver = r
        .model_version_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match (name, ver) {
        (Some(n), Some(v)) => format!("{n} · {v}"),
        (Some(n), None) => n.to_string(),
        (None, Some(v)) => v.to_string(),
        (None, None) => match (r.hash.as_deref(), r.model_version_id) {
            (Some(h), _) if !h.trim().is_empty() => format!("Unknown model ({})", h.trim()),
            (_, Some(id)) => format!("CivitAI version {id}"),
            _ => "Unknown model".to_string(),
        },
    }
}

fn ignored_problem(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "embed" | "embedding" | "textualinversion" | "textual inversion" => {
            "Pinhole doesn't support embeddings yet, so this one is skipped.".into()
        }
        "vae" => "Pinhole picks the image decoder (VAE) automatically.".into(),
        "" => "Pinhole doesn't know what this resource is.".into(),
        other => format!("Pinhole doesn't use {other} resources."),
    }
}

/// A pasted resource with a name but no CivitAI version id or file hash.
pub const NO_REFERENCE: &str =
    "Not installed. The pasted settings only name this model, without a CivitAI version or file hash, so Pinhole can't look it up. Find it in Models → Browse.";

/// CivitAI lookups (the real client, or a fake in tests).
pub trait VersionLookup {
    fn version(&self, id: u64) -> impl Future<Output = Result<ModelVersion, NetError>> + Send;
    fn by_hash(
        &self,
        hash: &str,
    ) -> impl Future<Output = Result<Option<ModelVersion>, NetError>> + Send;
}

impl VersionLookup for CivitaiClient {
    fn version(&self, id: u64) -> impl Future<Output = Result<ModelVersion, NetError>> + Send {
        let c = self.clone();
        async move { c.model_version(id).await }
    }
    fn by_hash(
        &self,
        hash: &str,
    ) -> impl Future<Output = Result<Option<ModelVersion>, NetError>> + Send {
        let c = self.clone();
        let h = hash.to_string();
        async move { CivitaiClient::by_hash(&c, &h).await }
    }
}

pub struct PasteEnv<'a> {
    pub registry: &'a Registry,
    pub index: &'a InstalledIndex,
    pub hw: &'a HwContext,
    pub filters: &'a CatalogFilters,
}

fn installed_resolved(
    env: &PasteEnv,
    r: &PastedResource,
    f: &InstalledFile,
    cat: Category,
) -> ResolvedResource {
    let fit = match (
        cat,
        f.family.as_deref().and_then(|id| env.registry.family(id)),
    ) {
        (Category::Checkpoint, Some(fam)) => Some(
            families::need_and_fit(
                env.registry,
                fam,
                env.hw,
                env.index,
                families::installed_need(env.registry, fam, f, env.hw, env.index),
                f.size_bytes,
            )
            .1,
        ),
        _ => None,
    };
    ResolvedResource {
        resource: r.clone(),
        installed_id: Some(f.id.clone()),
        installable_version_id: None,
        display_name: f.friendly_name.clone(),
        family_id: f.family.clone(),
        download_bytes: None,
        fit,
        problem: None,
    }
}

fn from_version(
    env: &PasteEnv,
    r: &PastedResource,
    v: &ModelVersion,
    cat: Category,
) -> ResolvedResource {
    let picked = select::select_file(&v.files, &env.filters.allowed_file_formats);
    let sha = picked.as_ref().ok().and_then(|f| f.sha256());
    if let Some(installed) = sha.as_deref().and_then(|h| env.index.find_by_sha(h)) {
        return installed_resolved(env, r, installed, cat);
    }
    let mut problem = picked.as_ref().err().cloned();
    if crate::api::version_is_person_or_minor(v, None) {
        problem = Some(crate::api::PERSON_OR_MINOR_REASON.into());
    }
    let family_id =
        match families::resolve_family(env.registry, sha.as_deref(), Some(&v.base_model), None) {
            FamilyResolution::Resolved(id) => Some(id),
            FamilyResolution::Ambiguous(ids)
                if !v
                    .base_model
                    .eq_ignore_ascii_case(families::OTHER_BASE_MODEL) =>
            {
                ids.into_iter().next()
            }
            FamilyResolution::Ambiguous(_) => None,
            FamilyResolution::Unsupported(base) => {
                problem.get_or_insert_with(|| families::unsupported_message(base.as_deref()));
                None
            }
        };
    let bytes = picked.as_ref().ok().map(|f| f.size_bytes());
    let fit = match (
        cat,
        family_id.as_deref().and_then(|id| env.registry.family(id)),
        bytes,
    ) {
        (Category::Checkpoint, Some(fam), Some(b)) => Some(
            families::need_and_fit(
                env.registry,
                fam,
                env.hw,
                env.index,
                families::family_need(env.registry, fam, env.hw, env.index, b),
                b,
            )
            .1,
        ),
        _ => None,
    };
    let name = v
        .model
        .as_ref()
        .map(|m| m.name.trim().to_string())
        .filter(|n| !n.is_empty());
    let display_name = match name {
        Some(n) if !v.name.trim().is_empty() => format!("{n} · {}", v.name.trim()),
        Some(n) => n,
        None => display_name(r),
    };
    ResolvedResource {
        resource: r.clone(),
        installed_id: None,
        installable_version_id: problem.is_none().then_some(v.id),
        display_name,
        family_id,
        download_bytes: bytes,
        fit,
        problem,
    }
}

/// CivitAI lookups a paste runs at the same time.
const LOOKUPS_AT_ONCE: usize = 4;

const UNREACHABLE: &str =
    "Couldn't reach CivitAI to look this up. Check your connection and try again.";

async fn resolve_one<L: VersionLookup>(
    env: &PasteEnv<'_>,
    lookup: Option<&L>,
    r: &PastedResource,
    cat: Category,
) -> ResolvedResource {
    if let Some(f) = match_installed(env.index, r, cat) {
        return installed_resolved(env, r, f, cat);
    }
    let unresolved = |problem: &str| ResolvedResource {
        resource: r.clone(),
        installed_id: None,
        installable_version_id: None,
        display_name: display_name(r),
        family_id: None,
        download_bytes: None,
        fit: None,
        problem: Some(problem.to_string()),
    };
    let hash = r.hash.as_deref().map(str::trim).filter(|h| !h.is_empty());
    if r.model_version_id.is_none() && hash.is_none() {
        // Only a name (e.g. ComfyUI images): nothing CivitAI can look up.
        return unresolved(NO_REFERENCE);
    }
    let Some(lookup) = lookup else {
        return unresolved("Not installed. Turn off Offline mode to look it up on CivitAI.");
    };
    let mut network_error = false;
    if let Some(id) = r.model_version_id {
        match lookup.version(id).await {
            Ok(v) => return from_version(env, r, &v, cat),
            Err(NetError::Status(404)) => {}
            // CivitAI unreachable: a by-hash lookup would only wait as long again.
            Err(NetError::Timeout | NetError::Transport(_) | NetError::Offline) => {
                return unresolved(UNREACHABLE)
            }
            Err(_) => network_error = true,
        }
    }
    if let Some(h) = hash {
        match lookup.by_hash(h).await {
            Ok(Some(v)) => return from_version(env, r, &v, cat),
            Ok(None) => {}
            Err(_) => network_error = true,
        }
    }
    if network_error {
        unresolved(UNREACHABLE)
    } else {
        unresolved("Not installed, and CivitAI doesn't know this file.")
    }
}

/// Resolve pasted resources. `lookup` is `None` in Offline mode.
pub async fn resolve_resources<L: VersionLookup + Sync>(
    env: &PasteEnv<'_>,
    lookup: Option<&L>,
    resources: &[PastedResource],
) -> ResolvedResources {
    let mut out = ResolvedResources {
        checkpoint: None,
        loras: Vec::new(),
        ignored: Vec::new(),
    };
    // The checkpoint (first one) and the LoRAs are looked up a few at a time,
    // each list keeping its pasted order.
    let mut to_resolve: Vec<(&PastedResource, Category)> = Vec::new();
    let mut has_checkpoint = false;
    for r in resources.iter().take(64) {
        let cat = category(env.filters, &r.kind);
        match cat {
            Category::Checkpoint if !has_checkpoint => {
                has_checkpoint = true;
                to_resolve.push((r, cat));
            }
            Category::Checkpoint => out.ignored.push(ResolvedResource {
                resource: r.clone(),
                installed_id: None,
                installable_version_id: None,
                display_name: display_name(r),
                family_id: None,
                download_bytes: None,
                fit: None,
                problem: Some("Only one model can be used at a time.".into()),
            }),
            Category::Lora => to_resolve.push((r, cat)),
            Category::Other => out.ignored.push(ResolvedResource {
                resource: r.clone(),
                installed_id: None,
                installable_version_id: None,
                display_name: display_name(r),
                family_id: None,
                download_bytes: None,
                fit: None,
                problem: Some(ignored_problem(&r.kind)),
            }),
        }
    }
    // Futures collected first: a lazy `map` closure in the stream would make
    // the whole future not `Send` (higher-ranked lifetime limits).
    let lookups: Vec<_> = to_resolve
        .iter()
        .map(|&(r, cat)| resolve_one(env, lookup, r, cat))
        .collect();
    let resolved: Vec<ResolvedResource> = stream::iter(lookups)
        .buffered(LOOKUPS_AT_ONCE)
        .collect()
        .await;
    for ((_, cat), res) in to_resolve.iter().zip(resolved) {
        match cat {
            Category::Checkpoint => out.checkpoint = Some(res),
            _ => out.loras.push(res),
        }
    }
    let ckpt_family = out.checkpoint.as_ref().and_then(|c| c.family_id.clone());
    if let Some(ckpt) = ckpt_family.as_deref() {
        for l in out.loras.iter_mut() {
            let Some(lf) = l.family_id.as_deref() else {
                continue;
            };
            if l.problem.is_none() && !families::same_architecture(env.registry, lf, ckpt) {
                let label = |id: &str| {
                    env.registry
                        .family(id)
                        .map(|f| f.label.clone())
                        .unwrap_or_else(|| id.to_string())
                };
                l.problem = Some(format!(
                    "This {} add-on doesn't work with {} models.",
                    label(lf),
                    label(ckpt)
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::tests::filters;
    use pinhole_store::datadir::ModelKind;
    use pinhole_store::installed::CivitaiRef;

    fn file(id: &str, kind: ModelKind, sha: &str, version: Option<u64>) -> InstalledFile {
        InstalledFile {
            id: id.into(),
            rel_path: format!("models/x/{id}.safetensors"),
            kind,
            sha256: sha.into(),
            size_bytes: 1,
            family: Some("sdxl".into()),
            component_id: None,
            friendly_name: id.into(),
            civitai: version.map(|v| CivitaiRef {
                model_id: 1,
                version_id: v,
                model_name: None,
                version_name: None,
                base_model: None,
                trained_words: vec![],
                license: None,
                creator_notes: None,
                sfw_only: false,
            }),
            added_at: 0,
            last_used: None,
            observed_vram_gb: None,
            dtype: None,
            trigger_words: None,
        }
    }

    const SHA_A: &str = "6a35a7855770ae9820a3c931d4964c3817b6d9e3c6f9c4dabb5b3a94e5643b80";
    const SHA_B: &str = "0f4168490e38b8447e11ba4bd656aa11b925bd22af30bac464bc153fdb608501";

    #[test]
    fn categories() {
        let f = filters();
        assert_eq!(category(&f, "checkpoint"), Category::Checkpoint);
        assert_eq!(category(&f, "LORA"), Category::Lora);
        assert_eq!(category(&f, "lycoris"), Category::Lora);
        assert_eq!(category(&f, "LoCon"), Category::Lora);
        assert_eq!(category(&f, "embed"), Category::Other);
        assert_eq!(
            ignored_problem("embed"),
            "Pinhole doesn't support embeddings yet, so this one is skipped."
        );
    }

    #[test]
    fn installed_matching_by_version_and_autov2() {
        let index = InstalledIndex {
            schema_version: 1,
            files: vec![
                file("ckpt", ModelKind::Checkpoint, SHA_A, Some(789646)),
                file("lora", ModelKind::Lora, SHA_B, None),
            ],
            ..Default::default()
        };
        let r = |kind: &str, vid: Option<u64>, hash: Option<&str>| PastedResource {
            kind: kind.into(),
            model_version_id: vid,
            hash: hash.map(Into::into),
            ..Default::default()
        };
        assert_eq!(
            match_installed(
                &index,
                &r("checkpoint", Some(789646), None),
                Category::Checkpoint
            )
            .unwrap()
            .id,
            "ckpt"
        );
        assert_eq!(
            match_installed(
                &index,
                &r("checkpoint", None, Some("6A35A78557")),
                Category::Checkpoint
            )
            .unwrap()
            .id,
            "ckpt"
        );
        assert_eq!(
            match_installed(
                &index,
                &r("checkpoint", Some(1), Some("6a35a78557")),
                Category::Checkpoint
            )
            .unwrap()
            .id,
            "ckpt"
        );
        assert_eq!(
            match_installed(&index, &r("lora", None, Some("0F4168490E")), Category::Lora)
                .unwrap()
                .id,
            "lora"
        );
        assert!(
            match_installed(&index, &r("lora", None, Some("6A35A78557")), Category::Lora).is_none(),
            "a checkpoint is not a LoRA"
        );
        assert!(match_installed(
            &index,
            &r("checkpoint", None, Some("0F4168490E38")),
            Category::Checkpoint
        )
        .is_none());
        assert!(
            match_installed(&index, &r("checkpoint", None, None), Category::Checkpoint).is_none()
        );
    }

    #[test]
    fn display_names() {
        let r = PastedResource {
            model_name: Some("Juggernaut XL".into()),
            model_version_name: Some("Ragnarok".into()),
            ..Default::default()
        };
        assert_eq!(display_name(&r), "Juggernaut XL · Ragnarok");
        let r = PastedResource {
            hash: Some("ABCDEF0123".into()),
            ..Default::default()
        };
        assert_eq!(display_name(&r), "Unknown model (ABCDEF0123)");
    }
}

#[cfg(test)]
mod resolve_tests {
    use std::collections::HashMap;

    use super::*;
    use crate::filters::tests::filters;
    use crate::testkit::{hw, index, model, registry};
    use pinhole_registry::vram::Fit;
    use pinhole_store::datadir::ModelKind;

    struct FakeLookup {
        by_id: HashMap<u64, ModelVersion>,
        by_hash: HashMap<String, ModelVersion>,
        fail: bool,
    }

    impl VersionLookup for FakeLookup {
        fn version(&self, id: u64) -> impl Future<Output = Result<ModelVersion, NetError>> + Send {
            let r = if self.fail {
                Err(NetError::Timeout)
            } else {
                self.by_id.get(&id).cloned().ok_or(NetError::Status(404))
            };
            async move { r }
        }
        fn by_hash(
            &self,
            hash: &str,
        ) -> impl Future<Output = Result<Option<ModelVersion>, NetError>> + Send {
            let r = if self.fail {
                Err(NetError::Timeout)
            } else {
                Ok(self.by_hash.get(&hash.to_ascii_uppercase()).cloned())
            };
            async move { r }
        }
    }

    fn lookup(fail: bool) -> FakeLookup {
        let jugg: ModelVersion =
            serde_json::from_str(include_str!("../tests/fixtures/model_version.json")).unwrap();
        let lora: ModelVersion =
            serde_json::from_str(include_str!("../tests/fixtures/by_hash_lora.json")).unwrap();
        FakeLookup {
            by_id: HashMap::from([(1759168, jugg)]),
            by_hash: HashMap::from([("0F4168490E".to_string(), lora)]),
            fail,
        }
    }

    fn res(kind: &str, vid: Option<u64>, hash: Option<&str>) -> PastedResource {
        PastedResource {
            kind: kind.into(),
            model_version_id: vid,
            hash: hash.map(Into::into),
            weight: Some(0.8),
            ..Default::default()
        }
    }

    /// Answers every version id after `delay` (or times out), counting calls.
    struct SlowLookup {
        delay: std::time::Duration,
        timeout: bool,
        versions: std::sync::atomic::AtomicUsize,
        hashes: std::sync::atomic::AtomicUsize,
    }

    impl VersionLookup for SlowLookup {
        fn version(&self, id: u64) -> impl Future<Output = Result<ModelVersion, NetError>> + Send {
            use std::sync::atomic::Ordering;
            self.versions.fetch_add(1, Ordering::SeqCst);
            let (delay, timeout) = (self.delay, self.timeout);
            async move {
                tokio::time::sleep(delay).await;
                if timeout {
                    return Err(NetError::Timeout);
                }
                Ok(ModelVersion {
                    id,
                    name: format!("v{id}"),
                    ..Default::default()
                })
            }
        }
        fn by_hash(
            &self,
            _hash: &str,
        ) -> impl Future<Output = Result<Option<ModelVersion>, NetError>> + Send {
            self.hashes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { Ok(None) }
        }
    }

    fn slow(delay_ms: u64, timeout: bool) -> SlowLookup {
        SlowLookup {
            delay: std::time::Duration::from_millis(delay_ms),
            timeout,
            versions: Default::default(),
            hashes: Default::default(),
        }
    }

    /// RELEASE-SPEC §5: a pasted real-person / minor model gets no "Get" button.
    #[tokio::test]
    async fn real_person_and_minor_models_are_not_installable() {
        let (reg, f, idx, h) = (registry(), filters(), index(vec![]), hw(12.0));
        let env = PasteEnv {
            registry: &reg,
            index: &idx,
            hw: &h,
            filters: &f,
        };
        let ok = resolve_resources(
            &env,
            Some(&lookup(false)),
            &[res("checkpoint", Some(1759168), None)],
        )
        .await;
        assert_eq!(ok.checkpoint.unwrap().installable_version_id, Some(1759168));
        for flag in ["poi", "minor"] {
            let mut l = lookup(false);
            let v = l.by_id.get_mut(&1759168).unwrap();
            let m = v.model.get_or_insert_with(Default::default);
            m.poi = flag == "poi";
            m.minor = flag == "minor";
            let out =
                resolve_resources(&env, Some(&l), &[res("checkpoint", Some(1759168), None)]).await;
            assert_eq!(
                out.checkpoint.as_ref().unwrap().installable_version_id,
                None,
                "{flag}"
            );
            assert_eq!(
                out.checkpoint.as_ref().unwrap().problem.as_deref(),
                Some(crate::api::PERSON_OR_MINOR_REASON)
            );
        }
    }

    /// Pasted LoRAs are looked up a few at a time, in their pasted order.
    #[tokio::test]
    async fn lookups_run_concurrently_in_order() {
        let (reg, f, idx, h) = (registry(), filters(), index(vec![]), hw(12.0));
        let env = PasteEnv {
            registry: &reg,
            index: &idx,
            hw: &h,
            filters: &f,
        };
        let l = slow(300, false);
        let pasted: Vec<PastedResource> = (1..=4)
            .map(|i| res("lora", Some(i), None))
            .chain([res("checkpoint", Some(9), None)])
            .collect();
        let t0 = std::time::Instant::now();
        let out = resolve_resources(&env, Some(&l), &pasted).await;
        assert!(
            t0.elapsed() < std::time::Duration::from_millis(1100),
            "5 lookups of 300 ms took {:?}",
            t0.elapsed()
        );
        let ids: Vec<Option<u64>> = out
            .loras
            .iter()
            .map(|r| r.resource.model_version_id)
            .collect();
        assert_eq!(ids, [Some(1), Some(2), Some(3), Some(4)]);
        assert_eq!(out.checkpoint.unwrap().resource.model_version_id, Some(9));
    }

    /// CivitAI didn't answer the version lookup: no by-hash lookup after it.
    #[tokio::test]
    async fn unreachable_civitai_skips_the_hash_lookup() {
        let (reg, f, idx, h) = (registry(), filters(), index(vec![]), hw(12.0));
        let env = PasteEnv {
            registry: &reg,
            index: &idx,
            hw: &h,
            filters: &f,
        };
        let l = slow(0, true);
        let out =
            resolve_resources(&env, Some(&l), &[res("lora", Some(1), Some("ABCDEF1234"))]).await;
        assert_eq!(out.loras[0].problem.as_deref(), Some(UNREACHABLE));
        assert_eq!(l.hashes.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(l.versions.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn resolves_installable_and_flags_mismatched_loras() {
        let reg = registry();
        let f = filters();
        let mut pony = model("pony-lora", "sdxl_pony", ModelKind::Lora, "p.safetensors");
        pony.sha256 = "1b6405d1ef5a816105210b20a8f0fc129869a25876e45c0891e4f6d972bf74c2".into();
        let idx = index(vec![pony]);
        let h = hw(12.0);
        let env = PasteEnv {
            registry: &reg,
            index: &idx,
            hw: &h,
            filters: &f,
        };
        let l = lookup(false);
        let out = resolve_resources(
            &env,
            Some(&l),
            &[
                res("checkpoint", Some(1759168), Some("DD08FA32F9")),
                res("lora", None, Some("0F4168490E")),
                res("lora", None, Some("1B6405D1EF")),
                res("embed", Some(5), None),
                res("checkpoint", Some(1), None),
                res("lora", Some(99), Some("ABCDEF1234")),
            ],
        )
        .await;
        let ckpt = out.checkpoint.unwrap();
        assert_eq!(ckpt.installed_id, None);
        assert_eq!(ckpt.installable_version_id, Some(1759168));
        assert_eq!(ckpt.display_name, "Juggernaut XL · Ragnarok");
        assert_eq!(ckpt.family_id.as_deref(), Some("sdxl"));
        assert_eq!(ckpt.download_bytes, Some(7_105_349_736));
        assert_eq!(ckpt.fit, Some(Fit::Fits));
        assert_eq!(ckpt.problem, None);
        assert_eq!(ckpt.resource.weight, Some(0.8));

        assert_eq!(out.loras.len(), 3);
        let sd15 = &out.loras[0];
        assert_eq!(sd15.installable_version_id, Some(91234));
        assert_eq!(sd15.family_id.as_deref(), Some("sd15"));
        assert_eq!(
            sd15.problem.as_deref(),
            Some("This Stable Diffusion 1.5 add-on doesn't work with SDXL models.")
        );
        let pony = &out.loras[1];
        assert_eq!(
            pony.installed_id.as_deref(),
            Some("pony-lora"),
            "installed LoRA matched by AutoV2"
        );
        assert_eq!(pony.problem, None, "Pony LoRAs are SDXL-architecture");
        let unknown = &out.loras[2];
        assert_eq!(
            unknown.problem.as_deref(),
            Some("Not installed, and CivitAI doesn't know this file.")
        );

        assert_eq!(out.ignored.len(), 2);
        assert!(out
            .ignored
            .iter()
            .any(|r| r.problem.as_deref() == Some("Only one model can be used at a time.")));
        assert!(out.ignored.iter().any(|r| r
            .problem
            .as_deref()
            .is_some_and(|p| p.contains("embeddings"))));
        let json = serde_json::to_value(&out.loras[0]).unwrap();
        assert_eq!(json["resource"]["type"], "lora");
        assert!(json.get("installableVersionId").is_some());
    }

    #[tokio::test]
    async fn installed_checkpoint_by_version_id() {
        let reg = registry();
        let f = filters();
        let idx = index(vec![crate::testkit::with_civitai(
            model("jugg", "sdxl", ModelKind::Checkpoint, "j.safetensors"),
            1759168,
        )]);
        let h = hw(8.0);
        let env = PasteEnv {
            registry: &reg,
            index: &idx,
            hw: &h,
            filters: &f,
        };
        let out =
            resolve_resources::<FakeLookup>(&env, None, &[res("checkpoint", Some(1759168), None)])
                .await;
        let c = out.checkpoint.unwrap();
        assert_eq!(c.installed_id.as_deref(), Some("jugg"));
        assert_eq!(c.fit, Some(Fit::Tight));
    }

    #[tokio::test]
    async fn offline_and_network_errors() {
        let reg = registry();
        let f = filters();
        let idx = index(vec![]);
        let h = hw(8.0);
        let env = PasteEnv {
            registry: &reg,
            index: &idx,
            hw: &h,
            filters: &f,
        };
        let out =
            resolve_resources::<FakeLookup>(&env, None, &[res("checkpoint", Some(1759168), None)])
                .await;
        assert!(out
            .checkpoint
            .unwrap()
            .problem
            .unwrap()
            .contains("Offline mode"));
        let l = lookup(true);
        let out =
            resolve_resources(&env, Some(&l), &[res("checkpoint", Some(1759168), None)]).await;
        assert!(out
            .checkpoint
            .unwrap()
            .problem
            .unwrap()
            .contains("Couldn't reach CivitAI"));
    }

    #[tokio::test]
    async fn name_only_resources_say_why_they_cannot_be_looked_up() {
        let reg = registry();
        let f = filters();
        let idx = index(vec![]);
        let h = hw(8.0);
        let env = PasteEnv {
            registry: &reg,
            index: &idx,
            hw: &h,
            filters: &f,
        };
        let named = PastedResource {
            kind: "checkpoint".into(),
            model_name: Some("juggernautXL_v9".into()),
            ..Default::default()
        };
        for l in [None, Some(&lookup(false))] {
            let c = resolve_resources(&env, l, std::slice::from_ref(&named))
                .await
                .checkpoint
                .unwrap();
            assert_eq!(c.problem.as_deref(), Some(NO_REFERENCE));
            assert_eq!(c.display_name, "juggernautXL_v9");
        }
    }
}
