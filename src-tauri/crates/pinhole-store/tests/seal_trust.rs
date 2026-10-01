//! Signing turned on for the whole process (its own test binary, so no other test sees it).

use std::collections::BTreeMap;

use pinhole_store::datadir::ModelKind;
use pinhole_store::installed::Lookup;
use pinhole_store::seal::{self, SealKey, Signer};
use pinhole_store::{DataDir, InstalledFile, InstalledIndex};

fn file(id: &str) -> InstalledFile {
    InstalledFile {
        id: id.into(),
        rel_path: format!("models/loras/{id}.safetensors"),
        kind: ModelKind::Lora,
        sha256: "aa".repeat(32),
        size_bytes: 1,
        family: None,
        component_id: None,
        friendly_name: id.into(),
        civitai: None,
        added_at: 0,
        last_used: None,
        observed_vram_gb: None,
        dtype: None,
        trigger_words: None,
        lookup: Some(Lookup::Found),
    }
}

/// An entry that didn't match its signature counts as "safe images only" and isn't signed by
/// saves until it is checked again; then it is.
#[test]
fn an_unsigned_entry_is_safe_images_only_until_checked_again() {
    let tmp = tempfile::tempdir().unwrap();
    let d = DataDir::at(tmp.path().join("Data"), false);
    let key = SealKey::from_bytes([5; 32]);
    seal::activate(Signer::new(key.clone(), BTreeMap::new(), vec!["b".into()]));
    let index = InstalledIndex {
        files: vec![file("a"), file("b")],
        ..Default::default()
    };
    assert!(!index.files[0].safe_images_only());
    assert!(index.files[1].safe_images_only());

    index.save_seals(&d).unwrap();
    let saved = seal::read(&d);
    assert!(key.verify(&index.files[0], &saved["a"]));
    assert!(!saved.contains_key("b"));

    seal::trust("b");
    assert!(!index.files[1].safe_images_only());
    index.save_seals(&d).unwrap();
    assert!(key.verify(&index.files[1], &seal::read(&d)["b"]));
}
