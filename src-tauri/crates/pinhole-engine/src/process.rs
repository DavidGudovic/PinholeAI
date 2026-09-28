//! Engine child processes (sd-server / llama-server).
//!
//! * binds to 127.0.0.1 on a free port (the caller passes the listen flags);
//! * working dir = the exe's folder; Linux gets `LD_LIBRARY_PATH` = exe dir;
//! * Windows: `CREATE_NO_WINDOW` + a Job Object with KILL_ON_JOB_CLOSE so the
//!   engine dies with the app even on a crash; Linux: `PR_SET_PDEATHSIG`;
//!   everywhere: `kill_on_drop`;
//! * stdout + stderr → [`LogBuffer`] (memory only, redacted);
//! * readiness = a caller-supplied probe (e.g. `GET /sdcpp/v1/capabilities`)
//!   polled until it succeeds, the process exits, the timeout hits or the
//!   caller cancels. Big models can take minutes to load;
//! * [`EngineProcess::stop`] / [`EngineProcess::kill`] wait until the process
//!   has really exited (its graphics memory is only freed then). Every engine
//!   this app started is listed in [`managed_pids`] until it is dropped, so the
//!   leftover-engine sweep ([`crate::orphans`]) never touches a live one.
//!
//! IMPORTANT (Linux): `PR_SET_PDEATHSIG` is tied to the *thread* that spawns the
//! child. Always call [`EngineProcess::spawn`] from an async task on a runtime
//! worker thread, never from `spawn_blocking` (those threads exit when idle).

use std::collections::BTreeSet;
use std::future::Future;
use std::io;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::logbuf::{LogBuffer, Stream};

/// Why an engine did not become ready.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadyError {
    #[error("the engine exited (code {code:?})")]
    Exited { code: Option<i32> },
    #[error("the engine did not become ready in time")]
    Timeout,
    #[error("cancelled")]
    Cancelled,
}

/// After a kill, how long to wait for the process to be gone. A CUDA process
/// can take a few seconds to exit while the driver frees its memory.
const KILL_WAIT: Duration = Duration::from_secs(15);

/// PIDs of engine processes started by this app that haven't been dropped.
static MANAGED: Mutex<BTreeSet<u32>> = Mutex::new(BTreeSet::new());

fn managed() -> MutexGuard<'static, BTreeSet<u32>> {
    MANAGED.lock().unwrap_or_else(|e| e.into_inner())
}

/// PIDs of the engines this app is running right now (any `AppCore`).
pub fn managed_pids() -> Vec<u32> {
    managed().iter().copied().collect()
}

/// A running engine process.
pub struct EngineProcess {
    child: Child,
    pid: Option<u32>,
    port: u16,
    args: Vec<String>,
    exe: PathBuf,
    logs: Arc<LogBuffer>,
    readers: Vec<JoinHandle<()>>,
    exit: Option<ExitStatus>,
    started: Instant,
}

impl std::fmt::Debug for EngineProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineProcess").field("exe", &self.exe).field("port", &self.port).finish()
    }
}

/// A free TCP port on 127.0.0.1 (bind :0, read, release).
pub fn free_port() -> io::Result<u16> {
    let l = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(l.local_addr()?.port())
}

impl EngineProcess {
    /// Spawn `exe args…`. `args` must already contain the loopback listen flags
    /// for `port`. Output goes to `logs` only.
    pub fn spawn(exe: &Path, args: &[String], port: u16, logs: Arc<LogBuffer>) -> io::Result<Self> {
        Self::spawn_with_env(exe, args, &[], port, logs)
    }

    /// [`EngineProcess::spawn`] with extra environment variables. Secrets (e.g.
    /// llama-server's per-launch API key) go here, not in `args`: other local
    /// users can read a process's command line, but not its environment.
    pub fn spawn_with_env(exe: &Path, args: &[String], env: &[(&str, &str)], port: u16, logs: Arc<LogBuffer>) -> io::Result<Self> {
        let exe_dir = exe.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        let mut cmd = Command::new(exe);
        cmd.envs(env.iter().copied());
        cmd.args(args)
            .current_dir(&exe_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        #[cfg(target_os = "linux")]
        {
            let mut ld = exe_dir.as_os_str().to_os_string();
            if let Some(old) = std::env::var_os("LD_LIBRARY_PATH") {
                if !old.is_empty() {
                    ld.push(":");
                    ld.push(old);
                }
            }
            cmd.env("LD_LIBRARY_PATH", ld);
            let parent = std::process::id();
            // SAFETY: only async-signal-safe libc calls between fork and exec.
            unsafe {
                cmd.pre_exec(move || {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                    if libc::getppid() as u32 != parent {
                        libc::_exit(1);
                    }
                    Ok(())
                });
            }
        }

        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        // Hold the registry lock across the spawn so a concurrent orphan sweep
        // (which lists processes first, then reads the registry) always sees it.
        let mut registry = managed();
        let mut child = cmd.spawn()?;
        let pid = child.id();
        if let Some(pid) = pid {
            registry.insert(pid);
        }
        drop(registry);

        #[cfg(windows)]
        if let Some(h) = child.raw_handle() {
            winjob::assign(h as _);
        }

        let mut readers = Vec::new();
        if let Some(mut out) = child.stdout.take() {
            let logs = logs.clone();
            readers.push(tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                while let Ok(n) = out.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    logs.push_bytes(&buf[..n]);
                }
                logs.flush();
            }));
        }
        if let Some(mut err) = child.stderr.take() {
            let logs = logs.clone();
            readers.push(tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                while let Ok(n) = err.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    logs.push_stream(Stream::Stderr, &buf[..n]);
                }
                logs.flush();
            }));
        }

        Ok(Self { child, pid, port, args: args.to_vec(), exe: exe.to_path_buf(), logs, readers, exit: None, started: Instant::now() })
    }

    /// OS process id (None once tokio reaped it).
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// `http://127.0.0.1:<port>`
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Arguments it was started with (to decide whether a restart is needed).
    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn exe(&self) -> &Path {
        &self.exe
    }

    pub fn logs(&self) -> &Arc<LogBuffer> {
        &self.logs
    }

    pub fn uptime(&self) -> Duration {
        self.started.elapsed()
    }

    /// `Some(status)` once the process has exited.
    pub fn try_exit(&mut self) -> Option<ExitStatus> {
        if self.exit.is_none() {
            if let Ok(Some(st)) = self.child.try_wait() {
                self.exit = Some(st);
            }
        }
        self.exit
    }

    pub fn is_running(&mut self) -> bool {
        self.try_exit().is_none()
    }

    /// Exit code if exited (Windows NTSTATUS values come through as negative i32).
    pub fn exit_code(&mut self) -> Option<i32> {
        self.try_exit().and_then(|s| s.code())
    }

    /// Poll `probe` every 500 ms until it returns `true`. `on_tick(elapsed)` is
    /// called each round (emit "loading" progress from it).
    pub async fn wait_ready<P, Fut>(
        &mut self,
        mut probe: P,
        timeout: Duration,
        cancel: &CancellationToken,
        mut on_tick: impl FnMut(Duration),
    ) -> Result<(), ReadyError>
    where
        P: FnMut() -> Fut,
        Fut: Future<Output = bool>,
    {
        let start = Instant::now();
        loop {
            if let Some(st) = self.try_exit() {
                // Let the readers drain the last lines (the error message).
                self.drain_readers().await;
                return Err(ReadyError::Exited { code: st.code() });
            }
            if cancel.is_cancelled() {
                return Err(ReadyError::Cancelled);
            }
            let ok = tokio::select! {
                ok = probe() => ok,
                _ = cancel.cancelled() => return Err(ReadyError::Cancelled),
            };
            if ok {
                return Ok(());
            }
            let elapsed = start.elapsed();
            if elapsed >= timeout {
                return Err(ReadyError::Timeout);
            }
            on_tick(elapsed);
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                _ = cancel.cancelled() => return Err(ReadyError::Cancelled),
            }
        }
    }

    async fn drain_readers(&mut self) {
        for r in self.readers.drain(..) {
            let _ = tokio::time::timeout(Duration::from_secs(2), r).await;
        }
    }

    /// Graceful stop: SIGTERM (Unix) and wait up to 3 s, then kill and wait
    /// until the process has exited (bounded; a process that still hasn't gone
    /// is left to the leftover-engine sweep before the next launch).
    pub async fn stop(mut self) {
        if self.try_exit().is_some() {
            self.drain_readers().await;
            return;
        }
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            // SAFETY: plain kill(2) on our own child's pid.
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
            if tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await.is_ok() {
                self.drain_readers().await;
                return;
            }
        }
        self.kill_and_wait().await;
    }

    /// Immediate kill (cancel while generating: sd-server can't interrupt a
    /// running job), then wait until the process has exited (bounded).
    pub async fn kill(mut self) {
        self.kill_and_wait().await;
    }

    async fn kill_and_wait(&mut self) {
        if self.try_exit().is_none() {
            let _ = self.child.start_kill();
            if let Ok(Ok(st)) = tokio::time::timeout(KILL_WAIT, self.child.wait()).await {
                self.exit = Some(st);
            }
        }
        self.drain_readers().await;
    }
}

impl Drop for EngineProcess {
    fn drop(&mut self) {
        // `kill_on_drop` ends a process that is still running; from now on it is
        // no longer ours, so a sweep may kill (and wait for) it if it lingers.
        if let Some(pid) = self.pid {
            managed().remove(&pid);
        }
    }
}

#[cfg(windows)]
mod winjob {
    //! One process-wide Job Object with KILL_ON_JOB_CLOSE: when Pinhole exits
    //! (or crashes) the OS closes the handle and terminates every engine.
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    struct Job(HANDLE);
    // SAFETY: a job handle is a kernel object handle, usable from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    static JOB: OnceLock<Option<Job>> = OnceLock::new();

    fn job() -> Option<HANDLE> {
        JOB.get_or_init(|| unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                CloseHandle(h);
                return None;
            }
            Some(Job(h))
        })
        .as_ref()
        .map(|j| j.0)
    }

    /// Put a child process into the job (best effort).
    pub fn assign(process: HANDLE) -> bool {
        match job() {
            // SAFETY: both handles are valid for the duration of the call.
            Some(j) => unsafe { AssignProcessToJobObject(j, process) != 0 },
            None => false,
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn script(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("fake-engine.sh");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[tokio::test]
    async fn captures_output_and_reports_exit() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = script(tmp.path(), "echo \"cwd=$(pwd)\"; echo '[ERROR] new_sd_ctx_t failed' 1>&2; printf '\\r  |==>   | 2/8 - 1.0it/s'; exit 3");
        let logs = Arc::new(LogBuffer::default());
        let mut p = EngineProcess::spawn(&exe, &["--listen-port".into(), "1".into()], 1, logs.clone()).unwrap();
        let cancel = CancellationToken::new();
        let err = p.wait_ready(|| async { false }, Duration::from_secs(10), &cancel, |_| {}).await.unwrap_err();
        assert_eq!(err, ReadyError::Exited { code: Some(3) });
        let text = logs.tail_text(10);
        assert!(text.contains("new_sd_ctx_t failed"), "{text}");
        let cwd = std::fs::canonicalize(tmp.path()).unwrap();
        assert!(text.contains(&format!("cwd={}", cwd.display())), "working dir = exe dir: {text}");
        let prog = logs.progress().unwrap();
        assert_eq!((prog.step, prog.total), (2, 8));
        assert_eq!(p.exit_code(), Some(3));
        p.stop().await;
    }

    #[tokio::test]
    async fn ready_probe_timeout_cancel_and_stop() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = script(tmp.path(), "echo started; exec sleep 30");
        let logs = Arc::new(LogBuffer::default());
        let mut p = EngineProcess::spawn(&exe, &[], 1, logs.clone()).unwrap();
        let cancel = CancellationToken::new();
        let mut calls = 0;
        p.wait_ready(
            || {
                calls += 1;
                let ready = calls >= 2;
                async move { ready }
            },
            Duration::from_secs(10),
            &cancel,
            |_| {},
        )
        .await
        .unwrap();
        assert!(p.is_running());
        let err = p.wait_ready(|| async { false }, Duration::from_millis(1), &cancel, |_| {}).await.unwrap_err();
        assert_eq!(err, ReadyError::Timeout);
        cancel.cancel();
        let err = p.wait_ready(|| async { false }, Duration::from_secs(5), &cancel, |_| {}).await.unwrap_err();
        assert_eq!(err, ReadyError::Cancelled);
        let t = Instant::now();
        p.stop().await;
        assert!(t.elapsed() < Duration::from_secs(4), "SIGTERM stops sleep quickly");
    }

    #[tokio::test]
    async fn secrets_go_through_the_environment_not_argv() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = script(tmp.path(), "echo \"key=$PINHOLE_TEST_KEY args=$*\"");
        let logs = Arc::new(LogBuffer::default());
        let mut p = EngineProcess::spawn_with_env(&exe, &["--port".into(), "1".into()], &[("PINHOLE_TEST_KEY", "k123")], 1, logs.clone()).unwrap();
        let _ = p.wait_ready(|| async { false }, Duration::from_secs(10), &CancellationToken::new(), |_| {}).await;
        assert!(logs.tail_text(5).contains("key=k123 args=--port 1"), "{}", logs.tail_text(5));
        assert!(!p.args().iter().any(|a| a.contains("k123")));
        p.stop().await;
    }

    #[tokio::test]
    async fn managed_until_dropped_and_kill_waits_for_exit() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = script(tmp.path(), "trap '' TERM; while :; do sleep 1; done");
        let logs = Arc::new(LogBuffer::default());
        let p = EngineProcess::spawn(&exe, &[], 1, logs.clone()).unwrap();
        let pid = p.pid().unwrap();
        assert!(managed_pids().contains(&pid));
        // SIGTERM is ignored: stop() must fall back to a kill and wait for the exit.
        p.stop().await;
        assert!(!managed_pids().contains(&pid));
        // SAFETY: signal 0 only checks whether the pid exists.
        let alive = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
        assert!(!alive, "the engine is gone once stop() returns");

        let p = EngineProcess::spawn(&exe, &[], 1, logs).unwrap();
        let pid = p.pid().unwrap();
        p.kill().await;
        assert!(!managed_pids().contains(&pid));
        assert!(unsafe { libc::kill(pid as libc::pid_t, 0) } != 0);
    }

    #[test]
    fn free_port_is_loopback_bindable() {
        // Tests run in parallel and other tests bind ephemeral ports too, so a
        // port can be taken between `free_port` and our bind: retry a few times.
        let bound = (0..5).any(|_| {
            let port = free_port().unwrap();
            assert!(port > 0);
            TcpListener::bind(("127.0.0.1", port)).is_ok()
        });
        assert!(bound, "free_port never returned a bindable loopback port");
    }
}
