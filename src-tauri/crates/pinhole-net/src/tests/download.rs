//! download_file: happy path, hashes, resume, cancel, offline, disk space.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::download::{check_free_space, download_file, part_path, sha256_file, DownloadError, DownloadSpec};
use crate::testutil::{MockResponse, MockServer};
use crate::{HttpClient, NetError, OfflineFlag};

pub(crate) fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 % 251) as u8).collect()
}

pub(crate) fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn client() -> HttpClient {
    HttpClient::new_for_tests(OfflineFlag::new(false), true).unwrap()
}

fn spec(url: String, dest: &Path, sha256: Option<String>, size: Option<u64>) -> DownloadSpec {
    DownloadSpec { url, dest: dest.to_path_buf(), sha256, size_bytes: size, label: "Test file".into(), ..Default::default() }
}

type Calls = Arc<Mutex<Vec<(u64, Option<u64>)>>>;

fn recorder() -> (Calls, impl Fn(u64, Option<u64>) + Send + Sync) {
    let calls: Calls = Arc::default();
    let c = calls.clone();
    (calls, move |d, t| c.lock().push((d, t)))
}

#[test]
fn sha256_file_known_vector() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("abc.txt");
    std::fs::write(&p, b"abc").unwrap();
    assert_eq!(sha256_file(&p).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
}

#[test]
fn free_space_check() {
    let dir = tempfile::tempdir().unwrap();
    check_free_space(dir.path(), 1024).unwrap();
    // Non-existent sub-directories are checked via their nearest existing ancestor.
    check_free_space(&dir.path().join("a/b/c"), 1024).unwrap();
    match check_free_space(dir.path(), u64::MAX / 4) {
        Err(e @ DownloadError::DiskSpace { .. }) => {
            let msg = e.user_message("");
            assert!(msg.starts_with("Not enough disk space — free up ") && msg.ends_with(" GB and try again."), "{msg}");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn happy_path_verifies_and_renames() {
    let body = data(300_000);
    let b = body.clone();
    let srv = MockServer::start(move |_| MockResponse::ok(b.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("models/vae/vae.safetensors");
    let (calls, progress) = recorder();

    let s = spec(srv.url("/vae"), &dest, Some(sha(&body).to_uppercase()), Some(body.len() as u64));
    let got = download_file(&client(), &s, &CancellationToken::new(), &progress).await.unwrap();

    assert_eq!(got.path, dest);
    assert_eq!(got.sha256, sha(&body));
    assert_eq!(got.size_bytes, body.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), body);
    assert!(!part_path(&dest).exists());
    let calls = calls.lock();
    assert_eq!(calls.first(), Some(&(0, Some(body.len() as u64))));
    assert_eq!(calls.last(), Some(&(body.len() as u64, Some(body.len() as u64))));
}

#[tokio::test]
async fn todo_or_missing_hash_is_not_enforced() {
    let srv = MockServer::start(|_| MockResponse::ok("payload")).await;
    let dir = tempfile::tempdir().unwrap();
    for (i, h) in [Some("TODO".to_string()), Some("todo".to_string()), Some(String::new()), None].into_iter().enumerate() {
        let dest = dir.path().join(format!("f{i}.bin"));
        let got = download_file(&client(), &spec(srv.url("/f"), &dest, h, None), &CancellationToken::new(), &|_, _| {})
            .await
            .unwrap();
        assert_eq!(got.sha256, sha(b"payload"));
        assert_eq!(std::fs::read(&dest).unwrap(), b"payload");
    }
}

#[tokio::test]
async fn hash_mismatch_deletes_part() {
    let srv = MockServer::start(|_| MockResponse::ok("tampered")).await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("model.safetensors");
    let s = spec(srv.url("/m"), &dest, Some(sha(b"original")), None);
    match download_file(&client(), &s, &CancellationToken::new(), &|_, _| {}).await {
        Err(e @ DownloadError::HashMismatch { .. }) => {
            assert_eq!(e.user_message(&s.url), "The download was corrupted (checksum mismatch). Try again.");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(!dest.exists());
    assert!(!part_path(&dest).exists());
}

#[tokio::test]
async fn resumes_partial_part_with_range() {
    let body = data(200_000);
    let b = body.clone();
    let srv = MockServer::start(move |req| MockResponse::ranged(req, &b)).await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("big.gguf");
    std::fs::write(part_path(&dest), &body[..70_000]).unwrap();

    let (calls, progress) = recorder();
    let s = spec(srv.url("/big"), &dest, Some(sha(&body)), Some(body.len() as u64));
    let got = download_file(&client(), &s, &CancellationToken::new(), &progress).await.unwrap();

    assert_eq!(got.sha256, sha(&body));
    assert_eq!(std::fs::read(&dest).unwrap(), body);
    let reqs = srv.requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].header("range"), Some("bytes=70000-"));
    // Progress starts from the bytes already on disk.
    assert_eq!(calls.lock().first(), Some(&(70_000, Some(body.len() as u64))));
}

#[tokio::test]
async fn restarts_when_server_ignores_range() {
    let body = data(50_000);
    let b = body.clone();
    let srv = MockServer::start(move |_| MockResponse::ok(b.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("f.bin");
    // A stale part whose bytes do NOT match the real file: must be discarded.
    std::fs::write(part_path(&dest), vec![0xFFu8; 20_000]).unwrap();

    let s = spec(srv.url("/f"), &dest, Some(sha(&body)), Some(body.len() as u64));
    let got = download_file(&client(), &s, &CancellationToken::new(), &|_, _| {}).await.unwrap();
    assert_eq!(got.size_bytes, body.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), body);
    assert_eq!(srv.requests()[0].header("range"), Some("bytes=20000-"));
}

#[tokio::test]
async fn range_not_satisfiable_on_complete_part_finalizes() {
    let body = data(10_000);
    let b = body.clone();
    let srv = MockServer::start(move |req| MockResponse::ranged(req, &b)).await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("f.bin");
    std::fs::write(part_path(&dest), &body).unwrap();
    // Size unknown → we must ask the server, which answers 416 `bytes */10000`.
    let s = spec(srv.url("/f"), &dest, Some(sha(&body)), None);
    download_file(&client(), &s, &CancellationToken::new(), &|_, _| {}).await.unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), body);
    assert_eq!(srv.requests().len(), 1);
}

#[tokio::test]
async fn complete_part_with_known_size_needs_no_network() {
    let srv = MockServer::start(|_| MockResponse::status(500)).await;
    let body = data(4096);
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("f.bin");
    std::fs::write(part_path(&dest), &body).unwrap();
    let s = spec(srv.url("/f"), &dest, Some(sha(&body)), Some(body.len() as u64));
    download_file(&client(), &s, &CancellationToken::new(), &|_, _| {}).await.unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), body);
    assert_eq!(srv.connections(), 0);
}

#[tokio::test]
async fn existing_verified_dest_is_not_downloaded_again() {
    let srv = MockServer::start(|_| MockResponse::ok("other")).await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("f.bin");
    std::fs::write(&dest, b"already here").unwrap();
    let s = spec(srv.url("/f"), &dest, Some(sha(b"already here")), Some(12));
    let got = download_file(&client(), &s, &CancellationToken::new(), &|_, _| {}).await.unwrap();
    assert_eq!(got.sha256, sha(b"already here"));
    assert_eq!(srv.connections(), 0);
}

#[tokio::test]
async fn cancel_keeps_part_and_resume_completes() {
    let body = data(400_000);
    let b = body.clone();
    let stall = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let st = stall.clone();
    let srv = MockServer::start(move |req| {
        let r = MockResponse::ranged(req, &b);
        if st.load(std::sync::atomic::Ordering::SeqCst) {
            r.stall_after(150_000)
        } else {
            r
        }
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("m.safetensors");
    let s = spec(srv.url("/m"), &dest, Some(sha(&body)), Some(body.len() as u64));
    let client = client();

    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    let part = part_path(&dest);
    let (calls, progress) = recorder();
    let watcher = tokio::spawn(async move {
        // Cancel once the stalled transfer has delivered its first bytes.
        tokio::time::sleep(Duration::from_millis(400)).await;
        c2.cancel();
    });
    let r = download_file(&client, &s, &cancel, &progress).await;
    watcher.await.unwrap();
    assert!(matches!(r, Err(DownloadError::Cancelled)), "{r:?}");
    assert!(!dest.exists());
    let kept = std::fs::read(&part).unwrap();
    assert_eq!(kept.len(), 150_000, "all received bytes are flushed to .part");
    assert_eq!(kept, body[..150_000]);
    assert!(!calls.lock().is_empty());

    stall.store(false, std::sync::atomic::Ordering::SeqCst);
    let got = download_file(&client, &s, &CancellationToken::new(), &|_, _| {}).await.unwrap();
    assert_eq!(got.sha256, sha(&body));
    assert_eq!(srv.requests().last().unwrap().header("range"), Some("bytes=150000-"));
    assert!(!part.exists());
}

#[tokio::test]
async fn offline_download_touches_nothing() {
    let srv = MockServer::start(|_| MockResponse::ok("x")).await;
    let client = HttpClient::new_for_tests(OfflineFlag::new(true), true).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("sub/f.bin");
    let r = download_file(&client, &spec(srv.url("/f"), &dest, None, None), &CancellationToken::new(), &|_, _| {}).await;
    match r {
        Err(e @ DownloadError::Net(NetError::Offline)) => assert!(e.user_message("").starts_with("Offline mode is on")),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(srv.connections(), 0);
    assert!(!dir.path().join("sub").exists());
}

#[tokio::test]
async fn not_enough_space_fails_before_connecting() {
    let srv = MockServer::start(|_| MockResponse::ok("x")).await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("huge.bin");
    let s = spec(srv.url("/f"), &dest, None, Some(u64::MAX / 4));
    let r = download_file(&client(), &s, &CancellationToken::new(), &|_, _| {}).await;
    assert!(matches!(r, Err(DownloadError::DiskSpace { .. })), "{r:?}");
    assert_eq!(srv.connections(), 0);
}

#[tokio::test]
async fn unauthorized_and_missing_files() {
    let srv = MockServer::start(|req| match req.path.as_str() {
        "/401" => MockResponse::status(401),
        _ => MockResponse::status(404),
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("f.bin");
    let r = download_file(&client(), &spec(srv.url("/401"), &dest, None, None), &CancellationToken::new(), &|_, _| {}).await;
    let e = r.unwrap_err();
    assert!(matches!(e, DownloadError::Net(NetError::Unauthorized(401))));
    assert_eq!(e.code(), "unauthorized");
    assert!(e.user_message("https://civitai.com/api/download/models/1").starts_with("CivitAI needs an API key"));
    assert!(e.user_message("https://huggingface.co/x/resolve/main/y").starts_with("Hugging Face refused"));
    let r = download_file(&client(), &spec(srv.url("/404"), &dest, None, None), &CancellationToken::new(), &|_, _| {}).await;
    assert!(matches!(r, Err(DownloadError::Net(NetError::Status(404)))));
    assert_eq!(srv.requests().len(), 2, "4xx errors are not retried");
}

#[tokio::test]
async fn api_key_header_reaches_origin_but_not_the_cdn() {
    let body = data(1000);
    let b = body.clone();
    let cdn = MockServer::start(move |req| MockResponse::ranged(req, &b)).await;
    let cdn_url = cdn.cdn_url("/presigned?X-Amz-Signature=abc");
    let origin = MockServer::start(move |_| MockResponse::redirect(&cdn_url).header("x", "y")).await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("lora.safetensors");
    let mut s = spec(origin.url("/api/download/models/1"), &dest, Some(sha(&body)), None);
    s.headers = vec![("Authorization".into(), "Bearer SECRET_KEY".into())];
    download_file(&client(), &s, &CancellationToken::new(), &|_, _| {}).await.unwrap();
    assert_eq!(origin.requests()[0].header("authorization"), Some("Bearer SECRET_KEY"));
    assert_eq!(cdn.requests()[0].header("authorization"), None);
    // DownloadSpec never serializes its headers.
    assert!(!serde_json::to_string(&s).unwrap().contains("SECRET_KEY"));
}
