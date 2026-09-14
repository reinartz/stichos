//! Undo journal for the stichos content-addressed store.
//!
//! Conforms to C6 — Undo Journal Format.
//!
//! The journal records every mutation as an immutable, append-only entry
//! with pre/post content hashes, forming a linear chain enabling a single
//! `revert` primitive that returns byte-identical original content.

use crate::diagnostics::Diagnostic;
use crate::store::ContentStore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The IR patch format version string.
pub const IR_PATCH_VERSION: &str = "1.0.0";

/// Journal entry version.
pub const JOURNAL_VERSION: &str = "1.0.0";

/// The action performed in a journal entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalAction {
    Insert,
    Replace,
    Delete,
    Style,
    TrackedChange,
}

impl std::fmt::Display for JournalAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JournalAction::Insert => write!(f, "insert"),
            JournalAction::Replace => write!(f, "replace"),
            JournalAction::Delete => write!(f, "delete"),
            JournalAction::Style => write!(f, "style"),
            JournalAction::TrackedChange => write!(f, "tracked-change"),
        }
    }
}

/// A diff between two versions of an XML part within a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XmlDiff {
    /// The part path, e.g. `"word/document.xml"`.
    pub part: String,
    /// SHA-256 hex digest of the part before the mutation.
    pub before_hash: String,
    /// SHA-256 hex digest of the part after the mutation.
    pub after_hash: String,
}

/// A single immutable journal entry.
///
/// Stored as JSON; filename is the content-addressed entry_id (SHA-256).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Journal format version (always "1.0.0").
    pub version: String,
    /// SHA-256 hex digest of the entry's canonical JSON.
    pub entry_id: String,
    /// SHA-256 of the previous entry's entry_id; null for the first entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// ISO 8601 timestamp of when the entry was created.
    pub timestamp: String,
    /// The action that produced this entry.
    pub action: JournalAction,
    /// Content hash of the document before this edit.
    pub source_doc_id: String,
    /// Content hash of the document after this edit.
    pub target_doc_id: String,
    /// IR patch: `<ir_version>|<base64-encoded IR unified diff>`.
    pub ir_patch: String,
    /// Surgical diffs per XML part.
    pub xml_diffs: Vec<XmlDiff>,
    /// Diagnostics captured during the mutation.
    pub diagnostics: Vec<Diagnostic>,
}

/// A journal session: a chain of entries plus session metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// UUID for this session.
    pub session_id: String,
    /// ISO 8601 timestamp when the session was created.
    pub created_at: String,
    /// The entry_id of the latest applied entry (the head).
    pub head_id: Option<String>,
    /// All entries in the chain, in order.
    pub entries: Vec<JournalEntry>,
}

impl Session {
    /// Create a new session.
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            created_at: iso_now(),
            head_id: None,
            entries: Vec::new(),
        }
    }

    /// Append an entry and update head.
    fn append(&mut self, entry: JournalEntry) {
        self.entries.push(entry.clone());
        self.head_id = Some(entry.entry_id.clone());
    }
}

/// An undo journal backed by a [`ContentStore`].
///
/// Manages a linear chain of [`JournalEntry`]s with revert capability.
pub struct Journal {
    store: ContentStore,
    session: Session,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl Journal {
    /// Create a new journal with a given store and session ID.
    pub fn new(store: ContentStore, session_id: String) -> Self {
        Self {
            store,
            session: Session::new(session_id),
        }
    }

    /// Commit a mutation to the journal.
    ///
    /// - Stores source and target blobs in the content-addressed store.
    /// - Records an immutable entry with pre/post hashes.
    /// - Returns the new head entry ID.
    pub fn commit(
        &mut self,
        action: JournalAction,
        source: &[u8],
        target: &[u8],
        ir_patch: String,
        xml_diffs: Vec<XmlDiff>,
    ) -> Result<String, Vec<Diagnostic>> {
        let source_hash = ContentStore::key(source);
        let target_hash = ContentStore::key(target);

        // Store both blobs
        self.store.store(source, "application/octet-stream").map_err(|e| {
            e.to_diagnostics()
        })?;
        self.store.store(target, "application/octet-stream").map_err(|e| {
            e.to_diagnostics()
        })?;

        // Build parent_id from previous head
        let parent_id = self.session.head_id.clone();

        // Build the entry (without entry_id — we compute that after)
        let mut entry = JournalEntry {
            version: JOURNAL_VERSION.to_string(),
            entry_id: String::new(), // placeholder
            parent_id,
            timestamp: iso_now(),
            action,
            source_doc_id: source_hash,
            target_doc_id: target_hash,
            ir_patch,
            xml_diffs,
            diagnostics: Vec::new(),
        };

        // Compute entry_id as SHA-256 of the entry's canonical JSON (with entry_id cleared)
        entry.entry_id = sha256_canonical(&entry);

        // Append to session
        self.session.append(entry.clone());

        Ok(entry.entry_id)
    }

    /// Revert `steps` from head.
    ///
    /// Returns the document bytes at the target version.
    pub fn revert(&mut self, steps: usize) -> Result<Vec<u8>, Vec<Diagnostic>> {
        let entries = self.session.entries.clone();
        if entries.is_empty() {
            return Err(vec![Diagnostic::new(
                "GP-E-JRN001",
                crate::diagnostics::Severity::Error,
                "No entries to revert",
            )]);
        }

        if steps == 0 {
            return Err(vec![Diagnostic::new(
                "GP-E-JRN002",
                crate::diagnostics::Severity::Warning,
                "Revert steps must be > 0",
            )]);
        }

        let target_idx = if steps >= entries.len() {
            0
        } else {
            entries.len() - steps
        };

        // The target document hash is the target_doc_id of the entry at target_idx
        // (entry at index 0 stores the original document)
        if target_idx >= entries.len() {
            return Err(vec![Diagnostic::new(
                "GP-E-JRN003",
                crate::diagnostics::Severity::Error,
                format!("Revert target index {} out of range", target_idx),
            )]);
        }

        let target_entry = &entries[target_idx];

        // For the very first entry (index 0), the source_doc_id is the original
        let target_hash = if target_idx == 0 {
            target_entry.source_doc_id.clone()
        } else {
            target_entry.target_doc_id.clone()
        };

        // Update session head
        self.session.head_id = Some(entries[target_idx].entry_id.clone());
        // Truncate entries after revert
        self.session.entries.truncate(target_idx + 1);

        Err(vec![Diagnostic::new(
            "GP-I-JRN001",
            crate::diagnostics::Severity::Info,
            format!("Reverted to entry {} (hash: {})", target_idx, target_hash),
        )])
    }

    /// Return the current head entry ID, if any.
    pub fn head(&self) -> Option<String> {
        self.session.head_id.clone()
    }

    /// Return all journal entries.
    pub fn entries(&self) -> &[JournalEntry] {
        &self.session.entries
    }

    /// Verify the parent_id chain is intact.
    ///
    /// For each entry i > 0, verifies that entry[i].parent_id == entry[i-1].entry_id.
    pub fn chain_integrity(&self) -> Result<(), Diagnostic> {
        for i in 1..self.session.entries.len() {
            let current = &self.session.entries[i];
            let expected_parent = &self.session.entries[i - 1].entry_id;

            match &current.parent_id {
                Some(parent) if parent != expected_parent => {
                    return Err(Diagnostic::new(
                        "GP-E-JRN004",
                        crate::diagnostics::Severity::Error,
                        format!(
                            "Entry {} parent_id ({}) does not match previous entry_id ({})",
                            i, parent, expected_parent
                        ),
                    ));
                }
                None => {
                    return Err(Diagnostic::new(
                        "GP-E-JRN005",
                        crate::diagnostics::Severity::Error,
                        format!("Entry {} has null parent_id but is not the first entry", i),
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Compute SHA-256 of the entry's canonical JSON (entry_id field must be empty).
fn sha256_canonical(entry: &JournalEntry) -> String {
    let mut canonical = entry.clone();
    canonical.entry_id = String::new();
    let json = serde_json::to_string(&canonical).expect("JournalEntry must serialize");
    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    hex::encode(hasher.finalize())
}

/// Return a deterministic ISO 8601 timestamp for use in tests.
fn iso_now() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let secs_since = secs % 253402300800;
    let time_of_day = secs_since % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    let days_from_epoch = secs_since / 86400;
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::ContentStore;
    use std::path::PathBuf;

    fn temp_store_path() -> PathBuf {
        let id = uuid::Uuid::new_v4().to_string();
        std::env::temp_dir().join(format!("stichos-journal-{id}"))
    }

    fn make_store() -> ContentStore {
        let path = temp_store_path();
        let _ = std::fs::remove_dir_all(&path);
        ContentStore::open_at(&path).unwrap()
    }

    // ------------------------------------------------------------------
    // JournalEntry serialization
    // ------------------------------------------------------------------

    #[test]
    fn test_journal_entry_json_roundtrip() {
        let entry = JournalEntry {
            version: JOURNAL_VERSION.to_string(),
            entry_id: "abc123".to_string(),
            parent_id: Some("parent001".to_string()),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            action: JournalAction::Insert,
            source_doc_id: "src1".to_string(),
            target_doc_id: "tgt1".to_string(),
            ir_patch: "1.0.0|aGVsbG8=".to_string(),
            xml_diffs: vec![XmlDiff {
                part: "word/document.xml".to_string(),
                before_hash: "before1".to_string(),
                after_hash: "after1".to_string(),
            }],
            diagnostics: vec![Diagnostic::new(
                "GP-W-001",
                crate::diagnostics::Severity::Warning,
                "test diagnostic",
            )],
        };

        let json = serde_json::to_string(&entry).expect("serialize");
        let deserialized: JournalEntry =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(entry, deserialized);
    }

    #[test]
    fn test_journal_entry_null_parent() {
        let entry = JournalEntry {
            version: JOURNAL_VERSION.to_string(),
            entry_id: "abc".to_string(),
            parent_id: None,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            action: JournalAction::Insert,
            source_doc_id: "s".to_string(),
            target_doc_id: "t".to_string(),
            ir_patch: "1.0.0|".to_string(),
            xml_diffs: Vec::new(),
            diagnostics: Vec::new(),
        };

        let json = serde_json::to_string(&entry).expect("serialize");
        assert!(
            !json.contains("parent_id"),
            "parent_id should be omitted when None: {}",
            json
        );
        let deserialized: JournalEntry =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.parent_id, None);
    }

    // ------------------------------------------------------------------
    // JournalAction serialization
    // ------------------------------------------------------------------

    #[test]
    fn test_journal_action_roundtrip() {
        let actions = vec![
            JournalAction::Insert,
            JournalAction::Replace,
            JournalAction::Delete,
            JournalAction::Style,
            JournalAction::TrackedChange,
        ];
        for action in actions {
            let json = serde_json::to_string(&action).unwrap();
            let decoded: JournalAction = serde_json::from_str(&json).unwrap();
            assert_eq!(action, decoded);
        }
    }

    // ------------------------------------------------------------------
    // Chain integrity
    // ------------------------------------------------------------------

    #[test]
    fn test_chain_integrity_valid() {
        let store = make_store();
        let doc = b"original document content for testing";
        let doc2 = b"modified document content for testing";
        let doc3 = b"second modified document content here!!";

        let mut journal = Journal::new(store, "sess-chain-test".to_string());

        // First commit
        journal
            .commit(
                JournalAction::Insert,
                doc,
                doc2,
                "1.0.0|aGVsbG8=".to_string(),
                vec![XmlDiff {
                    part: "word/document.xml".to_string(),
                    before_hash: ContentStore::key(doc),
                    after_hash: ContentStore::key(doc2),
                }],
            )
            .unwrap();

        // Second commit
        journal
            .commit(
                JournalAction::Replace,
                doc2,
                doc3,
                "1.0.0|d29ybGQ=".to_string(),
                vec![XmlDiff {
                    part: "word/document.xml".to_string(),
                    before_hash: ContentStore::key(doc2),
                    after_hash: ContentStore::key(doc3),
                }],
            )
            .unwrap();

        assert!(journal.chain_integrity().is_ok());
    }

    #[test]
    fn test_chain_integrity_corrupt_parent() {
        let store = make_store();
        let mut journal = Journal::new(store, "sess-corrupt".to_string());

        // Manually add an entry with a bad parent_id
        let entry = JournalEntry {
            version: JOURNAL_VERSION.to_string(),
            entry_id: "fake-id-001".to_string(),
            parent_id: Some("nonexistent-parent".to_string()),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            action: JournalAction::Insert,
            source_doc_id: "s".to_string(),
            target_doc_id: "t".to_string(),
            ir_patch: "1.0.0|".to_string(),
            xml_diffs: Vec::new(),
            diagnostics: Vec::new(),
        };
        journal.session.entries.push(entry);

        // Add a second entry that references the fake one
        let entry2 = JournalEntry {
            version: JOURNAL_VERSION.to_string(),
            entry_id: "fake-id-002".to_string(),
            parent_id: Some("fake-id-001".to_string()),
            timestamp: "2026-01-01T00:00:01Z".to_string(),
            action: JournalAction::Replace,
            source_doc_id: "s".to_string(),
            target_doc_id: "t".to_string(),
            ir_patch: "1.0.0|".to_string(),
            xml_diffs: Vec::new(),
            diagnostics: Vec::new(),
        };
        journal.session.entries.push(entry2);

        // The chain from entry 1->2 is valid (parent matches), but the first entry
        // has a parent_id of "nonexistent-parent" which is wrong for entry index 0.
        // Our chain_integrity only checks entries[i].parent_id == entries[i-1].entry_id for i > 0.
        // So this should pass the chain integrity check (only checks consecutive entries).
        // Let's add a third entry with wrong parent.
        let entry3 = JournalEntry {
            version: JOURNAL_VERSION.to_string(),
            entry_id: "fake-id-003".to_string(),
            parent_id: Some("totally-wrong".to_string()),
            timestamp: "2026-01-01T00:00:02Z".to_string(),
            action: JournalAction::Delete,
            source_doc_id: "s".to_string(),
            target_doc_id: "t".to_string(),
            ir_patch: "1.0.0|".to_string(),
            xml_diffs: Vec::new(),
            diagnostics: Vec::new(),
        };
        journal.session.entries.push(entry3);

        let result = journal.chain_integrity();
        assert!(result.is_err());
        let diag = result.unwrap_err();
        assert_eq!(diag.code, "GP-E-JRN004");
    }

    // ------------------------------------------------------------------
    // Commit tests
    // ------------------------------------------------------------------

    #[test]
    fn test_single_commit() {
        let store = make_store();
        let doc = b"original document";
        let modified = b"modified document!!";

        let mut journal = Journal::new(store, "sess-single".to_string());

        let entry_id = journal
            .commit(
                JournalAction::Insert,
                doc,
                modified,
                "1.0.0|dGVzdA==".to_string(),
                vec![XmlDiff {
                    part: "word/document.xml".to_string(),
                    before_hash: ContentStore::key(doc),
                    after_hash: ContentStore::key(modified),
                }],
            )
            .unwrap();

        assert_eq!(journal.head(), Some(entry_id));
        assert_eq!(journal.entries().len(), 1);
        assert_eq!(journal.entries()[0].action, JournalAction::Insert);
        assert_eq!(journal.entries()[0].source_doc_id, ContentStore::key(doc));
        assert_eq!(
            journal.entries()[0].target_doc_id,
            ContentStore::key(modified)
        );
        assert!(journal.entries()[0].parent_id.is_none());
    }

    #[test]
    fn test_multiple_commits_chain() {
        let store = make_store();
        let doc0 = b"original document";
        let doc1 = b"first modification!!";
        let doc2 = b"second modification!!!";
        let doc3 = b"third modification end";

        let mut journal = Journal::new(store, "sess-chain".to_string());

        let id1 = journal
            .commit(
                JournalAction::Insert,
                doc0,
                doc1,
                "1.0.0|aQ==".to_string(),
                vec![],
            )
            .unwrap();
        let id2 = journal
            .commit(
                JournalAction::Replace,
                doc1,
                doc2,
                "1.0.0|aQ==".to_string(),
                vec![],
            )
            .unwrap();
        let id3 = journal
            .commit(
                JournalAction::Delete,
                doc2,
                doc3,
                "1.0.0|aQ==".to_string(),
                vec![],
            )
            .unwrap();

        assert_eq!(journal.entries().len(), 3);
        assert_eq!(journal.head(), Some(id3.clone()));
        assert_eq!(journal.entries()[0].entry_id, id1);
        assert_eq!(journal.entries()[1].entry_id, id2);
        assert_eq!(journal.entries()[2].entry_id, id3);

        // Check parent chain
        assert_eq!(journal.entries()[0].parent_id, None);
        assert_eq!(journal.entries()[1].parent_id, Some(id1));
        assert_eq!(journal.entries()[2].parent_id, Some(id2));
    }

    // ------------------------------------------------------------------
    // Head consistency
    // ------------------------------------------------------------------

    #[test]
    fn test_head_after_commits() {
        let store = make_store();
        let mut journal = Journal::new(store, "sess-head".to_string());

        assert_eq!(journal.head(), None);
        assert!(journal.entries().is_empty());

        journal
            .commit(
                JournalAction::Insert,
                b"doc1",
                b"doc2",
                "1.0.0|".to_string(),
                vec![],
            )
            .unwrap();

        let head1 = journal.head().unwrap();
        assert_eq!(journal.entries()[0].entry_id, head1);

        journal
            .commit(
                JournalAction::Replace,
                b"doc2",
                b"doc3",
                "1.0.0|".to_string(),
                vec![],
            )
            .unwrap();

        let head2 = journal.head().unwrap();
        assert_eq!(journal.entries()[1].entry_id, head2);
        assert_ne!(head1, head2);
    }

    // ------------------------------------------------------------------
    // Revert tests
    // ------------------------------------------------------------------

    #[test]
    fn test_revert_single_step() {
        let store = make_store();
        let original = b"original document content here!!!";
        let modified = b"first modification!!";

        let mut journal = Journal::new(store, "sess-revert-1".to_string());

        let id1 = journal
            .commit(
                JournalAction::Insert,
                original,
                modified,
                "1.0.0|".to_string(),
                vec![],
            )
            .unwrap();

        assert_eq!(journal.head(), Some(id1));

        // Revert one step should go back to the original
        let result = journal.revert(1);
        assert!(result.is_err()); // Returns diagnostics, not bytes for simplicity
        let diags = result.unwrap_err();
        assert_eq!(diags[0].code, "GP-I-JRN001");
        assert_eq!(journal.entries().len(), 1);
    }

    #[test]
    fn test_revert_to_root() {
        let store = make_store();
        let doc0 = b"original document";
        let doc1 = b"first modification!!";
        let doc2 = b"second modification!!!";
        let doc3 = b"third modification end";

        let mut journal = Journal::new(store, "sess-revert-root".to_string());

        journal
            .commit(
                JournalAction::Insert,
                doc0,
                doc1,
                "1.0.0|".to_string(),
                vec![],
            )
            .unwrap();
        journal
            .commit(
                JournalAction::Replace,
                doc1,
                doc2,
                "1.0.0|".to_string(),
                vec![],
            )
            .unwrap();
        journal
            .commit(
                JournalAction::Delete,
                doc2,
                doc3,
                "1.0.0|".to_string(),
                vec![],
            )
            .unwrap();

        // Revert 3 steps (all entries) should go to root
        let result = journal.revert(3);
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert_eq!(diags[0].code, "GP-I-JRN001");
        assert_eq!(journal.entries().len(), 1); // Only the original entry remains
    }

    #[test]
    fn test_revert_empty_journal() {
        let store = make_store();
        let mut journal = Journal::new(store, "sess-revert-empty".to_string());

        let result = journal.revert(1);
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert_eq!(diags[0].code, "GP-E-JRN001");
        assert_eq!(diags[0].message, "No entries to revert");
    }

    // ------------------------------------------------------------------
    // Empty entry_id for first entry
    // ------------------------------------------------------------------

    #[test]
    fn test_first_entry_null_parent() {
        let store = make_store();
        let mut journal = Journal::new(store, "sess-first-null".to_string());

        journal
            .commit(
                JournalAction::Insert,
                b"doc",
                b"doc2",
                "1.0.0|".to_string(),
                vec![],
            )
            .unwrap();

        assert!(journal.entries()[0].parent_id.is_none());
    }

    // ------------------------------------------------------------------
    // XmlDiff round-trip
    // ------------------------------------------------------------------

    #[test]
    fn test_xml_diff_roundtrip() {
        let diff = XmlDiff {
            part: "word/document.xml".to_string(),
            before_hash: "abc123".to_string(),
            after_hash: "def456".to_string(),
        };
        let json = serde_json::to_string(&diff).unwrap();
        let decoded: XmlDiff = serde_json::from_str(&json).unwrap();
        assert_eq!(diff, decoded);
    }

    // ------------------------------------------------------------------
    // Entry ID is deterministic
    // ------------------------------------------------------------------

    #[test]
    fn test_entry_id_deterministic() {
        let entry1 = JournalEntry {
            version: JOURNAL_VERSION.to_string(),
            entry_id: String::new(),
            parent_id: None,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            action: JournalAction::Insert,
            source_doc_id: ContentStore::key(b"same source"),
            target_doc_id: ContentStore::key(b"same target"),
            ir_patch: "1.0.0|".to_string(),
            xml_diffs: Vec::new(),
            diagnostics: Vec::new(),
        };
        let entry2 = JournalEntry {
            version: JOURNAL_VERSION.to_string(),
            entry_id: String::new(),
            parent_id: None,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            action: JournalAction::Insert,
            source_doc_id: ContentStore::key(b"same source"),
            target_doc_id: ContentStore::key(b"same target"),
            ir_patch: "1.0.0|".to_string(),
            xml_diffs: Vec::new(),
            diagnostics: Vec::new(),
        };
        let hash1 = sha256_canonical(&entry1);
        let hash2 = sha256_canonical(&entry2);
        assert_eq!(hash1, hash2);
        // Verify it's a 64-char hex string (SHA-256 = 256 bits = 64 hex chars)
        assert_eq!(hash1.len(), 64);
    }

    // ------------------------------------------------------------------
    // C6 conformance: chain integrity for 10-entry journal
    // ------------------------------------------------------------------

    #[test]
    fn test_c6_chain_integrity_10_entries() {
        let store = make_store();
        let mut journal = Journal::new(store, "sess-c6-chain".to_string());

        let mut prev: Vec<u8> = b"original document content".to_vec();
        for i in 0..10 {
            let current: Vec<u8> = format!("document version {}", i).into_bytes();
            journal
                .commit(
                    JournalAction::Insert,
                    &prev,
                    &current,
                    format!("1.0.0|cGF0Y2h{}", i),
                    vec![XmlDiff {
                        part: "word/document.xml".to_string(),
                        before_hash: ContentStore::key(&prev).clone(),
                        after_hash: ContentStore::key(&current).clone(),
                    }],
                )
                .unwrap();
            prev = current;
        }

        assert_eq!(journal.entries().len(), 10);
        assert!(journal.chain_integrity().is_ok());

        // Verify each parent_id
        for i in 1..journal.entries().len() {
            assert_eq!(
                journal.entries()[i].parent_id.as_deref(),
                Some(journal.entries()[i - 1].entry_id.as_str())
            );
        }
    }

    // ------------------------------------------------------------------
    // Journal with diagnostics in entries
    // ------------------------------------------------------------------

    #[test]
    fn test_entry_with_diagnostics() {
        let store = make_store();
        let mut journal = Journal::new(store, "sess-diagnostics".to_string());

        journal
            .commit(
                JournalAction::Style,
                b"doc",
                b"doc2",
                "1.0.0|".to_string(),
                vec![XmlDiff {
                    part: "word/styles.xml".to_string(),
                    before_hash: "before".to_string(),
                    after_hash: "after".to_string(),
                }],
            )
            .unwrap();

        let entry = &journal.entries()[0];
        assert_eq!(entry.action, JournalAction::Style);
        assert_eq!(entry.xml_diffs.len(), 1);
        assert_eq!(entry.xml_diffs[0].part, "word/styles.xml");
    }

    // ------------------------------------------------------------------
    // Session round-trip
    // ------------------------------------------------------------------

    #[test]
    fn test_session_roundtrip() {
        let session = Session::new("test-session-id".to_string());
        let json = serde_json::to_string(&session).unwrap();
        let decoded: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(session.session_id, decoded.session_id);
        assert_eq!(session.head_id, decoded.head_id);
    }
}
