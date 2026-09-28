//! DownloadManager: ordering, wait, cancel, status history, events.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::download::{data, sha};
use crate::download::{part_path, DownloadManager, DownloadSpec, DownloadState, GroupStatus};
use crate::testutil::{MockResponse, MockServer};
use crate::{HttpClient, OfflineFlag};

fn manager() -> DownloadManager {
    DownloadManager::new(HttpClient::new_for_tests(OfflineFlag::new(false), true).unwrap())
}

fn file(srv: &MockServer, path: &str, dir: &std::path::Path, body: &[u8]) -> DownloadSpec {
    DownloadSpec {
        url: srv.url(path),
        dest: dir.join(path.trim_start_matches('/')),
        sha256: Some(sha(body)),
        size_bytes: Some(body.len() as u64),
        label: format!("file {path}"),
        headers: vec![],
    }
}

async fn wait_for_state(m: &DownloadManager, id: &str, state: DownloadState) {
    for _ in 0..200 {
        if m.status().iter().any(|s| s.group_id == id && s.state == state) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("group never reached {state:?}: {:?}", m.status());
}

#[tokio::test]
async fn groups_run_in_order_and_files_sequentially() {
    let body = data(20_000);
    let b = body.clone();
    let srv = MockServer::start(move |_| MockResponse::ok(b.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager();
    let mut rx = m.subscribe();

    let a = m.enqueue(
        "Model A".into(),
        vec![file(&srv, "/a1", dir.path(), &body), file(&srv, "/a2", dir.path(), &body)],
    );
    let b = m.enqueue("Model B".into(), vec![file(&srv, "/b1", dir.path(), &body)]);
    assert_ne!(a, b);

    let got_b = m.wait(&b).await.unwrap();
    let got_a = m.wait(&a).await.unwrap();
    assert_eq!(got_a.len(), 2);
    assert_eq!(got_b.len(), 1);
    assert_eq!(got_a[1].path, dir.path().join("a2"));
    let order: Vec<String> = srv.requests().into_iter().map(|r| r.path).collect();
    assert_eq!(order, ["/a1", "/a2", "/b1"]);

    // Events: A finishes before B starts downloading.
    let mut events: Vec<GroupStatus> = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    let a_done = events.iter().position(|e| e.group_id == a && e.state == DownloadState::Done).unwrap();
    let b_start = events.iter().position(|e| e.group_id == b && e.state == DownloadState::Downloading).unwrap();
    assert!(a_done < b_start);
    assert_eq!(events[0].group_id, a);
    assert_eq!(events[0].state, DownloadState::Queued);
    assert_eq!(events[0].total_bytes, 2 * body.len() as u64);

    let status = m.status();
    let sa = status.iter().find(|s| s.group_id == a).unwrap();
    assert_eq!(sa.state, DownloadState::Done);
    assert_eq!(sa.downloaded_bytes, 2 * body.len() as u64);
    assert_eq!(sa.total_bytes, sa.downloaded_bytes);
    assert_eq!(sa.file_count, 2);
    assert_eq!(sa.error, None);

    // wait() still works after the group finished.
    assert_eq!(m.wait(&a).await.unwrap().len(), 2);
    assert!(m.wait("no-such-group").await.is_err());
}

#[tokio::test]
async fn cancel_queued_and_running_groups() {
    let body = data(300_000);
    let b = body.clone();
    let stall = Arc::new(AtomicBool::new(true));
    let st = stall.clone();
    let srv = MockServer::start(move |req| {
        let r = MockResponse::ranged(req, &b);
        if st.load(Ordering::SeqCst) {
            r.stall_after(100_000)
        } else {
            r
        }
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager();

    let running = m.enqueue("Running".into(), vec![file(&srv, "/r", dir.path(), &body)]);
    let queued = m.enqueue("Queued".into(), vec![file(&srv, "/q", dir.path(), &body)]);
    wait_for_state(&m, &running, DownloadState::Downloading).await;

    m.cancel(&queued);
    let err = m.wait_detailed(&queued).await.unwrap_err();
    assert_eq!(err.code, "cancelled");
    assert_eq!(m.wait(&queued).await.unwrap_err(), "Cancelled");

    tokio::time::sleep(Duration::from_millis(200)).await;
    m.cancel(&running);
    assert_eq!(m.wait_detailed(&running).await.unwrap_err().code, "cancelled");
    let states: Vec<(String, DownloadState)> = m.status().into_iter().map(|s| (s.group_id, s.state)).collect();
    assert!(states.contains(&(running.clone(), DownloadState::Cancelled)));
    assert!(states.contains(&(queued.clone(), DownloadState::Cancelled)));
    // The queued group never hit the network; the running one kept its .part.
    assert!(srv.requests().iter().all(|r| r.path != "/q"));
    assert_eq!(std::fs::metadata(part_path(&dir.path().join("r"))).unwrap().len(), 100_000);

    // Cancelling a finished group is a no-op.
    m.cancel(&running);

    // A new group resumes from the kept .part.
    stall.store(false, Ordering::SeqCst);
    let again = m.enqueue("Running again".into(), vec![file(&srv, "/r", dir.path(), &body)]);
    let got = m.wait(&again).await.unwrap();
    assert_eq!(got[0].sha256, sha(&body));
    assert_eq!(srv.requests().last().unwrap().header("range"), Some("bytes=100000-"));
}

#[tokio::test]
async fn failures_carry_plain_language_errors() {
    let srv = MockServer::start(|req| match req.path.as_str() {
        "/bad" => MockResponse::ok("tampered"),
        _ => MockResponse::ok("fine"),
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager();
    let mut bad = file(&srv, "/bad", dir.path(), b"original");
    bad.size_bytes = None;
    let ok = file(&srv, "/ok", dir.path(), b"fine");
    let id = m.enqueue("Broken".into(), vec![ok, bad]);
    let err = m.wait_detailed(&id).await.unwrap_err();
    assert_eq!(err.code, "hash_mismatch");
    let s = m.status().into_iter().find(|s| s.group_id == id).unwrap();
    assert_eq!(s.state, DownloadState::Failed);
    assert_eq!(s.file_index, 1);
    assert_eq!(s.error.as_deref(), Some("The download was corrupted (checksum mismatch). Try again."));

    // Offline → plain message, and the queue keeps working afterwards.
    let offline_m = DownloadManager::new(HttpClient::new_for_tests(OfflineFlag::new(true), true).unwrap());
    let id = offline_m.enqueue("Offline".into(), vec![file(&srv, "/ok", dir.path(), b"fine")]);
    let err = offline_m.wait_detailed(&id).await.unwrap_err();
    assert_eq!(err.code, "offline");
    assert!(err.message.starts_with("Offline mode is on"));

    // Not enough disk for the whole group → one clear message, nothing requested.
    let before = srv.requests().len();
    let mut huge = file(&srv, "/ok", dir.path(), b"fine");
    huge.size_bytes = Some(u64::MAX / 4);
    huge.dest = dir.path().join("huge.bin");
    let id = m.enqueue("Huge".into(), vec![huge]);
    let err = m.wait_detailed(&id).await.unwrap_err();
    assert_eq!(err.code, "disk_space");
    assert!(err.message.starts_with("Not enough disk space — free up "), "{}", err.message);
    assert_eq!(srv.requests().len(), before);
}

#[tokio::test]
async fn empty_group_and_status_history() {
    let srv = MockServer::start(|_| MockResponse::ok("x")).await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager();
    assert!(m.wait(&m.enqueue("Nothing".into(), vec![])).await.unwrap().is_empty());

    let mut ids = Vec::new();
    for i in 0..25 {
        let path = format!("/f{i}");
        ids.push(m.enqueue(format!("G{i}"), vec![file(&srv, &path, dir.path(), b"x")]));
    }
    for id in &ids {
        m.wait(id).await.unwrap();
    }
    let status = m.status();
    assert_eq!(status.len(), 20, "only the last 20 finished groups are listed");
    assert_eq!(status.last().unwrap().group_id, *ids.last().unwrap());
    // Older groups are still waitable.
    assert!(m.wait(&ids[0]).await.is_ok());
}

#[test]
fn manager_created_and_used_outside_a_runtime() {
    // AppCore::new runs outside Tokio; enqueue from a non-runtime thread falls
    // back to a private worker thread.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let srv = rt.block_on(MockServer::start(|_| MockResponse::ok("hi")));
    let dir = tempfile::tempdir().unwrap();
    let m = manager();
    let id = m.enqueue("Outside".into(), vec![file(&srv, "/x", dir.path(), b"hi")]);
    let got = rt.block_on(m.wait(&id)).unwrap();
    assert_eq!(std::fs::read(&got[0].path).unwrap(), b"hi");
}
