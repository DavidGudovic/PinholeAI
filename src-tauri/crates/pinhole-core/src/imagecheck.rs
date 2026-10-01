//! The local image check at result intake (RELEASE-SPEC §4). Every generated picture
//! is measured by `pinhole_check` before it enters the session; if one picture of a
//! batch is blocked, the whole batch is dropped with the neutral block message.
//!
//! Fail closed: without all check files (or with a damaged one) Create and Edit stop
//! with `check_missing`, which the UI answers with "Set up safety check". The check is
//! always on; tests use a stand-in through the `test-util` feature
//! ([`crate::testing::use_check`]).
//!
//! PRIVACY: scores live in memory only; nothing is logged, counted or written. The
//! originals' readings are forgotten on Reset.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use pinhole_check::{files, CheckError, Checker, Original, Readings, Rule};
use pinhole_net::download::{DownloadKind, DownloadSpec};
use serde::Serialize;

use crate::session::Source;
use crate::text_check::BLOCKED_MESSAGE;
use crate::{AppCore, CoreError, CoreResult};

/// Models are dropped from memory after this long without a check.
const UNLOAD_AFTER: Duration = Duration::from_secs(5 * 60);

pub const MISSING: &str = "check_missing";

/// What measures images. The real one is [`Checker`]; tests put a stand-in in its place.
pub trait Inspector: Send + Sync {
    /// Labels of files that are absent or have the wrong size.
    fn missing(&self) -> Vec<&'static str>;
    fn readings(&self, png: &[u8]) -> Result<Readings, CheckError>;
    fn original(&self, png: &[u8]) -> Result<Original, CheckError>;
    /// Every step, for the dev builds' readings view.
    fn full_readings(&self, png: &[u8]) -> Result<Readings, CheckError> {
        self.readings(png)
    }
    fn preload(&self) -> Result<(), CheckError> {
        Ok(())
    }
    fn unload_if_idle(&self, _idle: Duration) {}
}

impl Inspector for Checker {
    fn missing(&self) -> Vec<&'static str> {
        self.missing_files().iter().map(|f| f.label).collect()
    }
    fn readings(&self, png: &[u8]) -> Result<Readings, CheckError> {
        Checker::readings(self, png)
    }
    fn original(&self, png: &[u8]) -> Result<Original, CheckError> {
        Checker::original(self, png)
    }
    fn full_readings(&self, png: &[u8]) -> Result<Readings, CheckError> {
        Checker::full_readings(self, png)
    }
    fn preload(&self) -> Result<(), CheckError> {
        Checker::preload(self)
    }
    fn unload_if_idle(&self, idle: Duration) {
        Checker::unload_if_idle(self, idle);
    }
}

/// A stand-in for the image check with fixed readings (test builds only: release
/// builds have no way to replace the check).
#[cfg(any(test, feature = "test-util"))]
#[derive(Debug, Clone, Default)]
pub struct FakeCheck {
    /// Labels of files to report as missing.
    pub missing: Vec<&'static str>,
    /// Readings for every result (default: an ordinary picture).
    pub readings: pinhole_check::Readings,
    /// Readings for every brought-in picture.
    pub original: pinhole_check::Original,
    /// How many results and originals were measured.
    pub counts: std::sync::Arc<parking_lot::Mutex<(usize, usize)>>,
    /// Width and height of every result measured, in order.
    pub sizes: std::sync::Arc<parking_lot::Mutex<Vec<(u32, u32)>>>,
    /// Readings for brought-in (or fed-in) pictures of one size, in place of `original`.
    pub original_by_size: Vec<((u32, u32), pinhole_check::Original)>,
}

#[cfg(any(test, feature = "test-util"))]
impl Inspector for FakeCheck {
    fn missing(&self) -> Vec<&'static str> {
        self.missing.clone()
    }
    fn readings(&self, png: &[u8]) -> Result<pinhole_check::Readings, pinhole_check::CheckError> {
        if !self.missing.is_empty() {
            return Err(pinhole_check::CheckError::Missing(self.missing[0]));
        }
        self.counts.lock().0 += 1;
        self.sizes
            .lock()
            .push(pinhole_engine::png::dimensions(png).unwrap_or_default());
        Ok(self.readings.clone())
    }
    fn original(&self, png: &[u8]) -> Result<pinhole_check::Original, pinhole_check::CheckError> {
        self.counts.lock().1 += 1;
        let size = pinhole_engine::png::dimensions(png).unwrap_or_default();
        Ok(self
            .original_by_size
            .iter()
            .find(|(s, _)| *s == size)
            .map_or(self.original, |(_, o)| *o))
    }
}

/// Put `fake` in place of the check (test builds only).
#[cfg(any(test, feature = "test-util"))]
pub fn use_fake(core: &AppCore, fake: FakeCheck) {
    *core.check.inspector.write() = Arc::new(fake);
}

pub struct CheckState {
    inspector: RwLock<Arc<dyn Inspector>>,
    /// Readings of brought-in pictures, by session image id (memory only).
    originals: Mutex<HashMap<String, Original>>,
    /// The running "Safety check" download, if any.
    install_group: Mutex<Option<String>>,
    /// Held while deciding what to download and queueing it, so two setups at once
    /// (engine setup + the button) can't queue the same files twice.
    install_start: tokio::sync::Mutex<()>,
    /// SHA-256 of each exported (saved) picture → the brought-in pictures it was made
    /// from, so opening a saved picture again keeps its chain (memory only, until Reset).
    exported: Mutex<HashMap<String, Arc<[Source]>>>,
}

impl CheckState {
    pub fn new(dir: std::path::PathBuf) -> Self {
        Self {
            inspector: RwLock::new(Arc::new(Checker::new(dir))),
            originals: Mutex::new(HashMap::new()),
            install_group: Mutex::new(None),
            install_start: tokio::sync::Mutex::new(()),
            exported: Mutex::new(HashMap::new()),
        }
    }

    fn inspector(&self) -> Arc<dyn Inspector> {
        self.inspector.read().clone()
    }

    /// Reset: forget everything measured.
    pub fn forget(&self) {
        self.originals.lock().clear();
        self.exported.lock().clear();
    }

    /// Save/Copy: remember what an exported picture was made from.
    pub fn note_export(&self, bytes: &[u8], made_from: Vec<Source>) {
        if made_from.is_empty() {
            return;
        }
        let key = hex_sha256(bytes);
        self.exported.lock().insert(key, Arc::from(made_from));
    }

    /// Import: a picture Pinhole exported earlier in this session keeps its chain.
    pub fn exported_from(&self, bytes: &[u8]) -> Option<Arc<[Source]>> {
        let map = self.exported.lock();
        if map.is_empty() {
            return None;
        }
        map.get(&hex_sha256(bytes)).cloned()
    }
}

/// `SafetyCheckStatus` in src/lib/types.ts.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SafetyCheckStatus {
    /// All files present (their SHA-256 is checked when they load).
    pub ready: bool,
    pub downloading: bool,
    /// Bytes still to download.
    pub download_bytes: u64,
}

fn hex_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

fn downloading(core: &AppCore) -> bool {
    let Some(gid) = core.check.install_group.lock().clone() else {
        return false;
    };
    core.downloads
        .status()
        .iter()
        .any(|g| g.group_id == gid && !g.state.is_finished())
}

pub fn status(core: &AppCore) -> SafetyCheckStatus {
    let missing = files::missing(&core.data.safety_check());
    SafetyCheckStatus {
        ready: core.check.inspector().missing().is_empty(),
        downloading: downloading(core),
        download_bytes: missing.iter().map(|f| f.size).sum(),
    }
}

fn missing_error(core: &AppCore) -> CoreError {
    if downloading(core) {
        CoreError::new(
            MISSING,
            "Pinhole's safety check is still downloading (see Downloads). Try again when it's done.",
        )
    } else {
        CoreError::new(
            MISSING,
            "Pinhole's safety check isn't set up yet. Click “Set up safety check” to download it (about 1.1 GB), then try again.",
        )
    }
}

fn check_error(core: &AppCore, e: CheckError) -> CoreError {
    match e {
        CheckError::Missing(_) => missing_error(core),
        CheckError::Damaged(f) => CoreError::new(
            MISSING,
            "Pinhole's safety check files are damaged. Click “Set up safety check” to download them again, then try again.",
        )
        .with_details(format!("{f} is damaged")),
        CheckError::Image | CheckError::Run(_) => CoreError::new(
            "check_failed",
            "The safety check couldn't run, so the picture wasn't shown. Try again.",
        )
        .with_details(e.to_string()),
    }
}

/// Before any engine work: every check file must be there. Also starts loading the
/// first model in the background so the check doesn't hold up the first result.
pub fn ensure_ready(core: &AppCore) -> CoreResult<()> {
    let inspector = core.check.inspector();
    if !inspector.missing().is_empty() {
        return Err(missing_error(core));
    }
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::spawn_blocking(move || {
            let _ = inspector.preload();
        });
    }
    Ok(())
}

/// Result intake: measure every picture of a batch. Returns the pictures unchanged
/// when none is blocked; otherwise the whole batch is dropped.
/// `also_check`: parts of the results measured on their own and judged the same way (a
/// Fix details box, which is too small to judge in a large whole picture); never kept.
/// `sources`: the brought-in pictures the batch was made from. `inputs`: made pictures from
/// those chains fed into this step; a person found in one counts like one in a brought-in
/// picture. `safe_images_only`: the model or a LoRA in use is marked "safe images only" on
/// CivitAI.
pub async fn check_results(
    core: &Arc<AppCore>,
    pngs: Vec<Vec<u8>>,
    also_check: Vec<Vec<u8>>,
    sources: Vec<Source>,
    inputs: Vec<Source>,
    safe_images_only: bool,
) -> CoreResult<Vec<CheckedPng>> {
    let made_from: Arc<[Source]> = Arc::from(sources.clone());
    let c = core.clone();
    let res = tokio::task::spawn_blocking(move || {
        let inspector = c.check.inspector();
        let mut all = Vec::with_capacity(pngs.len() + also_check.len());
        for png in pngs.iter().chain(&also_check) {
            all.push(inspector.readings(png)?);
        }
        // The originals only matter when a result is intimate.
        let originals = if all
            .iter()
            .any(|r| pinhole_check::rules::is_intimate(r.nudity, r.tags.as_ref()))
        {
            let mut found = originals(&c, inspector.as_ref(), &sources)?;
            // A face the brought-in picture didn't show clearly but a later step does (made
            // larger, straightened, sharpened) counts too. It's exempt as already intimate
            // only when every brought-in picture was.
            let exempt = !found.is_empty() && found.iter().all(|o| o.has_face && o.intimate);
            for o in originals(&c, inspector.as_ref(), &inputs)? {
                if o.has_face {
                    found.push(Original {
                        has_face: true,
                        intimate: exempt && o.intimate,
                    });
                }
            }
            found
        } else {
            Vec::new()
        };
        let blocked = all.iter().find_map(|r| {
            pinhole_check::rules::decide(r, &originals, safe_images_only)
                .map(|rule| (rule, r.clone()))
        });
        Ok::<_, CheckError>((pngs, blocked))
    })
    .await
    .map_err(|e| {
        CoreError::new(
            "check_failed",
            "The safety check couldn't run, so the picture wasn't shown. Try again.",
        )
        .with_details(e.to_string())
    })?;
    let (pngs, blocked) = res.map_err(|e| check_error(core, e))?;
    if let Some((rule, r)) = blocked {
        return Err(blocked_error(rule, &r));
    }
    Ok(pngs
        .into_iter()
        .map(|png| CheckedPng {
            png,
            made_from: made_from.clone(),
        })
        .collect())
}

/// A result picture that passed the image check, with the brought-in pictures it was made
/// from. Only [`check_results`] makes one (the fields are private to this module), and the
/// session only stores generated pictures as `CheckedPng` (`Session::insert_generated`), so
/// no picture reaches the UI without the check (RELEASE-SPEC §1 result intake).
pub struct CheckedPng {
    png: Vec<u8>,
    made_from: Arc<[Source]>,
}

impl CheckedPng {
    pub fn png(&self) -> &[u8] {
        &self.png
    }

    pub(crate) fn into_parts(self) -> (Vec<u8>, Arc<[Source]>) {
        (self.png, self.made_from)
    }

    /// Tests that exercise the session without running the check.
    #[cfg(any(test, feature = "test-util"))]
    pub fn unchecked_for_tests(png: Vec<u8>) -> Self {
        Self {
            png,
            made_from: Arc::from(Vec::new()),
        }
    }
}

fn blocked_error(rule: Rule, r: &Readings) -> CoreError {
    let e = CoreError::new("blocked", BLOCKED_MESSAGE);
    // Dev builds show which rule fired and the scores, to tune the rules on legal
    // test pictures. Release builds say nothing more than the message.
    if cfg!(debug_assertions) {
        e.with_details(format!("{}: {}", rule.key(), describe(r)))
    } else {
        let _ = (rule, r);
        e
    }
}

fn originals(
    core: &AppCore,
    inspector: &dyn Inspector,
    sources: &[Source],
) -> Result<Vec<Original>, CheckError> {
    let mut out = Vec::with_capacity(sources.len());
    for s in sources {
        let known = core.check.originals.lock().get(&s.id).copied();
        let o = match known {
            Some(o) => o,
            None => {
                let o = inspector.original(&s.bytes)?;
                core.check.originals.lock().insert(s.id.clone(), o);
                o
            }
        };
        out.push(o);
    }
    Ok(out)
}

/// Scores as one line of text (dev builds only; never includes prompt text).
pub fn describe(r: &Readings) -> String {
    let mut s = format!("nudity {:.2}", r.nudity);
    if let Some(t) = &r.tags {
        s += &format!(
            " · rating general {:.2} sensitive {:.2} questionable {:.2} explicit {:.2} · minor tag {:.2} · nude tag {:.2} · underwear tag {:.2} · photo {:.2}/{:.2}",
            t.general, t.sensitive, t.questionable, t.explicit, t.minor, t.nude, t.underwear, t.realistic, t.photorealistic
        );
    }
    if let Some(faces) = &r.faces {
        let f: Vec<String> = faces
            .iter()
            .map(|f| match f.child_face {
                Some(u) => format!("{:.2} ({:.0}px) child face {:.2}", f.score, f.side, u),
                None => format!("{:.2} ({:.0}px)", f.score, f.side),
            })
            .collect();
        s += &format!(" · faces [{}]", f.join(", "));
    }
    s
}

/// Dev builds: every reading of a session picture, measured on request (nothing is
/// kept). Release builds: always `None`, so ordinary users never see what the check saw.
pub async fn readings_of(core: &Arc<AppCore>, id: &str) -> CoreResult<Option<String>> {
    if !cfg!(debug_assertions) {
        return Ok(None);
    }
    let img = core
        .session
        .get(id)
        .ok_or_else(|| CoreError::not_found("That image isn't in this session anymore."))?;
    let inspector = core.check.inspector();
    let res = tokio::task::spawn_blocking(move || {
        let r = inspector.full_readings(&img.bytes)?;
        let o = inspector.original(&img.bytes)?;
        let verdict = pinhole_check::rules::decide(&r, &[], false).map_or("none", |rule| rule.key());
        let mark = pinhole_engine::image::decode_rgba(&img.bytes)
            .map(|(px, w, h)| pinhole_engine::watermark::is_marked(&px, w, h))
            .unwrap_or(false);
        Ok::<_, CheckError>(format!(
            "{} · sexual {} · as a brought-in picture: face {}, intimate {} · rule on its own: {verdict} · Made with AI watermark: {}",
            describe(&r),
            yes(pinhole_check::rules::is_sexual(r.nudity, r.tags.as_ref())),
            yes(o.has_face),
            yes(o.intimate),
            yes(mark),
        ))
    })
    .await
    .map_err(|e| CoreError::internal("The readings couldn't be measured.").with_details(e.to_string()))?;
    res.map(Some).map_err(|e| check_error(core, e))
}

fn yes(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

// ---------------------------------------------------------------- download

/// Queue the missing (or damaged) check files and resolve when they are in place.
/// A download already running is joined instead of queued twice.
pub async fn install(core: &Arc<AppCore>) -> CoreResult<SafetyCheckStatus> {
    let start = core.check.install_start.lock().await;
    // Clone first: `downloading` locks `install_group` again.
    let current = core.check.install_group.lock().clone();
    let running = current.filter(|_| downloading(core));
    let gid = match running {
        Some(gid) => gid,
        None => {
            let dir = core.data.safety_check();
            std::fs::create_dir_all(&dir)?;
            let d = dir.clone();
            // Files of the right size are hashed now: a damaged one is downloaded again.
            let need = tokio::task::spawn_blocking(move || {
                files::FILES
                    .iter()
                    .filter(|f| files::read_verified(&d, f).is_err())
                    .copied()
                    .collect::<Vec<_>>()
            })
            .await
            .map_err(|e| {
                CoreError::new("internal", "The safety check download couldn't start.")
                    .with_details(e.to_string())
            })?;
            if need.is_empty() {
                return Ok(status(core));
            }
            let specs = need
                .iter()
                .map(|f| {
                    let dest = files::path(&dir, f);
                    let _ = std::fs::remove_file(&dest);
                    DownloadSpec {
                        url: f.url.to_string(),
                        dest,
                        sha256: Some(f.sha256.to_string()),
                        size_bytes: Some(f.size),
                        label: f.label.to_string(),
                        headers: Vec::new(),
                        approx_size_bytes: None,
                        content_check: None,
                    }
                })
                .collect();
            let gid = core.downloads.enqueue_kind(
                "Safety check".into(),
                DownloadKind::SafetyCheck,
                specs,
            );
            *core.check.install_group.lock() = Some(gid.clone());
            gid
        }
    };
    drop(start);
    crate::downloads::wait(core, &gid).await?;
    // New files: load them fresh.
    core.check.inspector().unload_if_idle(Duration::ZERO);
    Ok(status(core))
}

/// Start the download in the background when files are missing (with the engine
/// setup, so a new install gets both).
pub fn install_in_background(core: &Arc<AppCore>) {
    if status(core).ready {
        return;
    }
    let core = core.clone();
    tokio::spawn(async move {
        let _ = install(&core).await;
    });
}

/// Drop the check's models from memory after [`UNLOAD_AFTER`] without use.
pub fn start_idle_unload(core: &Arc<AppCore>) {
    let weak = Arc::downgrade(core);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let Some(core) = weak.upgrade() else { break };
            let inspector = core.check.inspector();
            drop(core);
            let _ =
                tokio::task::spawn_blocking(move || inspector.unload_if_idle(UNLOAD_AFTER)).await;
        }
    });
}
