//! Leftover engine processes: an `sd-server` / `llama-server` that this app
//! doesn't manage but whose executable lives under our `Data/engine/` folder
//! (a previous Pinhole run that crashed, a stop that didn't finish in time…).
//! Such a process can hold gigabytes of graphics memory, so it is killed when
//! the app starts and before every engine launch.
//!
//! Other programs are never touched: the executable must be one of the engine
//! binaries AND sit inside the engine folder, and engines this app is running
//! right now ([`crate::process::managed_pids`]) and the app itself are skipped.
//! Blocking (lists every process): call it through `spawn_blocking`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

/// Engine executable names (as unpacked by [`crate::install`]).
const ENGINE_BINARIES: &[&str] = &["sd-server", "llama-server"];

/// A leftover engine that was found (and killed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orphan {
    pub pid: u32,
    pub exe: PathBuf,
    /// It was gone before the wait ran out.
    pub exited: bool,
}

/// `sd-server`, `sd-server.exe`, `llama-server`… Linux reports a replaced or
/// deleted executable as `sd-server (deleted)`.
pub fn is_engine_binary_name(name: &str) -> bool {
    let name = name.strip_suffix(" (deleted)").unwrap_or(name);
    let stem = match name.len().checked_sub(4) {
        Some(i) if name.is_char_boundary(i) && name[i..].eq_ignore_ascii_case(".exe") => &name[..i],
        _ => name,
    };
    ENGINE_BINARIES.iter().any(|b| if cfg!(windows) { stem.eq_ignore_ascii_case(b) } else { stem == *b })
}

/// Comparable form of a path: Windows verbatim prefixes removed (`\\?\C:\` →
/// `C:\`, `\\?\UNC\srv\share` → `\\srv\share`) and, on Windows, lowercased.
fn comparable(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    let s = if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.into_owned()
    };
    PathBuf::from(if cfg!(windows) { s.to_lowercase() } else { s })
}

/// The engine folder as given and as resolved (symlinks, `..`), comparable.
pub fn engine_roots(engine_root: &Path) -> Vec<PathBuf> {
    let mut roots = vec![comparable(engine_root)];
    if let Ok(c) = engine_root.canonicalize() {
        let c = comparable(&c);
        if !roots.contains(&c) {
            roots.push(c);
        }
    }
    roots
}

/// `exe` is an engine binary inside one of `roots` (from [`engine_roots`]).
/// Component-wise: `Data/engine-old/sd-server` is not inside `Data/engine`.
pub fn is_engine_exe(exe: &Path, roots: &[PathBuf]) -> bool {
    let Some(name) = exe.file_name().map(|n| n.to_string_lossy().into_owned()) else { return false };
    if !is_engine_binary_name(&name) {
        return false;
    }
    let exe = comparable(exe);
    roots.iter().any(|r| !r.as_os_str().is_empty() && exe.starts_with(r) && exe != *r)
}

fn list(sys: &mut System) {
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().without_tasks().with_exe(UpdateKind::Always));
}

/// Leftover engines under `engine_root` (nothing is killed).
pub fn find_orphans(engine_root: &Path) -> Vec<Orphan> {
    let mut sys = System::new();
    find_in(&mut sys, &engine_roots(engine_root))
}

fn find_in(sys: &mut System, roots: &[PathBuf]) -> Vec<Orphan> {
    list(sys);
    // Read the registry AFTER listing: an engine spawned while we listed is
    // registered by the time its pid can show up (see `EngineProcess::spawn`).
    let managed = crate::process::managed_pids();
    let me = std::process::id();
    let mut out: Vec<Orphan> = sys
        .processes()
        .iter()
        .filter_map(|(pid, p)| {
            let pid = pid.as_u32();
            if pid == me || managed.contains(&pid) || is_gone(p.status()) {
                return None;
            }
            let exe = p.exe()?;
            is_engine_exe(exe, roots).then(|| Orphan { pid, exe: exe.to_path_buf(), exited: false })
        })
        .collect();
    out.sort_by_key(|o| o.pid);
    out
}

fn is_gone(status: ProcessStatus) -> bool {
    matches!(status, ProcessStatus::Zombie | ProcessStatus::Dead)
}

/// Kill every leftover engine under `engine_root` and wait (up to `wait` in
/// total) until they are gone. Returns what was found.
pub fn kill_orphans(engine_root: &Path, wait: Duration) -> Vec<Orphan> {
    let mut sys = System::new();
    let mut found = find_in(&mut sys, &engine_roots(engine_root));
    if found.is_empty() {
        return found;
    }
    for o in &found {
        if let Some(p) = sys.process(Pid::from_u32(o.pid)) {
            p.kill();
        }
    }
    let pids: Vec<Pid> = found.iter().map(|o| Pid::from_u32(o.pid)).collect();
    let start = Instant::now();
    loop {
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&pids), true, ProcessRefreshKind::nothing().without_tasks());
        for o in found.iter_mut() {
            o.exited = sys.process(Pid::from_u32(o.pid)).is_none_or(|p| is_gone(p.status()));
        }
        if found.iter().all(|o| o.exited) || start.elapsed() >= wait {
            return found;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_binary_names() {
        for ok in ["sd-server", "llama-server", "sd-server.exe", "llama-server.EXE", "sd-server (deleted)"] {
            assert!(is_engine_binary_name(ok), "{ok}");
        }
        for no in ["sd-cli", "python", "python.exe", "sd-server2", "my-sd-server", "", ".exe", "llama-server.sh"] {
            assert!(!is_engine_binary_name(no), "{no}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn exe_must_be_an_engine_inside_the_engine_folder() {
        let roots = vec![PathBuf::from("/home/u/Pinhole/Data/engine")];
        let yes = |p: &str| is_engine_exe(Path::new(p), &roots);
        assert!(yes("/home/u/Pinhole/Data/engine/sd/master-929-3f8527a/cpu/sd-server"));
        assert!(yes("/home/u/Pinhole/Data/engine/llama/b1/vulkan/build/bin/llama-server"));
        assert!(yes("/home/u/Pinhole/Data/engine/sd/v/cpu/sd-server (deleted)"));
        assert!(!yes("/home/u/Pinhole/Data/engine-old/sd/v/cpu/sd-server"), "component-wise prefix");
        assert!(!yes("/usr/local/bin/sd-server"), "someone else's sd-server");
        assert!(!yes("/home/u/Pinhole/Data/engine/sd/v/cpu/python3"), "not an engine binary");
        assert!(!yes("/home/u/Pinhole/pinhole"));
        assert!(!is_engine_exe(Path::new("/home/u/Pinhole/Data/engine/sd-server"), &[PathBuf::new()]), "empty root matches nothing");
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_compare_case_insensitively_without_verbatim_prefix() {
        let roots = engine_roots(Path::new(r"C:\Users\Ana\Pinhole\Data\engine"));
        assert!(is_engine_exe(Path::new(r"c:\users\ana\pinhole\data\ENGINE\sd\v\cuda\sd-server.exe"), &roots));
        let verbatim = vec![comparable(Path::new(r"\\?\C:\Users\Ana\Pinhole\Data\engine"))];
        assert!(is_engine_exe(Path::new(r"C:\Users\Ana\Pinhole\Data\engine\sd\v\cuda\sd-server.exe"), &verbatim));
        assert!(!is_engine_exe(Path::new(r"C:\Program Files\Other\sd-server.exe"), &roots));
        assert!(!is_engine_exe(Path::new(r"C:\Users\Ana\Pinhole\Data\engine\sd\v\cuda\python.exe"), &roots));
    }

    /// A real process: a copy of `sleep` named `sd-server` under a temp
    /// `Data/engine/`. The sweep kills it (and waits), but leaves alone an
    /// engine this app manages and an `sd-server` outside the folder.
    #[cfg(unix)]
    #[tokio::test]
    async fn sweep_kills_only_unmanaged_engines_in_our_folder() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::Arc;
        let Some(sleep) = ["/usr/bin/sleep", "/bin/sleep"].iter().map(Path::new).find(|p| p.is_file()) else { return };
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Data").join("engine");
        let place = |dir: &Path| {
            std::fs::create_dir_all(dir).unwrap();
            let exe = dir.join("sd-server");
            std::fs::copy(sleep, &exe).unwrap();
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
            exe
        };
        let ours = place(&root.join("sd").join("v1").join("cpu"));
        let outside = place(&tmp.path().join("elsewhere"));

        let mut orphan = std::process::Command::new(&ours).arg("30").spawn().unwrap();
        let mut stranger = std::process::Command::new(&outside).arg("30").spawn().unwrap();
        let managed = crate::process::EngineProcess::spawn(&ours, &["30".into()], 1, Arc::new(crate::LogBuffer::default())).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        if orphan.try_wait().unwrap().is_some() {
            // `sleep` is a multi-call binary here (busybox): can't run under another name.
            let _ = stranger.kill();
            return;
        }

        let found = find_orphans(&root);
        assert_eq!(found.iter().map(|o| o.pid).collect::<Vec<_>>(), vec![orphan.id()], "{found:?}");

        let root2 = root.clone();
        let killed = tokio::task::spawn_blocking(move || kill_orphans(&root2, Duration::from_secs(10))).await.unwrap();
        assert_eq!(killed.len(), 1);
        assert_eq!(killed[0].pid, orphan.id());
        assert!(killed[0].exited, "{killed:?}");
        assert!(orphan.try_wait().unwrap().is_some(), "the leftover engine was killed");
        assert!(stranger.try_wait().unwrap().is_none(), "other programs are never touched");
        let pid = managed.pid().unwrap();
        // SAFETY: signal 0 only checks whether the pid exists.
        assert_eq!(unsafe { libc::kill(pid as libc::pid_t, 0) }, 0, "a managed engine is never touched");
        assert!(kill_orphans(&root, Duration::from_secs(1)).is_empty());

        managed.kill().await;
        let _ = stranger.kill();
        let _ = stranger.wait();
    }
}
