//! HttpClient / LocalClient behaviour against local mock servers.

use std::time::Duration;

use crate::testutil::{MockResponse, MockServer};
use crate::{HttpClient, LocalClient, NetError, OfflineFlag};

fn test_client() -> (HttpClient, OfflineFlag) {
    let offline = OfflineFlag::new(false);
    (HttpClient::new_for_tests(offline.clone(), true).unwrap(), offline)
}

#[test]
fn clients_build_outside_a_runtime() {
    HttpClient::new(OfflineFlag::new(false)).unwrap();
    LocalClient::new().unwrap();
}

#[test]
fn production_check_url_matrix() {
    let client = HttpClient::new(OfflineFlag::new(false)).unwrap();
    for ok in [
        "https://civitai.com/api/v1/models?limit=1",
        "https://image.civitai.com/xG1nkqKTMzGDvpLrqFT7WA/abc/width=450/1.jpeg",
        "https://huggingface.co/org/repo/resolve/main/model.safetensors",
        "https://github.com/leejet/stable-diffusion.cpp/releases/download/x/y.zip",
        "https://codeload.github.com/a/b/zip/refs/tags/v1",
    ] {
        client.check_url(ok).unwrap_or_else(|e| panic!("{ok}: {e}"));
    }
    for (bad, want_host) in [
        ("https://huggingface.co.evil.com/x", true),
        ("https://evilcivitai.com/x", true),
        ("https://civitai.com.evil/x", true),
        // CDN hosts are redirect-only.
        ("https://cas-bridge.xethub.hf.co/x", true),
        ("https://release-assets.githubusercontent.com/x", true),
        ("https://b.acct.r2.cloudflarestorage.com/x", true),
        ("https://raw.githubusercontent.com/x", true),
        ("https://example.com/", true),
        ("https://github.com:8443/", true),
        ("http://huggingface.co/x", false),
        ("http://127.0.0.1:1234/x", false),
        ("file:///etc/passwd", false),
        ("not a url", false),
    ] {
        match client.check_url(bad) {
            Err(NetError::HostNotAllowed(_)) if want_host => {}
            Err(NetError::BadUrl(_)) if !want_host => {}
            other => panic!("{bad}: unexpected {other:?}"),
        }
    }
}

#[test]
fn offline_is_checked_before_anything_else() {
    let offline = OfflineFlag::new(true);
    let client = HttpClient::new(offline.clone()).unwrap();
    assert_eq!(client.check_url("https://civitai.com/").unwrap_err(), NetError::Offline);
    assert_eq!(client.check_url("https://evil.com/").unwrap_err(), NetError::Offline);
    assert_eq!(client.check_url("garbage").unwrap_err(), NetError::Offline);
    assert!(matches!(client.get("https://civitai.com/"), Err(NetError::Offline)));
    offline.set(false);
    client.check_url("https://civitai.com/").unwrap();
}

#[tokio::test]
async fn offline_opens_no_socket() {
    let srv = MockServer::start(|_| MockResponse::ok("hello")).await;
    let (client, offline) = test_client();
    offline.set(true);
    assert_eq!(client.get_bytes(&srv.url("/a"), &[], 1024).await.unwrap_err(), NetError::Offline);
    assert_eq!(client.get_json::<serde_json::Value>(&srv.url("/b"), &[]).await.unwrap_err(), NetError::Offline);
    assert!(matches!(client.get(&srv.url("/c")), Err(NetError::Offline)));

    // A request built while online is still refused once Offline is switched on.
    offline.set(false);
    let rb = client.get(&srv.url("/d")).unwrap();
    offline.set(true);
    assert_eq!(client.send(rb).await.unwrap_err(), NetError::Offline);

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(srv.connections(), 0, "no connection may be opened in Offline mode");
}

#[tokio::test]
async fn resolver_refuses_when_offline() {
    use reqwest::dns::Resolve;
    use std::str::FromStr;
    let r = crate::GuardedResolver { offline: OfflineFlag::new(true) };
    let err = r.resolve(reqwest::dns::Name::from_str("civitai.com").unwrap()).await.err().unwrap();
    assert!(err.downcast_ref::<NetError>().is_some_and(|e| *e == NetError::Offline));
}

#[tokio::test]
async fn get_bytes_and_json() {
    let srv = MockServer::start(|req| match req.path.as_str() {
        "/json" => MockResponse::json(&serde_json::json!({"items": [1, 2, 3]})),
        "/bad-json" => MockResponse::ok("{nope"),
        "/big" => MockResponse::ok(vec![7u8; 5000]),
        "/big-chunked" => MockResponse::ok(vec![7u8; 5000]).without_content_length(),
        _ => MockResponse::ok("hello"),
    })
    .await;
    let (client, _) = test_client();

    assert_eq!(client.get_bytes(&srv.url("/x"), &[], 1024).await.unwrap(), b"hello");
    let v: serde_json::Value = client.get_json(&srv.url("/json"), &[]).await.unwrap();
    assert_eq!(v["items"][2], 3);
    assert!(matches!(client.get_json::<serde_json::Value>(&srv.url("/bad-json"), &[]).await, Err(NetError::Decode(_))));
    assert_eq!(client.get_bytes(&srv.url("/big"), &[], 4096).await.unwrap_err(), NetError::TooLarge);
    assert_eq!(client.get_bytes(&srv.url("/big-chunked"), &[], 4096).await.unwrap_err(), NetError::TooLarge);
    assert_eq!(client.get_bytes(&srv.url("/big-chunked"), &[], 5000).await.unwrap().len(), 5000);

    let reqs = srv.requests();
    let ua = reqs[0].header("user-agent").unwrap();
    assert!(ua.starts_with("Pinhole/"), "{ua}");
    assert_eq!(reqs[1].header("accept"), Some("application/json"));
}

#[tokio::test]
async fn status_mapping() {
    let srv = MockServer::start(|req| {
        let code: u16 = req.path.trim_start_matches('/').parse().unwrap_or(200);
        MockResponse::status(code)
    })
    .await;
    let (client, _) = test_client();
    for (code, want) in [
        (401, NetError::Unauthorized(401)),
        (403, NetError::Unauthorized(403)),
        (404, NetError::Status(404)),
        (429, NetError::Status(429)),
        (500, NetError::Status(500)),
    ] {
        assert_eq!(client.get_bytes(&srv.url(&format!("/{code}")), &[], 1024).await.unwrap_err(), want);
    }
}

#[tokio::test]
async fn timeout_maps_to_timeout() {
    let srv = MockServer::start(|_| MockResponse::ok("late").delay(Duration::from_secs(5))).await;
    let (client, _) = test_client();
    let rb = client.get(&srv.url("/slow")).unwrap().timeout(Duration::from_millis(300));
    assert_eq!(client.send(rb).await.unwrap_err(), NetError::Timeout);
}

#[tokio::test]
async fn transport_errors_never_contain_the_url() {
    // Bind then drop a listener to get a closed port.
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let (client, _) = test_client();
    let url = format!("http://127.0.0.1:{port}/file?token=SECRET_TOKEN");
    match client.get_bytes(&url, &[], 16).await.unwrap_err() {
        NetError::Transport(msg) => {
            assert!(!msg.contains("SECRET_TOKEN") && !msg.contains("127.0.0.1"), "{msg}");
            assert!(msg.contains("refused"), "root cause should be kept: {msg}");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(crate::scrub_urls("error for url (https://x.y/a?t=1) here"), "error for url (<url>) here");
}

#[tokio::test]
async fn redirect_to_disallowed_host_is_refused() {
    let srv = MockServer::start(|req| match req.path.as_str() {
        "/evil" => MockResponse::redirect("https://example.com/steal"),
        "/lookalike" => MockResponse::redirect("https://huggingface.co.evil.com/x"),
        "/downgrade" => MockResponse::redirect("http://github.com/x"),
        "/wrong-owner" => MockResponse::redirect("https://b.acct.r2.cloudflarestorage.com/x"),
        _ => MockResponse::ok("x"),
    })
    .await;
    let (client, _) = test_client();
    assert_eq!(
        client.get_bytes(&srv.url("/evil"), &[], 16).await.unwrap_err(),
        NetError::HostNotAllowed("example.com".into())
    );
    assert!(matches!(client.get_bytes(&srv.url("/lookalike"), &[], 16).await, Err(NetError::HostNotAllowed(_))));
    assert!(matches!(client.get_bytes(&srv.url("/downgrade"), &[], 16).await, Err(NetError::BadUrl(_))));
    // An R2 URL is only acceptable when the chain started on civitai.com.
    assert!(matches!(client.get_bytes(&srv.url("/wrong-owner"), &[], 16).await, Err(NetError::HostNotAllowed(_))));
}

#[tokio::test]
async fn cdn_host_only_reachable_by_redirect() {
    let cdn = MockServer::start(|_| MockResponse::ok("from-cdn")).await;
    let cdn_url = cdn.cdn_url("/blob?sig=abc");
    let origin = MockServer::start(move |_| MockResponse::redirect(&cdn_url)).await;
    let (client, _) = test_client();

    // Direct request to the CDN host: refused before connecting.
    assert!(matches!(client.get_bytes(&cdn.cdn_url("/blob"), &[], 64).await, Err(NetError::HostNotAllowed(_))));
    assert_eq!(cdn.connections(), 0);

    // Via a redirect from its owner: followed.
    assert_eq!(client.get_bytes(&origin.url("/file"), &[], 64).await.unwrap(), b"from-cdn");
    assert_eq!(cdn.requests().len(), 1);
    assert_eq!(cdn.requests()[0].path, "/blob?sig=abc");
}

#[tokio::test]
async fn authorization_is_stripped_on_cross_host_redirect() {
    let target = MockServer::start(|_| MockResponse::ok("ok")).await;
    let (t_primary, t_cdn) = (target.url("/other-port"), target.cdn_url("/cdn"));
    let origin = MockServer::start(move |req| match req.path.as_str() {
        "/to-other-port" => MockResponse::redirect(&t_primary),
        "/to-cdn" => MockResponse::redirect(&t_cdn),
        "/same-host" => MockResponse::redirect("/landing"),
        _ => MockResponse::ok("landed"),
    })
    .await;
    let (client, _) = test_client();

    for path in ["/to-other-port", "/to-cdn"] {
        let rb = client
            .get(&origin.url(path))
            .unwrap()
            .header("authorization", "Bearer SECRET_KEY")
            .header("range", "bytes=10-");
        client.send(rb).await.unwrap();
    }
    let seen = target.requests();
    assert_eq!(seen.len(), 2);
    for r in &seen {
        assert_eq!(r.header("authorization"), None, "API key leaked to {}", r.path);
        assert_eq!(r.header("referer"), None, "Referer leaks the origin URL");
        // Resume must survive the hop to the CDN.
        assert_eq!(r.header("range"), Some("bytes=10-"));
    }

    // Same host: the key is kept (reqwest only strips it across hosts).
    let rb = client.get(&origin.url("/same-host")).unwrap().header("authorization", "Bearer SECRET_KEY");
    client.send(rb).await.unwrap();
    let landing = origin.requests().into_iter().find(|r| r.path == "/landing").unwrap();
    assert_eq!(landing.header("authorization"), Some("Bearer SECRET_KEY"));
}

#[tokio::test]
async fn offline_switched_on_mid_redirect_stops_the_chain() {
    let (client, offline) = test_client();
    let cdn = MockServer::start(|_| MockResponse::ok("x")).await;
    let cdn_url = cdn.cdn_url("/x");
    let flag = offline.clone();
    let origin = MockServer::start(move |_| {
        flag.set(true);
        MockResponse::redirect(&cdn_url)
    })
    .await;
    assert_eq!(client.get_bytes(&origin.url("/start"), &[], 16).await.unwrap_err(), NetError::Offline);
    assert_eq!(cdn.connections(), 0);
}

#[tokio::test]
async fn redirect_loops_are_capped() {
    let srv = MockServer::start(|req| {
        let n: u32 = req.path.trim_start_matches('/').parse().unwrap_or(0);
        MockResponse::redirect(&format!("/{}", n + 1))
    })
    .await;
    let (client, _) = test_client();
    assert!(matches!(client.get_bytes(&srv.url("/0"), &[], 16).await, Err(NetError::Transport(_))));
    assert_eq!(srv.requests().len(), crate::allow::MAX_REDIRECTS + 1);
}

#[tokio::test]
async fn local_client_is_loopback_only() {
    let other = MockServer::start(|_| MockResponse::ok("elsewhere")).await;
    let other_url = other.url("/");
    let srv = MockServer::start(move |req| match req.path.as_str() {
        "/redirect" => MockResponse::redirect(&other_url),
        _ => MockResponse::json(&serde_json::json!({"ok": true})),
    })
    .await;
    let local = LocalClient::new().unwrap();
    let base = format!("http://127.0.0.1:{}", srv.port());

    for (b, p) in [
        ("https://example.com", "/x"),
        ("http://localhost:1234", "/x"),
        ("http://10.0.0.1:80", "/x"),
        ("http://[::1]:1234", "/x"),
        ("http://127.0.0.1", "/x"),
        (base.as_str(), "//evil.com/x"),
        (base.as_str(), "https://civitai.com/"),
    ] {
        assert!(
            matches!(local.get(b, p), Err(NetError::HostNotAllowed(_))),
            "{b} + {p} must be refused"
        );
    }

    // Works (and works regardless of Offline mode: it has no offline flag at all).
    let v: serde_json::Value = local.get(&base, "/sdcpp/v1/capabilities").unwrap().send().await.unwrap().json().await.unwrap();
    assert_eq!(v["ok"], true);
    let resp = local.post_json(&base, "/sdcpp/v1/img_gen", &serde_json::json!({"a": 1})).unwrap().send().await.unwrap();
    assert!(resp.status().is_success());
    let posted = srv.requests().into_iter().find(|r| r.method == "POST").unwrap();
    assert_eq!(posted.body, br#"{"a":1}"#);

    // Never follows redirects (could point off-box).
    let resp = local.get(&base, "/redirect").unwrap().send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 302);
    assert_eq!(other.connections(), 0);
}
