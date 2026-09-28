//! Pinhole application core: the service layer behind every Tauri command.
//! Tauri-free so integration and privacy tests can drive it directly.
//!
//! Module owners (see docs/ARCHITECTURE.md):
//! * `app`, `library`           — store agent
//! * `downloads`                — net agent
//! * `engine_setup`, `generate`, `describe`, `session` — engine agent
//! * `models`, `catalog`        — catalog agent
//! * `lib`, `events`, `error`   — orchestrator (change only if you must; keep it compiling)

pub mod app;
pub mod catalog;
pub mod describe;
pub mod downloads;
pub mod engine_setup;
pub mod error;
pub mod events;
pub mod generate;
pub mod library;
pub mod models;
pub mod session;

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use pinhole_hardware::HardwareInfo;
use pinhole_net::download::DownloadManager;
use pinhole_net::{HttpClient, LocalClient, OfflineFlag};
use pinhole_registry::Registry;
use pinhole_store::{DataDir, InstalledIndex, Settings};

pub use error::{CoreError, CoreResult};
pub use events::{CoreEvent, EventSink, NullSink};

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
    pub gen: generate::GenState,
    /// Captioner (llama-server) state.
    pub describe: describe::DescribeState,
    /// Catalog/model-install state (e.g. pending family choices, API key cache).
    pub models: models::ModelsState,
}

impl AppCore {
    /// Load registry, settings and installed index; build clients. Does not
    /// touch the network. Call [`AppCore::start_background`] afterwards.
    pub fn new(shipped: ShippedPaths, data: DataDir, events: Arc<dyn EventSink>) -> CoreResult<Arc<Self>> {
        data.ensure_layout()?;
        let settings = pinhole_store::settings::load(&data)?;
        let overrides = data.overrides_file();
        let registry = Registry::load(&shipped.config_dir, overrides.exists().then_some(overrides.as_path()))?;
        let installed = InstalledIndex::load(&data)?;
        let offline = OfflineFlag::new(settings.offline);
        let http = HttpClient::new(offline.clone())?;
        let local = LocalClient::new()?;
        let downloads = DownloadManager::new(http.clone());
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
            gen: generate::GenState::default(),
            describe: describe::DescribeState::default(),
            models: models::ModelsState::default(),
        }))
    }

    /// Kick off background work: hardware detection, download-event forwarding,
    /// captioner idle shutdown. Must be called inside a Tokio runtime.
    pub fn start_background(self: &Arc<Self>) {
        app::start_hardware_detection(self);
        downloads::start_event_forwarding(self);
        describe::start_idle_watchdog(self);
    }

    /// Stop engines (app exit).
    pub async fn shutdown(&self) {
        generate::shutdown(self).await;
        describe::shutdown(self).await;
    }

    pub fn registry(&self) -> Arc<Registry> {
        self.registry.read().clone()
    }

    pub fn emit(&self, event: CoreEvent) {
        self.events.emit(event);
    }
}
