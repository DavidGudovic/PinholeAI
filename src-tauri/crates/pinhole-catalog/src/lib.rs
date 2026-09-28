//! CivitAI catalog (SPEC §5.4): API client over `pinhole_net::HttpClient`,
//! plain-language filters from `config/catalog-filters.yaml`, safe file
//! selection (SafeTensor/GGUF only, pickle + virus scans must be Success), and
//! model-card view models. Also the pure logic behind the Installed view,
//! recommended picks (SPEC §6.1), install plans and "Paste from CivitAI", so it
//! can be tested without the Tauri/app layer.
//!
//! OWNER: catalog agent.
//!
//! PRIVACY: nothing in here touches prompt text. The CivitAI API key only
//! travels as an `Authorization: Bearer` header to civitai.com and is never
//! logged or stored (it lives in the OS keychain, see `pinhole_store::keychain`).

pub mod api;
pub mod browse;
pub mod cards;
pub mod families;
pub mod filters;
pub mod inventory;
pub mod lenient;
pub mod local;
pub mod paste;
pub mod plan;
pub mod recommend;
pub mod select;
#[cfg(test)]
pub(crate) mod testkit;
pub mod view;

pub use api::CivitaiClient;
pub use filters::{BrowseQuery, CatalogFilters};
pub use view::*;
