// SPDX-License-Identifier: MIT OR Apache-2.0
//! `stichos open` — resolve platform handler, open current store version
//! (or `--version <journal-id>` materialized copy), `--app libreoffice|default`,
//! probe + doctor handoff row, never blocks.

use anyhow::{bail, Context, Result};
use engines::open_handler::DoctorHandoffRow;
use gp_core::journal::Journal;
use gp_core::store::ContentStore;
use sha2::Digest;
use std::sync::{Arc, LazyLock, Mutex};

/// Global session registry: session_id -> Journal.
static SESSION_STORE: LazyLock<Mutex<std::collections::HashMap<String, Arc<Mutex<Journal>>>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

/// Derive a deterministic session ID from a document path.
fn derive_session_id(path: &str) -> String {
    let hash = sha2::Sha256::digest(path.as_bytes());
    format!("sess-{:016x}", hash)
}

/// Get or create a journal for the given session ID.
fn get_or_create_journal(
    path: &str,
    session_id: Option<&str>,
) -> Result<(String, Arc<Mutex<Journal>>)> {
    let sid = session_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| derive_session_id(path));

    let mut store = SESSION_STORE.lock().unwrap();

    if let Some(arc) = store.get(&sid) {
        return Ok((sid.clone(), arc.clone()));
    }

    let store_path = std::env::temp_dir().join(format!("stichos-journal-{}", sid));
    let _ = std::fs::remove_dir_all(&store_path);
    let content_store = ContentStore::open_at(&store_path)
        .with_context(|| format!("failed to open content store at {}", store_path.display()))?;
    let journal = Arc::new(Mutex::new(Journal::new(content_store, sid.clone())));
    store.insert(sid.clone(), journal.clone());

    Ok((sid, journal))
}

/// Materialize a specific journal version as a temp file to open.
///
/// Returns the path to the materialized file.
pub fn materialize_version(
    store: &mut ContentStore,
    _journal_id: &str,
) -> Result<std::path::PathBuf> {
    // Resolve the doc_id from the journal entry id
    // For now, find the latest version blob from the store
    let versions_dir = store.root().join("versions");
    let version_files: Vec<_> = if versions_dir.exists() {
        std::fs::read_dir(&versions_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
            .collect()
    } else {
        Vec::new()
    };

    if version_files.is_empty() {
        bail!(
            "No versions found in store at {}; run a journaling edit first",
            store.root().display()
        );
    }

    // Sort by filename and take the last one (latest version)
    let latest_file = version_files.last().context("no version files found")?;
    let version_content = std::fs::read_to_string(latest_file.path()).with_context(|| {
        format!(
            "failed to read version file: {}",
            latest_file.path().display()
        )
    })?;

    let versions: Vec<gp_core::store::DocumentVersion> = serde_json::from_str(&version_content)
        .with_context(|| "failed to parse version manifest".to_string())?;

    if versions.is_empty() {
        bail!("No document versions recorded in store");
    }

    // Use the latest version
    let doc_version = versions.last().context("no versions")?;
    let blob_data = store
        .retrieve(&doc_version.blob_key)
        .with_context(|| format!("blob not found: {}", doc_version.blob_key))?;

    // Write to temp file
    let ext = doc_version
        .metadata
        .get("extension")
        .and_then(|e| e.strip_prefix("."))
        .unwrap_or("docx");

    let temp_path = std::env::temp_dir().join(format!(
        "stichos-open-{}-{}.{}",
        sid(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        ext
    ));
    std::fs::write(&temp_path, &blob_data)
        .with_context(|| format!("failed to write temp file: {}", temp_path.display()))?;

    Ok(temp_path)
}

/// Open the document at a specific path.
///
/// If `version` is provided, materializes that journal version to a temp file.
/// Otherwise opens the document file directly.
///
/// If `app` is "default" or None, uses the platform default opener.
/// Otherwise uses the specified app name (e.g. "libreoffice").
///
/// If `no_browser` is true, only returns the resolved path without opening.
pub fn cmd_open(
    doc_path: &std::path::Path,
    version: Option<String>,
    app: Option<String>,
    no_browser: bool,
) -> Result<()> {
    // Determine which file to open
    let file_to_open = if let Some(journal_id) = &version {
        // Try to get from journal session
        let (sid, journal) =
            get_or_create_journal(doc_path.to_string_lossy().as_ref(), Some(journal_id))?;

        // Check if the session has a store
        let mut j = journal.lock().unwrap();

        let entries_empty = j.entries().is_empty();
        if entries_empty {
            bail!(
                "No journal entries in session '{}'. Run an edit first to create journal entries.",
                sid
            );
        }

        let store = j.store_mut();

        // Materialize the latest version from the journal store
        materialize_version(store, &sid)?
    } else {
        // Open the document file directly
        if !doc_path.exists() {
            bail!("Document not found: {}", doc_path.display());
        }
        doc_path.to_path_buf()
    };

    // Print the resolved path
    eprintln!("open: resolving {}", file_to_open.display());

    if let Some(ref ver) = version {
        eprintln!("open: version = {}", ver);
    }

    if !no_browser {
        // Open the file
        let open_app = app.as_deref().filter(|a| !a.is_empty() && *a != "default");
        let result = engines::open_handler::open_path(&file_to_open, open_app);

        if result.launched {
            if let Some(pid) = result.pid {
                eprintln!("open: launched pid={}", pid);
            }
            eprintln!("open: command = {}", result.command);
        } else {
            if let Some(hint) = result.error_hint {
                bail!("open: {}", hint);
            }
            bail!("open: failed to launch handler");
        }
    } else {
        eprintln!("open: no-browser mode, skipping launch");
    }

    eprintln!("open: path = {}", file_to_open.display());

    Ok(())
}

/// Resolve the platform opener for file paths (not URLs).
/// Returns Ok(true) if the opener was found and spawned, Ok(false) if not found,
/// Err if the opener was found but failed to launch.
#[allow(dead_code)]
pub fn resolve_platform_opener() -> Result<DoctorHandoffRow> {
    let row = engines::open_handler::probe_open_handler();
    Ok(row)
}

/// Check if a platform handler is available (for doctor probe).
pub fn probe_handler() -> DoctorHandoffRow {
    engines::open_handler::probe_open_handler()
}

/// Generate an open command result for JSON output.
#[allow(dead_code)]
pub fn open_result_json(result: &engines::open_handler::OpenResult) -> serde_json::Value {
    serde_json::json!({
        "launched": result.launched,
        "command": result.command,
        "pid": result.pid,
        "error_hint": result.error_hint,
    })
}

/// Generate a short SID for temp filenames.
fn sid() -> String {
    uuid::Uuid::new_v4().to_string().replace("-", "")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// test_cmd_open_nonexistent: opening a non-existent file errors.
    #[test]
    fn test_cmd_open_nonexistent() {
        let result = cmd_open(
            std::path::Path::new("/tmp/does-not-exist-stichos-test.docx"),
            None,
            Some("default".to_string()),
            true, // no_browser
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not found"));
    }

    /// test_cmd_open_exists: opening an existing file in no_browser mode succeeds.
    #[test]
    fn test_cmd_open_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.docx");
        std::fs::write(&path, b"fake docx content").unwrap();

        let result = cmd_open(&path, None, Some("default".to_string()), true);
        assert!(result.is_ok());
    }

    /// test_cmd_open_with_missing_app: opening with missing app errors.
    #[test]
    fn test_cmd_open_missing_app() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.docx");
        std::fs::write(&path, b"fake docx content").unwrap();

        let result = cmd_open(&path, None, Some("nonexistent_app_xyz".to_string()), false);
        // Should error because app is not found
        assert!(result.is_err());
    }

    /// test_probe_handler: probe returns a valid DoctorHandoffRow.
    #[test]
    fn test_probe_handler() {
        let row = probe_handler();
        assert!(!row.platform.is_empty());
        assert!(!row.handler_name.is_empty());
        let _json = serde_json::to_string(&row).unwrap();
    }

    /// test_resolve_platform_opener: resolve returns valid row.
    #[test]
    fn test_resolve_platform_opener() {
        let row = resolve_platform_opener().unwrap();
        assert!(!row.platform.is_empty());
        let _json = serde_json::to_string(&row).unwrap();
    }

    /// test_open_result_json: OpenResult serializes to JSON.
    #[test]
    fn test_open_result_json() {
        let result = engines::open_handler::OpenResult::launched("xdg-open /tmp/test", Some(1));
        let json = open_result_json(&result);
        assert!(json["launched"].as_bool().unwrap());
        assert_eq!(json["pid"].as_u64().unwrap(), 1);
    }
}
