// SPDX-License-Identifier: MIT OR Apache-2.0
//! CTX-03 temp-file image store (P6-03 part 1; contracts C14 §3.2/§3.3/§3.7 +
//! implementation guide §2).
//!
//! Images returned with `output="urls"` (the default for `render_document`,
//! `inspect_region`, and `annotate_pages`) are written to expirable temp
//! files; responses carry paths/URLs plus `expiresAt`, never inline base64.
//!
//! # Crate choice
//!
//! This lives in `gp-core` rather than `engines` because the Wave-C wiring
//! (part 2) needs it from both the CLI handlers (`cli`, the de-facto wiring
//! point) **and** the MCP spec/handler surface (`mcp`, whose allow-list is
//! `gp-core` only per `tools/check-deps.sh`). Putting it in `engines` would
//! force a new `mcp -> engines` edge; `gp-core` keeps the DAG clean. The
//! existing raster/cache machinery in `engines` (`raster.rs` produces PNG
//! bytes, `cache.rs` is a content-addressed master cache under
//! `~/.stichos/cache/rasters`) is complementary: Part-2 handlers will call
//! `rasterize()` for bytes, then [`TempFileManager::store`] to publish them
//! as expirable URL files. It is pure `std` + `sha2`/`hex`/`chrono` (all
//! already `gp-core` deps), so no new dependency surface.
//!
//! # Layout
//!
//! - dir: `$XDG_RUNTIME_DIR/stichos/renders/` (fallback:
//!   `std::env::temp_dir()/stichos/renders/`)
//! - filename: `{prefix}_{hash}_{page}.png` where `hash` is the first 12 hex
//!   chars of SHA-256 over the caller-supplied seed (e.g.
//!   `path + page_range + dpi`)
//! - TTL: 1 hour; [`TempFileManager::sweep_expired`] removes files whose
//!   mtime is older than the TTL, [`TempFileManager::sweep_all`] removes
//!   everything (startup sweep). `expiresAt` per file = write time + TTL,
//!   rendered as RFC 3339 UTC.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

/// Time-to-live for temp render files: 1 hour (C14 §3.2 behavior).
pub const RENDER_TTL: Duration = Duration::from_secs(3600);

/// Canonical filename prefixes (C14 §3.2/§3.3/§3.7).
pub const PREFIX_RENDER: &str = "render";
pub const PREFIX_REGION: &str = "region";
pub const PREFIX_ANNOT_MARKED: &str = "annot_m";
pub const PREFIX_ANNOT_CLEAN: &str = "annot_c";

/// Resolve the default renders directory.
///
/// `$XDG_RUNTIME_DIR/stichos/renders/` when `XDG_RUNTIME_DIR` is set and
/// non-empty, otherwise `<temp_dir>/stichos/renders/`.
pub fn default_renders_dir() -> PathBuf {
    match std::env::var("XDG_RUNTIME_DIR") {
        Ok(rt) if !rt.trim().is_empty() => PathBuf::from(rt).join("stichos").join("renders"),
        _ => std::env::temp_dir().join("stichos").join("renders"),
    }
}

/// First 12 hex chars of SHA-256 over `seed` (the `{hash}` filename slot).
pub fn hash_seed(seed: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(seed.as_bytes());
    hex::encode(hasher.finalize())[..12].to_string()
}

/// Keep only `[A-Za-z0-9_-]` in a caller-supplied prefix; anything else is
/// dropped. An empty result falls back to `"file"`. This blocks path
/// traversal (`../`, `/`) in crafted prefixes.
pub fn sanitize_prefix(prefix: &str) -> String {
    let kept: String = prefix
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if kept.is_empty() {
        "file".to_string()
    } else {
        kept
    }
}

/// `expiresAt` timestamp for a file written at `written_at`: `written_at + TTL`,
/// formatted as RFC 3339 UTC.
pub fn expires_at_rfc3339(written_at: SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Utc> = (written_at + RENDER_TTL).into();
    dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A stored render file: on-disk path, URL string (path form, as carried in
/// `output="urls"` responses), and per-file expiry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRender {
    /// Absolute on-disk path of the PNG.
    pub path: PathBuf,
    /// URL/path string carried in tool responses.
    pub url: String,
    /// Write time + [`RENDER_TTL`], RFC 3339 UTC.
    pub expires_at: String,
}

/// Manager for expirable temp render files.
#[derive(Debug, Clone)]
pub struct TempFileManager {
    base_dir: PathBuf,
}

impl TempFileManager {
    /// Open the manager at `base_dir`, creating it (and parents) if needed.
    pub fn new(base_dir: PathBuf) -> io::Result<Self> {
        std::fs::create_dir_all(&base_dir)?;
        Ok(Self { base_dir })
    }

    /// Open the manager at [`default_renders_dir`].
    pub fn open_default() -> io::Result<Self> {
        Self::new(default_renders_dir())
    }

    /// The directory this manager writes into.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Deterministic filename for a (`prefix`, `seed`, `page`) triple:
    /// `{sanitized_prefix}_{hash_seed(seed)}_{page}.png`. Pure (no I/O).
    pub fn file_name(prefix: &str, seed: &str, page: usize) -> String {
        format!(
            "{}_{}_{}.png",
            sanitize_prefix(prefix),
            hash_seed(seed),
            page
        )
    }

    /// Full path for a (`prefix`, `seed`, `page`) triple. Pure (no I/O).
    pub fn file_path(&self, prefix: &str, seed: &str, page: usize) -> PathBuf {
        self.base_dir.join(Self::file_name(prefix, seed, page))
    }

    /// Write `png_bytes` for (`prefix`, `seed`, `page`) and return the
    /// [`StoredRender`] record (path + url + `expiresAt`).
    pub fn store(
        &self,
        prefix: &str,
        seed: &str,
        page: usize,
        png_bytes: &[u8],
    ) -> io::Result<StoredRender> {
        let path = self.file_path(prefix, seed, page);
        std::fs::write(&path, png_bytes)?;
        let written_at = SystemTime::now();
        Ok(StoredRender {
            url: path.to_string_lossy().to_string(),
            expires_at: expires_at_rfc3339(written_at),
            path,
        })
    }

    /// TTL sweep: remove `*.png` files directly under `base_dir` whose mtime
    /// is older than [`RENDER_TTL`]. Returns the number of files removed.
    /// Missing/unreadable entries are skipped, never fatal.
    pub fn sweep_expired(&self) -> usize {
        let cutoff = SystemTime::now()
            .checked_sub(RENDER_TTL)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let entries = match std::fs::read_dir(&self.base_dir) {
            Ok(entries) => entries,
            Err(_) => return 0,
        };
        let mut removed = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "png") {
                continue;
            }
            let expired = entry
                .metadata()
                .and_then(|m| m.modified())
                .map(|mtime| mtime < cutoff)
                .unwrap_or(false);
            if expired && std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
        removed
    }

    /// Startup sweep: remove **all** `*.png` files directly under `base_dir`.
    /// Returns the number of files removed.
    pub fn sweep_all(&self) -> usize {
        let entries = match std::fs::read_dir(&self.base_dir) {
            Ok(entries) => entries,
            Err(_) => return 0,
        };
        let mut removed = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "png") {
                continue;
            }
            if std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    fn test_manager(tag: &str) -> TempFileManager {
        let dir = std::env::temp_dir().join(format!(
            "stichos-tempfiles-test-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        TempFileManager::new(dir).expect("create test manager dir")
    }

    fn age_file(path: &Path, age: Duration) {
        let old = SystemTime::now()
            .checked_sub(age)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        File::options()
            .write(true)
            .open(path)
            .expect("open for aging")
            .set_modified(old)
            .expect("set mtime");
    }

    /// test_default_dir_xdg: XDG_RUNTIME_DIR set → $XDG_RUNTIME_DIR/stichos/renders.
    #[test]
    fn test_default_dir_xdg() {
        let key = "XDG_RUNTIME_DIR";
        let saved = std::env::var(key).ok();
        std::env::set_var(key, "/run/user/1000");
        assert_eq!(
            default_renders_dir(),
            PathBuf::from("/run/user/1000/stichos/renders")
        );
        match saved {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    /// test_default_dir_fallback: XDG_RUNTIME_DIR unset/empty → temp_dir fallback.
    #[test]
    fn test_default_dir_fallback() {
        let key = "XDG_RUNTIME_DIR";
        let saved = std::env::var(key).ok();
        std::env::remove_var(key);
        assert_eq!(
            default_renders_dir(),
            std::env::temp_dir().join("stichos").join("renders")
        );
        std::env::set_var(key, "");
        assert_eq!(
            default_renders_dir(),
            std::env::temp_dir().join("stichos").join("renders")
        );
        match saved {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    /// test_filename_format: {prefix}_{12-hex-hash}_{page}.png.
    #[test]
    fn test_filename_format() {
        let name = TempFileManager::file_name("render", "doc-a|1-3|150", 1);
        let stem = name.strip_suffix(".png").expect("png suffix");
        let parts: Vec<&str> = stem.split('_').collect();
        assert_eq!(parts.len(), 3, "name must be prefix_hash_page: {name}");
        assert_eq!(parts[0], "render");
        assert_eq!(parts[1].len(), 12);
        assert!(
            parts[1].chars().all(|c| c.is_ascii_hexdigit()),
            "hash must be hex: {name}"
        );
        assert_eq!(parts[2], "1");
        assert_eq!(
            TempFileManager::file_name("render", "doc-a|1-3|150", 1),
            TempFileManager::file_name("render", "doc-a|1-3|150", 1),
            "same seed must give same name"
        );
        assert_ne!(
            TempFileManager::file_name("render", "doc-a|1-3|150", 1),
            TempFileManager::file_name("render", "doc-a|1-3|150", 2),
            "different pages must differ"
        );
    }

    /// test_prefix_sanitized_against_traversal: crafted prefixes cannot escape base_dir.
    #[test]
    fn test_prefix_sanitized_against_traversal() {
        let mgr = test_manager("traversal");
        for evil in ["../evil", "/abs", "a/b", "..", ""] {
            let p = mgr.file_path(evil, "seed", 1);
            assert_eq!(
                p.parent(),
                Some(mgr.base_dir()),
                "prefix {evil:?} must not escape base_dir"
            );
            assert!(p.file_name().unwrap().to_string_lossy().ends_with(".png"));
        }
        let _ = std::fs::remove_dir_all(mgr.base_dir());
    }

    /// test_store_round_trip: store writes bytes; record carries path/url/expiresAt.
    #[test]
    fn test_store_round_trip() {
        let mgr = test_manager("store");
        let seed = "some/doc.docx|1-3|150";
        let rec = mgr.store("render", seed, 1, b"PNG-DATA").expect("store");
        assert_eq!(rec.path, mgr.file_path("render", seed, 1));
        assert_eq!(rec.url, rec.path.to_string_lossy().to_string());
        assert_eq!(std::fs::read(&rec.path).expect("read back"), b"PNG-DATA");
        // expiresAt parses as RFC3339 and is ~1h in the future.
        let exp: chrono::DateTime<chrono::Utc> =
            rec.expires_at.parse().expect("expiresAt must be RFC3339");
        let now: chrono::DateTime<chrono::Utc> = SystemTime::now().into();
        let delta = (exp - now).num_seconds();
        assert!(
            (3590..=3610).contains(&delta),
            "expiresAt must be now+3600s, got {delta}s ({})",
            rec.expires_at
        );
        let _ = std::fs::remove_dir_all(mgr.base_dir());
    }

    /// test_ttl_expiry_sweep: aged files are swept; fresh files survive.
    #[test]
    fn test_ttl_expiry_sweep() {
        let mgr = test_manager("ttl");
        let old_rec = mgr
            .store("render", "doc|1|150", 1, b"OLD")
            .expect("store old");
        let fresh_rec = mgr
            .store("render", "doc|1|150", 2, b"NEW")
            .expect("store fresh");
        age_file(&old_rec.path, Duration::from_secs(3700));
        assert_eq!(mgr.sweep_expired(), 1, "exactly the aged file must go");
        assert!(!old_rec.path.exists(), "aged file must be swept");
        assert!(fresh_rec.path.exists(), "fresh file must survive");
        let _ = std::fs::remove_dir_all(mgr.base_dir());
    }

    /// test_ttl_boundary_fresh: a file just under the TTL survives the sweep.
    #[test]
    fn test_ttl_boundary_fresh() {
        let mgr = test_manager("ttl-boundary");
        let rec = mgr.store("region", "doc|2|150", 3, b"DATA").expect("store");
        age_file(&rec.path, Duration::from_secs(3500));
        assert_eq!(mgr.sweep_expired(), 0, "near-TTL file must survive");
        assert!(rec.path.exists());
        let _ = std::fs::remove_dir_all(mgr.base_dir());
    }

    /// test_startup_sweep_with_aged_files: sweep_all removes everything
    /// regardless of age (server-restart semantics); non-png files are kept.
    #[test]
    fn test_startup_sweep_with_aged_files() {
        let mgr = test_manager("startup");
        let r1 = mgr.store("render", "a|1|150", 1, b"1").expect("store 1");
        let r2 = mgr.store("annot_m", "a|1|150", 2, b"2").expect("store 2");
        age_file(&r1.path, Duration::from_secs(7200));
        age_file(&r2.path, Duration::from_secs(10));
        let keep = mgr.base_dir().join("README.txt");
        std::fs::write(&keep, b"keep me").expect("write non-png");
        assert_eq!(mgr.sweep_all(), 2, "both pngs must go");
        assert!(!r1.path.exists());
        assert!(!r2.path.exists());
        assert!(keep.exists(), "non-png files must be left alone");
        let _ = std::fs::remove_dir_all(mgr.base_dir());
    }

    /// test_sweep_ignores_non_png: the TTL sweep never touches non-png files.
    #[test]
    fn test_sweep_ignores_non_png() {
        let mgr = test_manager("nonpng");
        let keep = mgr.base_dir().join("notes.json");
        std::fs::write(&keep, b"{}").expect("write json");
        assert_eq!(mgr.sweep_expired(), 0);
        assert_eq!(mgr.sweep_all(), 0);
        assert!(keep.exists());
        let _ = std::fs::remove_dir_all(mgr.base_dir());
    }
}
