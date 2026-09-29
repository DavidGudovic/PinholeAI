//! Browse paging: fetch `/models` pages by cursor, apply client-side filters,
//! and keep fetching until the grid page is full — at most
//! `max_extra_requests` extra requests per scroll, then `partial: true`
//! ("Load more"). Each CivitAI request asks for `api_limit` models; every
//! card they yield is returned (a page can hold more than `page_size`).

use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pinhole_net::NetError;

use crate::api::{CivitaiClient, ModelsPage};
use crate::cards::{card_or_hidden, CardContext, CatalogEnv};
use crate::filters::{BrowseQuery, CatalogFilters, Hidden};
use crate::view::BrowsePage;

/// Something that returns `/models` pages (the real client, a cache, or a fake in tests).
pub trait PageSource {
    fn fetch(
        &self,
        params: Vec<(String, String)>,
    ) -> impl Future<Output = Result<Arc<ModelsPage>, NetError>> + Send;
}

impl PageSource for CivitaiClient {
    fn fetch(
        &self,
        params: Vec<(String, String)>,
    ) -> impl Future<Output = Result<Arc<ModelsPage>, NetError>> + Send {
        let client = self.clone();
        async move { client.search(&params).await.map(Arc::new) }
    }
}

/// Why [`browse`] stopped before filling a page.
#[derive(Debug)]
pub enum BrowseError {
    Net(NetError),
    /// `keep_going` said no: a newer Browse request replaced this one.
    Superseded,
}

impl From<NetError> for BrowseError {
    fn from(e: NetError) -> Self {
        BrowseError::Net(e)
    }
}

/// One Browse page. `base_models` = every CivitAI base model the registry can
/// run (sent as `baseModels` when "Works with Pinhole" is on). `keep_going` is
/// asked before each extra request; `false` (the user already changed the
/// filters) stops without spending more CivitAI requests.
pub async fn browse<S: PageSource>(
    source: &S,
    filters: &CatalogFilters,
    query: &BrowseQuery,
    base_models: &[String],
    env: &(dyn CatalogEnv + Sync),
    now: DateTime<Utc>,
    keep_going: impl Fn() -> bool,
) -> Result<BrowsePage, BrowseError> {
    let ctx = CardContext {
        filters,
        query,
        now,
    };
    let want = filters.page_size as usize;
    let max_requests = 1 + filters.max_extra_requests as usize;
    let mut cursor = query.cursor.clone();
    let mut out = BrowsePage {
        items: Vec::new(),
        next_cursor: None,
        offline: false,
        partial: false,
        checked: 0,
        hidden_by_content: 0,
        hidden_by_filters: 0,
        hidden_by_size: 0,
    };
    let mut seen = HashSet::new();
    let mut requests = 0usize;
    loop {
        if requests > 0 && !keep_going() {
            return Err(BrowseError::Superseded);
        }
        let params = filters.query_params(query, base_models, cursor.as_deref());
        let page = source.fetch(params).await?;
        requests += 1;
        for m in &page.items {
            if !seen.insert(m.id) {
                continue;
            }
            out.checked += 1;
            match card_or_hidden(&ctx, env, m) {
                Ok(card) => out.items.push(card),
                Err(Hidden::Content) => out.hidden_by_content += 1,
                Err(Hidden::Other) => out.hidden_by_filters += 1,
                Err(Hidden::TooBig) => out.hidden_by_size += 1,
            }
        }
        let next = page.next_cursor();
        // A cursor that doesn't move would loop forever.
        let stuck = next.is_some() && next == cursor;
        cursor = if stuck { None } else { next };
        if out.items.len() >= want || cursor.is_none() {
            out.next_cursor = cursor;
            return Ok(out);
        }
        if requests >= max_requests {
            out.next_cursor = cursor;
            out.partial = true;
            return Ok(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::api::Model;
    use crate::cards::tests::FakeEnv;
    use crate::cards::CatalogEnv;
    use crate::filters::tests::{filters, now};
    use crate::filters::ContentMode;

    struct FakeSource {
        pages: Vec<ModelsPage>,
        calls: Mutex<Vec<Vec<(String, String)>>>,
    }

    impl PageSource for FakeSource {
        fn fetch(
            &self,
            params: Vec<(String, String)>,
        ) -> impl Future<Output = Result<Arc<ModelsPage>, NetError>> + Send {
            let mut calls = self.calls.lock().unwrap();
            let idx = calls.len();
            calls.push(params);
            let page = self
                .pages
                .get(idx)
                .cloned()
                .map(Arc::new)
                .ok_or(NetError::Status(429));
            async move { page }
        }
    }

    fn go() -> bool {
        true
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
            items: ids
                .map(|i| model(i, nsfw_every > 0 && i % nsfw_every == 0))
                .collect(),
            metadata: crate::api::PageMetadata {
                next_cursor: cursor.map(Into::into),
                ..Default::default()
            },
        }
    }

    fn env() -> FakeEnv {
        FakeEnv {
            vram_gb: 12.0,
            installed_versions: vec![],
        }
    }

    fn cursor_of(call: &[(String, String)]) -> Option<&str> {
        call.iter()
            .find(|(k, _)| k == "cursor")
            .map(|(_, v)| v.as_str())
    }

    #[tokio::test]
    async fn full_page_in_one_request() {
        let src = FakeSource {
            pages: vec![page(1..25, 0, Some("c1"))],
            calls: Mutex::new(vec![]),
        };
        let out = browse(
            &src,
            &filters(),
            &BrowseQuery::default(),
            &["SDXL 1.0".into()],
            &env(),
            now(),
            go,
        )
        .await
        .unwrap();
        assert_eq!(out.items.len(), 24);
        assert_eq!(out.next_cursor.as_deref(), Some("c1"));
        assert!(!out.partial);
        assert_eq!(src.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn nsfw_tag_fetches_more_pages_until_full() {
        // 1 in 4 models is NSFW → 6 per page → 4 pages for 24 cards.
        let pages = (0..6)
            .map(|p| page(p * 24 + 1..p * 24 + 25, 4, Some(&format!("c{}", p + 1))))
            .collect();
        let src = FakeSource {
            pages,
            calls: Mutex::new(vec![]),
        };
        let q = BrowseQuery {
            content: ContentMode::All,
            tags: vec!["nsfw".into()],
            ..Default::default()
        };
        let out = browse(&src, &filters(), &q, &[], &env(), now(), go)
            .await
            .unwrap();
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
        let pages = (0..10)
            .map(|p| page(p * 24 + 1..p * 24 + 25, 24, Some(&format!("c{}", p + 1))))
            .collect();
        let src = FakeSource {
            pages,
            calls: Mutex::new(vec![]),
        };
        let q = BrowseQuery {
            content: ContentMode::All,
            tags: vec!["nsfw".into()],
            cursor: Some("start".into()),
            ..Default::default()
        };
        let out = browse(&src, &filters(), &q, &[], &env(), now(), go)
            .await
            .unwrap();
        assert_eq!(src.calls.lock().unwrap().len(), 6);
        assert_eq!(cursor_of(&src.calls.lock().unwrap()[0]), Some("start"));
        assert_eq!(out.items.len(), 6);
        assert!(out.partial);
        assert_eq!(
            out.next_cursor.as_deref(),
            Some("c6"),
            "Load more continues where we stopped"
        );
    }

    #[tokio::test]
    async fn empty_and_short_answers_keep_paging() {
        // CivitAI can answer an empty or short page that still has a cursor (text search,
        // filters applied after its own paging): that is not the end of the list.
        let pages = vec![
            page(0..0, 0, Some("c1")),
            page(1..6, 0, Some("c2")),
            page(6..40, 0, Some("c3")),
        ];
        let src = FakeSource {
            pages,
            calls: Mutex::new(vec![]),
        };
        let out = browse(
            &src,
            &filters(),
            &BrowseQuery::default(),
            &[],
            &env(),
            now(),
            go,
        )
        .await
        .unwrap();
        assert_eq!(src.calls.lock().unwrap().len(), 3);
        assert_eq!(out.items.len(), 39);
        assert_eq!(out.next_cursor.as_deref(), Some("c3"));
        assert!(!out.partial);
    }

    #[tokio::test]
    async fn stops_at_the_end_and_dedupes() {
        let src = FakeSource {
            pages: vec![page(1..11, 0, Some("c1")), page(5..15, 0, None)],
            calls: Mutex::new(vec![]),
        };
        let out = browse(
            &src,
            &filters(),
            &BrowseQuery::default(),
            &[],
            &env(),
            now(),
            go,
        )
        .await
        .unwrap();
        assert_eq!(out.items.len(), 14);
        assert_eq!(out.next_cursor, None);
        assert!(!out.partial);
    }

    #[tokio::test]
    async fn errors_propagate() {
        let src = FakeSource {
            pages: vec![],
            calls: Mutex::new(vec![]),
        };
        let err = browse(
            &src,
            &filters(),
            &BrowseQuery::default(),
            &[],
            &env(),
            now(),
            go,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, BrowseError::Net(NetError::Status(429))));
    }

    #[tokio::test]
    async fn counts_what_it_hides() {
        // 1 in 4 is NSFW (hidden by Safe mode); ids 5.. are LoRAs in the Models view.
        let mut p = page(1..9, 4, None);
        for m in p.items.iter_mut().filter(|m| m.id >= 5) {
            m.kind = "LORA".into();
        }
        let src = FakeSource {
            pages: vec![p],
            calls: Mutex::new(vec![]),
        };
        let out = browse(
            &src,
            &filters(),
            &BrowseQuery::default(),
            &[],
            &env(),
            now(),
            go,
        )
        .await
        .unwrap();
        assert_eq!(
            (
                out.checked,
                out.hidden_by_content,
                out.hidden_by_filters,
                out.items.len()
            ),
            (8, 1, 4, 3)
        );
    }

    #[tokio::test]
    async fn stops_when_superseded() {
        let pages = (0..6)
            .map(|p| page(p * 24 + 1..p * 24 + 25, 4, Some(&format!("c{}", p + 1))))
            .collect();
        let src = FakeSource {
            pages,
            calls: Mutex::new(vec![]),
        };
        let q = BrowseQuery {
            content: ContentMode::All,
            tags: vec!["nsfw".into()],
            ..Default::default()
        };
        let err = browse(&src, &filters(), &q, &[], &env(), now(), || false)
            .await
            .unwrap_err();
        assert!(matches!(err, BrowseError::Superseded));
        assert_eq!(
            src.calls.lock().unwrap().len(),
            1,
            "no extra CivitAI requests for a stale query"
        );
    }

    #[tokio::test]
    async fn live_sample_fills_a_page_in_one_request() {
        let src = FakeSource {
            pages: vec![crate::safe::tests::live("alltime")],
            calls: Mutex::new(vec![]),
        };
        let out = browse(
            &src,
            &filters(),
            &BrowseQuery {
                compatible_only: false,
                ..Default::default()
            },
            &[],
            &env(),
            now(),
            go,
        )
        .await
        .unwrap();
        assert_eq!(src.calls.lock().unwrap().len(), 1);
        assert_eq!(out.checked, 100);
        assert!(out.items.len() >= 24, "{}", out.items.len());
        assert_eq!(out.hidden_by_content, 48);
        assert!(!out.partial);
        let names: Vec<&str> = out.items.iter().take(6).map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Realistic Vision V6.0 B1",
                "DreamShaper",
                "Juggernaut XL",
                "majicMIX realistic 麦橘写实",
                "Pony Diffusion V6 XL",
                "epiCRealism"
            ]
        );
        // Every Safe preview is a small PG image.
        let page = crate::safe::tests::live("alltime");
        for c in &out.items {
            let Some(url) = c.preview_url.as_deref() else {
                continue;
            };
            assert!(url.contains("/width=450,optimized=true/"), "{url}");
            let name = url.rsplit('/').next().unwrap();
            let m = page.items.iter().find(|m| m.id == c.model_id).unwrap();
            let img = m
                .model_versions
                .iter()
                .flat_map(|v| v.images.iter())
                .find(|i| i.url.ends_with(&format!("/{name}")))
                .unwrap();
            assert_eq!(img.nsfw_level, Some(1), "{}", c.name);
        }
    }

    /// Every base model is one Pinhole runs (the query already asked CivitAI for those).
    struct AllCompatible;

    impl CatalogEnv for AllCompatible {
        fn family_for(&self, base: &str, _sha: Option<&str>) -> Option<crate::cards::FamilyInfo> {
            Some(crate::cards::FamilyInfo {
                id: base.to_lowercase(),
                label: base.into(),
                license_note: None,
                diffusion_only: false,
            })
        }
        fn vram_for(
            &self,
            _family: &str,
            _bytes: u64,
        ) -> Option<(
            pinhole_registry::vram::VramNeed,
            pinhole_registry::vram::Fit,
        )> {
            None
        }
        fn is_installed(&self, _version: u64, _sha: Option<&str>) -> bool {
            false
        }
    }

    /// The opening Browse page (Models · Safe mode on · Free · Works with Pinhole · Most
    /// downloaded · All time) on live data: mainstream models, normal previews.
    #[tokio::test]
    async fn live_default_safe_page() {
        let page = crate::safe::tests::live("alltime");
        let src = FakeSource {
            pages: vec![page],
            calls: Mutex::new(vec![]),
        };
        let q = BrowseQuery::default();
        let out = browse(
            &src,
            &filters(),
            &q,
            &["SDXL 1.0".into()],
            &AllCompatible,
            now(),
            go,
        )
        .await
        .unwrap();
        let names: Vec<&str> = out.items.iter().take(24).map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Realistic Vision V6.0 B1",
                "DreamShaper",
                "Juggernaut XL",
                "majicMIX realistic 麦橘写实",
                "Pony Diffusion V6 XL",
                "epiCRealism",
                "CyberRealistic Pony",
                "CyberRealistic",
                "RealVisXL V5.0",
                "epiCRealism XL",
                "ReV Animated",
                "MeinaMix",
                "Beautiful Realistic Asians",
                "万象熔炉 | Anything XL",
                "Nova Anime XL",
                "Counterfeit-V3.0",
                "SD XL",
                "epiCPhotoGasm",
                "FLUX",
                "AbsoluteReality",
                "ControlNetXL (CNXL)",
                "NoobAI-XL (NAI-XL)",
                "AnyLoRA - Checkpoint",
                "Cetus-Mix",
            ]
        );
        assert!(out.items.iter().all(|c| !c.model_nsfw && !c.preview_nsfw));
        assert!(
            out.items.iter().all(|c| c.preview_url.is_some()),
            "every kept model has a PG preview"
        );
        let p = &src.calls.lock().unwrap()[0];
        assert!(
            p.contains(&("nsfw".to_string(), "true".to_string())),
            "ratings of every sample image"
        );
    }

    #[tokio::test]
    async fn fixture_page_through_all_filters() {
        let fixture: ModelsPage =
            serde_json::from_str(include_str!("../tests/fixtures/models_page.json")).unwrap();
        let mut last = fixture.clone();
        last.metadata.next_cursor = None;
        last.metadata.next_page = None;
        let src = FakeSource {
            pages: vec![last],
            calls: Mutex::new(vec![]),
        };
        let out = browse(
            &src,
            &filters(),
            &BrowseQuery::default(),
            &[],
            &env(),
            now(),
            go,
        )
        .await
        .unwrap();
        let ids: Vec<u64> = out.items.iter().map(|c| c.model_id).collect();
        // Safe + Free + Checkpoint + compatible: SD 3.5 dropped (incompatible),
        // the LoRA dropped (kind), pickle/pending ones kept but blocked.
        assert_eq!(ids, vec![139562, 5000, 618692, 777001, 800002]);
    }
}
