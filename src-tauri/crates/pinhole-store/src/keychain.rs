//! CivitAI API key and the optional GitHub token (for updates while the
//! repository is private) in the OS keychain (`keyring`), never in `Data/`.
//!
//! Backends: Windows Credential Manager, the freedesktop Secret Service on
//! Linux (GNOME Keyring / KWallet), macOS Keychain. The calls block (D-Bus on
//! Linux): call them from `spawn_blocking` in async code. Error messages never
//! contain the key.

use keyring::{Entry, Error as KeyringError};

use crate::StoreError;

pub const SERVICE: &str = "pinhole";
pub const CIVITAI_ACCOUNT: &str = "civitai-api-key";
pub const GITHUB_ACCOUNT: &str = "github-token";

/// Longest key we accept (CivitAI keys are 32 hex characters).
const MAX_KEY_LEN: usize = 512;

pub fn get_civitai_key() -> Result<Option<String>, StoreError> {
    get_from(&entry()?)
}

pub fn set_civitai_key(key: &str) -> Result<(), StoreError> {
    set_on(&entry()?, key)
}

pub fn delete_civitai_key() -> Result<(), StoreError> {
    delete_on(&entry()?)
}

pub fn get_github_token() -> Result<Option<String>, StoreError> {
    get_with(&github_entry()?, map_github_err)
}

pub fn set_github_token(token: &str) -> Result<(), StoreError> {
    let entry = github_entry()?;
    set_github_on(&entry, token)
}

pub fn delete_github_token() -> Result<(), StoreError> {
    delete_with(&github_entry()?, map_github_err)
}

fn github_entry() -> Result<Entry, StoreError> {
    Entry::new(SERVICE, GITHUB_ACCOUNT).map_err(map_github_err)
}

fn set_github_on(entry: &Entry, token: &str) -> Result<(), StoreError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(StoreError::Invalid("Paste your GitHub token first.".into()));
    }
    if token.len() > MAX_KEY_LEN || !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(StoreError::Invalid(
            "That doesn't look like a GitHub token. Copy it again from GitHub → Settings → Developer settings → Personal access tokens.".into(),
        ));
    }
    entry.set_password(token).map_err(map_github_err)
}

fn entry() -> Result<Entry, StoreError> {
    Entry::new(SERVICE, CIVITAI_ACCOUNT).map_err(map_err)
}

fn get_from(entry: &Entry) -> Result<Option<String>, StoreError> {
    get_with(entry, map_err)
}

fn get_with(
    entry: &Entry,
    map: fn(KeyringError) -> StoreError,
) -> Result<Option<String>, StoreError> {
    match entry.get_password() {
        Ok(key) => {
            let key = key.trim();
            Ok((!key.is_empty()).then(|| key.to_string()))
        }
        Err(KeyringError::NoEntry) => Ok(None),
        Err(e) => Err(map(e)),
    }
}

fn set_on(entry: &Entry, key: &str) -> Result<(), StoreError> {
    let key = key.trim();
    if key.is_empty() {
        return Err(StoreError::Invalid(
            "Paste your CivitAI API key first.".into(),
        ));
    }
    if key.len() > MAX_KEY_LEN || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(StoreError::Invalid(
            "That doesn't look like a CivitAI API key. Copy it again from your CivitAI account settings.".into(),
        ));
    }
    entry.set_password(key).map_err(map_err)
}

/// Removing a key that isn't there is fine.
fn delete_on(entry: &Entry) -> Result<(), StoreError> {
    delete_with(entry, map_err)
}

fn delete_with(entry: &Entry, map: fn(KeyringError) -> StoreError) -> Result<(), StoreError> {
    match entry.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(e) => Err(map(e)),
    }
}

/// Plain-language messages; the platform error is deliberately not included
/// (it adds nothing the user can act on).
fn map_err(e: KeyringError) -> StoreError {
    let msg = match e {
        KeyringError::PlatformFailure(_) | KeyringError::NoStorageAccess(_) => {
            "Your system keychain isn't available, so Pinhole can't store the API key. \
             Unlock your keychain (on Linux: install or start GNOME Keyring or KWallet) and try again."
        }
        KeyringError::BadEncoding(_) => {
            "The saved CivitAI API key couldn't be read. Remove it in Settings and add it again."
        }
        KeyringError::Ambiguous(_) => {
            "Your system keychain has more than one CivitAI key for Pinhole. Remove the key in Settings and add it again."
        }
        KeyringError::TooLong(..) | KeyringError::Invalid(..) => {
            "Your system keychain refused the API key. Check the key and try again."
        }
        _ => "Your system keychain couldn't be used, so Pinhole can't store the API key.",
    };
    StoreError::Keychain(msg.into())
}

/// [`map_err`] for the GitHub token (Settings → Updates).
fn map_github_err(e: KeyringError) -> StoreError {
    let msg = match e {
        KeyringError::PlatformFailure(_) | KeyringError::NoStorageAccess(_) => {
            "Your system keychain isn't available, so Pinhole can't store the GitHub token. \
             Unlock your keychain (on Linux: install or start GNOME Keyring or KWallet) and try again."
        }
        KeyringError::BadEncoding(_) | KeyringError::Ambiguous(_) => {
            "The saved GitHub token couldn't be read. Remove it in Settings → Updates and add it again."
        }
        KeyringError::TooLong(..) | KeyringError::Invalid(..) => "Your system keychain refused the GitHub token. Check the token and try again.",
        _ => "Your system keychain couldn't be used, so Pinhole can't store the GitHub token.",
    };
    StoreError::Keychain(msg.into())
}

#[cfg(test)]
mod tests {
    //! Never touches the real keychain: every entry is a keyring mock credential.
    use super::*;
    use keyring::mock::MockCredential;

    fn mock_entry() -> Entry {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        Entry::new(SERVICE, CIVITAI_ACCOUNT).unwrap()
    }

    fn mock(entry: &Entry) -> &MockCredential {
        entry
            .get_credential()
            .downcast_ref::<MockCredential>()
            .expect("mock credential")
    }

    #[test]
    fn set_get_delete_round_trip() {
        let entry = mock_entry();
        assert_eq!(get_from(&entry).unwrap(), None);
        set_on(&entry, "  0123456789abcdef0123456789abcdef\n").unwrap();
        assert_eq!(
            get_from(&entry).unwrap().as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        set_on(&entry, "fedcba").unwrap();
        assert_eq!(get_from(&entry).unwrap().as_deref(), Some("fedcba"));
        delete_on(&entry).unwrap();
        assert_eq!(get_from(&entry).unwrap(), None);
        // Deleting again is fine.
        delete_on(&entry).unwrap();
    }

    #[test]
    fn github_token_round_trip_and_validation() {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        let entry = Entry::new(SERVICE, GITHUB_ACCOUNT).unwrap();
        assert!(matches!(
            set_github_on(&entry, " "),
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            set_github_on(&entry, "ghp_abc def"),
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            set_github_on(&entry, "https://x"),
            Err(StoreError::Invalid(_))
        ));
        set_github_on(&entry, " github_pat_11ABC_def123\n").unwrap();
        assert_eq!(
            get_with(&entry, map_github_err).unwrap().as_deref(),
            Some("github_pat_11ABC_def123")
        );
        mock(&entry).set_error(KeyringError::BadEncoding(vec![0xff]));
        match get_with(&entry, map_github_err).unwrap_err() {
            StoreError::Keychain(m) => {
                assert!(m.contains("GitHub token") && !m.contains("CivitAI"), "{m}")
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn rejects_bad_keys_without_storing() {
        let entry = mock_entry();
        assert!(matches!(set_on(&entry, "   "), Err(StoreError::Invalid(_))));
        assert!(matches!(
            set_on(&entry, "abc def"),
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            set_on(&entry, &"a".repeat(600)),
            Err(StoreError::Invalid(_))
        ));
        assert_eq!(get_from(&entry).unwrap(), None);
    }

    #[test]
    fn platform_failure_is_plain_and_secret_free() {
        let entry = mock_entry();
        let secret = "SECRET_KEY_123";
        mock(&entry).set_error(KeyringError::PlatformFailure(
            "org.freedesktop.DBus.Error.ServiceUnknown".into(),
        ));
        let e = set_on(&entry, secret).unwrap_err();
        match &e {
            StoreError::Keychain(m) => {
                assert!(m.starts_with(
                    "Your system keychain isn't available, so Pinhole can't store the API key"
                ));
                assert!(!m.contains(secret));
                assert!(!m.contains("DBus"));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            e.user_message(),
            match &e {
                StoreError::Keychain(m) => m.clone(),
                _ => unreachable!(),
            }
        );
        // The error was one-shot; storage works again afterwards.
        set_on(&entry, secret).unwrap();

        mock(&entry).set_error(KeyringError::NoStorageAccess("locked".into()));
        assert!(matches!(get_from(&entry), Err(StoreError::Keychain(_))));
        mock(&entry).set_error(KeyringError::BadEncoding(secret.as_bytes().to_vec()));
        let e = get_from(&entry).unwrap_err();
        assert!(!e.to_string().contains(secret));
        mock(&entry).set_error(KeyringError::PlatformFailure("x".into()));
        assert!(matches!(delete_on(&entry), Err(StoreError::Keychain(_))));
    }
}
