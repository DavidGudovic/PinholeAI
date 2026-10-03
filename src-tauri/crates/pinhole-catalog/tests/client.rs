//! CivitAI client against a local mock server (no internet).

use pinhole_catalog::api::CivitaiClient;
use pinhole_catalog::browse::browse;
use pinhole_catalog::cards::{CatalogEnv, FamilyInfo};
use pinhole_catalog::filters::{BrowseQuery, CatalogFilters};
use pinhole_net::testutil::{MockResponse, MockServer};
use pinhole_net::{HttpClient, NetError, OfflineFlag};
use pinhole_registry::vram::{Fit, VramNeed};

const PAGE: &str = include_str!("fixtures/models_page.json");
const VERSION: &str = include_str!("fixtures/model_version.json");
const LORA: &str = include_str!("fixtures/by_hash_lora.json");

async fn server() -> MockServer {
    MockServer::start(|req| {
        let path = req.path.as_str();
        if path.starts_with("/api/v1/models?") {
            let mut page: serde_json::Value = serde_json::from_str(PAGE).unwrap();
            if path.contains("cursor=") {
                page["metadata"] = serde_json::json!({});
            }
            MockResponse::json(&page)
        } else if path == "/api/v1/model-versions/1759168" {
            MockResponse::ok(VERSION.as_bytes().to_vec()).header("content-type", "application/json")
        } else if path == "/api/v1/model-versions/by-hash/0F4168490E" {
            MockResponse::ok(LORA.as_bytes().to_vec()).header("content-type", "application/json")
        } else if path.starts_with("/api/download/models/1") {
            if req.header("authorization").is_some() {
                MockResponse::new(206, b"x".to_vec())
            } else {
                MockResponse::status(401)
            }
        } else {
            MockResponse::status(404)
        }
    })
    .await
}

fn client(srv: &MockServer, offline: bool) -> CivitaiClient {
    let http = HttpClient::new_for_tests(OfflineFlag::new(offline), true).unwrap();
    CivitaiClient::with_base(http, Some("secret-key".into()), srv.url("/api/v1"))
}

struct AnyEnv;
impl CatalogEnv for AnyEnv {
    fn family_for(&self, base_model: &str, _: Option<&str>) -> Option<FamilyInfo> {
        (base_model != "SD 3.5 Large").then(|| FamilyInfo {
            id: "sdxl".into(),
            label: "SDXL".into(),
            license_note: None,
            diffusion_only: false,
            takes_reference: false,
        })
    }
    fn vram_for(&self, _: &str, _: u64) -> Option<(VramNeed, Fit)> {
        None
    }
    fn is_installed(&self, _: u64, _: Option<&str>) -> bool {
        false
    }
}

fn filters() -> CatalogFilters {
    CatalogFilters::from_yaml(include_str!("../../../../config/catalog-filters.yaml")).unwrap()
}

#[tokio::test]
async fn search_sends_query_without_key() {
    let srv = server().await;
    let c = client(&srv, false);
    let f = filters();
    let params = f.query_params(
        &BrowseQuery::default(),
        &["SDXL 1.0".into(), "Pony".into()],
        None,
    );
    let page = c.search(&params).await.unwrap();
    assert_eq!(page.items.len(), 7);
    let reqs = srv.requests();
    let r = &reqs[0];
    assert!(r.path.contains("sort=Most%20Downloaded"), "{}", r.path);
    assert!(r.path.contains("limit=50"), "{}", r.path);
    assert!(
        r.path.contains("baseModels=SDXL%201.0&baseModels=Pony"),
        "{}",
        r.path
    );
    assert!(
        r.path.contains("nsfw=true"),
        "Safe mode filters client-side"
    );
    assert!(!r.path.contains("page="));
    assert!(r.header("authorization").is_none(), "browsing is anonymous");
    assert!(!r.path.contains("secret-key"));
}

#[tokio::test]
async fn browse_pages_by_cursor() {
    let srv = server().await;
    let c = client(&srv, false);
    let f = filters();
    let now = chrono::Utc::now();
    let out = browse(&c, &f, &BrowseQuery::default(), &[], &AnyEnv, now, || true)
        .await
        .unwrap();
    // Fixture: 5 cards per page after filters; page 2 has no cursor → stop.
    assert_eq!(srv.requests().len(), 2);
    assert!(srv.requests()[1].path.contains("cursor=2%7C1718000000000"));
    assert_eq!(out.next_cursor, None);
    assert!(!out.partial);
    assert_eq!(
        out.items.len(),
        5,
        "second page repeats the same ids → deduped"
    );
}

#[tokio::test]
async fn versions_hashes_and_errors() {
    let srv = server().await;
    let c = client(&srv, false);
    let v = c.model_version(1759168).await.unwrap();
    assert_eq!(v.model.unwrap().name, "Juggernaut XL");
    let l = c.by_hash("0f4168490e").await.unwrap().unwrap();
    assert_eq!(l.trained_words, ["watercolor"]);
    assert!(
        c.by_hash("ABCDEF0123").await.unwrap().is_none(),
        "404 → unknown"
    );
    assert!(c.by_hash("not-hex!").await.unwrap().is_none());
    assert!(matches!(
        c.model_version(5).await,
        Err(NetError::Status(404))
    ));
    assert!(
        srv.requests()
            .iter()
            .all(|r| r.header("authorization").is_none()),
        "key never sent to non-CivitAI hosts"
    );
}

#[tokio::test]
async fn download_probe_reports_401() {
    let srv = server().await;
    let c = client(&srv, false);
    // The mock is not civitai.com, so no key is attached → 401.
    assert_eq!(
        c.probe_download(&srv.url("/api/download/models/1759168"))
            .await
            .unwrap(),
        401
    );
    let r = srv.requests().pop().unwrap();
    assert_eq!(r.header("range"), Some("bytes=0-0"));
}

#[tokio::test]
async fn offline_mode_blocks_before_connecting() {
    let srv = server().await;
    let c = client(&srv, true);
    assert!(matches!(c.search(&[]).await, Err(NetError::Offline)));
    assert!(matches!(
        c.model_version(1759168).await,
        Err(NetError::Offline)
    ));
    assert!(matches!(
        c.probe_download(&srv.url("/api/download/models/1")).await,
        Err(NetError::Offline)
    ));
    assert_eq!(srv.connections(), 0);
}
