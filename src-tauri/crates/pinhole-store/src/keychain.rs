//! CivitAI API key in the OS keychain (`keyring`), never in `Data/`.

use crate::StoreError;

pub const SERVICE: &str = "pinhole";
pub const CIVITAI_ACCOUNT: &str = "civitai-api-key";

pub fn get_civitai_key() -> Result<Option<String>, StoreError> {
    todo!("store agent")
}

pub fn set_civitai_key(key: &str) -> Result<(), StoreError> {
    let _ = key;
    todo!("store agent")
}

pub fn delete_civitai_key() -> Result<(), StoreError> {
    todo!("store agent")
}
