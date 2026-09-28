//! RAM-only cache of recent CivitAI `/models` answers (never written to disk),
//! so switching Look, Price or Content back and forth — or scrolling back to a
//! page — doesn't ask CivitAI again. Entries expire after the configured time
//! and the oldest one goes first when the cache is full.
//!
//! Cached pages are compacted: image URLs that can never become a card
//! preview are dropped (their ratings stay, "Safe only" needs them), and only
//! the SHA-256 of each file is kept.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use pinhole_net::NetError;

use crate::api::ModelsPage;
use crate::browse::PageSource;

/// Image URLs kept per version: the first few, plus the first few rated PG.
const KEEP_FIRST_IMAGES: usize = 4;
const KEEP_PG_IMAGES: usize = 2;

struct Entry {
    key: String,
    at: Instant,
    page: Arc<ModelsPage>,
}

pub struct PageCache {
    entries: Mutex<VecDeque<Entry>>,
    ttl: Duration,
    capacity: usize,
}

impl PageCache {
    /// `capacity == 0` or a zero `ttl` turns the cache off.
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self { entries: Mutex::new(VecDeque::new()), ttl, capacity }
    }

    fn lock(&self) -> MutexGuard<'_, VecDeque<Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn enabled(&self) -> bool {
        self.capacity > 0 && !self.ttl.is_zero()
    }

    pub fn get(&self, key: &str) -> Option<Arc<ModelsPage>> {
        self.get_at(key, Instant::now())
    }

    pub fn get_at(&self, key: &str, now: Instant) -> Option<Arc<ModelsPage>> {
        let mut entries = self.lock();
        entries.retain(|e| now.saturating_duration_since(e.at) < self.ttl);
        let pos = entries.iter().position(|e| e.key == key)?;
        let entry = entries.remove(pos)?;
        let page = entry.page.clone();
        entries.push_back(entry); // most recently used last
        Some(page)
    }

    pub fn put(&self, key: String, page: Arc<ModelsPage>) {
        self.put_at(key, page, Instant::now());
    }

    pub fn put_at(&self, key: String, page: Arc<ModelsPage>, now: Instant) {
        if !self.enabled() {
            return;
        }
        let mut entries = self.lock();
        entries.retain(|e| e.key != key && now.saturating_duration_since(e.at) < self.ttl);
        while entries.len() >= self.capacity {
            entries.pop_front();
        }
        entries.push_back(Entry { key, at: now, page });
    }

    /// Drop expired entries (called on a timer so idle RAM goes back down).
    pub fn purge_expired(&self) {
        let now = Instant::now();
        self.lock().retain(|e| now.saturating_duration_since(e.at) < self.ttl);
    }

    pub fn clear(&self) {
        self.lock().clear();
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }
}

/// Cache key: the exact query (order kept, values as sent).
pub fn cache_key(params: &[(String, String)]) -> String {
    let mut key = String::new();
    for (k, v) in params {
        key.push_str(k);
        key.push('=');
        key.push_str(v);
        key.push('\n');
    }
    key
}

/// Shrink a page before caching it (see the module docs).
pub fn compact(page: &mut ModelsPage) {
    for v in page.items.iter_mut().flat_map(|m| m.model_versions.iter_mut()) {
        let mut pg_kept = 0;
        for (i, img) in v.images.iter_mut().enumerate() {
            let pg = img.nsfw_level == Some(crate::safe::LEVEL_PG);
            if i < KEEP_FIRST_IMAGES {
                pg_kept += usize::from(pg);
            } else if pg && pg_kept < KEEP_PG_IMAGES {
                pg_kept += 1;
            } else {
                img.url = String::new();
                img.kind = None;
            }
        }
        for f in &mut v.files {
            f.hashes.retain(|k, _| k.eq_ignore_ascii_case("SHA256"));
        }
    }
}

/// A [`PageSource`] that answers from the cache when it can.
pub struct CachedSource<'a, S> {
    pub inner: &'a S,
    pub cache: &'a PageCache,
}

impl<S: PageSource + Sync> PageSource for CachedSource<'_, S> {
    async fn fetch(&self, params: Vec<(String, String)>) -> Result<Arc<ModelsPage>, NetError> {
        let key = cache_key(&params);
        if let Some(hit) = self.cache.get(&key) {
            return Ok(hit);
        }
        let page = self.inner.fetch(params).await?;
        if !self.cache.enabled() {
            return Ok(page);
        }
        let mut owned = Arc::unwrap_or_clone(page);
        compact(&mut owned);
        let page = Arc::new(owned);
        self.cache.put(key, page.clone());
        Ok(page)
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn page(id: u64) -> Arc<ModelsPage> {
        Arc::new(serde_json::from_value(serde_json::json!({ "items": [{ "id": id, "name": "m" }] })).unwrap())
    }

    #[test]
    fn expires_and_evicts_oldest() {
        let c = PageCache::new(Duration::from_secs(60), 2);
        let t0 = Instant::now();
        c.put_at("a".into(), page(1), t0);
        c.put_at("b".into(), page(2), t0);
        assert_eq!(c.get_at("a", t0).unwrap().items[0].id, 1, "a is now the most recent");
        c.put_at("c".into(), page(3), t0);
        assert!(c.get_at("b", t0).is_none(), "b was least recently used");
        assert!(c.get_at("a", t0).is_some());
        assert!(c.get_at("a", t0 + Duration::from_secs(61)).is_none(), "expired");
        assert_eq!(c.len(), 0, "expired entries are dropped");
        let off = PageCache::new(Duration::from_secs(60), 0);
        off.put("a".into(), page(1));
        assert!(off.is_empty());
    }

    #[test]
    fn keys_keep_order_and_repeats() {
        let p = |v: &[(&str, &str)]| v.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
        assert_ne!(cache_key(&p(&[("baseModels", "A"), ("baseModels", "B")])), cache_key(&p(&[("baseModels", "A")])));
        assert_eq!(cache_key(&p(&[("a", "1")])), "a=1\n");
    }

    #[test]
    fn compaction_keeps_ratings_and_preview_candidates() {
        let levels = [8, 16, 4, 2, 8, 1, 4, 1, 1, 16];
        let images: Vec<_> = levels.iter().enumerate().map(|(i, l)| serde_json::json!({ "url": format!("u{i}"), "nsfwLevel": l })).collect();
        let mut p: ModelsPage = serde_json::from_value(serde_json::json!({ "items": [{ "id": 1, "modelVersions": [{ "id": 2, "images": images,
            "files": [{ "name": "f", "hashes": { "SHA256": "AB", "AutoV2": "CD", "BLAKE3": "EF" } }] }] }] }))
        .unwrap();
        compact(&mut p);
        let v = &p.items[0].model_versions[0];
        let urls: Vec<&str> = v.images.iter().map(|i| i.url.as_str()).collect();
        assert_eq!(urls, ["u0", "u1", "u2", "u3", "", "u5", "", "u7", "", ""]);
        assert_eq!(v.images.iter().filter_map(|i| i.nsfw_level).collect::<Vec<_>>(), levels, "ratings stay");
        assert_eq!(v.files[0].hashes.len(), 1);
        assert_eq!(v.files[0].sha256(), None, "not 64 hex chars in this fixture, but the key survives");
        assert!(v.files[0].hash("sha256").is_some());
    }

    struct Counting(AtomicUsize);

    impl PageSource for Counting {
        fn fetch(&self, _params: Vec<(String, String)>) -> impl Future<Output = Result<Arc<ModelsPage>, NetError>> + Send {
            let n = self.0.fetch_add(1, Ordering::SeqCst) as u64;
            async move { Ok(page(n)) }
        }
    }

    #[tokio::test]
    async fn cached_source_hits() {
        let inner = Counting(AtomicUsize::new(0));
        let cache = PageCache::new(Duration::from_secs(60), 4);
        let src = CachedSource { inner: &inner, cache: &cache };
        let q = |c: &str| vec![("cursor".to_string(), c.to_string())];
        assert_eq!(src.fetch(q("a")).await.unwrap().items[0].id, 0);
        assert_eq!(src.fetch(q("a")).await.unwrap().items[0].id, 0, "from the cache");
        assert_eq!(src.fetch(q("b")).await.unwrap().items[0].id, 1);
        assert_eq!(inner.0.load(Ordering::SeqCst), 2);
    }
}
