// engines: master cache — 150-DPI masters with on-demand 300-DPI crops

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/stichos-home"))
}

/// Default LRU capacity for the master cache.
const DEFAULT_MAX_ENTRIES: usize = 5000;

/// Compute the cache key for a rasterized page.
///
/// Format: `<sha256(doc)>[0:16]-<engine>-<version>-<fontset>[0:16]-<dpi>`
///
/// Example: `a1b2c3d4e5f6a7b8-libreoffice-26.2.5-9f8e7d6c5b4a3211-150`
pub fn make_cache_key(
    doc_hash: &str,
    engine_id: &str,
    version: &str,
    fontset_hash: &str,
    dpi: u32,
) -> String {
    let doc_prefix = &doc_hash[..doc_hash.len().min(16)];
    let fontset_prefix = &fontset_hash[..fontset_hash.len().min(16)];
    format!(
        "{}-{}-{}-{}-{}",
        doc_prefix, engine_id, version, fontset_prefix, dpi
    )
}

/// Compute the SHA-256 hex digest of data.
pub fn sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Master cache: 150-DPI masters with on-demand 300-DPI crops.
///
/// The cache is backed by files on disk at `~/.stichos/cache/rasters/<first-2>/<full-key>`.
/// An in-memory LRU tracks recently-accessed keys.
pub struct MasterCache {
    root: PathBuf,
    lru: LRUCache,
    max_entries: usize,
}

impl MasterCache {
    /// Open the master cache at the default location (~/.stichos/cache/rasters).
    pub fn open() -> Result<Self, std::io::Error> {
        let root = dirs_home().join(".stichos").join("cache").join("rasters");
        Self::open_at(&root)
    }

    /// Open the master cache at a specific path.
    pub fn open_at(path: &Path) -> Result<Self, std::io::Error> {
        std::fs::create_dir_all(path)?;
        Ok(Self {
            root: path.to_path_buf(),
            lru: LRUCache::new(DEFAULT_MAX_ENTRIES),
            max_entries: DEFAULT_MAX_ENTRIES,
        })
    }

    /// Get a cached raster for the given key, or None if not present.
    ///
    /// Accessing a key updates its LRU position.
    pub fn get(&mut self, key: &str) -> Option<Vec<u8>> {
        if let Some(blob_path) = self.blob_path(key) {
            if blob_path.exists() {
                self.lru.access(key);
                return std::fs::read(&blob_path).ok();
            }
        }
        None
    }

    /// Store raster data under the given key.
    ///
    /// If the key already exists, the data is not overwritten (append-only).
    /// After insertion, LRU eviction is triggered if the cache is full.
    pub fn put(&mut self, key: &str, data: &[u8]) {
        if let Some(blob_path) = self.blob_path(key) {
            if blob_path.exists() {
                // Append-only: never overwrite existing blobs.
                self.lru.access(key);
                return;
            }
            if let Some(parent) = blob_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(mut f) = std::fs::File::create(&blob_path) {
                let _ = f.write_all(data);
            }
        }
        self.lru.access(key);
        self.evict_lru();
    }

    /// Check whether a key exists in the cache (without accessing it).
    pub fn contains(&self, key: &str) -> bool {
        self.blob_path(key).map(|p| p.exists()).unwrap_or(false)
    }

    /// Evict least-recently-used entries until the in-memory LRU is under capacity.
    ///
    /// This also removes the corresponding on-disk blobs.
    pub fn evict_lru(&mut self) {
        while self.lru.len() > self.max_entries {
            if let Some(lru_key) = self.lru.lru_key() {
                // Remove on-disk blob
                if let Some(blob_path) = self.blob_path(&lru_key) {
                    let _ = std::fs::remove_file(&blob_path);
                }
                self.lru.remove(&lru_key);
            } else {
                break;
            }
        }
    }

    /// Compute the on-disk path for a cache key.
    ///
    /// Layout: <root>/<first-2-chars>/<full-key>
    fn blob_path(&self, key: &str) -> Option<PathBuf> {
        let shard = key.get(..2).unwrap_or(key);
        Some(self.root.join(shard).join(key))
    }

    /// Return the cache root path.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// In-memory LRU cache backed by a VecDeque for O(1) eviction and O(n) lookup.
struct LRUCache {
    order: VecDeque<String>,
    max_size: usize,
}

impl LRUCache {
    fn new(max_size: usize) -> Self {
        Self {
            order: VecDeque::with_capacity(max_size),
            max_size,
        }
    }

    /// Access a key, moving it to the back (most recently used).
    fn access(&mut self, key: &str) {
        // Remove existing position if present
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
        }
        // Insert at back
        self.order.push_back(key.to_string());
        // Trim from front if over capacity
        while self.order.len() > self.max_size {
            self.order.pop_front();
        }
    }

    /// Get the least recently used key without removing it.
    fn lru_key(&self) -> Option<String> {
        self.order.front().cloned()
    }

    /// Remove a key from the cache.
    fn remove(&mut self, key: &str) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
        }
    }

    /// Current number of entries.
    fn len(&self) -> usize {
        self.order.len()
    }
}

/// Errors that can occur when operating a [`MasterCache`].
#[derive(Debug, thiserror::Error)]
pub enum MasterCacheError {
    /// I/O failure.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a temp directory for the cache and clean up after.
    fn temp_cache_root() -> PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "stichos-cache-test-{}",
            fastrand::u32(1_000_000..9_999_999)
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);
        tmp
    }

    /// test_cache_key_deterministic: same inputs always produce the same key.
    #[test]
    fn test_cache_key_deterministic() {
        let h = sha256(b"test doc");
        let k1 = make_cache_key(&h, "libreoffice", "26.2.5", "abc123def456789", 150);
        let k2 = make_cache_key(&h, "libreoffice", "26.2.5", "abc123def456789", 150);
        assert_eq!(k1, k2, "same inputs must produce same key");

        // Different DPI → different key
        let k3 = make_cache_key(&h, "libreoffice", "26.2.5", "abc123def456789", 300);
        assert_ne!(k1, k3, "different DPI must produce different key");

        // Different engine → different key
        let k4 = make_cache_key(&h, "pandoc", "26.2.5", "abc123def456789", 150);
        assert_ne!(k1, k4, "different engine must produce different key");

        // Key format check
        assert!(k1.ends_with("-150"));
        assert!(k3.ends_with("-300"));
    }

    /// test_cache_put_get: store and retrieve data.
    #[test]
    fn test_cache_put_get() {
        let root = temp_cache_root();
        let mut cache = MasterCache::open_at(&root).expect("open cache");

        let key = make_cache_key("aaaaaaaaaaaaaaaa", "eng1", "v1", "bbbbbbbbbbbbbbbb", 150);
        let data = b"PNG FAKE DATA".to_vec();

        assert!(!cache.contains(&key));
        cache.put(&key, &data);
        assert!(cache.contains(&key));

        let got = cache.get(&key);
        assert_eq!(got, Some(data.clone()));
    }

    /// test_cache_contains: check existence.
    #[test]
    fn test_cache_contains() {
        let root = temp_cache_root();
        let mut cache = MasterCache::open_at(&root).expect("open cache");

        let key = make_cache_key("cccccccccccccccc", "eng2", "v2", "dddddddddddddddd", 150);
        assert!(!cache.contains(&key), "new key should not exist");

        cache.put(&key, b"data");
        assert!(cache.contains(&key), "key should exist after put");
    }

    /// test_cache_lru_eviction: LRU evicts oldest entries when over capacity.
    #[test]
    fn test_cache_lru_eviction() {
        let root = temp_cache_root();
        let mut cache = MasterCache::open_at(&root).expect("open cache");

        // Override max_entries for faster testing
        cache.max_entries = 3;

        let k1 = make_cache_key("aaaa1111aaaa1111", "e1", "v1", "bbbb2222bbbb2222", 150);
        let k2 = make_cache_key("cccc3333cccc3333", "e2", "v2", "dddd4444dddd4444", 150);
        let k3 = make_cache_key("eeee5555eeee5555", "e3", "v3", "ffff6666ffff6666", 150);
        let k4 = make_cache_key("gggg7777gggg7777", "e4", "v4", "hhhh8888hhhh8888", 150);

        cache.put(&k1, b"1");
        cache.put(&k2, b"2");
        cache.put(&k3, b"3");

        // k4 should evict k1 (LRU)
        cache.put(&k4, b"4");

        assert!(!cache.contains(&k1), "k1 should have been evicted");
        assert!(cache.contains(&k2), "k2 should still be cached");
        assert!(cache.contains(&k3), "k3 should still be cached");
        assert!(cache.contains(&k4), "k4 should be in cache");
    }

    /// test_cache_key_rotation_on_fontset_change: changing fontset produces a new key.
    #[test]
    fn test_cache_key_rotation_on_fontset_change() {
        let doc_hash = sha256(b"same document");
        let k1 = make_cache_key(
            &doc_hash,
            "libreoffice",
            "26.2.5",
            "fontset_v1_hash_01",
            150,
        );
        let k2 = make_cache_key(
            &doc_hash,
            "libreoffice",
            "26.2.5",
            "fontset_v2_hash_02",
            150,
        );

        assert_ne!(
            k1, k2,
            "different fontset hashes must produce different cache keys"
        );

        // But same fontset should produce same key
        let k3 = make_cache_key(
            &doc_hash,
            "libreoffice",
            "26.2.5",
            "fontset_v1_hash_01",
            150,
        );
        assert_eq!(k1, k3, "same fontset hash must produce same key");
    }

    /// test_cache_access_updates_lru: accessing a key moves it to MRU position.
    #[test]
    fn test_cache_access_updates_lru() {
        let root = temp_cache_root();
        let mut cache = MasterCache::open_at(&root).expect("open cache");
        cache.max_entries = 3;

        let k1 = make_cache_key("aaaa0000aaaa0000", "e1", "v1", "bbbb1111bbbb1111", 150);
        let k2 = make_cache_key("cccc2222cccc2222", "e2", "v2", "dddd3333dddd3333", 150);
        let k3 = make_cache_key("eeee4444eeee4444", "e3", "v3", "ffff5555ffff5555", 150);
        let k4 = make_cache_key("gggg6666gggg6666", "e4", "v4", "hhhh7777hhhh7777", 150);

        cache.put(&k1, b"1");
        cache.put(&k2, b"2");
        cache.put(&k3, b"3");

        // Access k1 to make it MRU
        let _ = cache.get(&k1);

        // Now k2 is LRU, so adding k4 should evict k2
        cache.put(&k4, b"4");

        assert!(cache.contains(&k1), "k1 should survive (was accessed, MRU)");
        assert!(!cache.contains(&k2), "k2 should have been evicted (LRU)");
        assert!(cache.contains(&k3), "k3 should still be cached");
        assert!(cache.contains(&k4), "k4 should be in cache");
    }

    /// test_cache_append_only: putting same key twice does not overwrite.
    #[test]
    fn test_cache_append_only() {
        let root = temp_cache_root();
        let mut cache = MasterCache::open_at(&root).expect("open cache");

        let key = make_cache_key("aaaa1111aaaa1111", "e1", "v1", "bbbb2222bbbb2222", 150);
        let data1 = b"first".to_vec();
        let data2 = b"second".to_vec();

        cache.put(&key, &data1);
        cache.put(&key, &data2);

        let got = cache.get(&key).unwrap();
        assert_eq!(got, data1, "second put should not overwrite first");
    }

    /// test_sha256_deterministic: sha256 produces consistent output.
    #[test]
    fn test_sha256_deterministic() {
        let h1 = sha256(b"test");
        let h2 = sha256(b"test");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64, "SHA-256 hex digest should be 64 chars");

        let h3 = sha256(b"different");
        assert_ne!(h1, h3);
    }

    /// test_make_cache_key_formats: verify key format for various inputs.
    #[test]
    fn test_make_cache_key_formats() {
        // Short hashes
        let k = make_cache_key("abc", "eng", "v1", "xyz", 150);
        assert_eq!(k, "abc-eng-v1-xyz-150");

        // Long hashes (truncated to 16 chars)
        let long_hash = "aaaaaaaaaaaaaaaa1111111111111111";
        let k2 = make_cache_key(long_hash, "eng", "v1", long_hash, 150);
        assert!(k2.starts_with("aaaaaaaaaaaaaaaa-eng-v1-aaaaaaaaaaaaaaaa-150"));
    }

    /// test_cache_open_default: default open works when directory exists.
    #[test]
    fn test_cache_open_default() {
        // This may fail if ~/.stichos/cache/rasters can't be created.
        let result = MasterCache::open();
        if let Ok(mut cache) = result {
            let key = make_cache_key("test1111test1111", "eng", "v1", "test2222test2222", 150);
            cache.put(&key, b"data");
            assert!(cache.contains(&key));
        }
    }

    /// test_cache_evict_lru_clears_disk: evicted keys are removed from disk.
    #[test]
    fn test_cache_evict_lru_clears_disk() {
        let root = temp_cache_root();
        let mut cache = MasterCache::open_at(&root).expect("open cache");
        cache.max_entries = 2;

        let k1 = make_cache_key("aaaa0000aaaa0000", "e1", "v1", "bbbb1111bbbb1111", 150);
        let k2 = make_cache_key("cccc2222cccc2222", "e2", "v2", "dddd3333dddd3333", 150);
        let k3 = make_cache_key("eeee4444eeee4444", "e3", "v3", "ffff5555ffff5555", 150);

        cache.put(&k1, b"1");
        cache.put(&k2, b"2");
        assert!(cache.contains(&k1));
        assert!(cache.contains(&k2));

        cache.put(&k3, b"3");
        // k1 should be evicted
        assert!(!cache.contains(&k1), "k1 should be evicted");
        assert!(cache.contains(&k2), "k2 should remain");
        assert!(cache.contains(&k3), "k3 should be in cache");
    }
}
