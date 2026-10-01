//! Model lookup results kept with a signature (RELEASE-SPEC §5).
//!
//! What the CivitAI lookups found for each installed model and add-on (`InstalledFile::lookup`
//! and the CivitAI flags) is signed with HMAC-SHA256. The key is made once per computer and kept
//! in the OS keychain ([`crate::keychain`]); the signatures go to `Data/catalog/model-lookups.json`
//! (per computer, like the linked folders), keyed by file id. An entry whose saved result doesn't
//! match its signature counts as "safe images only" ([`InstalledFile::safe_images_only`]) until
//! Pinhole has checked it again on this computer (pinhole-core `lookup.rs`). Nothing in the
//! models list is changed for it, so a Models folder shared with another computer or system
//! keeps what that one found.
//!
//! No keychain (e.g. a Linux desktop without a Secret Service): nothing is signed or checked,
//! as before signing existed.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::Duration;

use hmac::{Hmac, Mac};
use parking_lot::Mutex;
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

/// The key as found at start.
pub enum KeyState {
    /// Made now (first start with signing on this computer).
    Created(SealKey),
    Existing(SealKey),
    /// No keychain to keep it in: nothing is signed or checked this run.
    Unavailable,
}

/// The signatures in use this run, and the entries that didn't match theirs.
pub struct Signer {
    key: SealKey,
    /// Every saved signature, also of entries not in the list now (a Models folder on a
    /// drive that isn't connected): a save replaces only those of the entries it has.
    seals: Mutex<BTreeMap<String, String>>,
    /// Entries that didn't match their signature at start, until checked again.
    unchecked: Mutex<HashSet<String>>,
}

impl Signer {
    pub fn new(key: SealKey, seals: BTreeMap<String, String>, unchecked: Vec<String>) -> Self {
        Self {
            key,
            seals: Mutex::new(seals),
            unchecked: Mutex::new(unchecked.into_iter().collect()),
        }
    }

    pub fn is_trusted(&self, id: &str) -> bool {
        !self.unchecked.lock().contains(id)
    }

    /// Pinhole has checked the entry again: sign it from the next save on.
    pub fn trust(&self, id: &str) {
        self.unchecked.lock().remove(id);
    }

    /// Undo [`Signer::trust`] (the change that came with it wasn't saved).
    pub fn distrust(&self, id: &str) {
        self.unchecked.lock().insert(id.to_string());
    }

    /// Sign `files` (all but the unchecked ones) and save every signature.
    pub fn write<'a>(
        &self,
        dir: &DataDir,
        files: impl Iterator<Item = &'a InstalledFile>,
    ) -> Result<(), StoreError> {
        let mut seals = self.seals.lock();
        {
            let unchecked = self.unchecked.lock();
            for f in files.filter(|f| !unchecked.contains(&f.id)) {
                seals.insert(f.id.clone(), self.key.seal(f));
            }
        }
        let mut json = serde_json::to_vec_pretty(&SealsOnDisk {
            seals: seals.clone(),
        })
        .map_err(|e| StoreError::Invalid(format!("could not encode the model lookups: {e}")))?;
        json.push(b'\n');
        write_atomic(&seals_file(dir), &json)
    }
}

/// See [`Signer::distrust`].
pub fn distrust(id: &str) {
    if let Some(s) = active() {
        s.distrust(id);
    }
}

/// The signer every save of the models list uses ([`crate::installed::InstalledIndex::save`]).
static ACTIVE: OnceLock<Signer> = OnceLock::new();

/// Start signing (once per run; later calls are ignored).
pub fn activate(signer: Signer) {
    let _ = ACTIVE.set(signer);
}

pub fn active() -> Option<&'static Signer> {
    ACTIVE.get()
}

/// Whether the saved lookup result of `id` can be used as it is (always, without signing).
pub fn trusted(id: &str) -> bool {
    active().is_none_or(|s| s.is_trusted(id))
}

/// See [`Signer::trust`].
pub fn trust(id: &str) {
    if let Some(s) = active() {
        s.trust(id);
    }
}

impl SealKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
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
    // Past the timeout this run goes on without signing: the late thread must not store a
    // key then, or the next start would find a key without the signatures made with it.
    let gave_up = Arc::new(AtomicBool::new(false));
    let late = gave_up.clone();
    std::thread::spawn(move || {
        let _ = tx.send(load_or_create_blocking(&late));
    });
    let key = rx.recv_timeout(KEY_TIMEOUT).ok().flatten();
    gave_up.store(true, Ordering::SeqCst);
    key.unwrap_or(KeyState::Unavailable)
}

fn load_or_create_blocking(gave_up: &AtomicBool) -> Option<KeyState> {
    let entry = keyring::Entry::new(crate::keychain::SERVICE, KEY_ACCOUNT).ok()?;
    match entry.get_password() {
        Ok(hex_key) => {
            let bytes: [u8; 32] = hex::decode(hex_key.trim()).ok()?.try_into().ok()?;
            Some(KeyState::Existing(SealKey::from_bytes(bytes)))
        }
        Err(keyring::Error::NoEntry) if !gave_up.load(Ordering::SeqCst) => {
            let bytes: [u8; 32] = rand::random();
            entry.set_password(&hex::encode(bytes)).ok()?;
            Some(KeyState::Created(SealKey::from_bytes(bytes)))
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
        let key = SealKey::from_bytes([7; 32]);
        let f = file();
        let seal = key.seal(&f);
        assert!(key.verify(&f, &seal));
        assert!(
            !SealKey::from_bytes([8; 32]).verify(&f, &seal),
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
        let key = SealKey::from_bytes([1; 32]);
        let f = file();
        Signer::new(key.clone(), BTreeMap::new(), Vec::new())
            .write(&d, std::iter::once(&f))
            .unwrap();
        let back = read(&d);
        assert!(key.verify(&f, &back["a"]));
        std::fs::write(seals_file(&d), b"{not json").unwrap();
        assert!(read(&d).is_empty());
    }

    /// A save signs the entries it has except unchecked ones, and keeps every other signature
    /// (entries of a Models folder that isn't connected now).
    #[test]
    fn a_save_keeps_other_signatures_and_signs_only_checked_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let d = DataDir::at(tmp.path().join("Data"), false);
        let key = SealKey::from_bytes([2; 32]);
        let mut edited = file();
        edited.id = "edited".into();
        let saved = BTreeMap::from([
            ("elsewhere".to_string(), "11".repeat(32)),
            ("edited".to_string(), "22".repeat(32)),
        ]);
        let signer = Signer::new(key.clone(), saved, vec!["edited".into()]);
        assert!(!signer.is_trusted("edited") && signer.is_trusted("a"));
        signer.write(&d, [file(), edited.clone()].iter()).unwrap();
        let back = read(&d);
        assert!(key.verify(&file(), &back["a"]));
        assert_eq!(back["elsewhere"], "11".repeat(32));
        assert_eq!(
            back["edited"],
            "22".repeat(32),
            "not signed while unchecked"
        );

        signer.trust("edited");
        signer.write(&d, std::iter::once(&edited)).unwrap();
        assert!(key.verify(&edited, &read(&d)["edited"]));
    }
}
