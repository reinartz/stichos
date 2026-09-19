// SPDX-License-Identifier: MIT OR Apache-2.0
//! Review session store — in-memory map of active sessions.
//!
//! Thread-safe, single-owner store for review sessions.
//! Used by the HTTP review surface (P4-12) and the review verb (P4-11).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gp_core::ir::ReviewSession;

/// Thread-safe review session store.
pub type ReviewSessionStore = Arc<Mutex<HashMap<String, ReviewSession>>>;

/// Create a new empty review session store.
pub fn new_review_session_store() -> ReviewSessionStore {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Generate a cryptographically random review session token.
///
/// Format: `stichos-review-<64 hex chars>` (128 bits of entropy).
pub fn generate_review_token() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.gen()).collect();
    let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
    format!("stichos-review-{}", hex)
}

/// Compute a SHA-256 hex digest of raw bytes.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Create a new review session and return its token.
///
/// Computes the document version hash from the document bytes and
/// stores the session in the session store.
pub fn create_review_session(
    store: &ReviewSessionStore,
    doc_path: String,
    doc_bytes: &[u8],
    revision_snapshot: Vec<serde_json::Value>,
) -> String {
    create_review_session_with_text_layer(store, doc_path, doc_bytes, revision_snapshot, false)
}

/// Create a new review session and return its token.
///
/// Computes the document version hash from the document bytes and
/// stores the session in the session store.
pub fn create_review_session_with_text_layer(
    store: &ReviewSessionStore,
    doc_path: String,
    doc_bytes: &[u8],
    revision_snapshot: Vec<serde_json::Value>,
    text_layer: bool,
) -> String {
    let token = generate_review_token();
    let doc_version_hash = sha256_hex(doc_bytes);
    let mut session =
        ReviewSession::new(token.clone(), doc_path, doc_version_hash, revision_snapshot);
    session.text_layer = text_layer;
    let mut store = store.lock().unwrap();
    store.insert(token.clone(), session);
    token
}

/// Create a new review session with explicit mode and doc_bytes_hash.
///
/// Used for PDF annotation mode sessions.
pub fn create_review_session_with_mode(
    store: &ReviewSessionStore,
    doc_path: String,
    doc_bytes: &[u8],
    revision_snapshot: Vec<serde_json::Value>,
    mode: String,
    doc_bytes_hash: Option<String>,
) -> String {
    create_review_session_with_mode_and_text_layer(
        store,
        doc_path,
        doc_bytes,
        revision_snapshot,
        mode,
        doc_bytes_hash,
        false,
    )
}

/// Create a new review session with explicit mode, doc_bytes_hash, and text_layer opt-in.
pub fn create_review_session_with_mode_and_text_layer(
    store: &ReviewSessionStore,
    doc_path: String,
    doc_bytes: &[u8],
    revision_snapshot: Vec<serde_json::Value>,
    mode: String,
    doc_bytes_hash: Option<String>,
    text_layer: bool,
) -> String {
    let token = generate_review_token();
    let doc_version_hash = sha256_hex(doc_bytes);
    let mut session = ReviewSession::new_with_mode(
        token.clone(),
        doc_path,
        doc_version_hash,
        revision_snapshot,
        mode,
        doc_bytes_hash,
    );
    session.text_layer = text_layer;
    let mut store = store.lock().unwrap();
    store.insert(token.clone(), session);
    token
}

/// Get a review session by token.
pub fn get_review_session(store: &ReviewSessionStore, token: &str) -> Option<ReviewSession> {
    let store = store.lock().unwrap();
    store.get(token).cloned()
}

/// Update a review session (record audit entry, etc.).
pub fn update_review_session<F>(store: &ReviewSessionStore, token: &str, f: F) -> Result<(), String>
where
    F: FnOnce(&mut ReviewSession),
{
    let mut store = store.lock().unwrap();
    let session = store
        .get_mut(token)
        .ok_or_else(|| "session not found".to_string())?;
    f(session);
    Ok(())
}

/// Remove a review session (close).
pub fn remove_review_session(store: &ReviewSessionStore, token: &str) -> bool {
    let mut store = store.lock().unwrap();
    store.remove(token).is_some()
}

/// List all active session tokens.
pub fn list_review_sessions(store: &ReviewSessionStore) -> Vec<String> {
    let store = store.lock().unwrap();
    store.keys().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_review_token() {
        let token = generate_review_token();
        assert!(token.starts_with("stichos-review-"));
        assert_eq!(token.len(), "stichos-review-".len() + 64);
        // Verify hex chars
        let hex_part = &token["stichos-review-".len()..];
        assert!(hex::decode(hex_part).is_ok());
    }

    #[test]
    fn test_session_lifecycle() {
        let store = new_review_session_store();
        let doc_bytes = b"test document content";
        let token =
            create_review_session(&store, "/path/to/doc.docx".to_string(), doc_bytes, vec![]);

        // Verify session exists
        let session = get_review_session(&store, &token).expect("session should exist");
        assert_eq!(session.doc_path, "/path/to/doc.docx");
        assert_eq!(session.audit_trail.len(), 1);
        assert_eq!(session.audit_trail[0].action, "create");

        // Verify version hash
        let expected_hash = sha256_hex(doc_bytes);
        assert_eq!(session.doc_version_hash, expected_hash);

        // Test version check
        assert!(session.version_matches(&expected_hash));
        assert!(!session.version_matches("wrong-hash"));

        // Test update
        update_review_session(&store, &token, |s| {
            s.audit("accept", serde_json::json!({"revision_id": "id-abc123"}));
        })
        .unwrap();
        let session = get_review_session(&store, &token).unwrap();
        assert_eq!(session.audit_trail.len(), 2);
        assert_eq!(session.audit_trail[1].action, "accept");

        // Test remove
        assert!(remove_review_session(&store, &token));
        assert!(get_review_session(&store, &token).is_none());
    }

    #[test]
    fn test_session_not_found() {
        let store = new_review_session_store();
        assert!(get_review_session(&store, "nonexistent").is_none());
    }
}
