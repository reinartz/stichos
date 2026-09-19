// SPDX-License-Identifier: MIT OR Apache-2.0
//! IR (Intermediate Representation) types for stichos.
//!
//! The IR is a format-agnostic representation of documents, spreadsheets,
//! presentations, and PDFs. Each document family has its own top-level type.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

// ====================================================================
// Creation path types (P3-19: pandoc creation path)
// ====================================================================

/// The source of content for creation (write-direction).
///
/// Used by the creation surface to represent the input format
/// before conversion to a target document format (docx, odt, pdf).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CreateSource {
    /// Markdown input (CommonMark / pandoc markdown).
    Markdown {
        /// Raw markdown bytes.
        content: Vec<u8>,
    },
    /// HTML input (full HTML document or fragment).
    Html {
        /// Raw HTML bytes.
        content: Vec<u8>,
    },
    /// Typst input for PDF compilation.
    Typst {
        /// Raw typst source bytes.
        content: Vec<u8>,
    },
}

impl CreateSource {
    /// Return the source format name for pandoc/typst commands.
    pub fn format_name(&self) -> &'static str {
        match self {
            CreateSource::Markdown { .. } => "markdown",
            CreateSource::Html { .. } => "html",
            CreateSource::Typst { .. } => "typ",
        }
    }

    /// Return the content bytes.
    pub fn content(&self) -> &[u8] {
        match self {
            CreateSource::Markdown { content } => content,
            CreateSource::Html { content } => content,
            CreateSource::Typst { content } => content,
        }
    }

    /// Return true if this source can be processed by pandoc
    /// (markdown and html only; typst requires typst engine).
    pub fn is_pandoc_compatible(&self) -> bool {
        matches!(
            self,
            CreateSource::Markdown { .. } | CreateSource::Html { .. }
        )
    }

    /// Return true if this source can be processed by typst (typst only).
    pub fn is_typst_compatible(&self) -> bool {
        matches!(self, CreateSource::Typst { .. })
    }
}

/// The target output format for creation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CreateTarget {
    /// Microsoft Word document (.docx).
    Docx,
    /// OpenDocument Text (.odt).
    Odt,
    /// PDF document.
    Pdf,
}

impl CreateTarget {
    /// Return the pandoc target format string.
    pub fn pandoc_format(&self) -> &'static str {
        match self {
            CreateTarget::Docx => "docx",
            CreateTarget::Odt => "odt",
            CreateTarget::Pdf => "pdf",
        }
    }
}

/// Response from a creation operation.
///
/// Contains the created document bytes along with provenance metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreationResponse {
    /// Unique document identifier assigned by the store.
    pub document_id: String,
    /// The created document bytes.
    pub bytes: Vec<u8>,
    /// The output format.
    pub format: String,
    /// Which engine rendered the document.
    pub rendered_by: crate::fidelity::RendererInfo,
    /// The source that was converted.
    pub source: CreateSource,
}

/// Metadata about a creation operation for QA logging.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreationMetadata {
    /// The engine used for creation.
    pub engine: String,
    /// The pandoc/typst version string.
    pub engine_version: Option<String>,
    /// The source format name.
    pub source_format: String,
    /// The target format name.
    pub target_format: String,
    /// Whether a reference document (template) was used.
    pub has_reference_doc: bool,
}

impl CreationMetadata {
    /// Serialize to JSON bytes.
    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

/// The current IR schema version constant.
pub const IR_VERSION: &str = "1.0.0";

/// Regex pattern for valid element IDs: `id-` followed by 12+ hex chars.
pub const ELEMENT_ID_PATTERN: &str = r"^id-[0-9a-f]{12,}$";

/// A unique, content-anchored identifier for an IR element.
///
/// Format: `id-<first 12+ hex chars of SHA-256(kind + "/" + path + "/" + content_hash)>`
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ElementId(String);

impl ElementId {
    /// Derive a content-anchored ID from a kind, path, and content hash.
    ///
    /// # Arguments
    /// * `kind` — element kind, e.g. "paragraph", "table", "cell", "slide"
    /// * `path` — hierarchical path segments like ["section:0", "paragraph:1", "run:2"]
    /// * `content_hash` — SHA-256 hex of the element's canonical JSON serialization
    ///
    /// # ID derivation
    /// `sha256(kind + "/" + path.join("/") + "/" + content_hash)`, first 12 hex chars.
    pub fn content_anchored(kind: &str, path: &[&str], content_hash: &str) -> Self {
        let path_str = path.join("/");
        let seed = format!("{}/{}{}", kind, path_str, content_hash);
        let mut hasher = Sha256::new();
        hasher.update(seed.as_bytes());
        let digest = hasher.finalize();
        let hex = hex::encode(&*digest);
        // Use 12 hex chars = 48 bits (2^48 space, collision prob ~10^-13 for 1000 items)
        let prefix = &hex[..12];
        Self(format!("id-{}", prefix))
    }

    /// Convenience: serialize a value to JSON, hash it, and derive an ID.
    ///
    /// Equivalent to `content_anchored(kind, path, &sha256_hex(serde_json::to_string(value)))`.
    pub fn derive<T: Serialize>(kind: &str, path: &[&str], value: &T) -> Self {
        let json = serde_json::to_string(value).expect("ElementId::derive: serde failed");
        let mut hasher = Sha256::new();
        hasher.update(json.as_bytes());
        let content_hash = hex::encode(hasher.finalize());
        Self::content_anchored(kind, path, &content_hash)
    }

    /// Derive an ID directly from a precomputed SHA-256 hex content hash.
    ///
    /// Useful for PDF/text elements where the content hash is already available.
    pub fn from_content_hash(kind: &str, path: &[&str], content_hash_hex: &str) -> Self {
        Self::content_anchored(kind, path, content_hash_hex)
    }

    /// Generate a random ID (deprecated — for tests only).
    ///
    /// # Deprecation
    /// This method is deprecated because it produces non-deterministic IDs.
    /// Use `content_anchored` or `derive` instead.
    #[deprecated(
        since = "0.1.0",
        note = "Use content_anchored or derive instead — random IDs break read-stability"
    )]
    pub fn new() -> Self {
        eprintln!(
            "WARNING: ElementId::new() produces random (non-deterministic) IDs. \
             Use ElementId::content_anchored() or ElementId::derive() for read-stability."
        );
        let mut hasher = Sha256::new();
        let fake_kind = "deprecated-new";
        let fake_path = "random";
        let fake_hash = "000000000000";
        hasher.update(format!("{}/{}{}", fake_kind, fake_path, fake_hash).as_bytes());
        let hex = hex::encode(hasher.finalize());
        Self(format!("id-{}", &hex[..12]))
    }

    /// Return the raw string value of this ID.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Construct an ElementId from a raw string value.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        Self(s.to_string())
    }

    /// Validate that this ID matches the expected format.
    pub fn is_valid(&self) -> bool {
        // Simple check: starts with "id-" and rest are hex chars, at least 12 of them
        let s = &self.0;
        if !s.starts_with("id-") {
            return false;
        }
        let hex_part = &s[3..];
        hex_part.len() >= 12 && hex_part.chars().all(|c| c.is_ascii_hexdigit())
    }
}

impl Default for ElementId {
    fn default() -> Self {
        #[allow(deprecated)]
        Self::new()
    }
}

impl std::fmt::Display for ElementId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ====================================================================
// IdMap: maps ElementIds to their metadata
// ====================================================================

/// An entry in an IdMap, recording where an ID comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdEntry {
    /// The element kind, e.g. "paragraph", "table", "cell".
    pub kind: String,
    /// Hierarchical path segments, e.g. ["section:0", "paragraph:1"].
    pub path: Vec<String>,
    /// Page number if applicable, else None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
}

/// A map from ElementId to IdEntry metadata.
///
/// Use the `Visit` trait to populate this map from any IR document type.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IdMap {
    #[serde(rename = "elements")]
    inner: BTreeMap<String, IdEntry>,
}

impl IdMap {
    /// Create a new empty IdMap.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert an entry, returning the previous value if the key existed.
    pub fn insert(&mut self, id: ElementId, entry: IdEntry) -> Option<IdEntry> {
        self.inner.insert(id.0, entry)
    }

    /// Get an entry by element ID.
    pub fn get(&self, id: &ElementId) -> Option<&IdEntry> {
        self.inner.get(&id.0)
    }

    /// Get all keys in this map.
    pub fn keys(&self) -> Vec<ElementId> {
        self.inner.keys().map(|s| ElementId(s.clone())).collect()
    }

    /// Get the total number of entries.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if the map is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Return the inner BTreeMap for iteration.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &IdEntry)> {
        self.inner.iter()
    }

    /// Return the inner BTreeSet of all element kinds.
    pub fn kinds(&self) -> BTreeSet<String> {
        self.inner.values().map(|e| e.kind.clone()).collect()
    }
}

/// Trait for IR types that can produce an IdMap via a Visit traversal.
pub trait Visit {
    /// Populate `map` with all element IDs found in this document.
    fn visit_ids(&self, map: &mut IdMap);

    /// Collect all IDs into a new IdMap.
    fn collect_ids(&self) -> IdMap {
        let mut map = IdMap::new();
        self.visit_ids(&mut map);
        map
    }
}

/// Derive a content-anchored ID for use in Visit implementations.
///
/// This is a convenience wrapper around `ElementId::content_anchored`.
pub fn element_id(kind: &str, path: &[&str], content: &str) -> ElementId {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let hash = hex::encode(hasher.finalize());
    ElementId::content_anchored(kind, path, &hash)
}

/// Convert &[&str] to Vec<String> for IdEntry.path.
#[allow(dead_code)]
fn to_strings(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|s| s.to_string()).collect()
}

// ====================================================================
// Document family
// ====================================================================

/// A complete document (Word-like).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub ir_version: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ir_version_migration: Option<String>,
    pub sections: Vec<Section>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub fields: Vec<Field>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub comments: Vec<Comment>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tracked_changes: Vec<TrackedChange>,
    /// Revisions tracked by this document (C11 — P4-03).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub revisions: Vec<Revision>,
    /// Optional reference to a template document ID. When set, the document
    /// may be created from this template, preserving style/theme/master parts
    /// byte-for-byte and replacing only content parts with IR-derived content.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub template: Option<String>,
}

/// A section of content within a document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub id: ElementId,
    pub kind: SectionKind,
    pub items: Vec<SectionItem>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub headers: Vec<Header>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub footers: Vec<Footer>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub footnotes: Vec<Footnote>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub endnotes: Vec<Endnote>,
}

/// The kind of section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionKind {
    Body,
    Header,
    Footer,
    Footnotes,
    Endnotes,
}

/// A header attached to a section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Header {
    pub id: ElementId,
    pub content: Vec<SectionItem>,
    pub is_even_page: bool,
}

/// A footer attached to a section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Footer {
    pub id: ElementId,
    pub content: Vec<SectionItem>,
    pub is_even_page: bool,
}

/// A footnote attached to a section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Footnote {
    pub id: ElementId,
    pub content: Vec<SectionItem>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reference_id: Option<String>,
}

/// An endnote attached to a section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Endnote {
    pub id: ElementId,
    pub content: Vec<SectionItem>,
}

/// A single item within a section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum SectionItem {
    Paragraph(Paragraph),
    Table(Table),
    Drawing(Drawing),
    List(List),
}

/// Numbering info for paragraphs with w:numPr.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NumberingInfo {
    pub num_id: usize,
    pub level: usize,
}

/// A paragraph consisting of runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Paragraph {
    pub id: ElementId,
    pub runs: Vec<Run>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub page_number: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub style_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub numbering_info: Option<NumberingInfo>,
}

/// A run of text with optional formatting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: ElementId,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub italic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub font_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub font_size_pt: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub underline: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub strikethrough: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub style_name: Option<String>,
}

/// A simple field (w:fldSimple) — page numbers, date, etc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldSimple {
    pub id: ElementId,
    pub instruction: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result: Option<String>,
}

/// A complex field boundary (w:fldChar with w:fldId).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldComplex {
    pub id: ElementId,
    pub fld_id: String,
    pub field_type: FieldCharType,
}

/// The type of complex field character.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldCharType {
    Begin,
    End,
    Separate,
    Dirty,
}

/// A field (simple or complex) within a section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Simple(FieldSimple),
    Complex(FieldComplex),
}

/// A comment attached to a document (from word/comments.xml).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub id: ElementId,
    /// The comment's numeric ID (w:comment/@w:id).
    pub comment_id: String,
    pub author: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub initials: Option<String>,
    pub content: Vec<SectionItem>,
}

/// A tracked change (inserted or deleted content).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackedChange {
    pub id: ElementId,
    pub kind: ChangeKind,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub change_id: Option<String>,
}

/// The kind of tracked change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Inserted,
    Deleted,
}

// ====================================================================
// Revision model (C11 — P4-03)
// ====================================================================

/// The kind of revision operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionKind {
    Insert,
    Delete,
    FormatChange,
}

/// A revision — the new model replacing the old TrackedChange.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Revision {
    /// Stable unique ID for this revision.
    pub id: ElementId,
    /// Kind of revision.
    pub kind: RevisionKind,
    /// Author of the revision.
    pub author: Option<String>,
    /// Timestamp (ISO-8601).
    pub date: Option<String>,
    /// The element this revision is anchored to (C2 stable ID).
    pub anchor_element_id: ElementId,
    /// Page hint where this revision appears (derivable, optional).
    pub page_hint: Option<usize>,
    /// The original content (for Delete) or new content (for Insert).
    pub content: String,
    /// For format changes, the property that changed (e.g. "bold", "font_size").
    pub property_name: Option<String>,
    /// The original value of the changed property.
    pub original_value: Option<String>,
    /// The new value of the changed property.
    pub new_value: Option<String>,
}

/// The set of revision operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RevisionOp {
    Accept { revision_id: ElementId },
    Reject { revision_id: ElementId },
    AcceptAll,
    RejectAll,
}

// ====================================================================
// Review Session model (ADR-0007 — P4-11)
// ====================================================================

/// An audit trail entry for a review session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Timestamp of the audit event.
    pub timestamp: String,
    /// The action performed: "create", "accept", "reject", "comment", "close".
    pub action: String,
    /// Additional details about the action.
    pub details: serde_json::Value,
}

/// A review session — scoped context for a single human reviewer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewSession {
    /// Unique session token (stichos-review-<64 hex chars>).
    pub token: String,
    /// Path to the document this session reviews.
    pub doc_path: String,
    /// SHA-256 hex digest of document bytes at session creation.
    pub doc_version_hash: String,
    /// When the session was created.
    pub created_at: String,
    /// Optional expiration time (None = no expiry).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Snapshot of revisions at session creation time.
    pub revision_snapshot: Vec<serde_json::Value>,
    /// Audit trail of actions within this session.
    pub audit_trail: Vec<AuditEntry>,
    /// Comment drafts created during this session.
    pub comment_drafts: Vec<serde_json::Value>,
    /// Review mode: "docx-revisions" or "pdf-annot".
    #[serde(default = "default_mode")]
    pub mode: String,
    /// SHA-256 hex digest of the original document bytes for write-back operations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_bytes_hash: Option<String>,
    /// Whether the text layer is enabled for this session (pdf.js text extraction).
    #[serde(default)]
    pub text_layer: bool,
}

fn default_mode() -> String {
    "docx-revisions".to_string()
}

impl ReviewSession {
    /// Create a new review session with the given parameters.
    pub fn new(
        token: String,
        doc_path: String,
        doc_version_hash: String,
        revision_snapshot: Vec<serde_json::Value>,
    ) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        ReviewSession {
            token,
            doc_path,
            doc_version_hash,
            created_at: now.clone(),
            expires_at: None,
            revision_snapshot,
            audit_trail: vec![AuditEntry {
                timestamp: now,
                action: "create".to_string(),
                details: serde_json::json!({}),
            }],
            comment_drafts: vec![],
            mode: "docx-revisions".to_string(),
            doc_bytes_hash: None,
            text_layer: false,
        }
    }

    /// Create a new review session with mode and doc_bytes_hash.
    pub fn new_with_mode(
        token: String,
        doc_path: String,
        doc_version_hash: String,
        revision_snapshot: Vec<serde_json::Value>,
        mode: String,
        doc_bytes_hash: Option<String>,
    ) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        ReviewSession {
            token,
            doc_path,
            doc_version_hash,
            created_at: now.clone(),
            expires_at: None,
            revision_snapshot,
            audit_trail: vec![AuditEntry {
                timestamp: now,
                action: "create".to_string(),
                details: serde_json::json!({}),
            }],
            comment_drafts: vec![],
            mode,
            doc_bytes_hash,
            text_layer: false,
        }
    }

    /// Record an action in the audit trail.
    pub fn audit(&mut self, action: &str, details: serde_json::Value) {
        let now = chrono::Utc::now().to_rfc3339();
        self.audit_trail.push(AuditEntry {
            timestamp: now,
            action: action.to_string(),
            details,
        });
    }

    /// Check if the document version still matches (optimistic concurrency).
    pub fn version_matches(&self, current_hash: &str) -> bool {
        self.doc_version_hash == current_hash
    }
}

/// A reference to a comment in the document body (w:commentReference).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommentReference {
    pub id: ElementId,
    /// The numeric comment ID referenced (w:commentReference/@w:id).
    pub comment_id: String,
}

// Missing IR types to append to ir.rs

/// A table element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Table {
    pub id: ElementId,
    pub rows: Vec<Row>,
}

/// A row within a table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub cells: Vec<Cell>,
}

/// A single cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub id: ElementId,
    pub contents: Vec<SectionItem>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub colspan: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub rowspan: Option<usize>,
}

/// A drawing/image embedded in the document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Drawing {
    pub id: ElementId,
    pub mime_type: String,
    /// Base64-encoded image data.
    pub data: String,
    /// Optional bounding box in points (x, y, width, height).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bbox_pt: Option<[f64; 4]>,
}

/// An ordered or unordered list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct List {
    pub id: ElementId,
    pub kind: ListKind,
    pub items: Vec<ListItem>,
}

/// The kind of list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListKind {
    Bullet,
    Numbered,
    Other,
}

/// A single item within a list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListItem {
    pub id: ElementId,
    pub content: Vec<SectionItem>,
    pub level: usize,
}

// ====================================================================
// Spreadsheet family
// ====================================================================

/// A spreadsheet (Excel-like).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spreadsheet {
    pub ir_version: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ir_version_migration: Option<String>,
    pub sheets: Vec<Sheet>,
}

/// A single sheet in a spreadsheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sheet {
    pub id: ElementId,
    pub name: String,
    pub rows: Vec<SheetRow>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub merged_ranges: Vec<MergedRange>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub header_row: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub footer_row: Option<usize>,
}

/// A row in a spreadsheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetRow {
    pub id: ElementId,
    pub cells: Vec<SheetCell>,
}

/// A cell in a spreadsheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetCell {
    pub id: ElementId,
    pub row: usize,
    pub col: usize,
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub formula: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cached_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub style_name: Option<String>,
}

/// A range of merged cells in a spreadsheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergedRange {
    pub id: ElementId,
    pub start_row: usize,
    pub start_col: usize,
    pub end_row: usize,
    pub end_col: usize,
}

// ===== Visit impls for Spreadsheet types =====

impl Visit for Spreadsheet {
    fn visit_ids(&self, map: &mut IdMap) {
        map.insert(
            ElementId::content_anchored("spreadsheet", &["spreadsheet:0"], &self.ir_version),
            IdEntry {
                kind: "spreadsheet".to_string(),
                path: vec!["spreadsheet:0".to_string()],
                page: None,
            },
        );
        for (si, sheet) in self.sheets.iter().enumerate() {
            sheet.visit_ids(map);
            let sheet_path = format!("sheet:{}", si);
            map.insert(
                sheet.id.clone(),
                IdEntry {
                    kind: "sheet".to_string(),
                    path: vec![sheet_path],
                    page: Some(sheet.header_row.unwrap_or(1)),
                },
            );
        }
    }
}

impl Visit for Sheet {
    fn visit_ids(&self, map: &mut IdMap) {
        for (ri, row) in self.rows.iter().enumerate() {
            row.visit_ids(map);
            let row_path = format!("row:{}", ri);
            map.insert(
                row.id.clone(),
                IdEntry {
                    kind: "sheet_row".to_string(),
                    path: vec![row_path],
                    page: None,
                },
            );
        }
        for (mi, m) in self.merged_ranges.iter().enumerate() {
            let merge_path = format!("merged:{}", mi);
            map.insert(
                m.id.clone(),
                IdEntry {
                    kind: "merged_range".to_string(),
                    path: vec![merge_path],
                    page: None,
                },
            );
        }
    }
}

impl Visit for SheetRow {
    fn visit_ids(&self, map: &mut IdMap) {
        for cell in &self.cells {
            cell.visit_ids(map);
        }
    }
}

impl Visit for SheetCell {
    fn visit_ids(&self, map: &mut IdMap) {
        let _ = map;
        let _ = self;
    }
}

impl Visit for MergedRange {
    fn visit_ids(&self, map: &mut IdMap) {
        let _ = map;
        let _ = self;
    }
}

// ===== Presentation family =====
// ====================================================================

/// A presentation (PowerPoint-like).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Presentation {
    pub ir_version: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ir_version_migration: Option<String>,
    pub slides: Vec<Slide>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub layouts: Vec<Layout>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub masters: Vec<Master>,
}

/// A single slide in a presentation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slide {
    pub id: ElementId,
    pub elements: Vec<SlideElement>,
}

/// A single element on a slide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlideElement {
    pub id: ElementId,
    pub kind: String,
}

/// A layout definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub id: ElementId,
    pub name: String,
}

/// A master slide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Master {
    pub id: ElementId,
    pub name: String,
}

// ====================================================================
// PDF family
// ====================================================================

/// A PDF document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfDocument {
    pub id: ElementId,
    pub pages: Vec<PdfPage>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub metadata: Option<PdfMetadata>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub annotations: Vec<PdfAnnotation>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub outlines: Vec<PdfOutlineItem>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub form_fields: Vec<PdfFormField>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tagged_structure: Vec<TaggedNode>,
    /// Signature information detected during parsing (C13 — P5-11).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub signatures: Vec<SignatureInfo>,
}

/// Signature information detected in a PDF document (C13 — P5-11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignatureInfo {
    /// Signature type: "pkcs7", "office-xml", "pdf-adobe", "pdf-pades", or "unknown".
    pub sig_type: String,
    /// Human-readable location description.
    pub location: String,
    /// Confidence 0.0..=1.0 of detection certainty.
    pub confidence: f64,
    /// Optional: X.509 subject CN (if certificate is parseable).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub subject: Option<String>,
    /// Optional: digest algorithm OID string (e.g. "2.16.840.1.101.3.4.2.1" for SHA-256).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub digest_algorithm: Option<String>,
    /// Optional: signature start byte offset in document.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub byte_offset: Option<usize>,
    /// Optional: signature end byte offset (byte-range end).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub byte_end: Option<usize>,
}

/// Metadata for a PDF document (including XMP-derived fields).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub creator: Option<String>,
    pub description: Option<String>,
    pub modified: Option<String>,
    pub keywords: Option<String>,
}

/// A single page in a PDF.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfPage {
    pub id: ElementId,
    pub number: usize,
    pub width_pt: f64,
    pub height_pt: f64,
    pub text_with_bbox: Vec<TextWithBbox>,
}

/// Text with bounding box in a PDF page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextWithBbox {
    pub id: ElementId,
    pub text: String,
    pub bbox: [f64; 4],
}

/// Annotation kinds supported by stichos PDF annotations.
///
/// Maps to PDF /Subtype values where applicable. Unknown subtypes
/// are preserved in `annotation_type` and `kind` remains `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationKind {
    Highlight,
    Text,
    FreeText,
    Stamp,
    Line,
    Square,
    Circle,
    Underline,
    StrikeOut,
    Squiggly,
    Caret,
    Ink,
    Popup,
    FileAttachment,
    Sound,
    Movie,
    Widget,
    Screen,
    PrinterMark,
    TrapNet,
    Watermark,
}

/// Annotation review state for the stichos review workflow.
///
/// Default state on construction is `Applied`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnnotationState {
    /// The annotation is active / under review.
    Applied,
    /// The annotation has been resolved.
    Resolved,
    /// The annotation has been rejected.
    Rejected,
}

/// A PDF annotation (links, text notes, highlights, stamps, etc.).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfAnnotation {
    pub id: ElementId,
    pub annotation_type: String,
    pub page_number: usize,
    pub rect: Option<[f64; 4]>,
    pub contents: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub kind: Option<AnnotationKind>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub quote: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub state: Option<AnnotationState>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub resolved: Option<bool>,
}

impl PdfAnnotation {
    /// Return `true` if this annotation's kind is supported for processing.
    ///
    /// `None` kind is treated as supported (raw subtype in `annotation_type`).
    pub fn is_supported_kind(&self) -> bool {
        matches!(
            self.kind,
            None | Some(AnnotationKind::Highlight)
                | Some(AnnotationKind::Text)
                | Some(AnnotationKind::FreeText)
                | Some(AnnotationKind::Stamp)
        )
    }

    /// Transition this annotation to a new state.
    pub fn transition_state(&mut self, new_state: AnnotationState) {
        self.state = Some(new_state);
    }
}

/// An outline item in a PDF outline tree (bookmarks / TOC).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfOutlineItem {
    pub id: ElementId,
    pub title: String,
    pub page_number: usize,
    pub dest: Option<String>,
    pub action: Option<String>,
    pub children: Vec<PdfOutlineItem>,
}

/// Flag bits for PDF form fields (PDF 1.7 Table 222).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PdfFormFieldFlags {
    pub readonly: bool,
    pub required: bool,
    pub noexport: bool,
}

impl PdfFormFieldFlags {
    pub fn from_bits(bits: u32) -> Self {
        PdfFormFieldFlags {
            readonly: bits & 0x1 != 0,
            required: bits & 0x2 != 0,
            noexport: bits & 0x4 != 0,
        }
    }
    pub fn to_bits(&self) -> u32 {
        let mut bits: u32 = 0;
        if self.readonly {
            bits |= 0x1;
        }
        if self.required {
            bits |= 0x2;
        }
        if self.noexport {
            bits |= 0x4;
        }
        bits
    }
}

/// A PDF form field parsed from AcroForm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfFormField {
    pub id: ElementId,
    pub field_type: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub flags: Option<PdfFormFieldFlags>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub rect: Option<[f64; 4]>,
}

// ====================================================================
// Tagged tree (for tagged PDF)
// ====================================================================

/// A tagged node in a PDF tag tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaggedNode {
    pub id: ElementId,
    pub tag: String,
    pub alt_text: Option<String>,
    /// Bounding box in points `[left, top, right, bottom]` when known.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub bbox: Option<[f64; 4]>,
    pub children: Vec<TaggedNode>,
}

/// A tagged IR document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IRDocument {
    pub ir_version: String,
    pub root: TaggedNode,
}

// ====================================================================
// Visit impls for IRDocument and document family
// ====================================================================

/// Visit impl for IRDocument — collect IDs from the tagged tree.
impl Visit for IRDocument {
    fn visit_ids(&self, map: &mut IdMap) {
        // Add the document root
        map.insert(
            self.root.id.clone(),
            IdEntry {
                kind: self.root.tag.clone(),
                path: vec!["document".to_string()],
                page: None,
            },
        );
        // Recurse into children
        visit_tagged_node(&self.root, map);
    }
}

fn visit_tagged_node(node: &TaggedNode, map: &mut IdMap) {
    for child in &node.children {
        map.insert(
            child.id.clone(),
            IdEntry {
                kind: child.tag.clone(),
                path: vec![child.tag.clone()],
                page: None,
            },
        );
        visit_tagged_node(child, map);
    }
}

/// Visit impl for Document.
impl Visit for Document {
    fn visit_ids(&self, map: &mut IdMap) {
        map.insert(
            ElementId::derive("document", &["document"], &self.ir_version),
            IdEntry {
                kind: "Document".to_string(),
                path: vec!["document".to_string()],
                page: None,
            },
        );
        for (i, section) in self.sections.iter().enumerate() {
            visit_section(section, i, map);
        }
    }
}

fn visit_section(section: &Section, section_idx: usize, map: &mut IdMap) {
    let section_path = format!("section:{}", section_idx);
    map.insert(
        section.id.clone(),
        IdEntry {
            kind: "Section".to_string(),
            path: vec![section_path.clone()],
            page: None,
        },
    );
    for (j, item) in section.items.iter().enumerate() {
        visit_section_item(item, format!("{}/item:{}", section_path, j).as_str(), map);
    }
}

fn visit_section_item(item: &SectionItem, path: &str, map: &mut IdMap) {
    match item {
        SectionItem::Paragraph(p) => {
            map.insert(
                p.id.clone(),
                IdEntry {
                    kind: "Paragraph".to_string(),
                    path: vec![path.to_string()],
                    page: p.page_number,
                },
            );
            for (j, run) in p.runs.iter().enumerate() {
                map.insert(
                    run.id.clone(),
                    IdEntry {
                        kind: "Run".to_string(),
                        path: vec![format!("{}/run:{}", path, j)],
                        page: None,
                    },
                );
            }
        }
        SectionItem::Table(t) => {
            map.insert(
                t.id.clone(),
                IdEntry {
                    kind: "Table".to_string(),
                    path: vec![path.to_string()],
                    page: None,
                },
            );
            for (ri, row) in t.rows.iter().enumerate() {
                for (ci, cell) in row.cells.iter().enumerate() {
                    map.insert(
                        cell.id.clone(),
                        IdEntry {
                            kind: "Cell".to_string(),
                            path: vec![format!("{}/row:{}:cell:{}", path, ri, ci)],
                            page: None,
                        },
                    );
                }
            }
        }
        SectionItem::Drawing(d) => {
            map.insert(
                d.id.clone(),
                IdEntry {
                    kind: "Drawing".to_string(),
                    path: vec![path.to_string()],
                    page: None,
                },
            );
        }
        SectionItem::List(l) => {
            map.insert(
                l.id.clone(),
                IdEntry {
                    kind: "List".to_string(),
                    path: vec![path.to_string()],
                    page: None,
                },
            );
            for (li, list_item) in l.items.iter().enumerate() {
                map.insert(
                    list_item.id.clone(),
                    IdEntry {
                        kind: "ListItem".to_string(),
                        path: vec![format!("{}/item:{}", path, li)],
                        page: None,
                    },
                );
            }
        }
    }
}

/// Visit impl for PdfDocument.
impl Visit for PdfDocument {
    fn visit_ids(&self, map: &mut IdMap) {
        map.insert(
            self.id.clone(),
            IdEntry {
                kind: "PdfDocument".to_string(),
                path: vec!["pdf_document".to_string()],
                page: None,
            },
        );
        for page in self.pages.iter() {
            let page_path = format!("page:{}", page.number);
            map.insert(
                page.id.clone(),
                IdEntry {
                    kind: "PdfPage".to_string(),
                    path: vec![page_path.clone()],
                    page: Some(page.number),
                },
            );
            for twb in &page.text_with_bbox {
                map.insert(
                    twb.id.clone(),
                    IdEntry {
                        kind: "TextWithBbox".to_string(),
                        path: vec![format!("{}/text:{}", page_path, twb.id.as_str())],
                        page: Some(page.number),
                    },
                );
            }
        }
        for (i, annot) in self.annotations.iter().enumerate() {
            let annot_path = format!("annotation:{}", i);
            map.insert(
                annot.id.clone(),
                IdEntry {
                    kind: "PdfAnnotation".to_string(),
                    path: vec![annot_path],
                    page: Some(annot.page_number),
                },
            );
        }
        for outline in &self.outlines {
            visit_outline_item(outline, map);
        }
    }
}

fn visit_outline_item(item: &PdfOutlineItem, map: &mut IdMap) {
    map.insert(
        item.id.clone(),
        IdEntry {
            kind: "PdfOutline".to_string(),
            path: vec![format!("outline:{}", item.title)],
            page: Some(item.page_number),
        },
    );
    for child in &item.children {
        visit_outline_item(child, map);
    }
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// test_basic_document — create a minimal Document and verify IR version.
    #[test]
    fn test_basic_document() {
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "empty"),
                kind: SectionKind::Body,
                items: vec![],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            revisions: vec![],
            template: None,
        };
        assert_eq!(doc.ir_version, IR_VERSION);
        assert_eq!(doc.sections.len(), 1);
    }

    /// test_paragraph_with_run — create a paragraph with a run and verify.
    #[test]
    fn test_paragraph_with_run() {
        let run = Run {
            id: ElementId::content_anchored("run", &["run:0"], "hello"),
            text: "hello".into(),
            bold: Some(true),
            italic: None,
            font_name: None,
            font_size_pt: None,
            underline: None,
            strikethrough: None,
            style_name: None,
        };
        let para = Paragraph {
            id: ElementId::content_anchored("paragraph", &["para:0"], &run.text),
            runs: vec![run],
            page_number: None,
            style_name: None,
            numbering_info: None,
        };
        assert_eq!(para.runs.len(), 1);
        assert_eq!(para.runs[0].text, "hello");
    }

    /// test_table_with_cells — create a table with rows and cells.
    #[test]
    fn test_table_with_cells() {
        let cell = Cell {
            id: ElementId::content_anchored("cell", &["cell:0"], "data"),
            contents: vec![],
            colspan: None,
            rowspan: None,
        };
        let row = Row { cells: vec![cell] };
        let table = Table {
            id: ElementId::content_anchored("table", &["table:0"], ""),
            rows: vec![row],
        };
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].cells.len(), 1);
    }

    /// test_list_with_items — create a list with numbered items.
    #[test]
    fn test_list_with_items() {
        let item = ListItem {
            id: ElementId::content_anchored("listitem", &["item:0"], ""),
            content: vec![],
            level: 0,
        };
        let list = List {
            id: ElementId::content_anchored("list", &["list:0"], ""),
            kind: ListKind::Numbered,
            items: vec![item],
        };
        assert_eq!(list.kind, ListKind::Numbered);
        assert_eq!(list.items.len(), 1);
    }

    /// test_numbering_info — create a paragraph with numbering info.
    #[test]
    fn test_numbering_info() {
        let para = Paragraph {
            id: ElementId::content_anchored("paragraph", &["para:0"], ""),
            runs: vec![],
            page_number: None,
            style_name: Some("List Paragraph".to_string()),
            numbering_info: Some(NumberingInfo {
                num_id: 1,
                level: 0,
            }),
        };
        assert_eq!(para.numbering_info.as_ref().unwrap().num_id, 1);
        assert_eq!(para.numbering_info.as_ref().unwrap().level, 0);
    }

    /// test_serde_roundtrip_document — serialize and deserialize a Document.
    #[test]
    fn test_serde_roundtrip_document() {
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "test"),
                kind: SectionKind::Body,
                items: vec![SectionItem::Paragraph(Paragraph {
                    id: ElementId::content_anchored("paragraph", &["para:0"], "test"),
                    runs: vec![Run {
                        id: ElementId::content_anchored("run", &["run:0"], "hello"),
                        text: "hello".into(),
                        bold: Some(true),
                        italic: None,
                        font_name: None,
                        font_size_pt: None,
                        underline: None,
                        strikethrough: None,
                        style_name: Some("Emphasis".to_string()),
                    }],
                    page_number: None,
                    style_name: Some("Heading1".to_string()),
                    numbering_info: None,
                })],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            revisions: vec![],
            template: None,
        };

        let json = serde_json::to_string(&doc).expect("serialize");
        let doc2: Document = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(doc.ir_version, doc2.ir_version);
        assert_eq!(doc.sections.len(), doc2.sections.len());
    }

    /// test_section_item_kinds — verify all SectionItem variants serialize.
    #[test]
    fn test_section_item_kinds() {
        let para = SectionItem::Paragraph(Paragraph {
            id: ElementId::content_anchored("paragraph", &["p0"], ""),
            runs: vec![],
            page_number: None,
            style_name: None,
            numbering_info: None,
        });
        let json = serde_json::to_string(&para).expect("serialize");
        eprintln!("Paragraph JSON: {}", json);
        assert!(json.contains("\"type\":\"paragraph\""), "JSON: {}", json);

        let table = SectionItem::Table(Table {
            id: ElementId::content_anchored("table", &["t0"], ""),
            rows: vec![Row { cells: vec![] }],
        });
        let json = serde_json::to_string(&table).expect("serialize");
        assert!(json.contains("\"type\":\"table\""));

        let list = SectionItem::List(List {
            id: ElementId::content_anchored("list", &["l0"], ""),
            kind: ListKind::Numbered,
            items: vec![],
        });
        let json = serde_json::to_string(&list).expect("serialize");
        assert!(json.contains("\"type\":\"list\""));
    }

    /// test_list_kind_serde — verify ListKind serialization.
    #[test]
    fn test_list_kind_serde() {
        let json = serde_json::to_string(&ListKind::Bullet).unwrap();
        assert_eq!(json, "\"bullet\"");
        let json = serde_json::to_string(&ListKind::Numbered).unwrap();
        assert_eq!(json, "\"numbered\"");
        let json = serde_json::to_string(&ListKind::Other).unwrap();
        assert_eq!(json, "\"other\"");
    }

    /// test_text_with_bbox — verify TextWithBbox type exists and serializes.
    #[test]
    fn test_text_with_bbox() {
        let twb = TextWithBbox {
            id: ElementId::content_anchored("text", &["text:0"], "hello"),
            text: "hello".into(),
            bbox: [0.0, 0.0, 100.0, 20.0],
        };
        let json = serde_json::to_string(&twb).expect("serialize");
        assert!(json.contains("hello"));
    }

    /// test_id_generation — verify ElementId derivation is deterministic.
    #[test]
    fn test_id_generation() {
        let id1 = ElementId::content_anchored("test", &["a", "b"], "hash1");
        let id2 = ElementId::content_anchored("test", &["a", "b"], "hash1");
        assert_eq!(id1, id2, "Same input should produce same ID");
        let id3 = ElementId::content_anchored("test", &["a", "b"], "hash2");
        assert_ne!(id1, id3, "Different content should produce different ID");
    }

    // ===== P1-12: Fields, Comments, Tracked Changes =====

    /// test_field_simple_serde — FieldSimple serializes with type tag.
    #[test]
    fn test_field_simple_serde() {
        let field = Field::Simple(FieldSimple {
            id: ElementId::content_anchored("field", &["field:0"], "PAGE"),
            instruction: "PAGE".to_string(),
            result: Some("1".to_string()),
        });
        let json = serde_json::to_string(&field).expect("serialize");
        assert!(json.contains("\"type\":\"simple\""));
        assert!(json.contains("\"instruction\":\"PAGE\""));
        assert!(json.contains("\"result\":\"1\""));
    }

    /// test_field_complex_serde — FieldComplex serializes with begin type.
    #[test]
    fn test_field_complex_serde() {
        let field = Field::Complex(FieldComplex {
            id: ElementId::content_anchored("field", &["field:0"], "begin"),
            fld_id: "12345678".to_string(),
            field_type: FieldCharType::Begin,
        });
        let json = serde_json::to_string(&field).expect("serialize");
        assert!(json.contains("\"type\":\"complex\""));
        assert!(json.contains("\"field_type\":\"begin\""));
        assert!(json.contains("\"fld_id\":\"12345678\""));
    }

    /// test_field_char_type_serde — all FieldCharType variants.
    #[test]
    fn test_field_char_type_serde() {
        let json = serde_json::to_string(&FieldCharType::Begin).unwrap();
        assert_eq!(json, "\"begin\"");
        let json = serde_json::to_string(&FieldCharType::End).unwrap();
        assert_eq!(json, "\"end\"");
        let json = serde_json::to_string(&FieldCharType::Separate).unwrap();
        assert_eq!(json, "\"separate\"");
        let json = serde_json::to_string(&FieldCharType::Dirty).unwrap();
        assert_eq!(json, "\"dirty\"");
    }

    /// test_change_kind_serde — ChangeKind variants.
    #[test]
    fn test_change_kind_serde() {
        let json = serde_json::to_string(&ChangeKind::Inserted).unwrap();
        assert_eq!(json, "\"inserted\"");
        let json = serde_json::to_string(&ChangeKind::Deleted).unwrap();
        assert_eq!(json, "\"deleted\"");
    }

    /// test_comment_serde — Comment serializes with content.
    #[test]
    fn test_comment_serde() {
        let comment = Comment {
            id: ElementId::content_anchored("comment", &["comment:0"], "0"),
            comment_id: "0".to_string(),
            author: "reviewer-a".to_string(),
            date: Some("2026-01-01T00:00:00Z".to_string()),
            initials: Some("A".to_string()),
            content: vec![],
        };
        let json = serde_json::to_string(&comment).expect("serialize");
        assert!(json.contains("\"author\":\"reviewer-a\""));
        assert!(json.contains("\"comment_id\":\"0\""));
        assert!(json.contains("\"date\":\"2026-01-01T00:00:00Z\""));
        assert!(json.contains("\"initials\":\"A\""));
    }

    /// test_tracked_change_serde — TrackedChange with Inserted kind.
    #[test]
    fn test_tracked_change_serde() {
        let change = TrackedChange {
            id: ElementId::content_anchored("trackedchange", &["change:0"], "inserted"),
            kind: ChangeKind::Inserted,
            content: "inserted text".to_string(),
            author: Some("author-a".to_string()),
            date: Some("2026-01-01T00:00:00Z".to_string()),
            change_id: Some("9001".to_string()),
        };
        let json = serde_json::to_string(&change).expect("serialize");
        assert!(json.contains("\"kind\":\"inserted\""));
        assert!(json.contains("\"content\":\"inserted text\""));
        assert!(json.contains("\"author\":\"author-a\""));
    }

    /// test_document_with_fields_comments_changes — Document struct has all new fields.
    #[test]
    fn test_document_with_fields_comments_changes() {
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "test"),
                kind: SectionKind::Body,
                items: vec![],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![Field::Simple(FieldSimple {
                id: ElementId::content_anchored("field", &["field:0"], "PAGE"),
                instruction: "PAGE".to_string(),
                result: Some("1".to_string()),
            })],
            comments: vec![Comment {
                id: ElementId::content_anchored("comment", &["comment:0"], "0"),
                comment_id: "0".to_string(),
                author: "bot-a".to_string(),
                date: Some("2026-01-01T00:00:00Z".to_string()),
                initials: Some("A".to_string()),
                content: vec![],
            }],
            tracked_changes: vec![TrackedChange {
                id: ElementId::content_anchored("trackedchange", &["change:0"], "ins"),
                kind: ChangeKind::Inserted,
                content: "added text".to_string(),
                author: Some("bot-a".to_string()),
                date: Some("2026-01-01T00:00:00Z".to_string()),
                change_id: Some("9001".to_string()),
            }],
            revisions: vec![],
            template: None,
        };
        assert_eq!(doc.fields.len(), 1);
        assert_eq!(doc.comments.len(), 1);
        assert_eq!(doc.tracked_changes.len(), 1);
        let json = serde_json::to_string(&doc).expect("serialize");
        assert!(json.contains("\"type\":\"simple\""));
        assert!(json.contains("\"author\":\"bot-a\""));
        assert!(json.contains("\"kind\":\"inserted\""));
    }

    /// test_document_empty_fields_comments_changes — empty vecs serialize correctly.
    #[test]
    fn test_document_empty_fields_comments_changes() {
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "empty"),
                kind: SectionKind::Body,
                items: vec![],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            revisions: vec![],
            template: None,
        };
        let json = serde_json::to_string(&doc).expect("serialize");
        // Empty vecs with skip_serializing_if are omitted from JSON
        assert!(json.contains("\"ir_version\""));
        assert!(json.contains("\"sections\""));
        // Deserialize and verify empty vecs
        let doc2: Document = serde_json::from_str(&json).expect("deserialize");
        assert!(doc2.fields.is_empty());
        assert!(doc2.comments.is_empty());
        assert!(doc2.tracked_changes.is_empty());
    }

    // ===== P4-03: Revision model tests =====

    /// test_revision_kind_serde — all three RevisionKind variants.
    #[test]
    fn test_revision_kind_serde() {
        let json = serde_json::to_string(&RevisionKind::Insert).unwrap();
        assert_eq!(json, "\"insert\"");
        let json = serde_json::to_string(&RevisionKind::Delete).unwrap();
        assert_eq!(json, "\"delete\"");
        let json = serde_json::to_string(&RevisionKind::FormatChange).unwrap();
        assert_eq!(json, "\"format_change\"");
    }

    /// test_revision_serde_roundtrip — create Revision, serialize, deserialize, verify.
    #[test]
    fn test_revision_serde_roundtrip() {
        let revision = Revision {
            id: ElementId::content_anchored("revision", &["rev:0"], "insert-test"),
            kind: RevisionKind::Insert,
            author: Some("reviewer-a".to_string()),
            date: Some("2026-09-16T12:00:00Z".to_string()),
            anchor_element_id: ElementId::content_anchored("run", &["run:0"], "hello"),
            page_hint: Some(1),
            content: "inserted text".to_string(),
            property_name: None,
            original_value: None,
            new_value: None,
        };
        let json = serde_json::to_string(&revision).expect("serialize");
        let rev2: Revision = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(revision.id, rev2.id);
        assert_eq!(revision.kind, rev2.kind);
        assert_eq!(revision.author, rev2.author);
        assert_eq!(revision.date, rev2.date);
        assert_eq!(revision.anchor_element_id, rev2.anchor_element_id);
        assert_eq!(revision.page_hint, rev2.page_hint);
        assert_eq!(revision.content, rev2.content);
        assert_eq!(revision.property_name, rev2.property_name);
        assert_eq!(revision.original_value, rev2.original_value);
        assert_eq!(revision.new_value, rev2.new_value);
    }

    /// test_revision_op_accept_serde — Accept op serializes with "op":"accept".
    #[test]
    fn test_revision_op_accept_serde() {
        let op = RevisionOp::Accept {
            revision_id: ElementId::content_anchored("revision", &["rev:0"], "test"),
        };
        let json = serde_json::to_string(&op).expect("serialize");
        assert!(json.contains("\"op\":\"accept\""));
        // ElementId serializes as the inner string directly
        assert!(json.contains("\"revision_id\":\"id-"));

        let op = RevisionOp::Reject {
            revision_id: ElementId::content_anchored("revision", &["rev:0"], "test"),
        };
        let json = serde_json::to_string(&op).expect("serialize");
        assert!(json.contains("\"op\":\"reject\""));

        let op = RevisionOp::AcceptAll;
        let json = serde_json::to_string(&op).expect("serialize");
        assert_eq!(json, "{\"op\":\"accept_all\"}");

        let op = RevisionOp::RejectAll;
        let json = serde_json::to_string(&op).expect("serialize");
        assert_eq!(json, "{\"op\":\"reject_all\"}");
    }

    /// test_document_with_revisions — Document with revisions field.
    #[test]
    fn test_document_with_revisions() {
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "test"),
                kind: SectionKind::Body,
                items: vec![],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            revisions: vec![Revision {
                id: ElementId::content_anchored("revision", &["rev:0"], "ins"),
                kind: RevisionKind::Insert,
                author: Some("author-a".to_string()),
                date: Some("2026-09-16T12:00:00Z".to_string()),
                anchor_element_id: ElementId::content_anchored("run", &["run:0"], "hello"),
                page_hint: Some(1),
                content: "inserted text".to_string(),
                property_name: None,
                original_value: None,
                new_value: None,
            }],
            template: None,
        };
        assert_eq!(doc.revisions.len(), 1);
        assert_eq!(doc.revisions[0].kind, RevisionKind::Insert);
        let json = serde_json::to_string(&doc).expect("serialize");
        assert!(json.contains("\"kind\":\"insert\""));
        assert!(json.contains("\"author\":\"author-a\""));
        assert!(json.contains("\"content\":\"inserted text\""));
        assert!(json.contains("\"anchor_element_id\""));
    }

    /// test_document_backward_compatible — old IR without revisions field deserializes (C1 additive).
    #[test]
    fn test_document_backward_compatible() {
        let old_json = r#"{
            "ir_version": "1.0.0",
            "sections": [{
                "id": "id-000000000000",
                "kind": "body",
                "items": []
            }]
        }"#;
        let doc: Document = serde_json::from_str(old_json).expect("deserialize old IR");
        assert_eq!(doc.ir_version, "1.0.0");
        assert_eq!(doc.sections.len(), 1);
        assert!(
            doc.revisions.is_empty(),
            "C1 additive: revisions defaults to empty vec"
        );
    }

    /// test_document_revisions_default_empty — empty revisions omitted from JSON,
    /// defaults to vec on deserialization.
    #[test]
    fn test_document_revisions_default_empty() {
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "empty"),
                kind: SectionKind::Body,
                items: vec![],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            revisions: vec![],
            template: None,
        };
        let json = serde_json::to_string(&doc).expect("serialize");
        assert!(
            !json.contains("\"revisions\""),
            "empty revisions should be omitted from JSON (skip_serializing_if)"
        );
        let doc2: Document = serde_json::from_str(&json).expect("deserialize");
        assert!(
            doc2.revisions.is_empty(),
            "revisions should default to empty vec on deserialization"
        );
    }

    // ===== P5-03: PDF annotation tests =====

    /// test_annotation_kind_serde — all AnnotationKind variants serialize to lowercase.
    #[test]
    fn test_annotation_kind_serde() {
        let json = serde_json::to_string(&AnnotationKind::Highlight).unwrap();
        assert_eq!(json, "\"highlight\"");
        let json = serde_json::to_string(&AnnotationKind::Text).unwrap();
        assert_eq!(json, "\"text\"");
        let json = serde_json::to_string(&AnnotationKind::FreeText).unwrap();
        assert_eq!(json, "\"free_text\"");
        let json = serde_json::to_string(&AnnotationKind::Stamp).unwrap();
        assert_eq!(json, "\"stamp\"");
        let json = serde_json::to_string(&AnnotationKind::Line).unwrap();
        assert_eq!(json, "\"line\"");
        let json = serde_json::to_string(&AnnotationKind::Square).unwrap();
        assert_eq!(json, "\"square\"");
        let json = serde_json::to_string(&AnnotationKind::Circle).unwrap();
        assert_eq!(json, "\"circle\"");
        let json = serde_json::to_string(&AnnotationKind::Underline).unwrap();
        assert_eq!(json, "\"underline\"");
        let json = serde_json::to_string(&AnnotationKind::StrikeOut).unwrap();
        assert_eq!(json, "\"strike_out\"");
        let json = serde_json::to_string(&AnnotationKind::Squiggly).unwrap();
        assert_eq!(json, "\"squiggly\"");
        let json = serde_json::to_string(&AnnotationKind::Caret).unwrap();
        assert_eq!(json, "\"caret\"");
        let json = serde_json::to_string(&AnnotationKind::Ink).unwrap();
        assert_eq!(json, "\"ink\"");
        let json = serde_json::to_string(&AnnotationKind::Popup).unwrap();
        assert_eq!(json, "\"popup\"");
        let json = serde_json::to_string(&AnnotationKind::FileAttachment).unwrap();
        assert_eq!(json, "\"file_attachment\"");
        let json = serde_json::to_string(&AnnotationKind::Sound).unwrap();
        assert_eq!(json, "\"sound\"");
        let json = serde_json::to_string(&AnnotationKind::Movie).unwrap();
        assert_eq!(json, "\"movie\"");
        let json = serde_json::to_string(&AnnotationKind::Widget).unwrap();
        assert_eq!(json, "\"widget\"");
        let json = serde_json::to_string(&AnnotationKind::Screen).unwrap();
        assert_eq!(json, "\"screen\"");
        let json = serde_json::to_string(&AnnotationKind::PrinterMark).unwrap();
        assert_eq!(json, "\"printer_mark\"");
        let json = serde_json::to_string(&AnnotationKind::TrapNet).unwrap();
        assert_eq!(json, "\"trap_net\"");
        let json = serde_json::to_string(&AnnotationKind::Watermark).unwrap();
        assert_eq!(json, "\"watermark\"");
    }

    /// test_annotation_state_serde — all AnnotationState variants.
    #[test]
    fn test_annotation_state_serde() {
        let json = serde_json::to_string(&AnnotationState::Applied).unwrap();
        assert_eq!(json, "\"applied\"");
        let json = serde_json::to_string(&AnnotationState::Resolved).unwrap();
        assert_eq!(json, "\"resolved\"");
        let json = serde_json::to_string(&AnnotationState::Rejected).unwrap();
        assert_eq!(json, "\"rejected\"");
    }

    /// test_annotation_full_roundtrip — create PdfAnnotation with all fields, roundtrip.
    #[test]
    fn test_annotation_full_roundtrip() {
        let annot = PdfAnnotation {
            id: ElementId::content_anchored("annotation", &["annot:0"], "highlight-test"),
            annotation_type: "Highlight".to_string(),
            page_number: 3,
            rect: Some([10.0, 20.0, 100.0, 30.0]),
            contents: Some("Review comment".to_string()),
            kind: Some(AnnotationKind::Highlight),
            author: Some("reviewer-a".to_string()),
            date: Some("2026-09-18T10:00:00Z".to_string()),
            quote: Some("highlighted text".to_string()),
            state: Some(AnnotationState::Applied),
            resolved: Some(false),
        };
        let json = serde_json::to_string(&annot).expect("serialize");
        let a2: PdfAnnotation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(annot.id, a2.id);
        assert_eq!(annot.annotation_type, a2.annotation_type);
        assert_eq!(annot.page_number, a2.page_number);
        assert_eq!(annot.rect, a2.rect);
        assert_eq!(annot.contents, a2.contents);
        assert_eq!(annot.kind, a2.kind);
        assert_eq!(annot.author, a2.author);
        assert_eq!(annot.date, a2.date);
        assert_eq!(annot.quote, a2.quote);
        assert_eq!(annot.state, a2.state);
        assert_eq!(annot.resolved, a2.resolved);
    }

    /// test_annotation_minimal_roundtrip — old-format annotation (new fields absent) roundtrips.
    #[test]
    fn test_annotation_minimal_roundtrip() {
        let annot = PdfAnnotation {
            id: ElementId::content_anchored("annotation", &["annot:0"], "minimal"),
            annotation_type: "Link".to_string(),
            page_number: 1,
            rect: None,
            contents: None,
            kind: None,
            author: None,
            date: None,
            quote: None,
            state: None,
            resolved: None,
        };
        let json = serde_json::to_string(&annot).expect("serialize");
        let a2: PdfAnnotation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(annot.id, a2.id);
        assert_eq!(annot.annotation_type, a2.annotation_type);
        assert!(a2.kind.is_none());
        assert!(a2.author.is_none());
        assert!(a2.date.is_none());
        assert!(a2.quote.is_none());
        assert!(a2.state.is_none());
        assert!(a2.resolved.is_none());
    }

    /// test_annotation_new_fields_omitted_when_none — optional fields absent from JSON.
    #[test]
    fn test_annotation_new_fields_omitted_when_none() {
        let annot = PdfAnnotation {
            id: ElementId::content_anchored("annotation", &["annot:0"], "minimal"),
            annotation_type: "Text".to_string(),
            page_number: 1,
            rect: None,
            contents: Some("note".to_string()),
            kind: None,
            author: None,
            date: None,
            quote: None,
            state: None,
            resolved: None,
        };
        let json = serde_json::to_string(&annot).expect("serialize");
        // Optional fields with None should be omitted from JSON
        assert!(!json.contains("\"kind\""));
        assert!(!json.contains("\"author\""));
        assert!(!json.contains("\"date\""));
        assert!(!json.contains("\"quote\""));
        assert!(!json.contains("\"state\""));
        assert!(!json.contains("\"resolved\""));
        // But contents is Some, so it should be present
        assert!(json.contains("\"contents\""));
        assert!(json.contains("\"note\""));
    }

    /// test_annotation_is_supported_kind — supported vs unsupported kinds.
    #[test]
    fn test_annotation_is_supported_kind() {
        let annot = PdfAnnotation {
            id: ElementId::content_anchored("annotation", &["annot:0"], "raw"),
            annotation_type: "Unknown".to_string(),
            page_number: 1,
            rect: None,
            contents: None,
            kind: None,
            author: None,
            date: None,
            quote: None,
            state: None,
            resolved: None,
        };
        // None kind is supported
        assert!(annot.is_supported_kind());

        // Supported kinds
        let mut highlight = annot.clone();
        highlight.kind = Some(AnnotationKind::Highlight);
        assert!(highlight.is_supported_kind());

        let mut text = annot.clone();
        text.kind = Some(AnnotationKind::Text);
        assert!(text.is_supported_kind());

        let mut freetext = annot.clone();
        freetext.kind = Some(AnnotationKind::FreeText);
        assert!(freetext.is_supported_kind());

        let mut stamp = annot.clone();
        stamp.kind = Some(AnnotationKind::Stamp);
        assert!(stamp.is_supported_kind());

        // Unsupported kinds
        let mut line = annot.clone();
        line.kind = Some(AnnotationKind::Line);
        assert!(!line.is_supported_kind());

        let mut circle = annot.clone();
        circle.kind = Some(AnnotationKind::Circle);
        assert!(!circle.is_supported_kind());

        let mut ink = annot.clone();
        ink.kind = Some(AnnotationKind::Ink);
        assert!(!ink.is_supported_kind());

        let mut widget = annot.clone();
        widget.kind = Some(AnnotationKind::Widget);
        assert!(!widget.is_supported_kind());
    }

    /// test_annotation_transition_state — state transitions.
    #[test]
    fn test_annotation_transition_state() {
        let mut annot = PdfAnnotation {
            id: ElementId::content_anchored("annotation", &["annot:0"], "state-test"),
            annotation_type: "Highlight".to_string(),
            page_number: 1,
            rect: None,
            contents: None,
            kind: Some(AnnotationKind::Highlight),
            author: None,
            date: None,
            quote: None,
            state: None,
            resolved: None,
        };
        assert!(annot.state.is_none());
        annot.transition_state(AnnotationState::Applied);
        assert_eq!(annot.state, Some(AnnotationState::Applied));
        annot.transition_state(AnnotationState::Resolved);
        assert_eq!(annot.state, Some(AnnotationState::Resolved));
        annot.transition_state(AnnotationState::Rejected);
        assert_eq!(annot.state, Some(AnnotationState::Rejected));
        annot.transition_state(AnnotationState::Applied);
        assert_eq!(annot.state, Some(AnnotationState::Applied));
    }

    /// test_annotation_backward_compatible — old IR without new fields deserializes (C1 additive).
    #[test]
    fn test_annotation_backward_compatible() {
        let old_json = r#"{
            "id": "id-000000000000",
            "annotation_type": "Highlight",
            "page_number": 1,
            "rect": [0, 0, 100, 20],
            "contents": "old annotation"
        }"#;
        let annot: PdfAnnotation =
            serde_json::from_str(old_json).expect("deserialize old IR annotation");
        assert_eq!(annot.annotation_type, "Highlight");
        assert_eq!(annot.page_number, 1);
        assert_eq!(annot.contents, Some("old annotation".to_string()));
        assert!(annot.kind.is_none(), "C1 additive: kind defaults to None");
        assert!(
            annot.author.is_none(),
            "C1 additive: author defaults to None"
        );
        assert!(annot.date.is_none(), "C1 additive: date defaults to None");
        assert!(annot.quote.is_none(), "C1 additive: quote defaults to None");
        assert!(annot.state.is_none(), "C1 additive: state defaults to None");
        assert!(
            annot.resolved.is_none(),
            "C1 additive: resolved defaults to None"
        );
    }
}
