//! Offline mode (CLAUDE.md "Privacy tests"): with Offline on, every network call
//! fails with `Offline` BEFORE a socket opens — proven with a local listener that
//! counts accepted connections. Also: the production client refuses hosts outside
//! the allow-list without connecting.

use pinhole_net::download::{self, DownloadSpec};
use pinhole_net::{HttpClient, NetError, OfflineFlag};
use pinhole_tests::CountingListener;
use tokio_util::sync::CancellationToken;

fn is_offline<T>(r: &Result<T, NetError>) -> bool {
    matches!(r, Err(NetError::Offline))
}

fn spec(url: String, dest: std::path::PathBuf) -> DownloadSpec {
    DownloadSpec { url, dest, sha256: None, size_bytes: None, label: "offline test".into(), headers: vec![] }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offline_mode_blocks_every_call_before_a_socket_opens() {
    let listener = CountingListener::start().await;
    let flag = OfflineFlag::new(true);
    let client = HttpClient::new_for_tests(flag.clone(), true).expect("test client");
    let url = listener.http_url("/api/v1/models");
    let tmp = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    let noop = |_: u64, _: Option<u64>| {};

    // get_json / get_bytes
    let r = client.get_json::<serde_json::Value>(&url, &[]).await;
    assert!(is_offline(&r), "get_json: expected Offline, got {:?}", r.map(|_| ()));
    let r = client.get_bytes(&url, &[("accept", "image/*")], 1024).await;
    assert!(is_offline(&r), "get_bytes: expected Offline, got {:?}", r.map(|_| ()));

    // get + send
    match client.get(&url) {
        Err(NetError::Offline) => {}
        Err(other) => panic!("get: expected Offline, got {other:?}"),
        Ok(rb) => {
            let r = client.send(rb).await;
            assert!(is_offline(&r), "send: expected Offline, got {:?}", r.map(|_| ()));
        }
    }

    // check_url on allow-listed https hosts (no DNS lookup, no socket)
    for u in ["https://huggingface.co/api/models", "https://civitai.com/api/v1/enums", "https://github.com/"] {
        assert!(matches!(client.check_url(u), Err(NetError::Offline)), "check_url({u}) must be Offline");
        let r = client.get_bytes(u, &[], 1024).await;
        assert!(is_offline(&r), "get_bytes({u}): expected Offline, got {:?}", r.map(|_| ()));
    }

    // resumable download
    let dest = tmp.path().join("file.bin");
    let r = download::download_file(&client, &spec(url.clone(), dest.clone()), &cancel, &noop).await;
    match &r {
        Err(download::DownloadError::Net(NetError::Offline)) => {}
        other => panic!("download_file: expected Net(Offline), got {:?}", other.as_ref().map(|_| ())),
    }
    assert!(!dest.exists(), "offline download must not create the destination file");
    let part_exists = std::fs::read_dir(tmp.path()).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".part"));
    assert!(!part_exists, "offline download must not create a .part file");

    assert_eq!(listener.accepted(), 0, "a socket was opened while Offline mode was on");

    // Sanity: the listener really works — with Offline off the same call connects…
    flag.set(false);
    let _ = client.get_bytes(&url, &[], 1024).await;
    assert_eq!(listener.accepted(), 1, "online request should reach the local listener (test harness check)");

    // …and switching Offline back on blocks again, including an already-built request.
    let prebuilt = client.get(&url).expect("builder while online");
    flag.set(true);
    let r = client.send(prebuilt).await;
    assert!(is_offline(&r), "request built before Offline was switched on must fail Offline, got {:?}", r.map(|_| ()));
    let r = client.get_json::<serde_json::Value>(&url, &[]).await;
    assert!(is_offline(&r));
    let r = download::download_file(&client, &spec(url.clone(), tmp.path().join("again.bin")), &cancel, &noop).await;
    assert!(matches!(r, Err(download::DownloadError::Net(NetError::Offline))));
    assert_eq!(listener.accepted(), 1, "no new connection may be opened after Offline was switched back on");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn production_client_refuses_hosts_outside_the_allow_list() {
    let listener = CountingListener::start().await;
    let client = HttpClient::new(OfflineFlag::new(false)).expect("production client");

    // Loopback / arbitrary hosts, http and https: refused before connecting.
    for url in [
        listener.http_url("/x"),
        listener.https_url("/x"),
        format!("https://localhost:{}/x", listener.port()),
    ] {
        assert!(client.check_url(&url).is_err(), "check_url must refuse {url}");
        let r = client.get_bytes(&url, &[], 1024).await;
        assert!(r.is_err(), "get_bytes must refuse {url}");
        assert!(!is_offline(&r), "{url}: should be refused by the allow-list, not Offline");
        assert!(client.get(&url).is_err(), "get() must refuse {url}");
    }
    assert_eq!(listener.accepted(), 0, "the production client connected to a non-allow-listed host");

    // Look-alike and unrelated domains (checked without any network access).
    for url in [
        "https://example.com/model.safetensors",
        "https://huggingface.co.evil.example/x",
        "https://evilhuggingface.co/x",
        "https://civitai.com@evil.example/x",
        "https://cdn.jsdelivr.net/npm/x",
        "https://fonts.googleapis.com/css2?family=Inter",
        "http://huggingface.co/x",
        "ftp://github.com/x",
        "file:///etc/passwd",
    ] {
        match client.check_url(url) {
            Err(NetError::HostNotAllowed(_)) | Err(NetError::BadUrl(_)) => {}
            other => panic!("check_url({url}) must be refused by the allow-list, got {:?}", other.map(|u| u.to_string())),
        }
    }

    // Allow-listed hosts pass the URL check (no request is made here).
    for url in [
        "https://huggingface.co/api/models",
        "https://civitai.com/api/v1/models",
        "https://github.com/leejet/stable-diffusion.cpp/releases",
    ] {
        assert!(client.check_url(url).is_ok(), "check_url({url}) should be allowed");
    }

    // Production client in Offline mode: Offline wins over everything.
    let offline = HttpClient::new(OfflineFlag::new(true)).expect("production client");
    let r = offline.get_json::<serde_json::Value>("https://civitai.com/api/v1/enums", &[]).await;
    assert!(is_offline(&r), "production client in Offline mode must return Offline");
}
