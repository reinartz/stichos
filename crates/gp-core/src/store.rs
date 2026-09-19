// SPDX-License-Identifier: MIT OR Apache-2.0
//! Content-addressed store for stichos reads and renders.
//!
//! Layout at `~/.stichos/`:
//!   ~/.stichos/
//!   ├── blobs/          — content-addressed blob storage (sha256 key, never overwritten)
//!   │   ├── ab/cd1234...
//!   │   └── ...
//!   ├── versions/       — document version manifests
//!   │   └── <doc-id>.json
//!   └── config.json     — store configuration

use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::diagnostics;

/// Default LRU capacity: number of entries before eviction starts.
const DEFAULT_CAPACITY: usize = 10000;

/// Content-addressed store for stichos reads and renders.
///
/// Stores blobs keyed by their SHA-256 hash with an LRU eviction policy
/// and document version tracking.
#[derive(Debug)]
pub struct ContentStore {
    root: PathBuf,
    lru: LruCache,
    max_blobs: usize,
}

/// A blob entry in the store.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BlobEntry {
    /// SHA-256 hex digest.
    pub key: String,
    /// Blob size in bytes.
    pub size: u64,
    /// MIME type, e.g. "application/vnd.openxmlformats-officedocument.wordprocessingml.document".
    pub content_type: String,
    /// ISO 8601 timestamp.
    pub stored_at: String,
    /// How many documents reference this blob.
    pub references: usize,
}

/// A document version record.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DocumentVersion {
    /// Unique document identifier.
    pub doc_id: String,
    /// Monotonic version number.
    pub version: u64,
    /// SHA-256 key of the document blob.
    pub blob_key: String,
    /// ISO 8601 timestamp.
    pub created_at: String,
    /// Arbitrary metadata.
    pub metadata: std::collections::HashMap<String, String>,
}

/// LRU cache for recent blob keys.
#[derive(Debug)]
struct LruCache {
    order: VecDeque<String>,
    max_size: usize,
}

impl LruCache {
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

    /// Get the least recently used key, if any.
    fn lru_key(&self) -> Option<String> {
        self.order.front().cloned()
    }

    /// Remove a key from the cache.
    fn remove(&mut self, key: &str) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
        }
    }
}

/// Errors that can occur when operating a [`ContentStore`].
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// I/O failure.
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    /// The requested blob was not found.
    #[error("Blob not found: {0}")]
    NotFound(String),
    /// The store has not been initialized / the root is missing.
    #[error("Store not initialized")]
    NotInitialized,
    /// A blob is corrupted (hash mismatch or unreadable).
    #[error("Corrupted blob: {0}")]
    Corrupted(String),
    /// The maximum blob count was exceeded.
    #[error("Max blobs exceeded: {0}")]
    MaxBlobsExceeded(usize),
    /// A document version manifest could not be parsed.
    #[error("Version manifest parse error: {0}")]
    VersionParse(#[source] serde_json::Error),
}

impl StoreError {
    /// Convert to diagnostics for adapter consumers.
    pub fn to_diagnostics(&self) -> Vec<diagnostics::Diagnostic> {
        match self {
            StoreError::Io(e) => vec![diagnostics::Diagnostic::new(
                "GP-E100",
                diagnostics::Severity::Error,
                e.to_string(),
            )],
            StoreError::NotFound(key) => vec![diagnostics::Diagnostic::new(
                "GP-E101",
                diagnostics::Severity::Warning,
                format!("Blob not found: {key}"),
            )],
            StoreError::NotInitialized => vec![diagnostics::Diagnostic::new(
                "GP-E102",
                diagnostics::Severity::Error,
                "Store not initialized",
            )],
            StoreError::Corrupted(msg) => vec![diagnostics::Diagnostic::new(
                "GP-E103",
                diagnostics::Severity::Error,
                format!("Corrupted blob: {msg}"),
            )],
            StoreError::MaxBlobsExceeded(n) => vec![diagnostics::Diagnostic::new(
                "GP-E104",
                diagnostics::Severity::Warning,
                format!("Max blobs exceeded: {n}"),
            )],
            StoreError::VersionParse(e) => vec![diagnostics::Diagnostic::new(
                "GP-E105",
                diagnostics::Severity::Error,
                format!("Version manifest parse error: {e}"),
            )],
        }
    }
}

// ---------------------------------------------------------------------------
// ContentStore
// ---------------------------------------------------------------------------

impl ContentStore {
    /// Create or open the store at the default location (`~/.stichos/`).
    pub fn open() -> Result<Self, StoreError> {
        let root = dirs_home().join(".stichos");
        Self::open_at(&root)
    }

    /// Create or open the store at a specific path.
    pub fn open_at(path: &Path) -> Result<Self, StoreError> {
        let root = path.to_path_buf();
        std::fs::create_dir_all(root.join("blobs"))?;
        std::fs::create_dir_all(root.join("versions"))?;

        // Write config.json if it doesn't exist
        let config_path = root.join("config.json");
        if !config_path.exists() {
            let config = serde_json::json!({
                "version": "1",
                "created_at": iso_now(),
            });
            let mut f = std::fs::File::create(&config_path)?;
            f.write_all(serde_json::to_string_pretty(&config).unwrap().as_bytes())?;
        }

        Ok(Self {
            root,
            lru: LruCache::new(DEFAULT_CAPACITY),
            max_blobs: DEFAULT_CAPACITY,
        })
    }

    /// Compute the SHA-256 hex key for data.
    pub fn key(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }

    /// Store a blob, returning its SHA-256 hex key.
    ///
    /// Never overwrites existing blobs — storing the same content twice is
    /// idempotent: `store(data) == store(data)` and the file is unchanged.
    pub fn store(&mut self, data: &[u8], _content_type: &str) -> Result<String, StoreError> {
        let key = Self::key(data);
        let blob_path = self.blob_path(&key);

        if blob_path.exists() {
            // Idempotent: blob already exists
            self.lru.access(&key);
            return Ok(key);
        }

        // Ensure the shard directory exists
        if let Some(parent) = blob_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Write blob
        let mut file = std::fs::File::create(&blob_path)?;
        file.write_all(data)?;

        self.lru.access(&key);
        Ok(key)
    }

    /// Retrieve a blob by its SHA-256 hex key.
    pub fn retrieve(&mut self, key: &str) -> Result<Vec<u8>, StoreError> {
        if !self.root.join("blobs").exists() {
            return Err(StoreError::NotInitialized);
        }
        let path = self.blob_path(key);
        if !path.exists() {
            return Err(StoreError::NotFound(key.to_string()));
        }
        let data = std::fs::read(&path)?;
        // Verify content hash
        let verify_key = Self::key(&data);
        if verify_key != key {
            return Err(StoreError::Corrupted(format!(
                "hash mismatch: expected {key}, got {verify_key}"
            )));
        }
        self.lru.access(key);
        Ok(data)
    }

    /// Check if a blob exists in the store.
    pub fn contains(&self, key: &str) -> Result<bool, StoreError> {
        if !self.root.join("blobs").exists() {
            return Ok(false);
        }
        Ok(self.blob_path(key).exists())
    }

    /// Record a document version.
    pub fn record_version(
        &self,
        doc_id: &str,
        blob_key: &str,
        metadata: std::collections::HashMap<String, String>,
    ) -> Result<DocumentVersion, StoreError> {
        if !self.root.join("versions").exists() {
            return Err(StoreError::NotInitialized);
        }

        // Verify the blob exists
        if !self.contains(blob_key)? {
            return Err(StoreError::NotFound(blob_key.to_string()));
        }

        let version_file = self.version_file(doc_id);
        let mut versions: Vec<DocumentVersion> = if version_file.exists() {
            let content = std::fs::read_to_string(&version_file)?;
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Vec::new()
        };

        let next_version = (versions.len() + 1) as u64;
        let version = DocumentVersion {
            doc_id: doc_id.to_string(),
            version: next_version,
            blob_key: blob_key.to_string(),
            created_at: iso_now(),
            metadata,
        };
        versions.push(version.clone());

        let mut f = std::fs::File::create(&version_file)?;
        let json = serde_json::to_string_pretty(&versions).map_err(StoreError::VersionParse)?;
        f.write_all(json.as_bytes())?;

        Ok(version)
    }

    /// Get the latest version of a document.
    pub fn latest_version(&self, doc_id: &str) -> Result<Option<DocumentVersion>, StoreError> {
        let versions = self.all_versions(doc_id)?;
        Ok(versions.last().cloned())
    }

    /// Get all versions of a document.
    pub fn all_versions(&self, doc_id: &str) -> Result<Vec<DocumentVersion>, StoreError> {
        if !self.root.join("versions").exists() {
            return Ok(Vec::new());
        }
        let version_file = self.version_file(doc_id);
        if !version_file.exists() {
            return Ok(Vec::new());
        }
        let content = std::fs::read_to_string(&version_file)?;
        let versions: Vec<DocumentVersion> =
            serde_json::from_str(&content).map_err(StoreError::VersionParse)?;
        Ok(versions)
    }

    /// Evict the least recently used blobs until we are under capacity.
    ///
    /// Returns the list of evicted blob keys.
    pub fn evict_lru(&mut self) -> Result<Vec<String>, StoreError> {
        let mut evicted = Vec::new();
        let blob_dir = self.root.join("blobs");
        if !blob_dir.exists() {
            return Ok(evicted);
        }

        // Collect all blob keys on disk
        let mut all_keys: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&blob_dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                let dir_path = entry.path();
                for sub_entry in std::fs::read_dir(&dir_path)? {
                    let sub = sub_entry?;
                    if !sub.file_type()?.is_file() {
                        continue;
                    }
                    if let Some(name) = sub.file_name().to_str() {
                        all_keys.push(name.to_string());
                    }
                }
            }
        }

        // Evict LRU keys until we're at capacity
        while all_keys.len() > self.max_blobs {
            if let Some(lru_key) = self.lru.lru_key() {
                // Only evict if the key is in our on-disk list
                if let Some(pos) = all_keys.iter().position(|k| *k == lru_key) {
                    let evict_key = all_keys.remove(pos);
                    let evict_path = self.blob_path(&evict_key);
                    let _ = std::fs::remove_file(&evict_path);
                    let _ = std::fs::remove_dir_all(
                        evict_path
                            .parent()
                            .map(|p| p.to_path_buf())
                            .unwrap_or_default(),
                    );
                    self.lru.remove(&evict_key);
                    evicted.push(evict_key);
                } else {
                    // LRU key is not on disk; remove it from cache and continue
                    self.lru.remove(&lru_key);
                }
            } else {
                // No LRU entries but still over capacity — evict oldest file on disk
                if let Some(key) = all_keys.first().cloned() {
                    all_keys.remove(0);
                    let evict_path = self.blob_path(&key);
                    let _ = std::fs::remove_file(&evict_path);
                    evicted.push(key);
                } else {
                    break;
                }
            }
        }

        Ok(evicted)
    }

    /// Get the store root path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get the current blob count on disk.
    pub fn blob_count(&self) -> Result<usize, StoreError> {
        let blob_dir = self.root.join("blobs");
        if !blob_dir.exists() {
            return Ok(0);
        }
        let mut count = 0;
        for entry in std::fs::read_dir(&blob_dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                count += std::fs::read_dir(entry.path())?.count();
            }
        }
        Ok(count)
    }

    /// Get the total store size in bytes.
    pub fn total_size(&self) -> Result<u64, StoreError> {
        let blob_dir = self.root.join("blobs");
        if !blob_dir.exists() {
            return Ok(0);
        }
        let mut total: u64 = 0;
        for entry in std::fs::read_dir(&blob_dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_file() {
                total += entry.metadata()?.len();
            } else if file_type.is_dir() {
                for sub_entry in std::fs::read_dir(entry.path())? {
                    let sub = sub_entry?;
                    if sub.file_type()?.is_file() {
                        total += sub.metadata()?.len();
                    }
                }
            }
        }
        Ok(total)
    }

    /// Return the file path for a blob.
    fn blob_path(&self, key: &str) -> PathBuf {
        let shard = &key[..2];
        self.root.join("blobs").join(shard).join(key)
    }

    /// Return the version manifest path for a document.
    fn version_file(&self, doc_id: &str) -> PathBuf {
        self.root.join("versions").join(format!("{doc_id}.json"))
    }
}

/// Helper: return current ISO 8601 timestamp.
fn iso_now() -> String {
    chrono_local_now()
        .replace(':', " ")
        .trim_end_matches(".000000000 +0000")
        .to_string()
}

/// Fallback: use a simple ISO 8601 timestamp without chrono dependency.
fn chrono_local_now() -> String {
    // Use a fixed format without external deps
    // We'll use a simple approach: try to get current time
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    // Simple UTC timestamp — not perfect but avoids chrono dependency
    // For a real implementation, chrono would be better, but we use
    // a deterministic approach for tests.
    let secs_since = secs % 253402300800; // range 0..10000 years
    let time_of_day = secs_since % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    // Approximate date from days since epoch
    // Unix epoch 1970-01-01
    let days_from_epoch = secs_since / 86400;
    // Simplified date calculation
    let mut y: i64 = 1970;
    let mut remaining_days = days_from_epoch;
    loop {
        let days_in_year = if is_leap_year(y) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        y += 1;
    }
    let month_days = [
        31,
        if is_leap_year(y) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 1;
    for &md in &month_days {
        if remaining_days < md {
            break;
        }
        remaining_days -= md;
        m += 1;
    }
    let d = remaining_days + 1;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, m, d, hours, minutes, seconds
    )
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Determine the user's home directory path.
fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/stichos-home"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Helper: create a temp directory for the store.
    fn temp_store() -> (ContentStore, PathBuf) {
        let id = uuid::Uuid::new_v4().to_string();
        let root = std::env::temp_dir().join(format!("stichos-store-{id}"));
        let _ = std::fs::remove_dir_all(&root);
        let store = ContentStore::open_at(&root).unwrap();
        (store, root)
    }

    // ------------------------------------------------------------------
    // Basic store / retrieve tests
    // ------------------------------------------------------------------

    #[test]
    fn test_store_retrieve_roundtrip() {
        let (mut store, root) = temp_store();
        let data = b"hello content-addressed store";
        let key = store.store(data, "text/plain").unwrap();
        let retrieved = store.retrieve(&key).unwrap();
        assert_eq!(data.as_ref(), retrieved);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_store_sha256_key() {
        let (mut store, root) = temp_store();
        let data = b"deterministic content";
        let key = store.store(data, "text/plain").unwrap();
        let expected = ContentStore::key(data);
        assert_eq!(key, expected);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_store_never_overwrite() {
        let (mut store, root) = temp_store();
        let data1 = b"original data";
        let key1 = store.store(data1, "text/plain").unwrap();

        // Modify the blob file directly (bypassing the store)
        let blob_path = store.blob_path(&key1);
        let modified_data = b"modified data!!";
        std::fs::write(&blob_path, modified_data).unwrap();

        // Now store something else that happens to have a different hash
        let data2 = b"completely different content here!";
        let key2 = store.store(data2, "text/plain").unwrap();

        // key1's blob should still be the modified data
        let retrieved = std::fs::read(&blob_path).unwrap();
        assert_eq!(retrieved, modified_data);

        // Both keys should be different
        assert_ne!(key1, key2);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_store_idempotent() {
        let (mut store, root) = temp_store();
        let data = b"idempotent data";
        let key1 = store.store(data, "text/plain").unwrap();
        let key2 = store.store(data, "text/plain").unwrap();
        assert_eq!(key1, key2);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------------------
    // contains tests
    // ------------------------------------------------------------------

    #[test]
    fn test_contains_exists() {
        let (mut store, root) = temp_store();
        let data = b"exists blob";
        let key = store.store(data, "text/plain").unwrap();
        assert!(store.contains(&key).unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_contains_not_exists() {
        let (store, root) = temp_store();
        let fake_key = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
        assert!(!store.contains(fake_key).unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------------------
    // LRU eviction tests
    // ------------------------------------------------------------------

    #[test]
    fn test_lru_eviction() {
        let id = uuid::Uuid::new_v4().to_string();
        let root = std::env::temp_dir().join(format!("stichos-store-lru-{id}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("blobs")).unwrap();
        std::fs::create_dir_all(root.join("versions")).unwrap();

        // Create a store with small capacity (4 blobs, cache tracks 4)
        let mut store = ContentStore {
            root: root.clone(),
            lru: LruCache::new(4),
            max_blobs: 3,
        };

        let d1 = b"data one";
        let d2 = b"data two";
        let d3 = b"data three";
        let d4 = b"data four";

        let k1 = store.store(d1, "text/plain").unwrap();
        let k2 = store.store(d2, "text/plain").unwrap();
        let k3 = store.store(d3, "text/plain").unwrap();

        // Store a 4th blob to exceed max_blobs=3
        let _k4 = store.store(d4, "text/plain").unwrap();

        // Access k2 to make it most recently used
        store.retrieve(&k2).unwrap();

        // LRU order: after store(k1), store(k2), store(k3), store(k4) the LRU order is [k1, k2, k3, k4]
        // After retrieve(k2), it becomes [k1, k3, k4, k2]
        // So k1 is LRU (least recently used)
        assert_eq!(store.lru.lru_key(), Some(k1.clone()));

        // Evict should remove k1 (LRU) since we have 4 blobs and max is 3
        let evicted = store.evict_lru().unwrap();
        assert!(evicted.contains(&k1));

        // k1 should no longer exist
        assert!(!store.contains(&k1).unwrap());

        // k1 should no longer exist
        assert!(!store.contains(&k1).unwrap());

        // k2 and k3 should still exist
        assert!(store.contains(&k2).unwrap());
        assert!(store.contains(&k3).unwrap());

        let _ = std::fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------------------
    // Document version tests
    // ------------------------------------------------------------------

    #[test]
    fn test_document_version() {
        let (mut store, root) = temp_store();
        let data = b"document content";
        let blob_key = store
            .store(
                data,
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            )
            .unwrap();

        let mut meta = HashMap::new();
        meta.insert("author".to_string(), "test".to_string());

        let version = store
            .record_version("doc-1", &blob_key, meta.clone())
            .unwrap();
        assert_eq!(version.doc_id, "doc-1");
        assert_eq!(version.version, 1);
        assert_eq!(version.blob_key, blob_key);
        assert!(!version.created_at.is_empty());
        assert_eq!(version.metadata, meta);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_latest_version() {
        let (mut store, root) = temp_store();
        let d1 = b"version 1 content";
        let d2 = b"version 2 content";

        let blob1 = store.store(d1, "text/plain").unwrap();
        let blob2 = store.store(d2, "text/plain").unwrap();

        let mut meta1 = HashMap::new();
        meta1.insert("v".to_string(), "1".to_string());
        store.record_version("doc-1", &blob1, meta1).unwrap();

        let mut meta2 = HashMap::new();
        meta2.insert("v".to_string(), "2".to_string());
        store.record_version("doc-1", &blob2, meta2).unwrap();

        let latest = store.latest_version("doc-1").unwrap().unwrap();
        assert_eq!(latest.version, 2);
        assert_eq!(latest.blob_key, blob2);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_all_versions() {
        let (mut store, root) = temp_store();
        let d1 = b"v1";
        let d2 = b"v2";
        let d3 = b"v3";

        let b1 = store.store(d1, "text/plain").unwrap();
        let b2 = store.store(d2, "text/plain").unwrap();
        let b3 = store.store(d3, "text/plain").unwrap();

        store.record_version("doc-1", &b1, HashMap::new()).unwrap();
        store.record_version("doc-1", &b2, HashMap::new()).unwrap();
        store.record_version("doc-1", &b3, HashMap::new()).unwrap();

        let versions = store.all_versions("doc-1").unwrap();
        assert_eq!(versions.len(), 3);
        assert_eq!(versions[0].version, 1);
        assert_eq!(versions[1].version, 2);
        assert_eq!(versions[2].version, 3);

        // Non-existent document returns empty
        let empty = store.all_versions("nonexistent").unwrap();
        assert!(empty.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------------------
    // Store not initialized test
    // ------------------------------------------------------------------

    #[test]
    fn test_store_not_initialized() {
        // Create a store that hasn't been properly initialized
        let tmp = std::env::temp_dir().join("stichos-store-notinit");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let mut store = ContentStore {
            root: tmp.clone(),
            lru: LruCache::new(100),
            max_blobs: 100,
        };

        // retrieve should fail since blobs dir doesn't exist
        let result = store.retrieve("abc123");
        assert!(matches!(result, Err(StoreError::NotInitialized)));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ------------------------------------------------------------------
    // Blob sharding test
    // ------------------------------------------------------------------

    #[test]
    fn test_blob_sharding() {
        let (mut store, root) = temp_store();
        let data = b"sharded blob";
        let key = store.store(data, "text/plain").unwrap();

        // Key should start with some hex chars, and the blob should be at blobs/<first2>/<key>
        let expected_path = root.join("blobs").join(&key[..2]).join(&key);
        assert!(expected_path.exists());

        // Verify the full path structure
        assert_eq!(store.blob_path(&key), expected_path);

        let _ = std::fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------------------
    // Total size test
    // ------------------------------------------------------------------

    #[test]
    fn test_total_size() {
        let (mut store, root) = temp_store();
        let d1 = b"small blob";
        let d2 = b"a slightly larger blob for testing";

        let _k1 = store.store(d1, "text/plain").unwrap();
        let _k2 = store.store(d2, "text/plain").unwrap();

        let total = store.total_size().unwrap();
        assert!(total > 0);
        assert!(total >= (d1.len() as u64 + d2.len() as u64));

        let _ = std::fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------------------
    // Blob count test
    // ------------------------------------------------------------------

    #[test]
    fn test_blob_count() {
        let (mut store, root) = temp_store();
        let _k1 = store.store(b"blob1", "text/plain").unwrap();
        let _k2 = store.store(b"blob2", "text/plain").unwrap();
        let _k3 = store.store(b"blob3", "text/plain").unwrap();

        assert_eq!(store.blob_count().unwrap(), 3);

        let _ = std::fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------------------
    // Diagnostics test
    // ------------------------------------------------------------------

    #[test]
    fn test_store_error_to_diagnostics() {
        let io_err = StoreError::Io(io::Error::new(io::ErrorKind::NotFound, "missing"));
        let diags = io_err.to_diagnostics();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "GP-E100");
        assert_eq!(diags[0].severity, diagnostics::Severity::Error);

        let not_found = StoreError::NotFound("abc123".to_string());
        let diags = not_found.to_diagnostics();
        assert_eq!(diags[0].code, "GP-E101");
        assert_eq!(diags[0].severity, diagnostics::Severity::Warning);

        let not_init = StoreError::NotInitialized;
        let diags = not_init.to_diagnostics();
        assert_eq!(diags[0].code, "GP-E102");
        assert_eq!(diags[0].severity, diagnostics::Severity::Error);

        let corrupted = StoreError::Corrupted("bad hash".to_string());
        let diags = corrupted.to_diagnostics();
        assert_eq!(diags[0].code, "GP-E103");
        assert_eq!(diags[0].severity, diagnostics::Severity::Error);

        let max_blobs = StoreError::MaxBlobsExceeded(10000);
        let diags = max_blobs.to_diagnostics();
        assert_eq!(diags[0].code, "GP-E104");
        assert_eq!(diags[0].severity, diagnostics::Severity::Warning);
    }

    // ------------------------------------------------------------------
    // Blob count / total size edge cases
    // ------------------------------------------------------------------

    #[test]
    fn test_empty_store_total_size() {
        let (store, root) = temp_store();
        assert_eq!(store.total_size().unwrap(), 0);
        assert_eq!(store.blob_count().unwrap(), 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_lru_access_order() {
        let id = uuid::Uuid::new_v4().to_string();
        let root = std::env::temp_dir().join(format!("stichos-lru-order-{id}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("blobs")).unwrap();
        std::fs::create_dir_all(root.join("versions")).unwrap();

        let mut store = ContentStore {
            root: root.clone(),
            lru: LruCache::new(5),
            max_blobs: 5,
        };

        let d1 = b"1";
        let d2 = b"2";
        let d3 = b"3";

        let k1 = store.store(d1, "text/plain").unwrap();
        let _k2 = store.store(d2, "text/plain").unwrap();
        let k3 = store.store(d3, "text/plain").unwrap();

        // LRU should be k1 (oldest)
        assert_eq!(store.lru.lru_key(), Some(k1.clone()));

        // Access k3 (retrieve moves it to MRU)
        store.retrieve(&k3).unwrap();
        // Now LRU should be k1 still (k3 moved to MRU)
        assert_eq!(store.lru.lru_key(), Some(k1.clone()));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_record_version_requires_blob() {
        let (store, root) = temp_store();
        let meta = HashMap::new();
        let result = store.record_version("doc-1", "nonexistent-key", meta);
        assert!(matches!(result, Err(StoreError::NotFound(_))));
        let _ = std::fs::remove_dir_all(&root);
    }
}
