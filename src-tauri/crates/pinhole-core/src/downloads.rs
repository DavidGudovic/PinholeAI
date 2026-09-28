//! Download list/cancel + forwarding `DownloadManager` progress to UI events.
//! OWNER: net agent.
//!
//! Other areas enqueue groups directly on `core.downloads` and, when they need
//! the result (to register files, unpack the engine…), `await`
//! [`wait`] which maps failures to a `CoreError` with the right code
//! (`unauthorized` → the UI asks for a CivitAI key, `disk_space`, `offline`,
//! `hash_mismatch`, `cancelled`, `network`, `io`).
use std::sync::Arc;

use pinhole_net::download::{DownloadedFile, GroupError, GroupStatus};
use tokio::sync::broadcast::error::RecvError;

use crate::{AppCore, CoreError, CoreEvent, CoreResult};

/// Active download groups plus the last 20 finished ones (for the downloads list).
pub fn list(core: &AppCore) -> Vec<GroupStatus> {
    core.downloads.status()
}

/// Cancel a queued or running group. Partial files are kept for a later resume.
pub fn cancel(core: &AppCore, group_id: &str) {
    core.downloads.cancel(group_id);
}

/// Wait for a group; failures become a plain-language `CoreError`.
pub async fn wait(core: &AppCore, group_id: &str) -> CoreResult<Vec<DownloadedFile>> {
    core.downloads.wait_detailed(group_id).await.map_err(group_error)
}

/// `GroupError` → `CoreError` (same code, same plain-language message).
pub fn group_error(e: GroupError) -> CoreError {
    CoreError::new(&e.code, e.message)
}

/// Forward every `GroupStatus` broadcast to the UI as `download-progress`.
/// Must be called inside a Tokio runtime. Holds only a weak reference to the
/// core, so it stops when the app shuts down.
pub fn start_event_forwarding(core: &Arc<AppCore>) {
    let mut rx = core.downloads.subscribe();
    let weak = Arc::downgrade(core);
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(status) => {
                    let Some(core) = weak.upgrade() else { break };
                    core.emit(CoreEvent::Download(status));
                }
                Err(RecvError::Lagged(_)) => {
                    // Missed some ticks: re-send the current state of every group.
                    let Some(core) = weak.upgrade() else { break };
                    for status in core.downloads.status() {
                        core.emit(CoreEvent::Download(status));
                    }
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}
