//! Model licence acceptance (RELEASE-SPEC §6): families and helpers with a
//! `license_accept` id in models.yaml are downloaded only after the user
//! accepted that licence once. Only the accepted ids are stored
//! (`Settings::accepted_licenses`).

use crate::{AppCore, CoreError, CoreResult};
use pinhole_store::settings::Settings;

/// Error code for "accept the licence first": `message` is the licence note,
/// `details` the licence id to pass to [`accept_license`].
pub const LICENSE_NEEDED: &str = "license_needed";

/// `Err(license_needed)` unless licence `id` (with display `note`) was accepted.
pub(crate) fn require(core: &AppCore, id: Option<&str>, note: Option<&str>) -> CoreResult<()> {
    let Some(id) = id else { return Ok(()) };
    if core
        .settings
        .read()
        .accepted_licenses
        .iter()
        .any(|a| a == id)
    {
        return Ok(());
    }
    Err(CoreError::new(
        LICENSE_NEEDED,
        format!(
            "This model comes with its own licence: {}.",
            note.unwrap_or(id)
        ),
    )
    .with_details(id))
}

/// [`require`] for a registry family (no family or no licence = nothing to accept).
pub(crate) fn require_family(core: &AppCore, family_id: Option<&str>) -> CoreResult<()> {
    let registry = core.registry();
    match family_id.and_then(|f| registry.family(f)) {
        Some(f) => require(core, f.license_accept.as_deref(), f.license_note.as_deref()),
        None => Ok(()),
    }
}

/// Record that the user accepted licence `id` (a `license_accept` id in the registry).
pub fn accept_license(core: &AppCore, id: &str) -> CoreResult<Settings> {
    let registry = core.registry();
    let known = registry
        .families()
        .filter_map(|f| f.license_accept.as_deref())
        .chain(
            registry
                .captioner()
                .helpers
                .iter()
                .filter_map(|h| h.license_accept.as_deref()),
        )
        .any(|l| l == id);
    if !known {
        return Err(CoreError::invalid(
            "That licence isn't in Pinhole's model list.",
        ));
    }
    let mut current = core.settings.write();
    let mut next = current.clone();
    if !next.accepted_licenses.iter().any(|a| a == id) {
        next.accepted_licenses.push(id.to_string());
    }
    let next = next.normalized();
    pinhole_store::settings::save(&core.data, &next).map_err(crate::library::store_err)?;
    *current = next.clone();
    Ok(next)
}
