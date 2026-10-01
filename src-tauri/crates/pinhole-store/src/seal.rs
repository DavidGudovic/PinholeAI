//! Model lookup results kept with a signature (RELEASE-SPEC §5).
//!
//! What the CivitAI lookups found for each installed model and add-on (`InstalledFile::lookup`
//! and the CivitAI flags) is signed with HMAC-SHA256. The key is made once per computer and kept
//! in the OS keychain ([`crate::keychain`]); the signatures go to `Data/catalog/model-lookups.json`
//! (per computer, like the linked folders), keyed by file id. At start, a file whose entry
//! doesn't match its signature goes back to "not looked up yet" (pinhole-core `lookup.rs`).
//!
//! No keychain (e.g. a Linux desktop without a Secret Service): nothing is signed or checked,
//! as before signing existed.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{mpsc, OnceLock};
use std::time::Duration;

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::datadir::DataDir;
use crate::installed::InstalledFile;
use crate::{write_atomic, StoreError};

pub const SEALS_FILE: &str = "model-lookups.json";
const KEY_ACCOUNT: &str = "model-lookups-key";
/// The keychain can block (a locked keyring, a stuck D-Bus): don't hold up the start.
const KEY_TIMEOUT: Duration = Duration::from_secs(5);

/// The signing key (32 random bytes). Never written to `Data/` or logged.
#[derive(Clone)]
pub struct SealKey([u8; 32]);

impl std::fmt::Debug for SealKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SealKey(..)")
    }
}

/// The key in use: every save of the models list signs it ([`crate::installed::InstalledIndex::save`]).
static ACTIVE: OnceLock<SealKey> = OnceLock::new();

/// Start signing with `key` (once per run; later calls are ignored).
pub fn activate(key: SealKey) {
    let _ = ACTIVE.set(key);
}

pub(crate) fn active() -> Option<&'static SealKey> {
    ACTIVE.get()
}

/// The key as found at start.
pub enum KeyState {
    /// Made now (first start with signing on this computer): what is installed is signed as it is.
    Created(SealKey),
    Existing(SealKey),
    /// No keychain to keep it in: nothing is signed or checked this run.
    Unavailable,
}

impl SealKey {
    #[cfg(any(test, feature = "test-util"))]
    pub fn for_tests(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The signature of what the lookups found for `f`.
    pub fn seal(&self, f: &InstalledFile) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.0).expect("any key length");
        mac.update(canonical(f).as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    /// Does `seal` match `f`? Constant-time.
    pub fn verify(&self, f: &InstalledFile, seal: &str) -> bool {
        let Ok(tag) = hex::decode(seal) else {
            return false;
        };
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.0).expect("any key length");
        mac.update(canonical(f).as_bytes());
        mac.verify_slice(&tag).is_ok()
    }
}

/// Everything about `f` that the lookups decide (one field per line).
fn canonical(f: &InstalledFile) -> String {
    let c = f.civitai.as_ref();
    format!(
        "pinhole-model-lookup-v1\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        f.id,
        json(&f.kind),
        f.sha256.to_ascii_lowercase(),
        json(&f.lookup),
        f.component_id.as_deref().unwrap_or("-"),
        c.is_some(),
        c.map_or(0, |c| c.model_id),
        c.map_or(0, |c| c.version_id),
        c.is_some_and(|c| c.sfw_only),
    )
}

fn json<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// Read the key from the keychain, or make and store one when there is none.
pub fn load_or_create_key() -> KeyState {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(load_or_create_blocking());
    });
    rx.recv_timeout(KEY_TIMEOUT)
        .ok()
        .flatten()
        .unwrap_or(KeyState::Unavailable)
}

fn load_or_create_blocking() -> Option<KeyState> {
    let entry = keyring::Entry::new(crate::keychain::SERVICE, KEY_ACCOUNT).ok()?;
    match entry.get_password() {
        Ok(hex_key) => {
            let bytes: [u8; 32] = hex::decode(hex_key.trim()).ok()?.try_into().ok()?;
            Some(KeyState::Existing(SealKey(bytes)))
        }
        Err(keyring::Error::NoEntry) => {
            let bytes: [u8; 32] = rand::random();
            entry.set_password(&hex::encode(bytes)).ok()?;
            Some(KeyState::Created(SealKey(bytes)))
        }
        Err(_) => None,
    }
}

#[derive(Serialize, Deserialize, Default)]
struct SealsOnDisk {
    #[serde(default)]
    seals: BTreeMap<String, String>,
}

pub fn seals_file(dir: &DataDir) -> PathBuf {
    dir.root.join("catalog").join(SEALS_FILE)
}

/// The saved signatures by file id. Missing or damaged → none (every entry is checked again).
pub fn read(dir: &DataDir) -> BTreeMap<String, String> {
    std::fs::read(seals_file(dir))
        .ok()
        .and_then(|b| serde_json::from_slice::<SealsOnDisk>(&b).ok())
        .map(|s| s.seals)
        .unwrap_or_default()
}

/// Sign `files` and save the signatures (replacing the old ones).
pub fn write<'a>(
    dir: &DataDir,
    key: &SealKey,
    files: impl Iterator<Item = &'a InstalledFile>,
) -> Result<(), StoreError> {
    let seals = files.map(|f| (f.id.clone(), key.seal(f))).collect();
    let mut json = serde_json::to_vec_pretty(&SealsOnDisk { seals })
        .map_err(|e| StoreError::Invalid(format!("could not encode the model lookups: {e}")))?;
    json.push(b'\n');
    write_atomic(&seals_file(dir), &json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datadir::ModelKind;
    use crate::installed::{CivitaiRef, Lookup};

    fn file() -> InstalledFile {
        InstalledFile {
            id: "a".into(),
            rel_path: "models/loras/a.safetensors".into(),
            kind: ModelKind::Lora,
            sha256: "AB".repeat(32),
            size_bytes: 1,
            family: None,
            component_id: None,
            friendly_name: "a".into(),
            civitai: Some(CivitaiRef {
                model_id: 1,
                version_id: 2,
                model_name: None,
                version_name: None,
                base_model: None,
                trained_words: vec![],
                license: None,
                creator_notes: None,
                sfw_only: true,
            }),
            added_at: 0,
            last_used: None,
            observed_vram_gb: None,
            dtype: None,
            trigger_words: None,
            lookup: Some(Lookup::Found),
        }
    }

    #[test]
    fn a_seal_matches_only_the_same_lookup_result() {
        let key = SealKey::for_tests([7; 32]);
        let f = file();
        let seal = key.seal(&f);
        assert!(key.verify(&f, &seal));
        assert!(
            !SealKey::for_tests([8; 32]).verify(&f, &seal),
            "another key"
        );
        assert!(!key.verify(&f, "zz"));

        // What the user may change (names, trigger words, last used) keeps it.
        let mut g = f.clone();
        g.friendly_name = "renamed".into();
        g.trigger_words = Some(vec!["word".into()]);
        g.last_used = Some(5);
        assert!(key.verify(&g, &seal));

        // What the lookups decide doesn't.
        let changes: [fn(&mut InstalledFile); 7] = [
            |f| f.civitai.as_mut().unwrap().sfw_only = false,
            |f| f.civitai = None,
            |f| f.lookup = Some(Lookup::NoMatch),
            |f| f.lookup = None,
            |f| f.sha256 = "cd".repeat(32),
            |f| f.kind = ModelKind::Diffusion,
            |f| f.id = "b".into(),
        ];
        for change in changes {
            let mut g = f.clone();
            change(&mut g);
            assert!(!key.verify(&g, &seal));
        }
    }

    #[test]
    fn seals_round_trip_and_a_damaged_file_reads_as_none() {
        let tmp = tempfile::tempdir().unwrap();
        let d = DataDir::at(tmp.path().join("Data"), false);
        let key = SealKey::for_tests([1; 32]);
        let f = file();
        write(&d, &key, std::iter::once(&f)).unwrap();
        let back = read(&d);
        assert!(key.verify(&f, &back["a"]));
        std::fs::write(seals_file(&d), b"{not json").unwrap();
        assert!(read(&d).is_empty());
    }
}
