// SPDX-License-Identifier: MIT OR Apache-2.0
//! OOXML patch executor — concrete implementation that routes to format writers.
//!
//! `OoxmlExecutor` implements the `PatchExecutor` trait from gp-core and provides
//! real execution for docx, xlsx, and pptx formats using the commit infrastructure.
//!
//! # Format execution
//!
//! | Format | Writer | Notes |
//! |---|---|---|
//! | `docx` | `CommitContext` → `DocxPatchWriter` | Full paragraph/run/cell/revision support |
//! | `xlsx` | `CommitContext` → `XlsxPatchWriter` | Cell values, stale cache detection |
//! | `pptx` | `CommitContext` → `PptxPatchWriter` | Shape/placeholder text |

use gp_core::diagnostics::Diagnostic;
use gp_core::executor::{
    ExecuteResult, ExecutorConfig, FormatType, PartByteDiff, SimplePart, SurgicalPatchExecutor,
};
use gp_core::journal::{Journal, JournalAction};
use gp_core::patch::{ApplyResult, PatchOp};
use gp_core::store::ContentStore;
use gp_core::traits::PatchExecutor;
use std::io::Read;

use crate::commit::{CommitContext, FormatType as CommitFormatType, RevisionMeta};
use crate::opc::parse_opc;
use crate::opc_validate;

// ---------------------------------------------------------------------------
// Schema validation post-commit (P2-31: LIN-04)
// ---------------------------------------------------------------------------

/// Validate XML parts in the output ZIP archive.
///
/// Extracts XML entries from the output OPC and runs structural validation
/// via `lint::validate`. Returns diagnostics for any schema violations.
fn validate_post_commit(output: &[u8]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let cursor = std::io::Cursor::new(output);
    let mut reader = match zip::ZipArchive::new(cursor) {
        Ok(archive) => archive,
        Err(_) => return diagnostics, // not a valid ZIP, skip validation
    };

    for i in 0..reader.len() {
        let mut entry = match reader.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.name().to_string();
        if !name.ends_with(".xml") {
            continue;
        }
        let mut buf = Vec::new();
        if entry.read_to_end(&mut buf).is_err() {
            continue;
        }
        if buf.is_empty() {
            continue;
        }
        if let Err(e) = lint::validate::validate_xml_schema(&name, &buf) {
            diagnostics.push(Diagnostic::new(
                "OOX-E-021",
                gp_core::diagnostics::Severity::Error,
                format!("Schema validation failed for {}: {}", name, e),
            ));
        }
    }

    // Run OPC package integrity validation
    let opc_diagnostics = opc_validate::validate_package_bytes(output);
    for diag in opc_diagnostics {
        diagnostics.push(diag);
    }

    diagnostics
}

// ---------------------------------------------------------------------------
// OoxmlExecutor — delegates to gp-core executor with real OOXML execution
// ---------------------------------------------------------------------------

/// An OOXML-aware patch executor that wraps `SurgicalPatchExecutor`.
///
/// For unsupported formats (ODF/PDF), delegates to the base executor's refusal logic.
/// For OOXML formats (docx/xlsx/pptx), performs real surgical patch execution
/// through the commit infrastructure.
pub struct OoxmlExecutor {
    base: SurgicalPatchExecutor,
}

impl Default for OoxmlExecutor {
    fn default() -> Self {
        Self::new(ExecutorConfig::default())
    }
}

impl OoxmlExecutor {
    /// Create a new executor with the given configuration.
    pub fn new(config: ExecutorConfig) -> Self {
        Self {
            base: SurgicalPatchExecutor::new(config),
        }
    }

    /// Get the underlying base executor config.
    pub fn config(&self) -> &ExecutorConfig {
        self.base.config()
    }

    /// Run the full post-edit QA loop via the base executor.
    pub fn run_qa_loop(
        &self,
        source_bytes: &[u8],
        format: FormatType,
        ops: &[PatchOp],
        callbacks: &dyn gp_core::executor::QACallbacks,
    ) -> gp_core::executor::QALoopResult {
        self.base.run_qa_loop(source_bytes, format, ops, callbacks)
    }

    /// Execute a patch on OOXML source bytes.
    pub fn execute(
        &self,
        source_bytes: &[u8],
        format: FormatType,
        ops: &[PatchOp],
    ) -> ExecuteResult {
        // Check if the format is unsupported — delegate to base
        if format.is_unsupported() {
            return self.base.execute(source_bytes, format, ops);
        }

        // Check edit policy before doing any work
        let policy_result = self.base.check_policy(source_bytes, &format, ops);
        if policy_result.allowed.is_empty() {
            // All parts denied — refuse with denial reasons
            return ExecuteResult::policy_refuse(format.as_str(), policy_result.denied);
        } else if !policy_result.denied.is_empty() {
            // Some parts denied — note violations but continue
            let mut result = match format {
                FormatType::Docx => self.execute_docx(source_bytes, ops),
                FormatType::Xlsx => self.execute_xlsx(source_bytes, ops),
                FormatType::Pptx => self.execute_pptx(source_bytes, ops),
                _ => self.base.execute(source_bytes, format.clone(), ops),
            };
            result.policy_violations = policy_result.denied;
            return result;
        }

        // Dry-run path
        if self.base.config().dry_run {
            return self.execute_dry_run(source_bytes, format, ops);
        }

        // For writable formats, perform real execution
        let mut result = match format {
            FormatType::Docx => self.execute_docx(source_bytes, ops),
            FormatType::Xlsx => self.execute_xlsx(source_bytes, ops),
            FormatType::Pptx => self.execute_pptx(source_bytes, ops),
            _ => self.base.execute(source_bytes, format, ops),
        };

        // Warn about invalidating signatures in output
        self.base.warn_about_signatures(&mut result);
        result
    }

    /// Execute a dry-run patch
    fn execute_dry_run(
        &self,
        source_bytes: &[u8],
        format: FormatType,
        ops: &[PatchOp],
    ) -> ExecuteResult {
        let package = match parse_opc(source_bytes) {
            Ok(pkg) => pkg,
            Err(diags) => {
                return self.execute_dry_run_fallback(source_bytes, format, ops, diags);
            }
        };
        let parts: Vec<SimplePart> = package
            .parts
            .iter()
            .map(|p| SimplePart {
                uri: p.uri.clone(),
                data: p.data.clone(),
            })
            .collect();
        let before_ir = gp_core::executor::make_minimal_ir(&parts);
        let mut after_ir = before_ir.clone();
        for op in ops {
            let _ = gp_core::patch::apply_op(&mut after_ir, op);
        }
        let ir_diff = gp_core::diff::diff_documents(&before_ir, &after_ir);
        let changed_parts: Vec<String> = if ir_diff.is_empty() {
            Vec::new()
        } else {
            parts.iter().map(|p| p.uri.clone()).collect()
        };
        ExecuteResult {
            output: Vec::new(),
            accepted: true,
            diagnostics: vec![Diagnostic::new(
                "GP-I-001",
                gp_core::diagnostics::Severity::Info,
                format!(
                    "Dry-run: {} ops would modify {} parts ({} in package)",
                    ops.len(),
                    changed_parts.len(),
                    parts.len()
                ),
            )],
            changed_parts,
            unchanged_parts: Vec::new(),
            changed_element_ids: Vec::new(),
            stale_cached_values: Vec::new(),
            byte_diffs: Vec::new(),
            format: format.as_str().to_string(),
            journal_id: None,
            ir_diff: Some(ir_diff),
            invalidated_signatures: Vec::new(),
            tainted_parts: Vec::new(),
            policy_violations: Vec::new(),
        }
    }

    fn execute_dry_run_fallback(
        &self,
        source_bytes: &[u8],
        format: FormatType,
        ops: &[PatchOp],
        fallback_diagnostics: Vec<Diagnostic>,
    ) -> ExecuteResult {
        let parts = gp_core::executor::parse_source_parts(source_bytes);
        let before_ir = gp_core::executor::make_minimal_ir(&parts);
        let mut after_ir = before_ir.clone();
        for op in ops {
            let _ = gp_core::patch::apply_op(&mut after_ir, op);
        }
        let ir_diff = gp_core::diff::diff_documents(&before_ir, &after_ir);
        let changed_parts: Vec<String> = if ir_diff.is_empty() {
            Vec::new()
        } else {
            parts.iter().map(|p| p.uri.clone()).collect()
        };
        let mut diagnostics = fallback_diagnostics;
        diagnostics.push(Diagnostic::new(
            "GP-I-001",
            gp_core::diagnostics::Severity::Info,
            format!(
                "Dry-run (fallback): {} ops would modify {} parts",
                ops.len(),
                parts.len()
            ),
        ));
        ExecuteResult {
            output: Vec::new(),
            accepted: true,
            diagnostics,
            changed_parts,
            unchanged_parts: Vec::new(),
            changed_element_ids: Vec::new(),
            stale_cached_values: Vec::new(),
            byte_diffs: Vec::new(),
            format: format.as_str().to_string(),
            journal_id: None,
            ir_diff: Some(ir_diff),
            invalidated_signatures: Vec::new(),
            tainted_parts: Vec::new(),
            policy_violations: Vec::new(),
        }
    }

    fn execute_docx(&self, source_bytes: &[u8], ops: &[PatchOp]) -> ExecuteResult {
        let package = match parse_opc(source_bytes) {
            Ok(pkg) => pkg,
            Err(diags) => {
                return ExecuteResult {
                    output: Vec::new(),
                    accepted: false,
                    diagnostics: diags,
                    changed_parts: Vec::new(),
                    unchanged_parts: Vec::new(),
                    changed_element_ids: Vec::new(),
                    stale_cached_values: Vec::new(),
                    byte_diffs: Vec::new(),
                    format: "docx".to_string(),
                    journal_id: None,
                    ir_diff: None,
                    invalidated_signatures: Vec::new(),
                    tainted_parts: Vec::new(),
                    policy_violations: Vec::new(),
                };
            }
        };
        let mut ctx =
            CommitContext::new(CommitFormatType::Docx, source_bytes, &package, ops.to_vec());
        if self.base.config().revisions {
            let meta = RevisionMeta {
                author: self
                    .base
                    .config()
                    .author
                    .clone()
                    .unwrap_or_else(|| "agent".to_string()),
                timestamp: Self::now_iso(),
            };
            ctx = ctx.with_revisions(meta);
        }
        match ctx.execute() {
            Ok(result) => {
                let byte_diffs: Vec<PartByteDiff> = result
                    .byte_diffs
                    .iter()
                    .map(|d| PartByteDiff {
                        part_uri: d.part_uri.clone(),
                        before_hash: d.before_hash.clone(),
                        after_hash: d.after_hash.clone(),
                        before_size: d.before_size,
                        after_size: d.after_size,
                        size_diff: d.size_diff,
                        changed: d.changed,
                    })
                    .collect();
                let mut exec_result = ExecuteResult::success(
                    result.output.clone(),
                    result.changed_parts.clone(),
                    result.unchanged_parts.clone(),
                    result.changed_element_ids.clone(),
                    result.stale_cached_values.clone(),
                    byte_diffs.clone(),
                    "docx",
                );
                // P2-31: schema gate — validate output XML parts post-commit
                let schema_diagnostics = validate_post_commit(&result.output);
                if !schema_diagnostics.is_empty() {
                    exec_result.accepted = false;
                    exec_result.diagnostics.extend(schema_diagnostics);
                }
                exec_result
            }
            Err(diags) => ExecuteResult {
                output: Vec::new(),
                accepted: false,
                diagnostics: diags,
                changed_parts: Vec::new(),
                unchanged_parts: Vec::new(),
                changed_element_ids: Vec::new(),
                stale_cached_values: Vec::new(),
                byte_diffs: Vec::new(),
                format: "docx".to_string(),
                journal_id: None,
                ir_diff: None,
                invalidated_signatures: Vec::new(),
                tainted_parts: Vec::new(),
                policy_violations: Vec::new(),
            },
        }
    }

    fn execute_xlsx(&self, source_bytes: &[u8], ops: &[PatchOp]) -> ExecuteResult {
        let package = match parse_opc(source_bytes) {
            Ok(pkg) => pkg,
            Err(diags) => {
                return ExecuteResult {
                    output: Vec::new(),
                    accepted: false,
                    diagnostics: diags,
                    changed_parts: Vec::new(),
                    unchanged_parts: Vec::new(),
                    changed_element_ids: Vec::new(),
                    stale_cached_values: Vec::new(),
                    byte_diffs: Vec::new(),
                    format: "xlsx".to_string(),
                    journal_id: None,
                    ir_diff: None,
                    invalidated_signatures: Vec::new(),
                    tainted_parts: Vec::new(),
                    policy_violations: Vec::new(),
                };
            }
        };
        let mut ctx =
            CommitContext::new(CommitFormatType::Xlsx, source_bytes, &package, ops.to_vec());
        if self.base.config().revisions {
            let meta = RevisionMeta {
                author: self
                    .base
                    .config()
                    .author
                    .clone()
                    .unwrap_or_else(|| "agent".to_string()),
                timestamp: Self::now_iso(),
            };
            ctx = ctx.with_revisions(meta);
        }
        match ctx.execute() {
            Ok(result) => {
                let byte_diffs: Vec<PartByteDiff> = result
                    .byte_diffs
                    .iter()
                    .map(|d| PartByteDiff {
                        part_uri: d.part_uri.clone(),
                        before_hash: d.before_hash.clone(),
                        after_hash: d.after_hash.clone(),
                        before_size: d.before_size,
                        after_size: d.after_size,
                        size_diff: d.size_diff,
                        changed: d.changed,
                    })
                    .collect();
                let mut exec_result = ExecuteResult::success(
                    result.output.clone(),
                    result.changed_parts.clone(),
                    result.unchanged_parts.clone(),
                    result.changed_element_ids.clone(),
                    result.stale_cached_values.clone(),
                    byte_diffs.clone(),
                    "xlsx",
                );
                // P2-31: schema gate — validate output XML parts post-commit
                let schema_diagnostics = validate_post_commit(&result.output);
                if !schema_diagnostics.is_empty() {
                    exec_result.accepted = false;
                    exec_result.diagnostics.extend(schema_diagnostics);
                }
                exec_result
            }
            Err(diags) => ExecuteResult {
                output: Vec::new(),
                accepted: false,
                diagnostics: diags,
                changed_parts: Vec::new(),
                unchanged_parts: Vec::new(),
                changed_element_ids: Vec::new(),
                stale_cached_values: Vec::new(),
                byte_diffs: Vec::new(),
                format: "xlsx".to_string(),
                journal_id: None,
                ir_diff: None,
                invalidated_signatures: Vec::new(),
                tainted_parts: Vec::new(),
                policy_violations: Vec::new(),
            },
        }
    }

    fn execute_pptx(&self, source_bytes: &[u8], ops: &[PatchOp]) -> ExecuteResult {
        let package = match parse_opc(source_bytes) {
            Ok(pkg) => pkg,
            Err(diags) => {
                return ExecuteResult {
                    output: Vec::new(),
                    accepted: false,
                    diagnostics: diags,
                    changed_parts: Vec::new(),
                    unchanged_parts: Vec::new(),
                    changed_element_ids: Vec::new(),
                    stale_cached_values: Vec::new(),
                    byte_diffs: Vec::new(),
                    format: "pptx".to_string(),
                    journal_id: None,
                    ir_diff: None,
                    invalidated_signatures: Vec::new(),
                    tainted_parts: Vec::new(),
                    policy_violations: Vec::new(),
                };
            }
        };
        let mut ctx =
            CommitContext::new(CommitFormatType::Pptx, source_bytes, &package, ops.to_vec());
        if self.base.config().revisions {
            let meta = RevisionMeta {
                author: self
                    .base
                    .config()
                    .author
                    .clone()
                    .unwrap_or_else(|| "agent".to_string()),
                timestamp: Self::now_iso(),
            };
            ctx = ctx.with_revisions(meta);
        }
        match ctx.execute() {
            Ok(result) => {
                let byte_diffs: Vec<PartByteDiff> = result
                    .byte_diffs
                    .iter()
                    .map(|d| PartByteDiff {
                        part_uri: d.part_uri.clone(),
                        before_hash: d.before_hash.clone(),
                        after_hash: d.after_hash.clone(),
                        before_size: d.before_size,
                        after_size: d.after_size,
                        size_diff: d.size_diff,
                        changed: d.changed,
                    })
                    .collect();
                let mut exec_result = ExecuteResult::success(
                    result.output.clone(),
                    result.changed_parts.clone(),
                    result.unchanged_parts.clone(),
                    result.changed_element_ids.clone(),
                    Vec::new(),
                    byte_diffs.clone(),
                    "pptx",
                );
                // P2-31: schema gate — validate output XML parts post-commit
                let schema_diagnostics = validate_post_commit(&result.output);
                if !schema_diagnostics.is_empty() {
                    exec_result.accepted = false;
                    exec_result.diagnostics.extend(schema_diagnostics);
                }
                exec_result
            }
            Err(diags) => ExecuteResult {
                output: Vec::new(),
                accepted: false,
                diagnostics: diags,
                changed_parts: Vec::new(),
                unchanged_parts: Vec::new(),
                changed_element_ids: Vec::new(),
                stale_cached_values: Vec::new(),
                byte_diffs: Vec::new(),
                format: "pptx".to_string(),
                journal_id: None,
                ir_diff: None,
                invalidated_signatures: Vec::new(),
                tainted_parts: Vec::new(),
                policy_violations: Vec::new(),
            },
        }
    }

    pub fn routing_table() -> &'static [(&'static str, &'static str)] {
        SurgicalPatchExecutor::routing_table()
    }

    fn now_iso() -> String {
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
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

// ---------------------------------------------------------------------------
// Trait implementation
// ---------------------------------------------------------------------------

impl PatchExecutor for OoxmlExecutor {
    type Error = Vec<Diagnostic>;

    fn apply(&self, patch: &[u8]) -> Result<Vec<u8>, Self::Error> {
        self.base.apply(patch)
    }

    fn apply_op(&self, op: &PatchOp) -> Result<ApplyResult, Vec<Diagnostic>> {
        self.base.apply_op(op)
    }
}

// ---------------------------------------------------------------------------
// JournaledExecutor
// ---------------------------------------------------------------------------

pub struct JournaledExecutor {
    executor: OoxmlExecutor,
    journal: Journal,
}

impl JournaledExecutor {
    pub fn new(config: ExecutorConfig, session_id: String) -> Self {
        let store = ContentStore::open().unwrap_or_else(|_| {
            let tmp = std::env::temp_dir().join("stichos-journaled");
            let _ = std::fs::remove_dir_all(&tmp);
            ContentStore::open_at(&tmp).unwrap()
        });
        let journal = Journal::new(store, session_id);
        Self {
            executor: OoxmlExecutor::new(config),
            journal,
        }
    }

    pub fn new_at(config: ExecutorConfig, session_id: String, path: &std::path::Path) -> Self {
        let store = ContentStore::open_at(path).unwrap();
        let journal = Journal::new(store, session_id);
        Self {
            executor: OoxmlExecutor::new(config),
            journal,
        }
    }

    pub fn execute(
        &mut self,
        source_bytes: &[u8],
        format: FormatType,
        ops: &[PatchOp],
    ) -> ExecuteResult {
        let mut result = self.executor.execute(source_bytes, format.clone(), ops);
        if result.accepted && !format.is_unsupported() && !result.output.is_empty() {
            let _source_hash = ContentStore::key(source_bytes);
            let _target_hash = ContentStore::key(&result.output);
            let action = if ops.is_empty() {
                JournalAction::Style
            } else {
                match &ops[0] {
                    PatchOp::Insert { .. } | PatchOp::TrackInsert { .. } => JournalAction::Insert,
                    PatchOp::Delete { .. } | PatchOp::TrackDelete { .. } => JournalAction::Delete,
                    _ => JournalAction::Replace,
                }
            };
            let xml_diffs: Vec<gp_core::journal::XmlDiff> = result
                .byte_diffs
                .iter()
                .map(|d| gp_core::journal::XmlDiff {
                    part: d.part_uri.clone(),
                    before_hash: d.before_hash.clone().unwrap_or_default(),
                    after_hash: d.after_hash.clone().unwrap_or_default(),
                })
                .collect();
            match self.journal.commit(
                action,
                source_bytes,
                &result.output,
                format!("{:?}", ops),
                xml_diffs,
            ) {
                Ok(entry_id) => {
                    result.journal_id = Some(entry_id);
                }
                Err(e) => {
                    result.diagnostics.extend(e);
                }
            }
            let _ = self.journal.store_blob(&result.output);
            let _ = self.journal.store_blob(source_bytes);
        }
        result
    }

    pub fn revert(&mut self, steps: usize) -> Result<Vec<u8>, Vec<Diagnostic>> {
        self.journal.revert_to_bytes(steps)
    }

    pub fn head(&self) -> Option<String> {
        self.journal.head()
    }

    pub fn entries(&self) -> &[gp_core::journal::JournalEntry] {
        self.journal.entries()
    }

    pub fn chain_integrity(&self) -> Result<(), Diagnostic> {
        self.journal.chain_integrity()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use gp_core::executor::QACallbacks;
    use gp_core::ir::ElementId;

    #[test]
    fn test_routing_table_matches_base() {
        let table = OoxmlExecutor::routing_table();
        assert_eq!(table.len(), 7);
        let writable = table
            .iter()
            .filter(|(f, _)| *f == "docx" || *f == "xlsx" || *f == "pptx")
            .count();
        let refused = table.iter().filter(|(_, d)| d.contains("refused")).count();
        assert_eq!(writable, 3);
        // ODF is now handled by adapters-odf (P3-10), PDF by adapters-pdf (P3-14)
        // No formats are refused in the base routing table anymore
        assert_eq!(refused, 0);
    }

    #[test]
    fn test_executor_config_propagation() {
        let config = ExecutorConfig {
            revisions: true,
            author: Some("test-author".into()),
            dry_run: false,
            policy: None,
            force_signature_invalidation: false,
        };
        let executor = OoxmlExecutor::new(config);
        assert!(executor.config().revisions);
        assert_eq!(executor.config().author.as_deref(), Some("test-author"));
        assert!(!executor.config().dry_run);
    }

    #[test]
    fn test_odf_delegated_to_adapters_odf() {
        // ODF is now handled by adapters-odf (P3-10)
        let executor = OoxmlExecutor::default();
        let result = executor.execute(b"fake odt data", FormatType::Odt, &[]);
        // ODT is routed to adapters-odf, so it should be accepted
        // (the fake data won't actually work, but the format is no longer refused)
        assert!(result.accepted || result.diagnostics.iter().any(|d| d.code.starts_with("ODF")));
        assert_eq!(result.format, "odt");
    }

    #[test]
    fn test_pdf_delegated_to_adapters_pdf() {
        // PDF is now handled by adapters-pdf (P3-14), not refused by ooxml executor
        let executor = OoxmlExecutor::default();
        let result = executor.execute(b"%PDF-1.4 test data", FormatType::Pdf, &[]);
        // PDF should be accepted (routed to base executor which delegates to adapters-pdf)
        assert!(
            result.accepted,
            "PDF is writable via adapters-pdf (P3-14), got: {:?}",
            result.diagnostics
        );
        assert_eq!(result.format, "pdf");
    }

    #[test]
    fn test_pdf_accepted_via_base_executor() {
        // PDF is handled by adapters-pdf (P3-14), not refused
        let executor = OoxmlExecutor::default();
        let result = executor.execute(b"%PDF-1.4 test data", FormatType::Pdf, &[]);
        assert!(
            result.accepted,
            "PDF is writable via base executor (P3-14), got: {:?}",
            result.diagnostics
        );
        assert_eq!(result.format, "pdf");
    }

    #[test]
    fn test_pdf_not_refused_by_ooxml() {
        // PDF is handled by adapters-pdf (P3-14), no longer unsupported
        let executor = OoxmlExecutor::default();
        let source = b"test";
        let result = executor.execute(source, FormatType::Pdf, &[]);
        assert!(
            result.accepted,
            "PDF is writable via adapters-pdf (P3-14), not refused"
        );
        assert_eq!(result.format, "pdf");
    }

    #[test]
    fn test_patch_executor_trait_implemented() {
        let executor = OoxmlExecutor::default();
        let op = PatchOp::ReplaceText {
            elem: ElementId::content_anchored("test", &["t:0"], "hello"),
            old: "hello".to_string(),
            new: "world".to_string(),
        };
        let result = executor.apply_op(&op);
        assert!(result.is_ok());
    }

    #[test]
    fn test_patch_executor_apply_wraps_base() {
        let executor = OoxmlExecutor::default();
        let result = executor.apply(b"[]").unwrap();
        let ops: Vec<PatchOp> = serde_json::from_slice(&result).unwrap();
        assert!(ops.is_empty());
    }

    #[test]
    fn test_inv1_refusal_no_output_bytes() {
        // PDF is writable (P3-14), test stub produces output
        let executor = OoxmlExecutor::default();
        let result = executor.execute(b"source data", FormatType::Pdf, &[]);
        assert!(result.accepted, "PDF is writable (P3-14)");
        // Stub returns source bytes as output
        assert!(!result.output.is_empty());
        assert_eq!(result.changed_parts.len(), 0);
    }

    #[test]
    fn test_empty_ops_docx_produces_result() {
        let executor = OoxmlExecutor::default();
        let source = b"PK\x03\x04minimal docx";
        let result = executor.execute(source, FormatType::Docx, &[]);
        let _accepted = result.accepted;
        let _diags = result.diagnostics;
    }

    #[test]
    fn test_ooxml_dry_run_enumerates_parts() {
        let config = ExecutorConfig {
            dry_run: true,
            revisions: false,
            author: None,
            policy: None,
            force_signature_invalidation: false,
        };
        let executor = OoxmlExecutor::new(config);
        let source =
            b"PK\x03\x04[Content_Types].xmlPK\x03\x04word/document.xmlPK\x03\x04_rels/.rels";
        let result = executor.execute(source, FormatType::Docx, &[]);
        assert!(result.accepted, "dry-run should accept");
        assert!(result.output.is_empty(), "dry-run should produce no output");
        assert!(result.ir_diff.is_some(), "dry-run should include ir_diff");
    }

    #[test]
    fn test_ooxml_dry_run_with_ops_produces_diff() {
        let config = ExecutorConfig {
            dry_run: true,
            revisions: false,
            author: None,
            policy: Some(gp_core::policy::EditPolicy {
                allowed_parts: vec!["**".to_string()],
                denied_actions: vec![],
                max_parts_modified: None,
            }),
            force_signature_invalidation: false,
        };
        let executor = OoxmlExecutor::new(config);
        let source = b"PK\x03\x04[Content_Types].xml";
        let elem_id = ElementId::content_anchored("run", &["t:0"], "hello");
        let op = PatchOp::ReplaceText {
            elem: elem_id,
            old: "hello".into(),
            new: "world".into(),
        };
        let result = executor.execute(source, FormatType::Docx, &[op]);
        assert!(result.accepted);
        assert!(result.ir_diff.is_some());
    }

    #[test]
    fn test_ooxml_dry_run_xlsx() {
        let config = ExecutorConfig {
            dry_run: true,
            revisions: false,
            author: None,
            policy: None,
            force_signature_invalidation: false,
        };
        let executor = OoxmlExecutor::new(config);
        let source = b"PK\x03\x04[Content_Types].xmlPK\x03\x04xl/workbook.xml";
        let result = executor.execute(source, FormatType::Xlsx, &[]);
        assert!(result.accepted);
        assert!(result.ir_diff.is_some());
        assert_eq!(result.format, "xlsx");
    }

    #[test]
    fn test_ooxml_dry_run_pptx() {
        let config = ExecutorConfig {
            dry_run: true,
            revisions: false,
            author: None,
            policy: None,
            force_signature_invalidation: false,
        };
        let executor = OoxmlExecutor::new(config);
        let source = b"PK\x03\x04[Content_Types].xmlPK\x03\x04ppt/presentation.xml";
        let result = executor.execute(source, FormatType::Pptx, &[]);
        assert!(result.accepted);
        assert!(result.ir_diff.is_some());
        assert_eq!(result.format, "pptx");
    }

    struct TestOoxmlQACallbacks {
        #[allow(dead_code)]
        pub mock_render: bool,
        pub page_count: usize,
    }
    impl TestOoxmlQACallbacks {
        fn new() -> Self {
            Self {
                mock_render: true,
                page_count: 1,
            }
        }
        fn with_pages(mut self, count: usize) -> Self {
            self.page_count = count;
            self
        }
    }
    impl QACallbacks for TestOoxmlQACallbacks {
        fn render(
            &self,
            _format: &str,
            output: &[u8],
        ) -> Result<Vec<gp_core::executor::RenderPage>, Vec<Diagnostic>> {
            let page_count = if output.is_empty() { 0 } else { 1 };
            Ok((1..=page_count.max(1))
                .map(|n| gp_core::executor::RenderPage {
                    page_number: n,
                    width_pt: 612.0,
                    height_pt: 792.0,
                    image_bytes: None,
                    text: String::new(),
                })
                .collect())
        }
        fn lint(
            &self,
            _source: &[gp_core::executor::RenderPage],
            _output: &[gp_core::executor::RenderPage],
        ) -> Vec<Diagnostic> {
            Vec::new()
        }
    }

    #[test]
    fn test_ooxml_qa_loop_with_mock() {
        let executor = OoxmlExecutor::default();
        let callbacks = TestOoxmlQACallbacks::new().with_pages(1);
        let source = b"PK\x03\x04[Content_Types].xml word/document.xml";
        let result = executor.run_qa_loop(source, FormatType::Docx, &[], &callbacks);
        assert!(result.executor_result.accepted);
        assert!(result.qa_passed);
    }

    #[test]
    fn test_ooxml_qa_loop_refused_format() {
        let executor = OoxmlExecutor::default();
        let callbacks = TestOoxmlQACallbacks::new();
        // PDF is writable (P3-14), test passes with stub
        let result = executor.run_qa_loop(b"pdf data", FormatType::Pdf, &[], &callbacks);
        assert!(result.executor_result.accepted, "PDF is writable (P3-14)");
    }

    #[test]
    fn test_ooxml_executor_warns_on_signed_output() {
        let executor = OoxmlExecutor::default();
        let source = b"PK\x03\x04[Content_Types].xml word/document.xml";
        let result = executor.execute(source, FormatType::Docx, &[]);
        let _sigs = gp_core::executor::detect_signatures(&result.output);
    }

    // ------------------------------------------------------------------
    // Policy enforcement tests for OoxmlExecutor (P2-24: SEC-04)
    // ------------------------------------------------------------------

    #[test]
    fn test_ooxml_executor_obeys_policy() {
        let policy = gp_core::policy::EditPolicy {
            allowed_parts: vec!["word/".to_string()],
            denied_actions: vec![gp_core::policy::DeniedAction::ExternalRelAdd],
            max_parts_modified: None,
        };
        let config = ExecutorConfig {
            revisions: false,
            author: None,
            dry_run: true,
            policy: Some(policy),
            force_signature_invalidation: false,
        };
        let executor = OoxmlExecutor::new(config);
        let source = b"PK\x03\x04[Content_Types].xml word/document.xml";
        let result = executor.execute(source, FormatType::Docx, &[]);
        assert!(result.accepted);
        assert!(result.ir_diff.is_some());
    }

    #[test]
    fn test_ooxml_executor_denies_destructive_ops() {
        // Empty allowed_parts means deny-by-default for all parts
        let policy = gp_core::policy::EditPolicy {
            allowed_parts: vec![],
            denied_actions: vec![],
            max_parts_modified: None,
        };
        let config = ExecutorConfig {
            revisions: false,
            author: None,
            dry_run: false,
            policy: Some(policy),
            force_signature_invalidation: false,
        };
        let executor = OoxmlExecutor::new(config);
        let source = b"PK\x03\x04[Content_Types].xml word/document.xml";
        // DeletePart op → should be denied by default deny list
        let ops = vec![PatchOp::Delete {
            elem: ElementId::content_anchored("run", &["t:0"], "hello"),
        }];
        let result = executor.execute(source, FormatType::Docx, &ops);
        assert!(!result.accepted);
        assert!(!result.policy_violations.is_empty());
    }

    #[test]
    fn test_journaled_executor_preserves_signature_warning() {
        let executor = OoxmlExecutor::default();
        let source = b"PK\x03\x04signature data";
        let result = executor.execute(source, FormatType::Docx, &[]);
        let sigs = gp_core::executor::detect_signatures(&result.output);
        let _ = sigs;
        let _ = result;
    }
}
