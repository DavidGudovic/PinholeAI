//! Signed model lookup results with signing turned on. Its own test binary: signing is turned on
//! for the whole process, so no other test runs with it.

use std::sync::Arc;

use pinhole_core::lookup::{look_up_existing, look_up_one, Outcome};
use pinhole_core::{AppCore, NullSink, ShippedPaths};
use pinhole_store::datadir::ModelKind;
use pinhole_store::installed::{CivitaiRef, InstalledFile, Lookup};
use pinhole_store::seal;
use pinhole_store::DataDir;
use pinhole_tests::config_dir;

/// An unsigned entry is checked against its file's own hash, never the one it holds.
#[tokio::test]
async fn an_unsigned_entry_is_checked_with_its_files_own_hash() {
    let tmp = tempfile::tempdir().unwrap();
    let core: Arc<AppCore> = AppCore::new(
        ShippedPaths {
            config_dir: config_dir(),
        },
        DataDir::at(tmp.path().join("Data"), true),
        Arc::new(NullSink),
    )
    .unwrap();
    // Nothing is looked up online: every lookup answers "failed".
    core.offline.set(true);
    let ids = ["same", "changed", "other"];
    seal::activate(seal::Signer::new(
        seal::SealKey::from_bytes([9; 32]),
        Default::default(),
        ids.iter().map(|s| s.to_string()).collect(),
    ));
    let lora_dir = core.data.models(ModelKind::Lora);
    std::fs::create_dir_all(&lora_dir).unwrap();
    for id in ids {
        std::fs::write(lora_dir.join(format!("{id}.safetensors")), id).unwrap();
    }
    let sha = |id: &str| {
        pinhole_net::download::sha256_file(&lora_dir.join(format!("{id}.safetensors"))).unwrap()
    };
    let entry = |id: &str, stored_sha: String| InstalledFile {
        id: id.into(),
        rel_path: format!("models/loras/{id}.safetensors"),
        kind: ModelKind::Lora,
        sha256: stored_sha,
        size_bytes: 1,
        family: None,
        component_id: None,
        friendly_name: id.into(),
        civitai: Some(CivitaiRef {
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
        lookup: Some(Lookup::Found),
    };
    core.installed.lock().files = vec![
        entry("same", sha("same")),
        entry("changed", "aa".repeat(32)),
        // Holds another file's hash.
        entry("other", sha("same")),
    ];
    let get = |id: &str| core.installed.lock().get(id).unwrap().clone();
    for id in ids {
        assert!(get(id).safe_images_only());
    }

    // Looked up with the hash it holds: left as it is.
    let held = get("other").sha256;
    assert_eq!(look_up_one(&core, "other", &held).await, Outcome::Failed);
    assert!(!seal::trusted("other"));

    // Same file, lookup result needs a lookup (none answers here): left unsigned.
    look_up_existing(&core, "same", &sha("same")).await;
    assert!(get("same").safe_images_only());
    assert_eq!(get("same").lookup, Some(Lookup::Found));

    // The file changed: it starts over as "not looked up yet", signed.
    look_up_existing(&core, "changed", &"aa".repeat(32)).await;
    let f = get("changed");
    assert_eq!(f.sha256, sha("changed"));
    assert_eq!(
        (f.lookup, f.civitai.is_none()),
        (Some(Lookup::NotYet), true)
    );
    assert!(seal::trusted("changed") && f.safe_images_only());

    // The entry holding another file's hash is checked with its own.
    look_up_existing(&core, "other", &held).await;
    assert_eq!(get("other").sha256, sha("other"));
}
