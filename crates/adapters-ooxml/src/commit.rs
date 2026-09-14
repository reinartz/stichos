//! OPC commit path — routes patch-sets through format writers, verifies, and generates byte-diff reports.
//!
//! Provides the `CommitContext` type which routes IR patch operations through the
//! appropriate format-specific writer (docx/xlsx/pptx), applies mutations via
//! `SurgicalWriter`, and generates a complete `CommitResult` with per-part byte-diff
//! reporting and post-commit verification.

use gp_core::diagnostics::{Diagnostic, Severity};
use gp_core::patch::PatchOp;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use super::docx_writer::DocxPatchWriter;
use super::opc::{parse_opc, OpcPackage};
use super::pptx_writer::PptxPatchWriter;
use super::xlsx_writer::XlsxPatchWriter;

// ===========================================================================
// Types
// ===========================================================================

/// The document format type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatType {
    /// Microsoft Word (.docx)
    Docx,
    /// Microsoft Excel (.xlsx)
    Xlsx,
    /// Microsoft PowerPoint (.pptx)
    Pptx,
}

impl FormatType {
    /// Return the format name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            FormatType::Docx => "docx",
            FormatType::Xlsx => "xlsx",
            FormatType::Pptx => "pptx",
        }
    }
}

/// Revision metadata for tracked changes.
#[derive(Debug, Clone)]
pub struct RevisionMeta {
    /// Author of the revision.
    pub author: String,
    /// Timestamp of the revision.
    pub timestamp: String,
}

/// A complete patch commit — routes IR patch through the appropriate format writer.
pub struct CommitContext<'a> {
    /// Format type.
    pub format: FormatType,
    /// Source package bytes.
    pub source_bytes: &'a [u8],
    /// The parsed OPC package.
    pub package: &'a OpcPackage,
    /// The patch operations.
    pub ops: Vec<PatchOp>,
    /// Whether to apply revision markup.
    pub revisions: bool,
    /// Revision metadata (if enabled).
    pub revision_meta: Option<RevisionMeta>,
}

/// Per-part byte-diff report entry.
#[derive(Debug, Clone, Serialize)]
pub struct PartByteDiff {
    /// The part URI.
    pub part_uri: String,
    /// SHA-256 hex before mutation (if part existed).
    pub before_hash: Option<String>,
    /// SHA-256 hex after mutation (if part was written).
    pub after_hash: Option<String>,
    /// Byte size before mutation.
    pub before_size: usize,
    /// Byte size after mutation.
    pub after_size: usize,
    /// Size difference (after - before).
    pub size_diff: i64,
    /// Whether the part was changed.
    pub changed: bool,
}

/// Complete commit result.
#[derive(Debug)]
pub struct CommitResult {
    /// The output OPC bytes.
    pub output: Vec<u8>,
    /// Per-part byte-diff report.
    pub byte_diffs: Vec<PartByteDiff>,
    /// URIs of parts that were changed.
    pub changed_parts: Vec<String>,
    /// URIs of parts that were unchanged.
    pub unchanged_parts: Vec<String>,
    /// Element IDs that were changed.
    pub changed_element_ids: Vec<String>,
    /// Cell references with stale cached values (xlsx only).
    pub stale_cached_values: Vec<String>,
    /// Diagnostics produced during the commit.
    pub diagnostics: Vec<Diagnostic>,
}

impl CommitResult {
    /// Serialize the byte-diff report as JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        let report: Vec<&PartByteDiff> = self.byte_diffs.iter().collect();
        serde_json::to_string_pretty(&report)
    }

    /// Serialize the byte-diff report as human-readable text.
    pub fn to_text(&self) -> String {
        let mut lines = Vec::new();
        lines.push("=== OPC Commit Byte-Diff Report ===".to_string());
        lines.push(format!(
            "Format: {} | Changed parts: {} | Unchanged parts: {}",
            match self.format_from_diagnostics() {
                FormatType::Docx => "docx",
                FormatType::Xlsx => "xlsx",
                FormatType::Pptx => "pptx",
            },
            self.changed_parts.len(),
            self.unchanged_parts.len()
        ));
        lines.push(String::new());

        for diff in &self.byte_diffs {
            if diff.changed {
                lines.push(format!(
                    "  CHANGED: {} ({} → {} bytes, diff: {})",
                    diff.part_uri, diff.before_size, diff.after_size, diff.size_diff
                ));
            }
        }

        if !self.changed_element_ids.is_empty() {
            lines.push(String::new());
            lines.push("Changed element IDs:".to_string());
            for id in &self.changed_element_ids {
                lines.push(format!("  - {}", id));
            }
        }

        if !self.stale_cached_values.is_empty() {
            lines.push(String::new());
            lines.push("Stale cached values:".to_string());
            for cell in &self.stale_cached_values {
                lines.push(format!("  - {}", cell));
            }
        }

        lines.join("\n")
    }

    fn format_from_diagnostics(&self) -> FormatType {
        for diag in &self.diagnostics {
            if diag.code.starts_with("OOX-R02") {
                return FormatType::Xlsx;
            }
            if diag.code.starts_with("OOX-R20") {
                return FormatType::Pptx;
            }
        }
        FormatType::Docx
    }
}

impl<'a> CommitContext<'a> {
    /// Create a new commit context.
    pub fn new(
        format: FormatType,
        source_bytes: &'a [u8],
        package: &'a OpcPackage,
        ops: Vec<PatchOp>,
    ) -> Self {
        Self {
            format,
            source_bytes,
            package,
            ops,
            revisions: false,
            revision_meta: None,
        }
    }

    /// Create a new commit context with revisions enabled.
    pub fn with_revisions(mut self, meta: RevisionMeta) -> Self {
        self.revisions = true;
        self.revision_meta = Some(meta);
        self
    }

    /// Execute the commit: apply patches, verify, generate report.
    pub fn execute(&self) -> Result<CommitResult, Vec<Diagnostic>> {
        let mut diags = Vec::new();

        // 1. Capture before-hashes for all parts.
        let before_parts: BTreeMap<String, Vec<u8>> = self
            .package
            .parts
            .iter()
            .map(|p| (p.uri.clone(), p.data.clone()))
            .collect();

        // 2. Route to appropriate format writer and apply.
        let (commit_result, changed_element_ids) = match self.apply_patches() {
            Ok(r) => r,
            Err(e) => {
                diags.extend(e);
                return Err(diags);
            }
        };

        let output = commit_result.output.clone();
        let mut changed_parts = commit_result.changed_parts.clone();
        let stale_cached_values = commit_result.stale_cached_values.clone();
        diags.extend(commit_result.diagnostics.clone());

        // 3. Capture after-hashes by parsing the output OPC.
        let after_package = match parse_opc(&output) {
            Ok(pkg) => pkg,
            Err(e) => {
                diags.extend(e);
                // Fall back to using original package parts with modified data
                return self.build_result_with_fallback(
                    &before_parts,
                    changed_parts,
                    changed_element_ids,
                    stale_cached_values,
                    diags,
                );
            }
        };

        // 4. Build byte-diff report.
        let mut byte_diffs = Vec::new();
        let mut unchanged_parts = Vec::new();

        // Check all parts from the original package.
        for (uri, before_data) in &before_parts {
            let before_hash = Self::sha256(before_data);
            let before_size = before_data.len();

            // Find matching part in output.
            let after_data = after_package
                .parts
                .iter()
                .find(|p| &p.uri == uri)
                .map(|p| p.data.clone())
                .or_else(|| before_parts.get(uri).cloned());

            if let Some(after) = after_data {
                let after_hash = Self::sha256(&after);
                let after_size = after.len();
                let size_diff = after_size as i64 - before_size as i64;
                let changed = after_hash != before_hash;

                byte_diffs.push(PartByteDiff {
                    part_uri: uri.clone(),
                    before_hash: Some(before_hash),
                    after_hash: Some(after_hash),
                    before_size,
                    after_size,
                    size_diff,
                    changed,
                });

                if changed {
                    if !changed_parts.contains(uri) {
                        changed_parts.push(uri.clone());
                    }
                } else {
                    unchanged_parts.push(uri.clone());
                }
            } else {
                // Part was removed.
                byte_diffs.push(PartByteDiff {
                    part_uri: uri.clone(),
                    before_hash: Some(before_hash),
                    after_hash: None,
                    before_size,
                    after_size: 0,
                    size_diff: -(before_size as i64),
                    changed: true,
                });
            }
        }

        // Check for new parts in the output.
        for part in &after_package.parts {
            if !before_parts.contains_key(&part.uri) {
                let after_hash = Self::sha256(&part.data);
                byte_diffs.push(PartByteDiff {
                    part_uri: part.uri.clone(),
                    before_hash: None,
                    after_hash: Some(after_hash),
                    before_size: 0,
                    after_size: part.data.len(),
                    size_diff: part.data.len() as i64,
                    changed: true,
                });
                if !changed_parts.contains(&part.uri) {
                    changed_parts.push(part.uri.clone());
                }
            }
        }

        // 5. Verify untouched parts.
        if let Err(e) = self.verify_untouched(&before_parts, &output, &unchanged_parts) {
            diags.extend(e);
        }

        // 6. Build result.
        Ok(CommitResult {
            output,
            byte_diffs,
            changed_parts,
            unchanged_parts,
            changed_element_ids,
            stale_cached_values,
            diagnostics: diags,
        })
    }

    /// Apply patches through the format-specific writer.
    fn apply_patches(&self) -> Result<(CommitResult, Vec<String>), Vec<Diagnostic>> {
        match self.format {
            FormatType::Docx => {
                let writer = DocxPatchWriter::new(self.package, self.ops.clone());
                let result = writer.write()?;
                let commit_result = CommitResult {
                    output: result.output,
                    byte_diffs: Vec::new(),
                    changed_parts: result.modified_parts,
                    unchanged_parts: Vec::new(),
                    changed_element_ids: result.changed_element_ids.clone(),
                    stale_cached_values: Vec::new(),
                    diagnostics: result.diagnostics,
                };
                Ok((commit_result, result.changed_element_ids))
            }
            FormatType::Xlsx => {
                let writer = XlsxPatchWriter::new(self.package, self.ops.clone());
                let result = writer.write()?;
                let commit_result = CommitResult {
                    output: result.output,
                    byte_diffs: Vec::new(),
                    changed_parts: result.modified_parts,
                    unchanged_parts: Vec::new(),
                    changed_element_ids: result.changed_element_ids.clone(),
                    stale_cached_values: result.stale_cached_values.clone(),
                    diagnostics: result.diagnostics,
                };
                Ok((commit_result, result.changed_element_ids))
            }
            FormatType::Pptx => {
                let writer = PptxPatchWriter::new(self.package, self.ops.clone());
                let result = writer.write()?;
                let commit_result = CommitResult {
                    output: result.output,
                    byte_diffs: Vec::new(),
                    changed_parts: result.modified_parts,
                    unchanged_parts: Vec::new(),
                    changed_element_ids: result.changed_element_ids.clone(),
                    stale_cached_values: Vec::new(),
                    diagnostics: result.diagnostics,
                };
                Ok((commit_result, result.changed_element_ids))
            }
        }
    }

    /// Build result with fallback when output OPC parsing fails.
    fn build_result_with_fallback(
        &self,
        before_parts: &BTreeMap<String, Vec<u8>>,
        changed_parts: Vec<String>,
        changed_element_ids: Vec<String>,
        stale_cached_values: Vec<String>,
        diags: Vec<Diagnostic>,
    ) -> Result<CommitResult, Vec<Diagnostic>> {
        let mut byte_diffs = Vec::new();
        let mut unchanged_parts = Vec::new();

        for (uri, before_data) in before_parts {
            let before_hash = Self::sha256(before_data);
            let before_size = before_data.len();

            let changed = changed_parts.contains(uri);
            let after_size = before_size; // Fallback: assume unchanged
            let after_hash = if changed {
                Some(format!("fallback-{}", before_hash))
            } else {
                Some(before_hash.clone())
            };

            byte_diffs.push(PartByteDiff {
                part_uri: uri.clone(),
                before_hash: Some(before_hash),
                after_hash,
                before_size,
                after_size,
                size_diff: 0,
                changed,
            });

            if !changed {
                unchanged_parts.push(uri.clone());
            }
        }

        Ok(CommitResult {
            output: Vec::new(),
            byte_diffs,
            changed_parts,
            unchanged_parts,
            changed_element_ids,
            stale_cached_values,
            diagnostics: diags,
        })
    }

    /// Compute SHA-256 of data and return hex string.
    pub fn sha256(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }

    /// Verify that untouched parts are byte-identical.
    pub fn verify_untouched(
        &self,
        before_parts: &BTreeMap<String, Vec<u8>>,
        after_bytes: &[u8],
        unchanged_parts: &[String],
    ) -> Result<(), Vec<Diagnostic>> {
        let mut diags = Vec::new();

        // Parse the output OPC.
        let after_package = match parse_opc(after_bytes) {
            Ok(pkg) => pkg,
            Err(e) => {
                diags.extend(e);
                return Err(diags);
            }
        };

        // Verify each unchanged part.
        for uri in unchanged_parts {
            if let Some(before_data) = before_parts.get(uri) {
                let before_hash = Self::sha256(before_data);

                if let Some(after_part) = after_package.parts.iter().find(|p| &p.uri == uri) {
                    let after_hash = Self::sha256(&after_part.data);
                    if before_hash != after_hash {
                        diags.push(Diagnostic::new(
                            "OOX-E-030",
                            Severity::Error,
                            format!(
                                "Unchanged part '{}' was modified during commit: before={}, after={}",
                                uri, before_hash, after_hash
                            ),
                        ));
                    }
                } else {
                    // Part was removed — this is unexpected for an unchanged part.
                    diags.push(Diagnostic::new(
                        "OOX-E-031",
                        Severity::Error,
                        format!("Unchanged part '{}' was removed during commit", uri),
                    ));
                }
            }
        }

        if !diags.is_empty() {
            return Err(diags);
        }

        Ok(())
    }
}

// ===========================================================================
// Tests
// ===========================================================================
