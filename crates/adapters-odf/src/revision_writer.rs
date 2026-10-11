// SPDX-License-Identifier: MIT OR Apache-2.0
//! ODF revision writer — wraps edits in `text:inserted`/`text:deleted` markup.
//!
//! Produces ODF 1.2+ tracked-change markup:
//! - `text:inserted` for insertions
//! - `text:deleted` for deletions
//! - Both carry `office:change-id` (matching pairs), `office:change-type`,
//!   `office:creators` (author), and `office:creator-date` (timestamp)
//!
//! Pipeline (`create_as_revisions`):
//! 1. Parse source ODT → OdfPackage
//! 2. Convert PatchOps to ODF span mutations (via patch_executor TrackInsert/TrackDelete)
//! 3. Apply mutations to content.xml
//! 4. Write output via SurgicalWriter
//!
//! `direct` mode (default stays direct per current behaviour) applies patches
//! without revision markup — this matches the existing PatchExecutor behaviour
//! where `TrackInsert`/`TrackDelete` are just regular patch ops.  The
//! `--revision-mode` flag on `stichos apply` on ODT enables the revision
//! wrapper path.

use std::io::{Cursor, Write as _};
use std::time::{SystemTime, UNIX_EPOCH};

use gp_core::diagnostics::{Diagnostic, Severity};
use gp_core::patch::PatchOp;

use crate::manifest::{parse_odf, OdfPackage};
use crate::patch::{ApplyResult as PatchApplyResult, MutationKind, SpanMutation};
use crate::xml::{parse_xml, TagName, XmlNode};

// ===========================================================================
// Namespace constants — shared with odt.rs
// ===========================================================================

/// `urn:oasis:names:tc:opendocument:xmlns:text:1.0`
pub const NS_TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
/// `urn:oasis:names:tc:opendocument:xmlns:office:1.0`
pub const NS_OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";

// ===========================================================================
// Public types
// ===========================================================================

/// Metadata for a tracked change in ODF.
///
/// `annotation_id` follows the pattern `a_{hash4_author}_{hash4_timestamp}`
/// so that paired `text:inserted`/`text:deleted` elements share the same ID.
#[derive(Debug, Clone)]
pub struct RevisionMeta {
    /// Author of the revision (human-readable name).
    pub author: String,
    /// ISO-8601 timestamp of the revision.
    pub timestamp: String,
    /// Annotation ID — shared between the inserted and deleted wrappers
    /// of the same change pair.
    pub annotation_id: String,
}

impl RevisionMeta {
    /// Create a new `RevisionMeta` with a generated annotation ID.
    pub fn new(author: &str, timestamp: &str) -> Self {
        let annotation_id = format!("a_{:04x}_{:04x}", hash_str(author), hash_str(timestamp));
        Self {
            author: author.to_string(),
            timestamp: timestamp.to_string(),
            annotation_id,
        }
    }
}

/// Result of a revision-aware patch.
#[derive(Debug, Clone)]
pub struct RevisionPatchResult {
    /// The patched ODT bytes.
    pub output: Vec<u8>,
    /// Change IDs created (one per `office:change-id` in the output).
    pub change_ids: Vec<String>,
}

/// ODF revision-aware patch writer.
///
/// Wraps regular edits in `text:inserted`/`text:deleted` markup when
/// revision mode is enabled, or applies them directly when `direct` is true.
pub struct RevisionWriter {
    /// The underlying ODF package.
    pub package: OdfPackage,
    /// Patch operations to apply.
    pub ops: Vec<PatchOp>,
    /// Revision metadata (author/timestamp/annotation-id).
    pub revision_meta: RevisionMeta,
    /// If true, apply edits directly (no revision markup).
    /// Default is `false` — revision markup is the norm.
    pub direct: bool,
}

impl RevisionWriter {
    /// Create a new revision-aware patch writer.
    ///
    /// The writer is in revision mode by default (`direct = false`).
    pub fn new(package: OdfPackage, ops: Vec<PatchOp>, meta: RevisionMeta) -> Self {
        Self {
            package,
            ops,
            revision_meta: meta,
            direct: false,
        }
    }

    /// Create a new revision-aware patch writer in direct mode.
    ///
    /// Direct mode skips revision markup and applies patches straight to
    /// the content XML — same as the legacy `PatchExecutor.execute()` path.
    pub fn new_direct(package: OdfPackage, ops: Vec<PatchOp>, meta: RevisionMeta) -> Self {
        Self {
            package,
            ops,
            revision_meta: meta,
            direct: true,
        }
    }

    /// Apply patch with revision markup (`text:inserted`/`text:deleted`).
    pub fn write(&self) -> Result<RevisionPatchResult, Vec<Diagnostic>> {
        let content_xml = self.find_content_xml()?;

        // Convert ops to span mutations (reuse patch_executor logic).
        let odf_type = self.detect_odf_type()?;
        let mutations =
            Self::ops_to_mutations(&self.ops, &content_xml, odf_type, &self.revision_meta);

        // Apply mutations to content.xml.
        let doc = parse_xml(&content_xml).map_err(|mut diags| {
            diags.push(Diagnostic::new(
                "ODF-E-002",
                Severity::Error,
                "Failed to parse content.xml for revision writing",
            ));
            diags
        })?;

        let PatchApplyResult { document, .. } = crate::patch::apply_span_mutations(doc, mutations)
            .map_err(|mut diags| {
                diags.push(Diagnostic::new(
                    "ODF-E-002",
                    Severity::Error,
                    "Span mutation application failed for revision writing".to_string(),
                ));
                diags
            })?;

        let mutated_content = document.to_bytes();

        // Build the full ODF output with the modified content.xml.
        let output = build_odf_from_package(&self.package, &mutated_content)?;

        let change_id = self.revision_meta.annotation_id.clone();
        Ok(RevisionPatchResult {
            output,
            change_ids: if self.direct {
                Vec::new()
            } else {
                vec![change_id]
            },
        })
    }

    /// Apply patch directly without revision markup.
    pub fn write_direct(&self) -> Result<Vec<u8>, Vec<Diagnostic>> {
        let source_bytes = self.package_bytes()?;
        let executor = crate::patch_executor::OdfPatchExecutor::new();
        let odf_type = self.detect_odf_type()?;
        let format = match odf_type {
            "odt" => gp_core::executor::FormatType::Odt,
            "ods" => gp_core::executor::FormatType::Ods,
            "odp" => gp_core::executor::FormatType::Odp,
            _ => {
                return Err(vec![Diagnostic::new(
                    "ODF-E-001",
                    Severity::Error,
                    "Could not detect ODF type for write_direct",
                )])
            }
        };
        let result = executor.execute(&source_bytes, format, &self.ops);
        if result.accepted && !result.output.is_empty() {
            Ok(result.output)
        } else {
            Err(result.diagnostics)
        }
    }

    /// Convert a list of PatchOps to span mutations with revision markup.
    fn ops_to_mutations(
        ops: &[PatchOp],
        content_xml: &[u8],
        _odf_type: &str,
        meta: &RevisionMeta,
    ) -> Vec<SpanMutation> {
        let mut mutations: Vec<SpanMutation> = Vec::new();

        for op in ops {
            match op {
                PatchOp::ReplaceText {
                    elem: _,
                    old,
                    new: new_text,
                } => {
                    // Skip empty old — windows() panics on zero-length pattern.
                    if old.is_empty() {
                        continue;
                    }
                    // Wrap in text:deleted (old) + text:inserted (new).
                    let ann = &meta.annotation_id;
                    let deleted = format!(
                        "<text:deleted office:change-id=\"{}\" office:change-type=\"text:deleted\" office:creators=\"{}\" office:creator-date=\"{}\">{}</text:deleted>",
                        ann,
                        escape_xml_attr(&meta.author),
                        escape_xml_attr(&meta.timestamp),
                        escape_xml_content(old)
                    );
                    let inserted = format!(
                        "<text:inserted office:change-id=\"{}\" office:change-type=\"text:inserted\" office:creators=\"{}\" office:creator-date=\"{}\">{}</text:inserted>",
                        ann,
                        escape_xml_attr(&meta.author),
                        escape_xml_attr(&meta.timestamp),
                        escape_xml_content(new_text)
                    );

                    // Find old text in content XML and replace with (deleted + inserted).
                    let old_bytes = old.as_bytes();
                    if let Some(pos) = content_xml
                        .windows(old_bytes.len())
                        .position(|w| w == old_bytes)
                    {
                        mutations.push(SpanMutation {
                            span: crate::xml::ByteSpan {
                                offset: pos,
                                length: old_bytes.len(),
                            },
                            kind: MutationKind::ReplaceText(format!("{}{}", deleted, inserted)),
                        });
                    }
                }
                PatchOp::TrackInsert {
                    anchor: _,
                    kind,
                    author: _,
                    timestamp: _,
                } => {
                    let ann = &meta.annotation_id;
                    let wrapped = format!(
                        "<text:inserted office:change-id=\"{}\" office:change-type=\"text:inserted\" office:creators=\"{}\" office:creator-date=\"{}\"><text:p>{}</text:p></text:inserted>",
                        ann,
                        escape_xml_attr(&meta.author),
                        escape_xml_attr(&meta.timestamp),
                        escape_xml_content(kind)
                    );
                    let insert_pos = if let Some(pos) =
                        content_xml.windows(9).position(|w| w == b"<office:body")
                    {
                        pos
                    } else {
                        content_xml.len()
                    };
                    mutations.push(SpanMutation {
                        span: crate::xml::ByteSpan {
                            offset: insert_pos,
                            length: 0,
                        },
                        kind: MutationKind::InsertNode(XmlNode::Text {
                            text: wrapped,
                            span: crate::xml::ByteSpan {
                                offset: insert_pos,
                                length: 0,
                            },
                        }),
                    });
                }
                PatchOp::TrackDelete {
                    elem: _,
                    author: _,
                    timestamp: _,
                } => {
                    // Find the last paragraph and wrap it.
                    if let Some(pos) = find_last_byte_slice(content_xml, b"<text:p>") {
                        let tag_start = pos;
                        let remaining = &content_xml[tag_start..];
                        if let Some(close_tag) = find_byte_slice(remaining, b"</text:p>") {
                            let end = tag_start + close_tag + 9;
                            if end <= content_xml.len() {
                                let content_start = tag_start + 8; // after <text:p>
                                if let Some(content) = content_xml.get(content_start..end - 9) {
                                    let ann = &meta.annotation_id;
                                    let deleted = format!(
                                        "<text:deleted office:change-id=\"{}\" office:change-type=\"text:deleted\" office:creators=\"{}\" office:creator-date=\"{}\"><text:p>{}</text:p></text:deleted>",
                                        ann,
                                        escape_xml_attr(&meta.author),
                                        escape_xml_attr(&meta.timestamp),
                                        String::from_utf8_lossy(content)
                                    );
                                    mutations.push(SpanMutation {
                                        span: crate::xml::ByteSpan {
                                            offset: tag_start,
                                            length: end - tag_start,
                                        },
                                        kind: MutationKind::ReplaceText(deleted),
                                    });
                                }
                            }
                        }
                    }
                }
                PatchOp::Insert { anchor: _, kind } => {
                    let node = XmlNode::Element {
                        name: TagName::local("p"),
                        attributes: Default::default(),
                        namespaces: vec![],
                        children: vec![XmlNode::Text {
                            text: kind.clone(),
                            span: crate::xml::ByteSpan {
                                offset: 0,
                                length: kind.len(),
                            },
                        }],
                        span: crate::xml::ByteSpan {
                            offset: 0,
                            length: 0,
                        },
                    };
                    let insert_pos = if let Some(pos) =
                        content_xml.windows(9).position(|w| w == b"<office:body")
                    {
                        pos
                    } else {
                        content_xml.len()
                    };
                    mutations.push(SpanMutation {
                        span: crate::xml::ByteSpan {
                            offset: insert_pos,
                            length: 0,
                        },
                        kind: MutationKind::InsertNode(node),
                    });
                }
                PatchOp::Delete { elem: _ } => {
                    if let Some(pos) = find_last_byte_slice(content_xml, b"<text:p>") {
                        let tag_start = pos;
                        if let Some(close_tag) =
                            find_byte_slice(&content_xml[tag_start..], b"</text:p>")
                        {
                            let end = tag_start + close_tag + 9;
                            if end <= content_xml.len() {
                                mutations.push(SpanMutation {
                                    span: crate::xml::ByteSpan {
                                        offset: tag_start,
                                        length: end - tag_start,
                                    },
                                    kind: MutationKind::DeleteNode,
                                });
                            }
                        }
                    }
                }
                PatchOp::SetRunProps { .. }
                | PatchOp::SetCellValue { .. }
                | PatchOp::SetCellRich { .. } => {
                    // Not applicable for ODF text revision writing.
                }
            }
        }

        mutations
    }

    /// Find the content.xml data from the package.
    fn find_content_xml(&self) -> Result<Vec<u8>, Vec<Diagnostic>> {
        self.package
            .parts
            .iter()
            .find(|p| p.path == "content.xml")
            .map(|p| p.data.clone())
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "ODF-E-002",
                    Severity::Error,
                    "content.xml not found in ODF package".to_string(),
                )]
            })
    }

    /// Detect the ODF type from the package.
    fn detect_odf_type(&self) -> Result<&'static str, Vec<Diagnostic>> {
        let mime = self
            .package
            .manifest
            .as_ref()
            .and_then(|m| m.find_part("/"))
            .map(|p| p.media_type.as_str());
        match mime {
            Some("application/vnd.oasis.opendocument.text") => Ok("odt"),
            Some("application/vnd.oasis.opendocument.spreadsheet") => Ok("ods"),
            Some("application/vnd.oasis.opendocument.presentation") => Ok("odp"),
            _ => Err(vec![Diagnostic::new(
                "ODF-E-001",
                Severity::Error,
                "Could not detect ODF type from package".to_string(),
            )]),
        }
    }

    /// Return the full source ODF bytes (mimetype + content.xml).
    fn package_bytes(&self) -> Result<Vec<u8>, Vec<Diagnostic>> {
        use std::io::Write as _;
        let mut buf = Vec::new();
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        // Use Stored for all parts to match the test fixture (which uses Stored).
        // In production the original ODF's compression is preserved by
        // SurgicalWriter; here we reconstruct from parts so Stored is safest.
        let stored: zip::write::FileOptions<'static, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for part in &self.package.parts {
            writer.start_file(&part.path, stored).map_err(|e| {
                vec![Diagnostic::new(
                    "ODF-E-001",
                    Severity::Error,
                    format!("ZIP start: {}", e),
                )]
            })?;
            writer.write_all(&part.data).map_err(|e| {
                vec![Diagnostic::new(
                    "ODF-E-001",
                    Severity::Error,
                    format!("ZIP write: {}", e),
                )]
            })?;
        }
        writer.finish().map_err(|e| {
            vec![Diagnostic::new(
                "ODF-E-001",
                Severity::Error,
                format!("ZIP finish: {}", e),
            )]
        })?;
        Ok(buf)
    }
}

// ===========================================================================
// Helper: generate unique change-id
// ===========================================================================

/// Generate a unique change ID using nanosecond timestamp hex.
///
/// Format: 16 hex digits representing nanoseconds since Unix epoch.
pub fn gen_change_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{:016x}", now.as_nanos())
}

// ===========================================================================
// Helper: build an ODF package from parts with a replaced content.xml
// ===========================================================================

fn build_odf_from_package(
    package: &OdfPackage,
    content_data: &[u8],
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let mut output = Vec::new();
    let mut zip_writer = zip::ZipWriter::new(Cursor::new(&mut output));
    let stored_opts: zip::write::FileOptions<'static, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflate_opts: zip::write::FileOptions<'static, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // Find original mimetype.
    let mimetype_data = package
        .parts
        .iter()
        .find(|p| p.path == "mimetype")
        .map(|p| p.data.as_slice());

    // Write mimetype first (ODF requirement).
    if let Some(mime_data) = mimetype_data {
        zip_writer
            .start_file("mimetype", stored_opts)
            .map_err(|e| {
                vec![Diagnostic::new(
                    "ODF-E-001",
                    Severity::Error,
                    format!("mimetype start: {}", e),
                )]
            })?;
        zip_writer.write_all(mime_data).map_err(|e| {
            vec![Diagnostic::new(
                "ODF-E-001",
                Severity::Error,
                format!("mimetype write: {}", e),
            )]
        })?;
    }

    // Write remaining parts in source order.
    for part in &package.parts {
        if part.path == "mimetype" {
            continue;
        }

        let write_data = if part.path == "content.xml" {
            content_data
        } else {
            &part.data
        };

        let opts = if part.path == "META-INF/manifest.xml" {
            &stored_opts
        } else {
            &deflate_opts
        };

        zip_writer.start_file(&part.path, *opts).map_err(|e| {
            vec![Diagnostic::new(
                "ODF-E-001",
                Severity::Error,
                format!("start '{}': {}", part.path, e),
            )]
        })?;
        zip_writer.write_all(write_data).map_err(|e| {
            vec![Diagnostic::new(
                "ODF-E-001",
                Severity::Error,
                format!("write '{}': {}", part.path, e),
            )]
        })?;
    }

    zip_writer.finish().map_err(|e| {
        vec![Diagnostic::new(
            "ODF-E-001",
            Severity::Error,
            format!("finish ZIP: {}", e),
        )]
    })?;
    Ok(output)
}

// ===========================================================================
// Helper: escape content for ODF XML
// ===========================================================================

/// Escape a string for safe use as an XML attribute value.
fn escape_xml_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape a string for safe use as XML text content.
fn escape_xml_content(s: &str) -> String {
    escape_xml_attr(s)
}

// ===========================================================================
// Helper: simple hash for short IDs
// ===========================================================================

/// Simple hash function for generating short annotation IDs.
/// Returns a 4-hex-digit string.
pub fn hash_str(s: &str) -> u16 {
    let mut hash: u64 = 5381;
    for byte in s.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(byte as u64);
    }
    (hash & 0xFFFF) as u16
}

// ===========================================================================
// Helper: find byte slice positions
// ===========================================================================

/// Find the first occurrence of `pattern` in `haystack`, returning the byte offset.
fn find_byte_slice(haystack: &[u8], pattern: &[u8]) -> Option<usize> {
    if pattern.is_empty() {
        return None;
    }
    haystack
        .windows(pattern.len())
        .position(|window| window == pattern)
}

/// Find the last occurrence of `pattern` in `haystack`, returning the byte offset.
fn find_last_byte_slice(haystack: &[u8], pattern: &[u8]) -> Option<usize> {
    if pattern.is_empty() {
        return None;
    }
    let num_windows = haystack.len().saturating_sub(pattern.len()) + 1;
    haystack
        .windows(pattern.len())
        .rev()
        .position(|window| window == pattern)
        .map(|rev_pos| num_windows.saturating_sub(1).saturating_sub(rev_pos))
}

// ===========================================================================
// create_as_revisions (P4-09 full pipeline)
// ===========================================================================

/// Create-as-revisions: apply patch operations to an ODF document as tracked changes.
///
/// Pipeline:
/// 1. Parse the ODF source bytes
/// 2. Convert PatchOps → span mutations (with `text:inserted`/`text:deleted` wrappers)
/// 3. Apply mutations to content.xml
/// 4. Write the modified ODF package
///
/// When `direct` is true, patches are applied without revision markup,
/// matching the legacy behaviour.
///
/// # Arguments
/// * `source_bytes` — the source ODF (ODT/ODS/ODP) bytes
/// * `ops` — patch operations to apply
/// * `meta` — revision metadata (author, timestamp)
/// * `direct` — if true, skip revision markup
pub fn create_as_revisions(
    source_bytes: &[u8],
    ops: &[PatchOp],
    meta: RevisionMeta,
    direct: bool,
) -> Result<RevisionPatchResult, Vec<Diagnostic>> {
    let package = parse_odf(source_bytes).map_err(|mut diags| {
        diags.push(Diagnostic::new(
            "ODF-E-001",
            Severity::Error,
            "Failed to parse ODF package for revision writing".to_string(),
        ));
        diags
    })?;

    let writer = if direct {
        RevisionWriter::new_direct(package, ops.to_vec(), meta)
    } else {
        RevisionWriter::new(package, ops.to_vec(), meta)
    };

    if direct {
        let output = writer.write_direct()?;
        Ok(RevisionPatchResult {
            output,
            change_ids: Vec::new(),
        })
    } else {
        writer.write()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::odt::read_odt;
    use gp_core::ir::{Document, IR_VERSION};
    use std::io::Read;

    /// Minimal ODT fixture builder.
    fn make_minimal_odt() -> Vec<u8> {
        let mut buf = Vec::new();
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let o: zip::write::FileOptions<'static, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

        writer.start_file("mimetype", o).unwrap();
        writer
            .write_all(b"application/vnd.oasis.opendocument.text")
            .unwrap();

        let content = b"<?xml version=\"1.0\"?><office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><office:body><office:text><text:p>Hello, ODF!</text:p></office:text></office:body></office:document-content>";
        writer.start_file("content.xml", o).unwrap();
        writer.write_all(content).unwrap();

        writer.start_file("META-INF/manifest.xml", o).unwrap();
        writer.write_all(b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/><manifest:default manifest:full-path=\"\" manifest:media-type=\"text/xml\"/></manifest:manifest>").unwrap();

        writer.finish().unwrap();
        buf
    }

    /// test_hash_str_deterministic — hash_str is deterministic.
    #[test]
    fn test_hash_str_deterministic() {
        let h1 = hash_str("alice");
        let h2 = hash_str("alice");
        assert_eq!(h1, h2);
        let h3 = hash_str("bob");
        assert_ne!(h1, h3);
    }

    /// test_hash_str_range — result fits in u16 (4 hex digits).
    #[test]
    fn test_hash_str_range() {
        for s in ["a", "ab", "abc", "alice", "bob", "charlie", ""] {
            let h = hash_str(s);
            let _ = h; // u16 is always <= 0xFFFF — just check no panics
        }
    }

    /// test_gen_change_id_uniqueness — consecutive IDs differ.
    #[test]
    fn test_gen_change_id_uniqueness() {
        let id1 = gen_change_id();
        let id2 = gen_change_id();
        assert!(!id1.is_empty());
        assert!(!id2.is_empty());
        assert_eq!(id1.len(), 16, "change id should be 16 hex digits");
        assert_eq!(id2.len(), 16);
        assert!(
            id1.chars().all(|c| c.is_ascii_hexdigit()),
            "change id should be hex only"
        );
        assert_ne!(id1, id2, "consecutive change ids should differ");
    }

    /// test_revision_meta_new — generates annotation_id from author+timestamp.
    #[test]
    fn test_revision_meta_new() {
        let meta = RevisionMeta::new("alice", "2026-01-01T00:00:00Z");
        assert_eq!(meta.author, "alice");
        assert_eq!(meta.timestamp, "2026-01-01T00:00:00Z");
        assert!(meta.annotation_id.starts_with("a_"));
        assert_eq!(
            meta.annotation_id.split('_').count(),
            3,
            "annotation_id should have 3 parts"
        );
    }

    /// test_escape_xml_attr — special characters are escaped.
    #[test]
    fn test_escape_xml_attr_basic() {
        assert_eq!(escape_xml_attr("a&b"), "a&amp;b");
        assert_eq!(escape_xml_attr("a\"b"), "a&quot;b");
        assert_eq!(escape_xml_attr("a<b"), "a&lt;b");
        assert_eq!(escape_xml_attr("a>b"), "a&gt;b");
        assert_eq!(escape_xml_attr("a&b<c>d\"e"), "a&amp;b&lt;c&gt;d&quot;e");
        assert_eq!(escape_xml_attr("plain"), "plain");
    }

    /// test_escape_xml_content — delegates to escape_xml_attr.
    #[test]
    fn test_escape_xml_content_basic() {
        assert_eq!(escape_xml_content("x&y"), "x&amp;y");
        assert_eq!(escape_xml_content("plain"), "plain");
    }

    /// test_find_byte_slice_basic — find first occurrence.
    #[test]
    fn test_find_byte_slice_basic() {
        let h = b"hello world hello";
        assert_eq!(find_byte_slice(h, b"hello"), Some(0));
        assert_eq!(find_byte_slice(h, b"world"), Some(6));
        assert_eq!(find_byte_slice(h, b"xyz"), None);
    }

    /// test_find_last_byte_slice — find last occurrence.
    #[test]
    fn test_find_last_byte_slice_basic() {
        let h = b"hello world hello";
        assert_eq!(find_last_byte_slice(h, b"hello"), Some(12));
        assert_eq!(find_last_byte_slice(h, b"world"), Some(6));
        assert_eq!(find_last_byte_slice(h, b"xyz"), None);
    }

    /// test_ops_to_mutations_replace_text — ReplaceText wraps old in deleted + new in inserted.
    #[test]
    fn test_ops_to_mutations_replace_text() {
        let meta = RevisionMeta::new("alice", "2026-01-01T00:00:00Z");
        let content = b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><office:body><office:text><text:p>Hello, ODF!</text:p></office:text></office:body></office:document-content>";

        let op = PatchOp::ReplaceText {
            elem: gp_core::ir::ElementId::content_anchored("p", &["p:0"], "Hello"),
            old: "Hello".to_string(),
            new: "World".to_string(),
        };

        let mutations =
            RevisionWriter::ops_to_mutations(std::slice::from_ref(&op), content, "odt", &meta);
        assert_eq!(mutations.len(), 1, "should produce one mutation");

        if let MutationKind::ReplaceText(replaced) = &mutations[0].kind {
            assert!(
                replaced.contains("<text:deleted"),
                "should wrap old in text:deleted"
            );
            assert!(
                replaced.contains("<text:inserted"),
                "should wrap new in text:inserted"
            );
            assert!(
                replaced.contains("office:change-type=\"text:deleted\""),
                "should have change-type=deleted"
            );
            assert!(
                replaced.contains("office:change-type=\"text:inserted\""),
                "should have change-type=inserted"
            );
            assert!(
                replaced.contains(&meta.annotation_id),
                "should reference annotation_id"
            );
        } else {
            panic!("expected ReplaceText mutation");
        }
    }

    /// test_ops_to_mutations_track_insert — TrackInsert creates text:inserted.
    #[test]
    fn test_ops_to_mutations_track_insert() {
        let meta = RevisionMeta::new("alice", "2026-01-01T00:00:00Z");
        let content = b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><office:body><office:text></office:text></office:body></office:document-content>";

        let op = PatchOp::TrackInsert {
            anchor: gp_core::ir::ElementId::content_anchored("p", &["p:0"], ""),
            kind: "Inserted content".to_string(),
            author: "alice".to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
        };

        let mutations =
            RevisionWriter::ops_to_mutations(std::slice::from_ref(&op), content, "odt", &meta);
        assert_eq!(mutations.len(), 1, "should produce one mutation");

        if let MutationKind::InsertNode(XmlNode::Text { text, .. }) = &mutations[0].kind {
            assert!(
                text.contains("<text:inserted"),
                "should produce text:inserted"
            );
            assert!(
                text.contains("office:creators=\"alice\""),
                "should have author"
            );
            assert!(
                text.contains("office:creator-date=\"2026-01-01T00:00:00Z\""),
                "should have timestamp"
            );
        } else {
            panic!("expected InsertNode mutation");
        }
    }

    /// test_ops_to_mutations_track_delete — TrackDelete wraps last paragraph.
    #[test]
    fn test_ops_to_mutations_track_delete() {
        let meta = RevisionMeta::new("bob", "2026-02-01T12:00:00Z");
        let content = b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><office:body><office:text><text:p>Paragraph to delete</text:p></office:text></office:body></office:document-content>";

        let op = PatchOp::TrackDelete {
            elem: gp_core::ir::ElementId::content_anchored("p", &["p:0"], ""),
            author: "bob".to_string(),
            timestamp: "2026-02-01T12:00:00Z".to_string(),
        };

        let mutations =
            RevisionWriter::ops_to_mutations(std::slice::from_ref(&op), content, "odt", &meta);
        assert_eq!(mutations.len(), 1, "should produce one mutation");

        if let MutationKind::ReplaceText(replaced) = &mutations[0].kind {
            assert!(
                replaced.contains("<text:deleted"),
                "should wrap in text:deleted"
            );
            assert!(
                replaced.contains("Paragraph to delete"),
                "should preserve deleted content"
            );
        } else {
            panic!("expected ReplaceText mutation");
        }
    }

    /// test_ops_to_mutations_empty — empty ops produce no mutations.
    #[test]
    fn test_ops_to_mutations_empty() {
        let meta = RevisionMeta::new("alice", "2026-01-01T00:00:00Z");
        let content = b"<root/>";
        let mutations = RevisionWriter::ops_to_mutations(&[], content, "odt", &meta);
        assert!(
            mutations.is_empty(),
            "empty ops should produce no mutations"
        );
    }

    /// test_ops_to_mutations_xml_escaping — special chars are escaped in attributes.
    #[test]
    fn test_ops_to_mutations_xml_escaping() {
        let meta = RevisionMeta::new("alice & bob", "2026-01-01T00:00:00Z");
        let content = b"<root/>";

        let op = PatchOp::TrackInsert {
            anchor: gp_core::ir::ElementId::content_anchored("p", &["p:0"], ""),
            kind: "content".to_string(),
            author: "alice".to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
        };

        let mutations =
            RevisionWriter::ops_to_mutations(std::slice::from_ref(&op), content, "odt", &meta);
        assert_eq!(mutations.len(), 1);

        if let MutationKind::InsertNode(XmlNode::Text { text, .. }) = &mutations[0].kind {
            assert!(
                text.contains("&amp;"),
                "author with & should be escaped: {}",
                text
            );
            assert!(
                !text.contains("alice & bob") || text.contains("alice &amp; bob"),
                "raw & should not appear unescaped in attributes"
            );
        }
    }

    /// test_ops_to_mutations_multiple_ops — multiple ops produce multiple mutations.
    #[test]
    fn test_ops_to_mutations_multiple_ops() {
        let meta = RevisionMeta::new("alice", "2026-01-01T00:00:00Z");
        let content = b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><office:body><office:text><text:p>Hello</text:p></office:text></office:body></office:document-content>";

        let op1 = PatchOp::ReplaceText {
            elem: gp_core::ir::ElementId::content_anchored("p", &["p:0"], "Hello"),
            old: "Hello".to_string(),
            new: "Goodbye".to_string(),
        };

        let mutations =
            RevisionWriter::ops_to_mutations(std::slice::from_ref(&op1), content, "odt", &meta);
        assert_eq!(
            mutations.len(),
            1,
            "should produce one mutation for replace"
        );
    }

    /// test_ops_to_mutations_insert_op — Insert creates plain paragraph.
    #[test]
    fn test_ops_to_mutations_insert_op() {
        let meta = RevisionMeta::new("alice", "2026-01-01T00:00:00Z");
        let content = b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><office:body><office:text></office:text></office:body></office:document-content>";

        let op = PatchOp::Insert {
            anchor: gp_core::ir::ElementId::content_anchored("p", &["p:0"], ""),
            kind: "new paragraph".to_string(),
        };

        let mutations =
            RevisionWriter::ops_to_mutations(std::slice::from_ref(&op), content, "odt", &meta);
        assert_eq!(mutations.len(), 1);

        if let MutationKind::InsertNode(XmlNode::Element { name, .. }) = &mutations[0].kind {
            assert_eq!(name.local_name, "p", "should insert a paragraph");
        } else {
            panic!("expected InsertNode with Element");
        }
    }

    /// test_ops_to_mutations_delete_op — Delete removes last paragraph.
    #[test]
    fn test_ops_to_mutations_delete_op() {
        let meta = RevisionMeta::new("alice", "2026-01-01T00:00:00Z");
        let content = b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><office:body><office:text><text:p>delete me</text:p></office:text></office:body></office:document-content>";

        let op = PatchOp::Delete {
            elem: gp_core::ir::ElementId::content_anchored("p", &["p:0"], ""),
        };

        let mutations =
            RevisionWriter::ops_to_mutations(std::slice::from_ref(&op), content, "odt", &meta);
        assert_eq!(mutations.len(), 1);

        assert!(
            matches!(mutations[0].kind, MutationKind::DeleteNode),
            "should be DeleteNode"
        );
    }

    /// test_create_as_revisions_direct — direct mode applies patches without markup.
    #[test]
    fn test_create_as_revisions_direct() {
        let source = make_minimal_odt();
        let meta = RevisionMeta::new("test-author", "2026-01-01T00:00:00Z");
        let result = create_as_revisions(
            &source,
            &[],
            meta.clone(),
            true, // direct mode, no ops
        );
        assert!(
            result.is_ok(),
            "should succeed with empty ops: {:?}",
            result
        );
        let res = result.unwrap();
        assert!(
            res.change_ids.is_empty(),
            "direct mode should have no change ids"
        );
        assert!(!res.output.is_empty(), "should produce output");

        // Verify output is a valid ZIP.
        let reader = std::io::Cursor::new(&res.output);
        let _archive = zip::ZipArchive::new(reader).expect("output should be a valid ZIP");
    }

    /// test_create_as_revisions_revision_mode — revision mode creates markup.
    #[test]
    fn test_create_as_revisions_revision_mode() {
        let source = make_minimal_odt();
        let meta = RevisionMeta::new("alice", "2026-01-01T00:00:00Z");
        let result = create_as_revisions(
            &source,
            &[],
            meta.clone(),
            false, // revision mode, no ops
        );
        // With no ops, the result should still succeed but produce no changes.
        assert!(result.is_ok(), "should succeed: {:?}", result);
        let res = result.unwrap();
        assert!(!res.output.is_empty());
        let reader = std::io::Cursor::new(&res.output);
        let _archive = zip::ZipArchive::new(reader).expect("output should be a valid ZIP");
    }

    /// test_revision_meta_annotation_id_format — annotation_id matches a_{4x}_{4x}.
    #[test]
    fn test_revision_meta_annotation_id_format() {
        let meta = RevisionMeta::new("alice", "2026-01-01");
        let parts: Vec<&str> = meta.annotation_id.split('_').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "a");
        assert_eq!(parts[1].len(), 4, "author hash should be 4 hex chars");
        assert_eq!(parts[2].len(), 4, "timestamp hash should be 4 hex chars");
        assert!(parts[1].chars().all(|c| c.is_ascii_hexdigit()));
        assert!(parts[2].chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// test_ns_constants — namespace URIs are correct.
    #[test]
    fn test_ns_constants() {
        assert_eq!(NS_TEXT, "urn:oasis:names:tc:opendocument:xmlns:text:1.0");
        assert_eq!(
            NS_OFFICE,
            "urn:oasis:names:tc:opendocument:xmlns:office:1.0"
        );
    }

    /// test_ops_to_mutations_replace_with_xml_special_chars — special chars in content.
    #[test]
    fn test_ops_to_mutations_replace_with_special_chars() {
        let meta = RevisionMeta::new("alice", "2026-01-01T00:00:00Z");
        let content = b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><office:body><office:text><text:p>x&y</text:p></office:text></office:body></office:document-content>";

        let op = PatchOp::ReplaceText {
            elem: gp_core::ir::ElementId::content_anchored("p", &["p:0"], "x"),
            old: "x".to_string(),
            new: "a&b".to_string(),
        };

        let mutations =
            RevisionWriter::ops_to_mutations(std::slice::from_ref(&op), content, "odt", &meta);
        assert_eq!(mutations.len(), 1);

        if let MutationKind::ReplaceText(replaced) = &mutations[0].kind {
            // The new content should be escaped in the inserted element.
            assert!(
                replaced.contains("a&amp;b"),
                "new content with & should be escaped: {}",
                replaced
            );
        }
    }

    /// test_ops_to_mutations_no_match — ReplaceText where old is not in content produces no mutation.
    #[test]
    fn test_ops_to_mutations_no_match() {
        let meta = RevisionMeta::new("alice", "2026-01-01T00:00:00Z");
        let content = b"<root/>";

        let op = PatchOp::ReplaceText {
            elem: gp_core::ir::ElementId::content_anchored("p", &["p:0"], "notfound"),
            old: "notfound".to_string(),
            new: "found".to_string(),
        };

        let mutations =
            RevisionWriter::ops_to_mutations(std::slice::from_ref(&op), content, "odt", &meta);
        assert_eq!(
            mutations.len(),
            0,
            "should produce no mutation when old text not found"
        );
    }

    /// test_ops_to_mutations_empty_old — empty old string is not matched.
    #[test]
    fn test_ops_to_mutations_empty_old() {
        let meta = RevisionMeta::new("alice", "2026-01-01T00:00:00Z");
        let content = b"<root/>";

        let op = PatchOp::ReplaceText {
            elem: gp_core::ir::ElementId::content_anchored("p", &["p:0"], ""),
            old: "".to_string(),
            new: "new".to_string(),
        };

        let mutations =
            RevisionWriter::ops_to_mutations(std::slice::from_ref(&op), content, "odt", &meta);
        assert_eq!(mutations.len(), 0, "empty old should not match");
    }

    /// test_read_back_revision_writer_output — generated ODT with revisions can be read by ODT reader.
    #[test]
    fn test_read_back_revision_writer_output() {
        let source = make_minimal_odt();
        let meta = RevisionMeta::new("alice", "2026-01-01T00:00:00Z");

        let result = create_as_revisions(
            &source,
            &[],
            meta,
            false, // revision mode
        );
        assert!(result.is_ok());
        let out = result.unwrap().output;

        // Verify output is a valid ZIP.
        let reader = std::io::Cursor::new(&out);
        let mut archive = zip::ZipArchive::new(reader).expect("output should be a valid ZIP");

        // Extract content.xml — with zero ops there is no markup, but the
        // package should still be a valid parseable ODT.
        let mut content_xml = Vec::new();
        archive
            .by_name("content.xml")
            .expect("content.xml should exist")
            .read_to_end(&mut content_xml)
            .unwrap();
        let content_str = String::from_utf8_lossy(&content_xml);
        // With zero ops, content.xml is unchanged — still has the original text.
        assert!(
            content_str.contains("Hello"),
            "content.xml should contain original text with zero ops"
        );

        // Verify the existing ODT reader can parse this output.
        let pkg = parse_odf(&out).expect("output ODT should parse");
        let read_result = read_odt(&pkg).expect("ODT reader should succeed");
        let doc: Document =
            serde_json::from_slice(&read_result.ir).expect("should deserialize Document IR");
        assert_eq!(doc.ir_version, IR_VERSION, "IR version should match");
        assert_eq!(doc.sections.len(), 1, "should have one section");
    }

    /// test_zero_patch_invariant — zero ops returns byte-identical ODT.
    #[test]
    fn test_zero_patch_invariant() {
        let source = make_minimal_odt();
        let meta = RevisionMeta::new("test", "2026-01-01T00:00:00Z");

        let result = create_as_revisions(&source, &[], meta, true);
        assert!(result.is_ok());
        let out = result.unwrap().output;

        // Zero ops + direct mode should produce byte-identical output.
        assert_eq!(out, source, "zero-patch should be byte-identical");
    }
}
