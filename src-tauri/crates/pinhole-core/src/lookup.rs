//! The CivitAI by-hash lookup of models and add-ons added by hand ("Add a file") or found in
//! a linked folder (RELEASE-SPEC §5). A file CivitAI marks as showing a real person or a
//! minor is refused; a file it marks "safe images only", doesn't know, or that hasn't been
//! looked up yet (Offline mode, a failed lookup) can't make intimate pictures (§3.2 rule 3).
//! Files Pinhole offers itself (known by their hash) never count as "safe images only"; they
//! are looked up only when added by hand.
//!
//! PRIVACY: only the file's SHA-256 goes to civitai.com, and only when the user adds a file,
//! adds a linked folder, presses "Check again" on linked folders, or turns Offline mode off
//! (one retry of what wasn't looked up yet). Never at start or in the background.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use pinhole_catalog::api::CivitaiClient;
use pinhole_registry::Registry;
use pinhole_store::datadir::ModelKind;
use pinhole_store::installed::{linked_folder_id, CivitaiRef, InstalledFile, Lookup, NotUsed};
use pinhole_store::InstalledIndex;

use crate::{AppCore, CoreEvent};

const TIMEOUT: Duration = Duration::from_secs(20);

/// What CivitAI said about one file.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Known: the model's data and flags, and the name to show.
    Found {
        civitai: Box<CivitaiRef>,
        friendly_name: String,
    },
    /// Marked as showing a real person or someone under 18.
    PersonOrMinor,
    /// CivitAI doesn't know the file.
    NoMatch,
    /// Offline, or the lookup failed: try again when Offline mode is turned off.
    Failed,
}

impl Outcome {
    /// The state stored on the file.
    pub fn lookup(&self) -> Lookup {
        match self {
            Outcome::Found { .. } => Lookup::Found,
            Outcome::PersonOrMinor => Lookup::Refused,
            Outcome::NoMatch => Lookup::NoMatch,
            Outcome::Failed => Lookup::NotYet,
        }
    }
}

/// Main models and add-ons get the lookup; parts (VAE, encoders) and helpers don't.
pub(crate) fn looked_up_kind(kind: ModelKind) -> bool {
    matches!(
        kind,
        ModelKind::Checkpoint | ModelKind::Diffusion | ModelKind::Lora
    )
}

/// The state a file added by hand or found in a linked folder starts with: `Shipped` for a
/// file Pinhole offers itself (known by the hash of its own bytes), else not looked up yet.
/// Parts and helpers have none.
pub fn initial(registry: &Registry, kind: ModelKind, sha256: &str) -> Option<Lookup> {
    if !looked_up_kind(kind) {
        None
    } else if registry.is_shipped_file(sha256) {
        Some(Lookup::Shipped)
    } else {
        Some(Lookup::NotYet)
    }
}

/// Files added by hand or linked before lookups existed: mark them "not looked up yet", so
/// they count as "safe images only" until Offline mode is turned off or they're added again.
/// A file without CivitAI data whose hash isn't one Pinhole offers was added by hand; only
/// the hash counts, never the file name or family (a one-click download whose pin changed
/// keeps its trust because the old hash moves to `known_files` in `models.yaml`). Linked
/// models and add-ons (also those of a folder whose drive isn't connected) lose the hash they
/// had, which may have come from another app's note, so the next look hashes the file itself.
/// Returns whether anything changed.
pub fn mark_unchecked(registry: &Registry, index: &mut InstalledIndex) -> bool {
    let mut changed = false;
    let parked = index.linked.parked.iter_mut().map(|e| &mut e.file);
    for f in index.files.iter_mut().chain(parked) {
        if f.lookup.is_some() || f.component_id.is_some() || !looked_up_kind(f.kind) {
            continue;
        }
        if f.is_linked() {
            f.sha256.clear();
            f.lookup = Some(Lookup::NotYet);
            changed = true;
        } else if f.civitai.is_none() && !registry.is_shipped_file(&f.sha256) {
            f.lookup = Some(Lookup::NotYet);
            changed = true;
        }
    }
    changed
}

/// Ask CivitAI about one file by its SHA-256 (and the model's flags, as for a Browse install).
pub async fn look_up_with(client: &CivitaiClient, sha256: &str) -> Outcome {
    let v = match tokio::time::timeout(TIMEOUT, client.by_hash(sha256)).await {
        Ok(Ok(Some(v))) => v,
        Ok(Ok(None)) => return Outcome::NoMatch,
        _ => return Outcome::Failed,
    };
    let model = if v.model_id > 0 {
        tokio::time::timeout(TIMEOUT, client.model(v.model_id))
            .await
            .ok()
            .and_then(Result::ok)
    } else {
        None
    };
    if pinhole_catalog::api::version_is_person_or_minor(&v, model.as_ref()) {
        return Outcome::PersonOrMinor;
    }
    Outcome::Found {
        friendly_name: pinhole_catalog::plan::friendly_name(&v, None),
        civitai: Box::new(CivitaiRef {
            model_id: v.model_id,
            version_id: v.id,
            model_name: Some(pinhole_catalog::plan::model_name(&v, None)),
            version_name: Some(v.name.clone()).filter(|n| !n.is_empty()),
            base_model: Some(v.base_model.clone()).filter(|b| !b.is_empty()),
            trained_words: v.trained_words.clone(),
            license: None,
            creator_notes: None,
            // Fails closed when the model's data couldn't be fetched.
            sfw_only: pinhole_catalog::api::sfw_only_of(&v, model.as_ref()),
        }),
    }
}

/// [`look_up_with`] through Pinhole's CivitAI client. Offline mode: `Failed`, nothing sent.
pub async fn look_up(core: &AppCore, sha256: &str) -> Outcome {
    if core.offline.get() || sha256.trim().is_empty() {
        return Outcome::Failed;
    }
    // Tests never reach the real CivitAI: only a mock server set for the test.
    #[cfg(test)]
    if core.models.test_civitai.lock().is_none() {
        return Outcome::Failed;
    }
    let client = crate::catalog::civitai_client(core).await;
    look_up_with(&client, sha256).await
}

/// Store what a lookup found on an installed or linked file. A linked file CivitAI marks as
/// showing a real person or a minor leaves the list (like one whose note says so); a file
/// added by hand stays listed but can't be used. Returns whether the index changed.
pub fn apply(index: &mut InstalledIndex, id: &str, sha256: &str, outcome: &Outcome) -> bool {
    let Some(f) = index.get(id) else {
        return false; // removed meanwhile
    };
    if !f.sha256.eq_ignore_ascii_case(sha256) {
        return false; // the file changed meanwhile; its new state is looked up on its own
    }
    if *outcome == Outcome::PersonOrMinor && f.is_linked() {
        let rel = f.rel_path.clone();
        let stamp = index.linked.stamps.get(id).copied().unwrap_or_default();
        index.remove(id);
        index.linked.not_used.insert(
            rel,
            NotUsed {
                stamp,
                flagged: true,
            },
        );
        return true;
    }
    let Some(f) = index.get_mut(id) else {
        return false;
    };
    let before = f.clone();
    f.lookup = Some(outcome.lookup());
    if let Outcome::Found { civitai: c, .. } = outcome {
        if f.trigger_words.is_none() && c.trained_words.is_empty() {
            // Keep a linked note's trigger words when CivitAI lists none.
            let kept = f.civitai.as_ref().map(|old| old.trained_words.clone());
            f.civitai = Some((**c).clone());
            if let (Some(words), Some(new)) = (kept, f.civitai.as_mut()) {
                new.trained_words = words;
            }
        } else {
            f.civitai = Some((**c).clone());
        }
    }
    *f != before
}

/// Look up every file not looked up yet, one at a time, once. `folder`: only the files of that
/// linked folder. Runs after a linked folder is added or looked through again on request, and
/// when Offline mode is turned off; a failed lookup stays "not looked up yet".
pub async fn look_up_pending(core: &Arc<AppCore>, folder: Option<&str>) {
    let todo: Vec<(String, String)> = {
        let index = core.installed.lock();
        index
            .files
            .iter()
            .filter(|f| f.lookup == Some(Lookup::NotYet) && !f.sha256.is_empty())
            .filter(|f| folder.is_none() || linked_folder_id(&f.rel_path) == folder)
            .map(|f| (f.id.clone(), f.sha256.clone()))
            .collect()
    };
    if todo.is_empty() || core.offline.get() {
        return;
    }
    // A file being looked up by another pass is left to it.
    let mine: Vec<(String, String)> = {
        let mut running = core.models.lookups.lock();
        todo.into_iter()
            .filter(|(id, _)| running.insert(id.clone()))
            .collect()
    };
    let _release = Release(core, mine.iter().map(|(id, _)| id.clone()).collect());
    for (id, sha) in &mine {
        if core.offline.get() {
            break;
        }
        look_up_one(core, id, sha).await;
    }
}

/// Look up one installed or linked file and store what CivitAI said.
pub async fn look_up_one(core: &AppCore, id: &str, sha256: &str) -> Outcome {
    let outcome = look_up(core, sha256).await;
    let saved = {
        let mut index = core.installed.lock();
        let before = index.clone();
        let linked = index.get(id).is_some_and(InstalledFile::is_linked);
        if !apply(&mut index, id, sha256, &outcome) {
            return outcome;
        }
        // A linked file's result goes to the linked list only (the Models folder may be
        // on a drive that isn't connected).
        let saved = if linked {
            index.save_linked(&core.data)
        } else {
            index.save(&core.data)
        };
        if saved.is_err() {
            *index = before;
        }
        saved.is_ok()
    };
    if saved {
        core.emit(CoreEvent::ModelsChanged);
    }
    outcome
}

/// Releases a pass's files from [`crate::models::ModelsState::lookups`].
struct Release<'a>(&'a AppCore, HashSet<String>);

impl Drop for Release<'_> {
    fn drop(&mut self) {
        let mut running = self.0.models.lookups.lock();
        for id in &self.1 {
            running.remove(id);
        }
    }
}

/// Offline mode was just turned off: one retry of what wasn't looked up yet.
pub fn went_online(core: &Arc<AppCore>) {
    let Ok(rt) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let core = core.clone();
    rt.spawn(async move { look_up_pending(&core, None).await });
}

/// For a file in use: refuse one CivitAI marks as showing a real person or a minor.
pub fn refuse_if_flagged(f: &InstalledFile) -> crate::CoreResult<()> {
    if f.refused() {
        return Err(crate::CoreError::invalid(
            pinhole_catalog::api::PERSON_OR_MINOR_REASON,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use pinhole_net::testutil::{MockResponse, MockServer};

    use super::*;
    use crate::app::tests::{test_core, Recorder};
    use crate::linked::fixtures;

    /// CivitAI stand-in: `answers` maps an upper-case SHA-256 to the model behind it
    /// (`Some(person_or_minor)`); any other hash is unknown (404).
    async fn civitai(answers: Vec<(String, bool)>) -> MockServer {
        MockServer::start(move |req| {
            if let Some(hash) = req.path.strip_prefix("/model-versions/by-hash/") {
                return match answers.iter().position(|(h, _)| h == hash) {
                    Some(i) => MockResponse::json(&serde_json::json!({
                        "id": 100 + i, "modelId": 10 + i, "name": "v1", "baseModel": "SDXL 1.0",
                        "trainedWords": ["inkwash"]
                    })),
                    None => MockResponse::status(404),
                };
            }
            if let Some(id) = req.path.strip_prefix("/models/") {
                let i: usize = id.parse::<usize>().unwrap() - 10;
                return MockResponse::json(&serde_json::json!({
                    "id": 10 + i, "name": "A model", "poi": answers[i].1
                }));
            }
            MockResponse::status(500)
        })
        .await
    }

    fn sha(path: &Path) -> String {
        pinhole_catalog::local::hash_file(path).unwrap().0
    }

    fn by_hash_requests(srv: &MockServer) -> usize {
        srv.requests()
            .iter()
            .filter(|r| r.path.starts_with("/model-versions/by-hash/"))
            .count()
    }

    fn set_offline(core: &Arc<AppCore>, offline: bool) {
        let mut s = crate::app::get_settings(core);
        s.offline = offline;
        crate::app::set_settings(core, s).unwrap();
    }

    async fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(20), "{what}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn lora(src: &Path, style: &str) {
        fixtures::write_safetensors(
            src,
            &[
                (
                    "lora_unet_input_blocks_1_1_proj_in.lora_down.weight",
                    &[8, 320],
                ),
                (
                    "lora_unet_input_blocks_1_1_proj_in.lora_up.weight",
                    &[320, 8],
                ),
            ],
            &[
                ("ss_base_model_version", "sdxl_base_v1-0"),
                ("style", style),
            ],
        );
    }

    fn only_file(core: &AppCore) -> InstalledFile {
        let idx = core.installed.lock();
        assert_eq!(idx.files.len(), 1, "{:?}", idx.files);
        idx.files[0].clone()
    }

    /// Regression: the lookup used to run only when the family wasn't known locally, so a
    /// file whose family Pinhole knew by its hash was never checked for a real person.
    #[tokio::test]
    async fn a_resolved_family_is_still_looked_up_and_a_person_match_refused() {
        let (tmp, core) = test_core(Arc::new(Recorder::default()));
        let src = tmp.path().join("Known.safetensors");
        fixtures::sdxl(&src);
        let hash = sha(&src);
        // Pinhole knows this file's family by its hash (as if the shipped models.yaml listed it).
        let yaml = std::fs::read_to_string(core.shipped.config_dir.join("models.yaml")).unwrap();
        let yaml = yaml.replacen(
            "\nknown_files:\n",
            &format!("\nknown_files:\n  - {{ sha256: {hash}, family: sdxl }}\n"),
            1,
        );
        *core.registry.write() = Arc::new(Registry::from_yaml(&yaml, None).unwrap());
        assert!(core.registry().is_shipped_file(&hash));
        let srv = civitai(vec![(hash.to_ascii_uppercase(), true)]).await;
        *core.models.test_civitai.lock() = Some(srv.url(""));

        let e = crate::models::add_local_model(&core, src.to_str().unwrap())
            .await
            .unwrap_err();
        assert_eq!(e.message, pinhole_catalog::api::PERSON_OR_MINOR_REASON);
        assert_eq!(by_hash_requests(&srv), 1);
        assert!(core.installed.lock().files.is_empty());
        let copies = std::fs::read_dir(core.data.models(ModelKind::Checkpoint)).unwrap();
        assert_eq!(copies.count(), 0, "the copy is removed");
    }

    /// Regression: Offline mode used to skip the lookup and leave the file unmarked. It now
    /// counts as "safe images only" until the one retry when Offline mode is turned off.
    #[tokio::test]
    async fn an_offline_add_is_safe_images_only_until_going_online() {
        let (tmp, core) = test_core(Arc::new(Recorder::default()));
        let src = tmp.path().join("Inkwash.safetensors");
        lora(&src, "ink");
        let srv = civitai(vec![(sha(&src).to_ascii_uppercase(), false)]).await;
        *core.models.test_civitai.lock() = Some(srv.url(""));
        set_offline(&core, true);

        crate::models::add_local_model(&core, src.to_str().unwrap())
            .await
            .unwrap();
        let f = only_file(&core);
        assert_eq!(f.lookup, Some(Lookup::NotYet));
        assert!(f.safe_images_only());
        assert_eq!(by_hash_requests(&srv), 0, "nothing sent offline");
        let view = crate::models::list_loras(&core).unwrap();
        assert!(view[0].safe_images_only);

        set_offline(&core, false);
        wait_for("looked up", || {
            only_file(&core).lookup == Some(Lookup::Found)
        })
        .await;
        let f = only_file(&core);
        assert!(!f.safe_images_only());
        assert_eq!(f.civitai.as_ref().unwrap().trained_words, ["inkwash"]);
        assert_eq!(by_hash_requests(&srv), 1);

        // Found files aren't looked up again.
        set_offline(&core, true);
        set_offline(&core, false);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(by_hash_requests(&srv), 1);
    }

    #[tokio::test]
    async fn a_file_civitai_does_not_know_is_safe_images_only() {
        let (tmp, core) = test_core(Arc::new(Recorder::default()));
        let src = tmp.path().join("Mine.safetensors");
        lora(&src, "mine");
        let srv = civitai(vec![]).await;
        *core.models.test_civitai.lock() = Some(srv.url(""));

        crate::models::add_local_model(&core, src.to_str().unwrap())
            .await
            .unwrap();
        let f = only_file(&core);
        assert_eq!(f.lookup, Some(Lookup::NoMatch));
        assert!(f.safe_images_only());
        assert_eq!(by_hash_requests(&srv), 1);
        // No match is an answer: going online doesn't ask again.
        set_offline(&core, true);
        set_offline(&core, false);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(by_hash_requests(&srv), 1);
    }

    /// Regression: linked files used to be screened only when another app left a
    /// `.civitai.info` note next to them.
    #[tokio::test]
    async fn linked_files_without_a_note_are_looked_up() {
        let (tmp, core) = test_core(Arc::new(Recorder::default()));
        let comfy = tmp.path().join("ComfyUI");
        let clean = comfy.join("models/loras/clean.safetensors");
        lora(&clean, "clean");
        let srv = civitai(vec![(sha(&clean).to_ascii_uppercase(), false)]).await;
        *core.models.test_civitai.lock() = Some(srv.url(""));

        crate::linked::add(&core, &comfy.display().to_string()).unwrap();
        wait_for("looked up", || {
            core.installed.lock().files.first().and_then(|f| f.lookup) == Some(Lookup::Found)
        })
        .await;
        assert!(!only_file(&core).safe_images_only());

        // A new file found by the look at start or when Models opens waits.
        let person = comfy.join("models/loras/person.safetensors");
        lora(&person, "person");
        let person_sha = sha(&person);
        crate::linked::rescan_all(&core, false);
        wait_for("scanned", || core.installed.lock().files.len() == 2).await;
        wait_for("scan done", || core.linked.scans.lock().running.is_empty()).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let waiting = core
            .installed
            .lock()
            .files
            .iter()
            .find(|f| f.sha256 == person_sha)
            .cloned()
            .unwrap();
        assert_eq!(waiting.lookup, Some(Lookup::NotYet));
        assert!(waiting.safe_images_only());
        assert_eq!(by_hash_requests(&srv), 1);

        // "Check again": looked up; CivitAI marks it as a real person, so it leaves the list.
        *core.models.test_civitai.lock() = None;
        let srv2 = civitai(vec![
            (sha(&clean).to_ascii_uppercase(), false),
            (person_sha.to_ascii_uppercase(), true),
        ])
        .await;
        *core.models.test_civitai.lock() = Some(srv2.url(""));
        crate::linked::rescan_all(&core, true);
        wait_for("refused", || core.installed.lock().files.len() == 1).await;
        assert_eq!(only_file(&core).sha256, sha(&clean));
        assert_eq!(
            by_hash_requests(&srv2),
            1,
            "only the file not looked up yet"
        );
        assert!(core
            .installed
            .lock()
            .linked
            .not_used
            .values()
            .any(|n| n.flagged));
        // Another look keeps both answers.
        crate::linked::rescan_all(&core, false);
        wait_for("scan done", || core.linked.scans.lock().running.is_empty()).await;
        let f = only_file(&core);
        assert_eq!((f.sha256, f.lookup), (sha(&clean), Some(Lookup::Found)));
    }

    /// Linked models and add-ons found before they were hashed get their SHA-256 at the next
    /// look, so they can be looked up.
    #[tokio::test]
    async fn linked_files_from_before_are_hashed_again() {
        let (tmp, core) = test_core(Arc::new(Recorder::default()));
        let comfy = tmp.path().join("ComfyUI");
        let style = comfy.join("models/loras/style.safetensors");
        lora(&style, "style");
        set_offline(&core, true);
        crate::linked::add(&core, &comfy.display().to_string()).unwrap();
        wait_for("scanned", || core.installed.lock().files.len() == 1).await;
        wait_for("scan done", || core.linked.scans.lock().running.is_empty()).await;
        {
            let mut idx = core.installed.lock();
            idx.files[0].sha256.clear();
            idx.files[0].lookup = None;
            assert!(mark_unchecked(&core.registry(), &mut idx));
        }
        crate::linked::rescan_all(&core, false);
        wait_for("hashed", || only_file(&core).sha256 == sha(&style)).await;
        assert_eq!(only_file(&core).lookup, Some(Lookup::NotYet));
    }

    /// Regression (1.0.1): a linked copy of a file Pinhole offers itself got no lookup state,
    /// so every start cleared its hash and the whole file was read again.
    #[test]
    fn a_linked_copy_of_a_shipped_file_keeps_its_hash() {
        let (_tmp, core) = test_core(Arc::new(Recorder::default()));
        let registry = core.registry();
        let shipped = registry.known_files()[0].sha256.clone();
        let lookup = initial(&registry, ModelKind::Diffusion, &shipped);
        assert_eq!(lookup, Some(Lookup::Shipped));
        assert_eq!(
            initial(&registry, ModelKind::Lora, &"ab".repeat(32)),
            Some(Lookup::NotYet)
        );
        assert_eq!(initial(&registry, ModelKind::Vae, &shipped), None);
        let mut index = InstalledIndex {
            files: vec![InstalledFile {
                id: "linked".into(),
                rel_path: "linked/abc/diffusion_models/z.safetensors".into(),
                kind: ModelKind::Diffusion,
                sha256: shipped.clone(),
                size_bytes: 1,
                family: None,
                component_id: None,
                friendly_name: "z".into(),
                civitai: None,
                added_at: 0,
                last_used: None,
                observed_vram_gb: None,
                dtype: None,
                trigger_words: None,
                lookup,
            }],
            ..Default::default()
        };
        assert!(!mark_unchecked(&registry, &mut index), "next start");
        assert_eq!(index.files[0].sha256, shipped);
        assert!(!index.files[0].safe_images_only());
    }

    #[test]
    fn files_from_before_are_marked_unchecked() {
        let (_tmp, core) = test_core(Arc::new(Recorder::default()));
        let registry = core.registry();
        let shipped = registry.known_files()[0].sha256.clone();
        let file = |id: &str, kind, sha: &str, civitai: bool, component: bool| InstalledFile {
            id: id.into(),
            rel_path: format!("models/x/{id}.safetensors"),
            kind,
            sha256: sha.into(),
            size_bytes: 1,
            family: None,
            component_id: component.then(|| "vae".into()),
            friendly_name: id.into(),
            civitai: civitai.then(|| CivitaiRef {
                model_id: 1,
                version_id: 2,
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
            lookup: None,
        };
        // Regression (Codex audit): a file named like a family's download, with a hash
        // Pinhole doesn't know, used to be trusted by its name. Only the hash counts now.
        let fam = registry.families().find(|f| f.download.is_some()).unwrap();
        let dl = fam.download.as_ref().unwrap();
        let mut old_pin = file(
            "old_pin",
            ModelKind::Checkpoint,
            &"aa".repeat(32),
            false,
            false,
        );
        old_pin.family = Some(fam.id.clone());
        old_pin.rel_path = format!("models/checkpoints/{}", dl.file);
        // The same download with its pinned hash stays trusted.
        let mut download = old_pin.clone();
        download.id = "download".into();
        download.sha256 = dl.sha256.clone();
        // Linked, with a hash taken from another app's note: hashed again at the next look.
        let mut linked = file("linked", ModelKind::Lora, &shipped, false, false);
        linked.rel_path = "linked/abc/loras/x.safetensors".into();
        let mut parked = linked.clone();
        parked.id = "parked".into();
        let mut index = InstalledIndex {
            files: vec![
                file("hand", ModelKind::Lora, &"ab".repeat(32), false, false),
                file("browse", ModelKind::Lora, &"cd".repeat(32), true, false),
                file("shipped", ModelKind::Diffusion, &shipped, false, false),
                file("part", ModelKind::Vae, &"ef".repeat(32), false, true),
                old_pin,
                linked,
                download,
            ],
            ..Default::default()
        };
        index
            .linked
            .parked
            .push(pinhole_store::installed::LinkedEntry {
                file: parked,
                stamp: Default::default(),
            });
        assert!(mark_unchecked(&registry, &mut index));
        let state: Vec<_> = index.files.iter().map(|f| f.lookup).collect();
        let not_yet = Some(Lookup::NotYet);
        assert_eq!(state, [not_yet, None, None, None, not_yet, not_yet, None]);
        assert_eq!(index.files[5].sha256, "");
        assert_eq!(index.linked.parked[0].file.lookup, not_yet);
        assert_eq!(index.linked.parked[0].file.sha256, "");
        assert!(!mark_unchecked(&registry, &mut index), "only once");
    }
}
