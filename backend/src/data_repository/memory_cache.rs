//! Volatile, non-persistent Preview cache (data-model.md; FR-011/FR-013).
//! Keyed by the content-equivalence key from
//! `domain::pipeline_snapshot::compute_equivalence_key`. Nothing here
//! survives a service restart or an explicit clear — losing an entry is
//! always a recoverable cache miss for the caller to recompute, never a
//! data-loss condition (FR-011), and this cache is never consulted by
//! official Run persistence (FR-013, Constitution Principle III).

use std::collections::HashMap;
use std::sync::Mutex;

use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct PreviewCacheEntry {
    pub output_content_identity: String,
    pub producing_node_id: String,
    pub pipeline_snapshot_id: Uuid,
    pub image_asset_id: Uuid,
    pub duration_ms: u64,
}

#[derive(Default)]
pub struct PreviewCache {
    entries: Mutex<HashMap<String, PreviewCacheEntry>>,
}

impl PreviewCache {
    pub fn get(&self, equivalence_key: &str) -> Option<PreviewCacheEntry> {
        self.entries.lock().unwrap().get(equivalence_key).cloned()
    }

    pub fn insert(&self, equivalence_key: String, entry: PreviewCacheEntry) {
        self.entries.lock().unwrap().insert(equivalence_key, entry);
    }

    /// Backs `POST /cache/clear` (FR-029): every subsequent lookup becomes a
    /// plain cache miss, never a data-loss condition (FR-011).
    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }

    /// Like `clear`, but returns what was dropped so `POST /cache/clear` can
    /// report exactly which node/image pairs now need recomputation.
    pub fn drain(&self) -> Vec<PreviewCacheEntry> {
        self.entries.lock().unwrap().drain().map(|(_, v)| v).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(output: &str) -> PreviewCacheEntry {
        PreviewCacheEntry {
            output_content_identity: output.into(),
            producing_node_id: "blur".into(),
            pipeline_snapshot_id: Uuid::new_v4(),
            image_asset_id: Uuid::new_v4(),
            duration_ms: 5,
        }
    }

    #[test]
    fn miss_then_hit_after_insert() {
        let cache = PreviewCache::default();
        assert!(cache.get("key-1").is_none());
        cache.insert("key-1".into(), entry("out-1"));
        assert_eq!(cache.get("key-1").unwrap().output_content_identity, "out-1");
    }

    #[test]
    fn clear_produces_recoverable_miss() {
        let cache = PreviewCache::default();
        cache.insert("key-1".into(), entry("out-1"));
        assert!(!cache.is_empty());
        cache.clear();
        assert!(cache.get("key-1").is_none());
        assert!(cache.is_empty());
    }
}
