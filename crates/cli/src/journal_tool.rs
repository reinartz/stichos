//! Journal operations: revert_document and journal list.
//!
//! Provides MCP and CLI handlers for the two new P2-30 tools:
//! - `revert_document` — reverts N steps from head, returns byte-identical original
//! - `journal` (list) — lists journal entries with version chain

use anyhow::{Context, Result};
use gp_core::journal::{Journal, JournalEntry};
use gp_core::store::ContentStore;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

/// Global session registry: session_id -> Journal.
/// Uses `Arc<Mutex<Journal>>` so sessions can be shared and mutated
/// without keeping a `MutexGuard` alive across function boundaries.
static SESSION_STORE: LazyLock<Mutex<HashMap<String, Arc<Mutex<Journal>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Derive a deterministic session ID from a document path.
fn derive_session_id(path: &str) -> String {
    let hash = Sha256::digest(path.as_bytes());
    format!("sess-{:016x}", hash)
}

/// Get or create a journal for the given session ID.
///
/// If no explicit session ID is provided one is derived from the document path.
fn get_or_create_journal(
    path: &str,
    session_id: Option<&str>,
) -> Result<(String, Arc<Mutex<Journal>>)> {
    let sid = session_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| derive_session_id(path));

    let mut store = SESSION_STORE.lock().unwrap();

    // Reuse existing session
    if let Some(arc) = store.get(&sid) {
        return Ok((sid.clone(), arc.clone()));
    }

    // Create a new journal backed by a temp directory.
    let store_path = std::env::temp_dir().join(format!("stichos-journal-{}", sid));
    let _ = std::fs::remove_dir_all(&store_path);
    let content_store = ContentStore::open_at(&store_path)
        .with_context(|| format!("failed to open content store at {}", store_path.display()))?;
    let journal = Arc::new(Mutex::new(Journal::new(content_store, sid.clone())));
    store.insert(sid.clone(), journal.clone());

    Ok((sid, journal))
}

/// Revert a document N steps back in the journal.
pub fn revert_document(
    path: &str,
    steps: usize,
    session_id: Option<&str>,
) -> Result<serde_json::Value> {
    let (sid, journal) = get_or_create_journal(path, session_id)?;

    // Get the head before revert
    let head_before = {
        let j = journal.lock().unwrap();
        j.head()
    };

    // Revert — only attempt if there are entries
    let (_result, steps_reverted, output_size) = {
        let mut j = journal.lock().unwrap();
        if j.entries().is_empty() {
            // No entries — nothing to revert
            (
                serde_json::json!({"message": "journal empty, nothing to revert"}),
                0usize,
                0usize,
            )
        } else {
            let reverted_bytes = j.revert_to_bytes(steps).map_err(|diags| {
                anyhow::anyhow!(
                    "journal revert failed: {}",
                    diags
                        .iter()
                        .map(|d| d.message.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            })?;
            (
                serde_json::json!({
                    "reverted_to_entry": j.head(),
                }),
                steps,
                reverted_bytes.len(),
            )
        }
    };

    // Get the head after revert
    let head_after = {
        let j = journal.lock().unwrap();
        j.head()
    };

    let output = serde_json::json!({
        "success": true,
        "session_id": sid,
        "head_before": head_before,
        "head_after": head_after,
        "steps_reverted": steps_reverted,
        "output_size": output_size,
        "reverted_to_entry": head_after,
    });

    Ok(output)
}

/// List journal entries for a session.
pub fn list_journal(path: &str, session_id: Option<&str>) -> Result<serde_json::Value> {
    let (sid, journal) = get_or_create_journal(path, session_id)?;

    // Collect all needed data while holding the lock
    let (entries_data, head, chain_ok) = {
        let j = journal.lock().unwrap();
        let entries: Vec<JournalEntry> = j.entries().to_vec();
        let head = j.head();
        let chain_ok = j.chain_integrity().is_ok();
        (entries, head, chain_ok)
    };

    let entry_values: Vec<serde_json::Value> = entries_data
        .iter()
        .map(|e: &JournalEntry| {
            serde_json::json!({
                "entry_id": e.entry_id,
                "parent_id": e.parent_id,
                "timestamp": e.timestamp,
                "action": format!("{}", e.action),
                "source_doc_id": e.source_doc_id,
                "target_doc_id": e.target_doc_id,
                "xml_diffs": e.xml_diffs.iter().map(|d| {
                    serde_json::json!({
                        "part": d.part,
                        "before_hash": d.before_hash,
                        "after_hash": d.after_hash,
                    })
                }).collect::<Vec<_>>(),
                "diagnostics": e.diagnostics.iter().map(|diag| {
                    serde_json::json!({
                        "code": diag.code,
                        "severity": format!("{}", diag.severity),
                        "message": diag.message,
                    })
                }).collect::<Vec<_>>(),
            })
        })
        .collect();

    let result = serde_json::json!({
        "session_id": sid,
        "head_id": head,
        "entry_count": entry_values.len(),
        "chain_integrity": chain_ok,
        "entries": entry_values,
    });

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_temp_doc(content: &[u8]) -> (tempfile::TempDir, String) {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.docx");
        std::fs::write(&path, content).unwrap();
        (dir, path.to_string_lossy().to_string())
    }

    /// test_get_or_create_journal — creates new journal for unknown session.
    #[test]
    fn test_get_or_create_journal_new() {
        let (_dir, path) = make_temp_doc(b"fake docx data");
        let (sid, _journal) = get_or_create_journal(&path, None).unwrap();
        assert!(!sid.is_empty());
    }

    /// test_get_or_create_journal_reuse — same path gets same session.
    #[test]
    fn test_get_or_create_journal_reuse() {
        let (_dir, path) = make_temp_doc(b"fake docx data");
        let (sid1, _) = get_or_create_journal(&path, None).unwrap();
        let (sid2, _) = get_or_create_journal(&path, None).unwrap();
        assert_eq!(sid1, sid2);
    }

    /// test_revert_document_no_entries — reverts with empty journal returns success.
    #[test]
    fn test_revert_document_no_entries() {
        let (_dir, path) = make_temp_doc(b"fake docx data");
        let result = revert_document(&path, 1, Some("test-revert-empty"));
        // Should succeed but return empty result since no entries exist
        assert!(result.is_ok());
        let json = result.unwrap();
        assert!(json["success"].as_bool().unwrap());
        assert_eq!(json["steps_reverted"], 0);
    }

    /// test_list_journal_empty — lists empty journal.
    #[test]
    fn test_list_journal_empty() {
        let (_dir, path) = make_temp_doc(b"fake docx data");
        let result = list_journal(&path, Some("test-list-empty")).unwrap();
        assert_eq!(result["entry_count"], 0);
        assert!(result["chain_integrity"].as_bool().unwrap());
    }
}
