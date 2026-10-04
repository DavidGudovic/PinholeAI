//! Pinhole application core: the service layer behind every Tauri command.
//! Tauri-free so integration and privacy tests can drive it directly.
//! Modules are described in docs/ARCHITECTURE.md.

pub mod app;
pub mod catalog;
pub mod describe;
pub mod downloads;
pub(crate) mod engine;
pub mod engine_setup;
pub mod error;
pub mod events;
pub mod gallery;
pub mod generate;
pub mod imagecheck;
pub mod library;
pub mod licence;
pub mod linked;
pub mod lookup;
pub(crate) mod memory;
pub mod models;
pub mod models_folder;
#[cfg(test)]
mod one_way;
pub mod session;
#[cfg(feature = "test-util")]
pub mod testing;
#[cfg(all(test, feature = "test-util"))]
mod tests;
pub mod text_check;
pub mod update;

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use pinhole_hardware::HardwareInfo;
use pinhole_net::download::DownloadManager;
use pinhole_net::{HttpClient, LocalClient, OfflineFlag};
use pinhole_registry::Registry;
use pinhole_store::{seal, DataDir, InstalledIndex, Settings};

pub use error::{CoreError, CoreResult};
pub use events::{CoreEvent, EventSink, InstallStarted, NullSink};

/// Paths to read-only shipped files (Tauri resources in release, repo `config/` in dev).
#[derive(Debug, Clone)]
pub struct ShippedPaths {
    /// Contains models.yaml, engine.yaml, catalog-filters.yaml, styles/, presets/.
    pub config_dir: PathBuf,
}

impl ShippedPaths {
    pub fn styles(&self) -> PathBuf {
        self.config_dir.join("styles")
    }
    pub fn presets(&self) -> PathBuf {
        self.config_dir.join("presets")
    }
}

/// Everything the app knows at runtime. Shared as `Arc<AppCore>`.
pub struct AppCore {
    pub shipped: ShippedPaths,
    pub data: DataDir,
    pub registry: RwLock<Arc<Registry>>,
    pub settings: RwLock<Settings>,
    pub installed: Mutex<InstalledIndex>,
    /// Filled asynchronously at startup (detection spawns `nvidia-smi`).
    pub hardware: RwLock<Option<HardwareInfo>>,
    pub offline: OfflineFlag,
    pub http: HttpClient,
    pub local: LocalClient,
    pub downloads: DownloadManager,
    pub events: Arc<dyn EventSink>,
    /// In-memory generated/imported images (never written until Save).
    pub session: session::Session,
    /// Engine processes + current generation job.
    pub gen: crate::engine::GenState,
    /// Captioner (llama-server) state.
    pub describe: describe::DescribeState,
    /// Catalog/model-install state (e.g. pending family choices, API key cache).
    pub models: models::ModelsState,
    /// Other apps' models folders being looked through (RAM only).
    pub linked: linked::LinkedRuntime,
    /// The local image check (RELEASE-SPEC §4).
    pub check: imagecheck::CheckState,
}

impl AppCore {
    /// Load registry, settings and installed index; build clients. Does not
    /// touch the network. Call [`AppCore::start_background`] afterwards.
    pub fn new(
        shipped: ShippedPaths,
        data: DataDir,
        events: Arc<dyn EventSink>,
    ) -> CoreResult<Arc<Self>> {
        // Settings live in Data/config; they say where the Models folder is.
        std::fs::create_dir_all(data.config())?;
        let settings = pinhole_store::settings::load(&data)?;
        let data = data.with_models_home(
            settings
                .models_folder
                .as_ref()
                .map(std::path::PathBuf::from),
        );
        data.ensure_layout()?;
        let overrides = data.overrides_file();
        let registry = Registry::load(
            &shipped.config_dir,
            overrides.exists().then_some(overrides.as_path()),
        )?;
        let mut installed = InstalledIndex::load(&data)?;
        // Saved lookup results are signed with a key in the OS keychain (not in tests).
        let key = if cfg!(any(test, feature = "test-util")) {
            seal::KeyState::Unavailable
        } else {
            seal::load_or_create_key()
        };
        let sign_all = match key {
            // First start with signing: what is installed is signed as it is.
            seal::KeyState::Created(key) if !seal::seals_file(&data).exists() => {
                seal::activate(seal::Signer::new(key, Default::default(), Vec::new()));
                true
            }
            seal::KeyState::Created(key) | seal::KeyState::Existing(key) => {
                let seals = seal::read(&data, &key);
                let unsigned = lookup::unsigned(&installed, &key, &seals);
                seal::activate(seal::Signer::new(key, seals, unsigned));
                false
            }
            seal::KeyState::Unavailable => false,
        };
        // Files added by hand or linked before the CivitAI lookup covered them all.
        if lookup::mark_unchecked(&registry, &mut installed) {
            let _ = installed.save(&data);
        }
        if sign_all {
            let _ = installed.save_seals(&data);
        }
        let offline = OfflineFlag::new(settings.offline);
        let http = HttpClient::new(offline.clone())?;
        let local = LocalClient::new()?;
        let downloads = DownloadManager::new(http.clone());
        let check = imagecheck::CheckState::new(data.safety_check());
        Ok(Arc::new(Self {
            shipped,
            data,
            registry: RwLock::new(Arc::new(registry)),
            settings: RwLock::new(settings),
            installed: Mutex::new(installed),
            hardware: RwLock::new(None),
            offline,
            http,
            local,
            downloads,
            events,
            session: session::Session::default(),
            gen: crate::engine::GenState::default(),
            describe: describe::DescribeState::default(),
            models: models::ModelsState::default(),
            linked: linked::LinkedRuntime::default(),
            check,
        }))
    }

    /// Kick off background work: hardware detection, download-event forwarding,
    /// captioner idle shutdown, killing leftover engines from an earlier run
    /// and removing unused engine files.
    /// Must be called inside a Tokio runtime.
    pub fn start_background(self: &Arc<Self>) {
        engine_setup::start_orphan_sweep(self);
        app::start_hardware_detection(self);
        downloads::start_event_forwarding(self);
        describe::start_idle_watchdog(self);
        linked::start(self);
        imagecheck::start_idle_unload(self);
        lookup::start_recheck(self);
    }

    /// Stop engines (app exit).
    pub async fn shutdown(&self) {
        crate::engine::shutdown(self).await;
        describe::shutdown(self).await;
        // Copies still waiting for a family choice would never be listed again.
        models::discard_pending(self);
    }

    pub fn registry(&self) -> Arc<Registry> {
        self.registry.read().clone()
    }

    pub fn emit(&self, event: CoreEvent) {
        self.events.emit(event);
    }
}
