//! Browse paging: fetch `/models` pages by cursor, apply client-side filters,
//! and keep fetching until the grid page is full — at most
//! `max_extra_requests` extra requests per scroll, then `partial: true`
//! ("Load more").

use std::collections::HashSet;
use std::future::Future;

use chrono::{DateTime, Utc};
use pinhole_net::NetError;

use crate::api::{CivitaiClient, ModelsPage};
use crate::cards::{build_card, CardContext, CatalogEnv};
use crate::filters::{BrowseQuery, CatalogFilters};
use crate::view::BrowsePage;

/// Something that returns `/models` pages (the real client, or a fake in tests).
pub trait PageSource {
    fn fetch(&self, params: Vec<(String, String)>) -> impl Future<Output = Result<ModelsPage, NetError>> + Send;
}

impl PageSource for CivitaiClient {
    fn fetch(&self, params: Vec<(String, String)>) -> impl Future<Output = Result<ModelsPage, NetError>> + Send {
        let client = self.clone();
        async move { client.search(&params).await }
    }
}

/// One Browse page. `base_models` = every CivitAI base model the registry can
/// run (sent as `baseModels` when "Works with Pinhole" is on).
pub async fn browse<S: PageSource>(
    source: &S,
    filters: &CatalogFilters,
    query: &BrowseQuery,
    base_models: &[String],
    env: &(dyn CatalogEnv + Sync),
    now: DateTime<Utc>,
) -> Result<BrowsePage, NetError> {
    let ctx = CardContext { filters, query, now };
    let want = filters.page_size as usize;
    let max_requests = 1 + filters.max_extra_requests as usize;
    let mut cursor = query.cursor.clone();
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut requests = 0usize;
    loop {
        let params = filters.query_params(query, base_models, cursor.as_deref());
        let page = source.fetch(params).await?;
        requests += 1;
        for m in &page.items {
            if seen.insert(m.id) {
                if let Some(card) = build_card(&ctx, env, m) {
                    items.push(card);
                }
            }
        }
        let next = page.next_cursor();
        // A cursor that doesn't move would loop forever.
        let stuck = next.is_some() && next == cursor;
        cursor = if stuck { None } else { next };
        if items.len() >= want || cursor.is_none() {
            return Ok(BrowsePage { items, next_cursor: cursor, offline: false, partial: false });
        }
        if requests >= max_requests {
            return Ok(BrowsePage { items, next_cursor: cursor, offline: false, partial: true });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::api::Model;
    use crate::cards::tests::FakeEnv;
    use crate::filters::tests::{filters, now};
    use crate::filters::ContentMode;

    struct FakeSource {
        pages: Vec<ModelsPage>,
        calls: Mutex<Vec<Vec<(String, String)>>>,
    }

    impl PageSource for FakeSource {
        fn fetch(&self, params: Vec<(String, String)>) -> impl Future<Output = Result<ModelsPage, NetError>> + Send {
            let mut calls = self.calls.lock().unwrap();
            let idx = calls.len();
            calls.push(params);
            let page = self.pages.get(idx).cloned().ok_or(NetError::Status(429));
            async move { page }
        }
    }

    fn model(id: u64, nsfw: bool) -> Model {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": format!("m{id}"), "type": "Checkpoint", "nsfw": nsfw,
            "allowCommercialUse": ["Image"], "tags": ["anime"],
            "modelVersions": [{ "id": id * 10, "name": "v1", "baseModel": "SDXL 1.0", "availability": "Public",
                "files": [{ "name": "m.safetensors", "sizeKB": 100.0, "type": "Model", "primary": true,
                    "pickleScanResult": "Success", "virusScanResult": "Success",
                    "metadata": { "format": "SafeTensor" }, "hashes": {}, "downloadUrl": "https://civitai.com/api/download/models/1" }],
                "images": [] }]
        }))
        .unwrap()
    }

    fn page(ids: std::ops::Range<u64>, nsfw_every: u64, cursor: Option<&str>) -> ModelsPage {
        ModelsPage {
            items: ids.map(|i| model(i, nsfw_every > 0 && i % nsfw_every == 0)).collect(),
            metadata: crate::api::PageMetadata { next_cursor: cursor.map(Into::into), ..Default::default() },
        }
    }

    fn env() -> FakeEnv {
        FakeEnv { vram_gb: 12.0, installed_versions: vec![] }
    }

    fn cursor_of(call: &[(String, String)]) -> Option<&str> {
        call.iter().find(|(k, _)| k == "cursor").map(|(_, v)| v.as_str())
    }

    #[tokio::test]
    async fn full_page_in_one_request() {
        let src = FakeSource { pages: vec![page(1..25, 0, Some("c1"))], calls: Mutex::new(vec![]) };
        let out = browse(&src, &filters(), &BrowseQuery::default(), &["SDXL 1.0".into()], &env(), now()).await.unwrap();
        assert_eq!(out.items.len(), 24);
        assert_eq!(out.next_cursor.as_deref(), Some("c1"));
        assert!(!out.partial);
        assert_eq!(src.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn only_18plus_fetches_more_pages_until_full() {
        // 1 in 4 models is NSFW → 6 per page → 4 pages for 24 cards.
        let pages = (0..6).map(|p| page(p * 24 + 1..p * 24 + 25, 4, Some(&format!("c{}", p + 1)))).collect();
        let src = FakeSource { pages, calls: Mutex::new(vec![]) };
        let q = BrowseQuery { content: ContentMode::Only18Plus, ..Default::default() };
        let out = browse(&src, &filters(), &q, &[], &env(), now()).await.unwrap();
        assert_eq!(out.items.len(), 24);
        assert!(out.items.iter().all(|c| c.model_nsfw));
        assert!(!out.partial);
        assert_eq!(out.next_cursor.as_deref(), Some("c4"));
        let calls = src.calls.lock().unwrap();
        assert_eq!(calls.len(), 4);
        assert_eq!(cursor_of(&calls[0]), None);
        assert_eq!(cursor_of(&calls[1]), Some("c1"));
        assert_eq!(cursor_of(&calls[3]), Some("c3"));
    }

    #[tokio::test]
    async fn caps_extra_requests_and_reports_partial() {
        // 1 in 24 is NSFW → 1 per page; after 1 + 5 requests: 6 cards, partial.
        let pages = (0..10).map(|p| page(p * 24 + 1..p * 24 + 25, 24, Some(&format!("c{}", p + 1)))).collect();
        let src = FakeSource { pages, calls: Mutex::new(vec![]) };
        let q = BrowseQuery { content: ContentMode::Only18Plus, cursor: Some("start".into()), ..Default::default() };
        let out = browse(&src, &filters(), &q, &[], &env(), now()).await.unwrap();
        assert_eq!(src.calls.lock().unwrap().len(), 6);
        assert_eq!(cursor_of(&src.calls.lock().unwrap()[0]), Some("start"));
        assert_eq!(out.items.len(), 6);
        assert!(out.partial);
        assert_eq!(out.next_cursor.as_deref(), Some("c6"), "Load more continues where we stopped");
    }

    #[tokio::test]
    async fn stops_at_the_end_and_dedupes() {
        let src = FakeSource { pages: vec![page(1..11, 0, Some("c1")), page(5..15, 0, None)], calls: Mutex::new(vec![]) };
        let out = browse(&src, &filters(), &BrowseQuery::default(), &[], &env(), now()).await.unwrap();
        assert_eq!(out.items.len(), 14);
        assert_eq!(out.next_cursor, None);
        assert!(!out.partial);
    }

    #[tokio::test]
    async fn errors_propagate() {
        let src = FakeSource { pages: vec![], calls: Mutex::new(vec![]) };
        let err = browse(&src, &filters(), &BrowseQuery::default(), &[], &env(), now()).await.unwrap_err();
        assert!(matches!(err, NetError::Status(429)));
    }

    #[tokio::test]
    async fn fixture_page_through_all_filters() {
        let fixture: ModelsPage = serde_json::from_str(include_str!("../tests/fixtures/models_page.json")).unwrap();
        let mut last = fixture.clone();
        last.metadata.next_cursor = None;
        last.metadata.next_page = None;
        let src = FakeSource { pages: vec![last], calls: Mutex::new(vec![]) };
        let out = browse(&src, &filters(), &BrowseQuery::default(), &[], &env(), now()).await.unwrap();
        let ids: Vec<u64> = out.items.iter().map(|c| c.model_id).collect();
        // Safe + Free + Checkpoint + compatible: SD 3.5 dropped (incompatible),
        // the LoRA dropped (kind), pickle/pending ones kept but blocked.
        assert_eq!(ids, vec![139562, 5000, 618692, 777001, 800002]);
    }
}
