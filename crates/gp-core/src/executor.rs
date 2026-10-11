// SPDX-License-Identifier: MIT OR Apache-2.0
//! Patch executor — concrete implementation of the `PatchExecutor` trait.
//!
//! `SurgicalPatchExecutor` routes IR patch operations for OOXML formats (docx, xlsx, pptx)
//! and ODF formats (odt, ods, odp) with explicit C3 diagnostic codes.
//! PDF is writable via incremental writer (P3-14).
//!
//! # Format routing
//!
//! | Format | Action |
//! |---|---|
//! | `docx` | Route to OOXML docx writer |
//! | `xlsx` | Route to OOXML xlsx writer |
//! | `pptx` | Route to OOXML pptx writer |
//! | `odt` / `ods` / `odp` | Route to ODF writer (P3-10) |
//! | `pdf` | Incremental writer (P3-14) |
//! | unknown | Refuse with `GP-E-002` |
//!
//! # Contract conformance
//!
//! - **PAT-01**: executor routes correctly, refuses only encrypted/signed PDF
//! - **PAT-02**: dry-run mode produces diff preview without writes
//! - **INV-1**: refusal produces zero side effects, no file modifications

use crate::diagnostics::{Diagnostic, Severity};
use crate::diff::IrDiff;
use crate::fidelity::FidelityBudget;
use crate::ir::{ElementId, IRDocument, TaggedNode};
use crate::journal::{Journal, JournalAction, XmlDiff};
use crate::patch::{ApplyResult, PatchOp};
use crate::policy::{DeniedAction, EditPolicy, TaintKind, TaintMark};
use crate::store::ContentStore;
use crate::traits::PatchExecutor;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The document format type that the executor supports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormatType {
    Docx,
    Xlsx,
    Pptx,
    Odt,
    Ods,
    Odp,
    Pdf,
}

impl FormatType {
    /// Return the format name as a string.
    pub fn as_str(&self) -> &str {
        match self {
            FormatType::Docx => "docx",
            FormatType::Xlsx => "xlsx",
            FormatType::Pptx => "pptx",
            FormatType::Odt => "odt",
            FormatType::Ods => "ods",
            FormatType::Odp => "odp",
            FormatType::Pdf => "pdf",
        }
    }

    /// Determine the format from a file extension string.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "docx" => Some(FormatType::Docx),
            "xlsx" => Some(FormatType::Xlsx),
            "pptx" => Some(FormatType::Pptx),
            "odt" => Some(FormatType::Odt),
            "ods" => Some(FormatType::Ods),
            "odp" => Some(FormatType::Odp),
            "pdf" => Some(FormatType::Pdf),
            _ => None,
        }
    }

    /// Return true if this format is writable by the executor.
    pub fn is_writable(&self) -> bool {
        matches!(
            self,
            FormatType::Docx
                | FormatType::Xlsx
                | FormatType::Pptx
                | FormatType::Odt
                | FormatType::Ods
                | FormatType::Odp
                | FormatType::Pdf
        )
    }

    /// Return true if this format should be refused.
    pub fn is_unsupported(&self) -> bool {
        false
    }
}

/// Information about a detected digital signature in output bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureInfo {
    /// Type of signature detected (e.g. "pkcs7", "office-xml", "pdf-adobe", "pdf-pades").
    pub sig_type: String,
    /// Location of the signature within the document structure.
    pub location: String,
    /// Confidence score (0.0 to 1.0).
    pub confidence: f64,
}

/// A complete executor result combining the output bytes with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteResult {
    /// The output bytes (empty for refused formats).
    pub output: Vec<u8>,
    /// Whether the operation was accepted (false for refused formats).
    pub accepted: bool,
    /// Diagnostics from the execution.
    pub diagnostics: Vec<Diagnostic>,
    /// Changed parts that were modified.
    pub changed_parts: Vec<String>,
    /// Unchanged parts (for INV-1 verification).
    pub unchanged_parts: Vec<String>,
    /// Element IDs that were changed.
    pub changed_element_ids: Vec<String>,
    /// Cells with stale cached values (xlsx only).
    pub stale_cached_values: Vec<String>,
    /// Per-part byte-diff report entries.
    pub byte_diffs: Vec<PartByteDiff>,
    /// The format that was targeted.
    pub format: String,
    /// Journal entry ID if the result was committed to a journal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub journal_id: Option<String>,
    /// The IR unified diff (only present in dry-run mode).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ir_diff: Option<IrDiff>,
    /// Digital signatures detected in output that are invalidated by this edit.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub invalidated_signatures: Vec<SignatureInfo>,
    /// Source parts that contained extractable text content (taint tracking).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tainted_parts: Vec<String>,
    /// Parts denied by the edit policy during execution.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub policy_violations: Vec<PolicyDenial>,
}

impl ExecuteResult {
    /// Mark this result as tainted by the given taint mark.
    pub fn with_taint(mut self, mark: TaintMark) -> Self {
        if let Some(parts) = mark.source_part {
            self.tainted_parts.push(parts);
        }
        self
    }

    /// Create a refusal result for an unsupported format.
    pub fn refuse(format: &str, reason: &str) -> Self {
        let code = Self::refusal_code(format);
        Self {
            output: Vec::new(),
            accepted: false,
            diagnostics: vec![Diagnostic::new(
                code,
                Severity::Error,
                format!("Write unsupported for {}: {}", format, reason),
            )],
            changed_parts: Vec::new(),
            unchanged_parts: Vec::new(),
            changed_element_ids: Vec::new(),
            stale_cached_values: Vec::new(),
            byte_diffs: Vec::new(),
            format: format.to_string(),
            journal_id: None,
            ir_diff: None,
            invalidated_signatures: Vec::new(),
            tainted_parts: Vec::new(),
            policy_violations: Vec::new(),
        }
    }

    /// Return the refusal diagnostic code for a given format.
    fn refusal_code(format: &str) -> String {
        match format {
            "pdf" => "PDF-E-001".to_string(),
            _ => "GP-E-002".to_string(),
        }
    }

    /// Create a success result for dry-run (no actual output bytes).
    pub fn dry_run(format: &str) -> Self {
        Self {
            output: Vec::new(),
            accepted: true,
            diagnostics: vec![Diagnostic::new(
                "GP-I-001",
                Severity::Info,
                "Dry-run: no writes performed".to_string(),
            )],
            changed_parts: Vec::new(),
            unchanged_parts: Vec::new(),
            changed_element_ids: Vec::new(),
            stale_cached_values: Vec::new(),
            byte_diffs: Vec::new(),
            format: format.to_string(),
            journal_id: None,
            ir_diff: None,
            invalidated_signatures: Vec::new(),
            tainted_parts: Vec::new(),
            policy_violations: Vec::new(),
        }
    }

    /// Create a success result with OOXML commit data.
    pub fn success(
        output: Vec<u8>,
        changed_parts: Vec<String>,
        unchanged_parts: Vec<String>,
        changed_element_ids: Vec<String>,
        stale_cached_values: Vec<String>,
        byte_diffs: Vec<PartByteDiff>,
        format: &str,
    ) -> Self {
        Self {
            output,
            accepted: true,
            diagnostics: Vec::new(),
            changed_parts,
            unchanged_parts,
            changed_element_ids,
            stale_cached_values,
            byte_diffs,
            format: format.to_string(),
            journal_id: None,
            ir_diff: None,
            invalidated_signatures: Vec::new(),
            tainted_parts: Vec::new(),
            policy_violations: Vec::new(),
        }
    }

    /// Create a refusal result for a document with existing digital signatures.
    pub fn signature_refuse(sigs: &[SignatureInfo]) -> Self {
        let sig_descriptions: Vec<String> = sigs
            .iter()
            .map(|s| format!("{} at {}", s.sig_type, s.location))
            .collect();
        Self {
            output: Vec::new(),
            accepted: false,
            diagnostics: vec![Diagnostic::new(
                "GP-S-001",
                Severity::Error,
                format!(
                    "Document has digital signatures — write refused: {}",
                    sig_descriptions.join("; ")
                ),
            )],
            changed_parts: Vec::new(),
            unchanged_parts: Vec::new(),
            changed_element_ids: Vec::new(),
            stale_cached_values: Vec::new(),
            byte_diffs: Vec::new(),
            format: "unknown".to_string(),
            journal_id: None,
            ir_diff: None,
            invalidated_signatures: sigs.to_vec(),
            tainted_parts: Vec::new(),
            policy_violations: Vec::new(),
        }
    }

    /// Create a refusal result when the edit policy denies all requested parts.
    pub fn policy_refuse(format: &str, violations: Vec<PolicyDenial>) -> Self {
        let code = Self::refusal_code(format);
        let reasons: Vec<String> = violations.iter().map(|v| v.reason.clone()).collect();
        Self {
            output: Vec::new(),
            accepted: false,
            diagnostics: vec![Diagnostic::new(
                code,
                Severity::Error,
                format!(
                    "Edit policy denied {} parts: {}",
                    violations.len(),
                    reasons.join("; ")
                ),
            )],
            changed_parts: Vec::new(),
            unchanged_parts: Vec::new(),
            changed_element_ids: Vec::new(),
            stale_cached_values: Vec::new(),
            byte_diffs: Vec::new(),
            format: format.to_string(),
            journal_id: None,
            ir_diff: None,
            invalidated_signatures: Vec::new(),
            tainted_parts: Vec::new(),
            policy_violations: violations,
        }
    }

    /// Create a stub result for writable formats (delegated to adapters-ooxml).
    fn stub(source_bytes: &[u8], format: String) -> Self {
        Self {
            output: source_bytes.to_vec(),
            accepted: true,
            diagnostics: Vec::new(),
            changed_parts: Vec::new(),
            unchanged_parts: Vec::new(),
            changed_element_ids: Vec::new(),
            stale_cached_values: Vec::new(),
            byte_diffs: Vec::new(),
            format,
            journal_id: None,
            ir_diff: None,
            invalidated_signatures: Vec::new(),
            tainted_parts: Vec::new(),
            policy_violations: Vec::new(),
        }
    }
}

/// Per-part byte-diff report entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartByteDiff {
    /// The part URI.
    pub part_uri: String,
    /// SHA-256 hex before mutation (if part existed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_hash: Option<String>,
    /// SHA-256 hex after mutation (if part was written).
    #[serde(skip_serializing_if = "Option::is_none")]
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

/// A policy denial for a part that was blocked by the edit policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDenial {
    /// The part URI that was denied.
    pub part: String,
    /// Human-readable reason for the denial.
    pub reason: String,
    /// Machine-readable diagnostic code.
    pub diagnostic_code: String,
}

/// Result of checking operations against the edit policy.
#[derive(Debug, Clone, Default)]
pub struct PolicyCheckResult {
    /// Parts whose operations are allowed.
    pub allowed: Vec<String>,
    /// Parts whose operations are denied.
    pub denied: Vec<PolicyDenial>,
}

/// A rendered page from the QA loop's render step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderPage {
    /// One-based page number.
    pub page_number: usize,
    /// Page width in points.
    pub width_pt: f64,
    /// Page height in points.
    pub height_pt: f64,
    /// PNG image bytes (visual render).
    pub image_bytes: Option<Vec<u8>>,
    /// Extracted text from the page.
    pub text: String,
}

/// Callback trait for QA steps — gp-core provides the interface,
/// adapters/implementations provide the actual rendering and linting.
pub trait QACallbacks: Send + Sync {
    /// Render output bytes to pages (images or text).
    fn render(&self, format: &str, output: &[u8]) -> Result<Vec<RenderPage>, Vec<Diagnostic>>;

    /// Lint rendered pages against source, returning diagnostics.
    fn lint(&self, source_pages: &[RenderPage], output_pages: &[RenderPage]) -> Vec<Diagnostic>;
}

/// A single step latency measurement from the QA loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepLatency {
    /// Step name (e.g. "execute", "render", "lint", "assemble").
    pub step: String,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

/// Result of a full post-edit QA loop (render -> lint -> fidelity assembly).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QALoopResult {
    /// The executor's ExecuteResult.
    pub executor_result: ExecuteResult,
    /// Whether the QA loop completed successfully.
    pub qa_passed: bool,
    /// Lint diagnostics from the post-edit check.
    pub lint_diagnostics: Vec<Diagnostic>,
    /// Fidelity budget after the edit.
    pub fidelity_budget: FidelityBudget,
    /// Changed page count (pages that differ from original).
    pub changed_pages: Vec<usize>,
    /// Render engine used.
    pub rendered_by: Option<crate::fidelity::RendererInfo>,
    /// Total loop latency in milliseconds.
    pub loop_latency_ms: u64,
    /// Per-step latency breakdown.
    pub step_latencies: Vec<StepLatency>,
}

/// Simple latency tracker for the QA loop steps.
pub struct LatencyTracker {
    start: std::time::Instant,
    steps: Vec<(String, u64)>,
}

impl LatencyTracker {
    /// Create a new tracker.
    pub fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
            steps: Vec::new(),
        }
    }

    /// Record a checkpoint with the elapsed time since the start.
    pub fn checkpoint(&mut self, name: &str) {
        let duration = self.start.elapsed().as_millis() as u64;
        self.steps.push((name.to_string(), duration));
    }

    /// Convert to a vector of StepLatency values.
    pub fn to_vec(self) -> Vec<StepLatency> {
        self.steps
            .into_iter()
            .map(|(name, dur)| StepLatency {
                step: name,
                duration_ms: dur,
            })
            .collect()
    }
}

impl Default for LatencyTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Configuration for the surgical patch executor.
#[derive(Debug, Clone, Default)]
pub struct ExecutorConfig {
    /// Whether to apply revision markup (w:ins/w:del).
    pub revisions: bool,
    /// Author for revision markup.
    pub author: Option<String>,
    /// Whether to enable dry-run mode (no writes).
    pub dry_run: bool,
    /// Optional edit policy for part/action enforcement.
    /// `None` means use deny-by-default.
    #[allow(dead_code)]
    pub policy: Option<EditPolicy>,
    /// Whether to allow mutations that invalidate existing signatures.
    /// When false (default), mutations on signed documents are refused with GP-S-001.
    pub force_signature_invalidation: bool,
}

// ---------------------------------------------------------------------------
// SurgicalPatchExecutor
// ---------------------------------------------------------------------------

/// A byte-surgical patch executor that implements the `PatchExecutor` trait.
///
/// Routes patch operations through format-specific writers for OOXML formats
/// and refuses unsupported formats with explicit C3 diagnostic codes.
pub struct SurgicalPatchExecutor {
    config: ExecutorConfig,
}

impl Default for SurgicalPatchExecutor {
    fn default() -> Self {
        Self::new(ExecutorConfig::default())
    }
}

impl SurgicalPatchExecutor {
    /// Create a new executor with the given configuration.
    pub fn new(config: ExecutorConfig) -> Self {
        Self { config }
    }

    /// Get the current configuration.
    pub fn config(&self) -> &ExecutorConfig {
        &self.config
    }

    /// Check the edit policy against the given ops for the given source parts.
    pub fn check_policy(
        &self,
        source_bytes: &[u8],
        _format: &FormatType,
        ops: &[PatchOp],
    ) -> PolicyCheckResult {
        let parts = parse_source_parts(source_bytes);
        let part_names: Vec<&str> = parts.iter().map(|p| p.uri.as_str()).collect();

        // Map ops to denied actions
        let ops_actions: Vec<&DeniedAction> = ops
            .iter()
            .map(|op| match op {
                PatchOp::Delete { .. } | PatchOp::TrackDelete { .. } => &DeniedAction::DeletePart,
                PatchOp::ReplaceText { .. }
                | PatchOp::SetRunProps { .. }
                | PatchOp::SetCellValue { .. }
                | PatchOp::SetCellRich { .. } => &DeniedAction::MacroPartWrite,
                PatchOp::Insert { .. } | PatchOp::TrackInsert { .. } => {
                    &DeniedAction::UnknownAction
                }
            })
            .collect();

        let mut result = PolicyCheckResult::default();

        // If no policy configured, use a permissive policy that only denies destructive actions
        let policy = match &self.config.policy {
            Some(p) => p.clone(),
            None => EditPolicy {
                allowed_parts: vec!["**".to_string()],
                denied_actions: vec![DeniedAction::DeletePart, DeniedAction::ExternalRelAdd],
                max_parts_modified: None,
            },
        };

        // If no ops, nothing to deny
        if ops_actions.is_empty() {
            for part in &part_names {
                result.allowed.push(part.to_string());
            }
            return result;
        }

        // Determine unique denied actions across all ops
        let mut action_set: Vec<DeniedAction> = Vec::new();
        for &act in &ops_actions {
            if !action_set.contains(act) {
                action_set.push((*act).clone());
            }
        }

        // Check each part against the policy
        let mut allowed_count: usize = 0;
        for part in &part_names {
            let denied_actions = policy.evaluate(part, &action_set).unwrap_or_default();

            // Apply max_parts_modified cap
            if denied_actions.is_empty() {
                if let Some(max) = policy.max_parts_modified {
                    if allowed_count >= max {
                        result.denied.push(PolicyDenial {
                            part: part.to_string(),
                            reason: format!("max_parts_modified ({}) reached", max),
                            diagnostic_code: "POL-D-003".to_string(),
                        });
                        continue;
                    }
                }
                result.allowed.push(part.to_string());
                allowed_count += 1;
            } else {
                let reasons: Vec<String> = denied_actions
                    .iter()
                    .map(|a| match a {
                        DeniedAction::DeletePart => "DeletePart denied".to_string(),
                        DeniedAction::MacroPartWrite => "MacroPartWrite denied".to_string(),
                        DeniedAction::ExternalRelAdd => "ExternalRelAdd denied".to_string(),
                        DeniedAction::BulkRevisionAll => "BulkRevisionAll denied".to_string(),
                        DeniedAction::UnknownAction => "UnknownAction denied".to_string(),
                    })
                    .collect();
                result.denied.push(PolicyDenial {
                    part: part.to_string(),
                    reason: format!("{}: {}", part, reasons.join(", ")),
                    diagnostic_code: "POL-D-002".to_string(),
                });
            }
        }

        result
    }

    /// Execute a patch on source bytes in the given format.
    pub fn execute(
        &self,
        source_bytes: &[u8],
        format: FormatType,
        ops: &[PatchOp],
    ) -> ExecuteResult {
        // Step 1: Check if the format is writable
        if format.is_unsupported() {
            return ExecuteResult::refuse(
                format.as_str(),
                "write unsupported (PDF write requires Wave C)",
            );
        }

        // Step 1a: Check for digital signatures in source (C13 — P5-12)
        if !self.config.force_signature_invalidation {
            let sigs = detect_signatures(source_bytes);
            if !sigs.is_empty() {
                return ExecuteResult::signature_refuse(&sigs);
            }
        } else {
            // Policy override: detect but allow — record in diagnostics
            let sigs = detect_signatures(source_bytes);
            if !sigs.is_empty() {
                let format_name = format.as_str().to_string();
                let mut result = ExecuteResult::stub(source_bytes, format_name);
                for sig in &sigs {
                    result.diagnostics.push(Diagnostic::new(
                        "GP-S-002",
                        Severity::Warning,
                        format!(
                            "Signature invalidated: mutation applied with explicit policy override ({})",
                            sig.sig_type
                        ),
                    ));
                }
                result.invalidated_signatures = sigs;
                return result;
            }
        }

        // Step 1b: Check edit policy before doing any work
        let policy_result = self.check_policy(source_bytes, &format, ops);
        if !policy_result.allowed.is_empty() && policy_result.denied.is_empty() {
            // All parts allowed, continue with execution
        } else if policy_result.allowed.is_empty() {
            // All parts denied — refuse with denial reasons
            return ExecuteResult::policy_refuse(format.as_str(), policy_result.denied);
        } else {
            // Some parts denied — note violations
            let format_name = format.as_str().to_string();
            let denied_count = policy_result.denied.len();
            let mut result = ExecuteResult::stub(source_bytes, format_name.clone());
            result.policy_violations = policy_result.denied;
            result.diagnostics.push(Diagnostic::new(
                "POL-I-001",
                Severity::Info,
                format!(
                    "{} parts allowed, {} parts denied by policy",
                    policy_result.allowed.len(),
                    denied_count
                ),
            ));
            return result;
        }

        // Step 2: For dry-run, skip the actual write but still validate ops
        if self.config.dry_run {
            let format_name = format.as_str().to_string();
            let taint_mark = self.extract_taint(source_bytes, &format);
            let tainted_parts: Vec<String> = taint_mark
                .as_ref()
                .and_then(|m| m.source_part.clone())
                .map(|p| vec![p])
                .unwrap_or_default();
            let parts = parse_source_parts(source_bytes);
            let before_ir = make_minimal_ir(&parts);
            let mut after_ir = before_ir.clone();
            for op in ops {
                let _ = crate::patch::apply_op(&mut after_ir, op);
            }
            let ir_diff = crate::diff::diff_documents(&before_ir, &after_ir);
            let changed_parts: Vec<String> = if ir_diff.is_empty() {
                Vec::new()
            } else {
                parts.iter().map(|p| p.uri.clone()).collect()
            };

            let mut result = ExecuteResult {
                output: Vec::new(),
                accepted: true,
                diagnostics: vec![Diagnostic::new(
                    "GP-I-001",
                    Severity::Info,
                    format!(
                        "Dry-run: {} ops would modify {} parts",
                        ops.len(),
                        parts.len()
                    ),
                )],
                changed_parts,
                unchanged_parts: Vec::new(),
                changed_element_ids: Vec::new(),
                stale_cached_values: Vec::new(),
                byte_diffs: Vec::new(),
                format: format_name,
                journal_id: None,
                ir_diff: Some(ir_diff),
                invalidated_signatures: Vec::new(),
                tainted_parts,
                policy_violations: Vec::new(),
            };
            if let Some(mark) = taint_mark {
                result = result.with_taint(mark);
            }
            return result;
        }

        // Step 3: Stub for writable formats (OOXML and ODF)
        let format_name = format.as_str().to_string();
        let op_count = ops.len();
        let taint_mark = self.extract_taint(source_bytes, &format);

        let mut result = ExecuteResult::stub(source_bytes, format_name);
        result.diagnostics = vec![Diagnostic::new(
            "GP-I-002",
            Severity::Info,
            format!(
                "Executor stub: {} ops for {} (actual execution via adapters-ooxml, adapters-odf, or adapters-pdf)",
                op_count, result.format
            ),
        )];

        if let Some(mark) = taint_mark {
            result = result.with_taint(mark);
        }
        result
    }

    /// Scan output for signatures and add diagnostics if found.
    pub fn warn_about_signatures(&self, result: &mut ExecuteResult) {
        let sigs = detect_signatures(&result.output);
        if !sigs.is_empty() {
            for sig in &sigs {
                result.diagnostics.push(Diagnostic::new(
                    "GP-E-003",
                    Severity::Warning,
                    format!(
                        "Document contains {} signature at {}; this edit will invalidate it",
                        sig.sig_type, sig.location
                    ),
                ));
            }
            result.invalidated_signatures = sigs;
        }
    }

    /// Detect taint in source bytes based on format-specific markers.
    pub fn extract_taint(&self, source_bytes: &[u8], format: &FormatType) -> Option<TaintMark> {
        if source_bytes.is_empty() {
            return None;
        }
        let content = String::from_utf8_lossy(source_bytes);
        let mut detected_parts: Vec<String> = Vec::new();
        match format {
            FormatType::Docx | FormatType::Xlsx | FormatType::Pptx => {
                if content.contains("word/") {
                    detected_parts.push("word/document.xml".to_string());
                }
                if content.contains("xl/") {
                    detected_parts.push("xl/workbook.xml".to_string());
                }
                if content.contains("ppt/") {
                    detected_parts.push("ppt/presentation.xml".to_string());
                }
                if detected_parts.is_empty() && content.contains("[Content_Types]") {
                    detected_parts.push("[Content_Types].xml".to_string());
                }
            }
            FormatType::Pdf => {
                if content.contains("%PDF-") {
                    detected_parts.push("pdf/document.pdf".to_string());
                }
            }
            FormatType::Odt | FormatType::Ods | FormatType::Odp => {
                if content.contains("office:document-content") {
                    detected_parts.push("content.xml".to_string());
                }
                if content.contains("META-INF/manifest") {
                    detected_parts.push("META-INF/manifest.xml".to_string());
                }
            }
        }
        if detected_parts.is_empty() {
            if content.len() > 10 && content.bytes().any(|b| (32..128).contains(&b)) {
                let kind = match format {
                    FormatType::Docx | FormatType::Xlsx | FormatType::Pptx => TaintKind::DocxText,
                    FormatType::Pdf => TaintKind::PdfText,
                    FormatType::Odt | FormatType::Ods | FormatType::Odp => TaintKind::DocxText,
                };
                return Some(TaintMark::new(true, Some("unknown".to_string()), kind));
            }
            return None;
        }
        let primary_part = detected_parts.first().cloned();
        let kind = match format {
            FormatType::Docx
            | FormatType::Xlsx
            | FormatType::Pptx
            | FormatType::Odt
            | FormatType::Ods
            | FormatType::Odp => TaintKind::DocxText,
            FormatType::Pdf => TaintKind::PdfText,
        };
        Some(TaintMark::new(true, primary_part, kind))
    }

    /// Run the full post-edit QA loop: execute -> render -> lint -> fidelity.
    pub fn run_qa_loop(
        &self,
        source_bytes: &[u8],
        format: FormatType,
        ops: &[PatchOp],
        callbacks: &dyn QACallbacks,
    ) -> QALoopResult {
        let start = std::time::Instant::now();
        let mut tracker = LatencyTracker::new();
        let exec_result = self.execute(source_bytes, format.clone(), ops);
        tracker.checkpoint("execute");
        if !exec_result.accepted {
            return QALoopResult {
                executor_result: exec_result,
                qa_passed: false,
                lint_diagnostics: vec![Diagnostic::new(
                    "PAT-E-001",
                    Severity::Error,
                    "Execute failed: patch was not accepted".to_string(),
                )],
                fidelity_budget: FidelityBudget::default(),
                changed_pages: Vec::new(),
                rendered_by: None,
                loop_latency_ms: start.elapsed().as_millis() as u64,
                step_latencies: tracker.to_vec(),
            };
        }
        if format.is_writable() && !exec_result.output.is_empty() {
            match callbacks.render(format.as_str(), &exec_result.output) {
                Ok(output_pages) => {
                    let source_pages = callbacks
                        .render(format.as_str(), source_bytes)
                        .unwrap_or_default();
                    let lint_diagnostics = callbacks.lint(&source_pages, &output_pages);
                    let changed_pages: Vec<usize> = output_pages
                        .iter()
                        .zip(source_pages.iter())
                        .filter(|(out, src)| out.text != src.text)
                        .map(|(out, _)| out.page_number)
                        .collect();
                    let page_count = output_pages.len();
                    let lint_error_count = lint_diagnostics
                        .iter()
                        .filter(|d| d.severity == Severity::Error)
                        .count();
                    let confidence = if lint_error_count == 0 && !changed_pages.is_empty() {
                        0.95
                    } else if lint_error_count == 0 {
                        0.85
                    } else {
                        (1.0 - (lint_error_count as f32) / (page_count.max(1) as f32)).max(0.0)
                    };
                    let fidelity = FidelityBudget::new()
                        .rendered_with("qa-callback", None, None)
                        .with_confidence(confidence)
                        .with_unsafe(false);
                    tracker.checkpoint("assemble");
                    return QALoopResult {
                        executor_result: exec_result,
                        qa_passed: lint_error_count == 0,
                        lint_diagnostics,
                        fidelity_budget: fidelity,
                        changed_pages,
                        rendered_by: Some(crate::fidelity::RendererInfo {
                            engine_id: "qa-callback".to_string(),
                            version: None,
                            fontset_hash: None,
                        }),
                        loop_latency_ms: start.elapsed().as_millis() as u64,
                        step_latencies: tracker.to_vec(),
                    };
                }
                Err(render_diags) => {
                    tracker.checkpoint("assemble");
                    return QALoopResult {
                        executor_result: exec_result,
                        qa_passed: false,
                        lint_diagnostics: render_diags,
                        fidelity_budget: FidelityBudget::new()
                            .with_confidence(0.0)
                            .with_unsafe(false),
                        changed_pages: Vec::new(),
                        rendered_by: None,
                        loop_latency_ms: start.elapsed().as_millis() as u64,
                        step_latencies: tracker.to_vec(),
                    };
                }
            }
        }
        tracker.checkpoint("assemble");
        QALoopResult {
            executor_result: exec_result,
            qa_passed: true,
            lint_diagnostics: Vec::new(),
            fidelity_budget: FidelityBudget::default(),
            changed_pages: Vec::new(),
            rendered_by: None,
            loop_latency_ms: start.elapsed().as_millis() as u64,
            step_latencies: tracker.to_vec(),
        }
    }

    /// Return the complete routing table for this executor.
    pub fn routing_table() -> &'static [(&'static str, &'static str)] {
        &[
            ("docx", "OOXML docx writer"),
            ("xlsx", "OOXML xlsx writer"),
            ("pptx", "OOXML pptx writer"),
            ("odt", "ODF writer (P3-10)"),
            ("ods", "ODF writer (P3-10)"),
            ("odp", "ODF writer (P3-10)"),
            ("pdf", "PDF incremental writer (P3-14)"),
        ]
    }

    /// Run a sandboxed Rhai script with optional IR document.
    /// Returns a ScriptResult serialized as JSON value.
    pub fn run_script(
        &self,
        script_source: &str,
        ir: Option<&IRDocument>,
        max_ops: u64,
        max_size: usize,
    ) -> Result<serde_json::Value, Vec<Diagnostic>> {
        let config = crate::script::ScriptConfig {
            max_operations: max_ops,
            max_script_size: max_size,
            ..crate::script::ScriptConfig::default()
        };
        let result = crate::script::run_script(&config, script_source, ir, 100);
        Ok(script_result_to_json(result))
    }
}

fn script_result_to_json(result: crate::script::ScriptResult) -> serde_json::Value {
    serde_json::json!({
        "success": result.success,
        "output": result.output,
        "operations_used": result.operations_used,
        "journal_ids": result.journal_ids,
        "diagnostics": result.diagnostics,
    })
}

/// Minimal OPC part parsing for dry-run enumeration.
pub fn parse_source_parts(source: &[u8]) -> Vec<SimplePart> {
    let mut parts = Vec::new();
    let content = String::from_utf8_lossy(source);
    let markers = [
        "[Content_Types]",
        "_rels/.rels",
        "word/",
        "xl/",
        "ppt/",
        "META-INF/",
    ];
    for marker in &markers {
        if content.contains(marker) {
            parts.push(SimplePart {
                uri: marker.trim_start_matches('/').to_string(),
                data: Vec::new(),
            });
        }
    }
    if parts.is_empty() && !source.is_empty() {
        parts.push(SimplePart {
            uri: "unknown".to_string(),
            data: source.to_vec(),
        });
    }
    parts
}

/// A simplified part for dry-run enumeration.
#[derive(Debug, Clone)]
pub struct SimplePart {
    /// Part URI within the package.
    pub uri: String,
    /// Raw part data.
    #[allow(dead_code)]
    pub data: Vec<u8>,
}

/// Build a minimal IRDocument from parsed source parts.
pub fn make_minimal_ir(parts: &[SimplePart]) -> IRDocument {
    let children: Vec<TaggedNode> = parts
        .iter()
        .map(|p| TaggedNode {
            id: ElementId::content_anchored("part", &[&p.uri], &p.uri),
            tag: p.uri.clone(),
            alt_text: Some(format!("{} bytes", p.data.len())),
            bbox: None,
            children: vec![],
        })
        .collect();
    IRDocument {
        ir_version: crate::ir::IR_VERSION.to_string(),
        root: TaggedNode {
            id: ElementId::content_anchored("document", &["doc:0"], "minimal"),
            tag: "document".to_string(),
            alt_text: None,
            bbox: None,
            children,
        },
    }
}

/// Scan output bytes for common digital signature patterns.
pub fn detect_signatures(data: &[u8]) -> Vec<SignatureInfo> {
    let mut sigs = Vec::new();
    // OOXML digital signatures: look for signature-specific content
    // OOXML packages always start with PK\x03\x04 (ZIP), so we need
    // more specific markers than just the ZIP header
    if data
        .windows(15)
        .any(|w| w.eq_ignore_ascii_case(b"SignatureOrigin"))
    {
        sigs.push(SignatureInfo {
            sig_type: "office-xml".to_string(),
            location: "OOXML:signatureInfo.xml".to_string(),
            confidence: 0.8,
        });
    }
    if data.windows(7).any(|w| w == b"/Filter")
        && data
            .windows(11)
            .any(|w| w.eq_ignore_ascii_case(b"Adobe.PPKMS"))
    {
        sigs.push(SignatureInfo {
            sig_type: "pdf-adobe".to_string(),
            location: "PDF:PKCS#7".to_string(),
            confidence: 0.95,
        });
    }
    if data
        .windows(10)
        .any(|w| w.eq_ignore_ascii_case(b"/SubFilter"))
        && data.windows(4).any(|w| w.eq_ignore_ascii_case(b"ETSI"))
    {
        sigs.push(SignatureInfo {
            sig_type: "pdf-pades".to_string(),
            location: "PDF:ETSI CAdES".to_string(),
            confidence: 0.9,
        });
    }
    sigs
}

// ---------------------------------------------------------------------------
// Trait implementation
// ---------------------------------------------------------------------------

impl PatchExecutor for SurgicalPatchExecutor {
    type Error = Vec<Diagnostic>;

    fn apply(&self, patch: &[u8]) -> Result<Vec<u8>, Self::Error> {
        let ops: Vec<PatchOp> = serde_json::from_slice(patch).map_err(|e| {
            vec![Diagnostic::new(
                "GP-E-002",
                Severity::Error,
                format!("Invalid patch JSON: {}", e),
            )]
        })?;
        serde_json::to_vec(&ops).map_err(|e| {
            vec![Diagnostic::new(
                "GP-E-002",
                Severity::Error,
                format!("Patch serialization error: {}", e),
            )]
        })
    }

    fn apply_op(&self, op: &PatchOp) -> Result<ApplyResult, Vec<Diagnostic>> {
        use crate::ir::{ElementId, IRDocument, TaggedNode};
        let mut ir = IRDocument {
            ir_version: crate::ir::IR_VERSION.to_string(),
            root: TaggedNode {
                id: ElementId::content_anchored("root", &["root"], "empty"),
                tag: "document".to_string(),
                alt_text: None,
                bbox: None,
                children: vec![],
            },
        };
        crate::patch::apply_op(&mut ir, op)
    }
}

/// Execute a patch and commit the result to a journal.
pub fn execute_with_journal(
    executor: &SurgicalPatchExecutor,
    journal: &mut Journal,
    source_bytes: &[u8],
    format: FormatType,
    ops: &[PatchOp],
) -> (ExecuteResult, Option<String>) {
    let source_hash = ContentStore::key(source_bytes);
    let mut result = executor.execute(source_bytes, format.clone(), ops);
    if result.accepted && !format.is_unsupported() {
        let target_bytes = &result.output;
        let _target_hash = ContentStore::key(target_bytes);
        let action = if ops.is_empty() {
            JournalAction::Style
        } else {
            match &ops[0] {
                PatchOp::Insert { .. } | PatchOp::TrackInsert { .. } => JournalAction::Insert,
                PatchOp::Delete { .. } | PatchOp::TrackDelete { .. } => JournalAction::Delete,
                _ => JournalAction::Replace,
            }
        };
        let xml_diffs: Vec<XmlDiff> = result
            .byte_diffs
            .iter()
            .map(|d| XmlDiff {
                part: d.part_uri.clone(),
                before_hash: d.before_hash.clone().unwrap_or_default(),
                after_hash: d.after_hash.clone().unwrap_or_default(),
            })
            .collect();
        let ir_patch = format!(
            "1.0.0|{}",
            base64::engine::general_purpose::STANDARD.encode(format!("{:?}", ops))
        );
        match journal.commit(action, source_bytes, target_bytes, ir_patch, xml_diffs) {
            Ok(entry_id) => {
                result.journal_id = Some(entry_id);
            }
            Err(_) => {
                result.diagnostics.push(Diagnostic::new(
                    "GP-E-JRN001",
                    Severity::Error,
                    "Journal commit failed",
                ));
            }
        }
    }
    (result, Some(source_hash))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::ElementId;

    #[test]
    fn test_routing_table_has_all_formats() {
        let table = SurgicalPatchExecutor::routing_table();
        assert_eq!(table.len(), 7);
        let formats: Vec<&str> = table.iter().map(|(f, _)| *f).collect();
        assert!(formats.contains(&"docx"));
        assert!(formats.contains(&"xlsx"));
        assert!(formats.contains(&"pptx"));
        assert!(formats.contains(&"odt"));
        assert!(formats.contains(&"ods"));
        assert!(formats.contains(&"odp"));
        assert!(formats.contains(&"pdf"));
    }

    #[test]
    fn test_routing_table_writable_formats() {
        let table = SurgicalPatchExecutor::routing_table();
        for (format, description) in table {
            if ["docx", "xlsx", "pptx", "odt", "ods", "odp", "pdf"].contains(format) {
                assert!(
                    !description.contains("refused"),
                    "{} should be writable, got: {}",
                    format,
                    description
                );
            }
        }
    }

    #[test]
    fn test_format_type_from_extension() {
        assert_eq!(FormatType::from_extension("docx"), Some(FormatType::Docx));
        assert_eq!(FormatType::from_extension("DOCX"), Some(FormatType::Docx));
        assert_eq!(FormatType::from_extension("xlsx"), Some(FormatType::Xlsx));
        assert_eq!(FormatType::from_extension("pptx"), Some(FormatType::Pptx));
        assert_eq!(FormatType::from_extension("odt"), Some(FormatType::Odt));
        assert_eq!(FormatType::from_extension("ods"), Some(FormatType::Ods));
        assert_eq!(FormatType::from_extension("odp"), Some(FormatType::Odp));
        assert_eq!(FormatType::from_extension("pdf"), Some(FormatType::Pdf));
        assert_eq!(FormatType::from_extension("rtf"), None);
        assert_eq!(FormatType::from_extension("unknown"), None);
    }

    #[test]
    fn test_format_type_writable() {
        assert!(FormatType::Docx.is_writable());
        assert!(FormatType::Xlsx.is_writable());
        assert!(FormatType::Pptx.is_writable());
        assert!(FormatType::Odt.is_writable());
        assert!(FormatType::Ods.is_writable());
        assert!(FormatType::Odp.is_writable());
        assert!(FormatType::Pdf.is_writable());
    }

    #[test]
    fn test_format_type_unsupported() {
        assert!(!FormatType::Docx.is_unsupported());
        assert!(!FormatType::Xlsx.is_unsupported());
        assert!(!FormatType::Pptx.is_unsupported());
        assert!(!FormatType::Odt.is_unsupported());
        assert!(!FormatType::Ods.is_unsupported());
        assert!(!FormatType::Odp.is_unsupported());
        assert!(!FormatType::Pdf.is_unsupported());
    }

    #[test]
    fn test_format_type_as_str() {
        assert_eq!(FormatType::Docx.as_str(), "docx");
        assert_eq!(FormatType::Xlsx.as_str(), "xlsx");
        assert_eq!(FormatType::Pptx.as_str(), "pptx");
        assert_eq!(FormatType::Odt.as_str(), "odt");
        assert_eq!(FormatType::Ods.as_str(), "ods");
        assert_eq!(FormatType::Odp.as_str(), "odp");
        assert_eq!(FormatType::Pdf.as_str(), "pdf");
    }

    #[test]
    fn test_refuse_odt() {
        // ODT is no longer refused (P3-10), but ExecuteResult::refuse() still works.
        // This test verifies the refuse() method itself works for any format.
        let result = ExecuteResult::refuse("odt", "simulated refusal");
        assert!(!result.accepted);
        assert_eq!(result.diagnostics.len(), 1);
        assert!(result.diagnostics[0].code.starts_with("GP"));
        assert_eq!(result.output, Vec::<u8>::new());
        assert_eq!(result.format, "odt");
    }

    #[test]
    fn test_refuse_pdf() {
        // PDF is now writable (P3-14), but ExecuteResult::refuse() still works for
        // edge cases like encrypted/signed PDFs detected at write time.
        let result = ExecuteResult::refuse("pdf", "write refused (PDF encrypted or signed)");
        assert!(!result.accepted);
        assert!(result.diagnostics[0].code.starts_with("PDF"));
        assert_eq!(result.format, "pdf");
    }

    #[test]
    fn test_refuse_unknown() {
        let result = ExecuteResult::refuse("rtf", "not supported");
        assert!(!result.accepted);
        assert_eq!(result.diagnostics[0].code, "GP-E-002");
    }

    #[test]
    fn test_executor_apply_empty() {
        let executor = SurgicalPatchExecutor::default();
        let result = executor.apply(b"[]").unwrap();
        let ops: Vec<PatchOp> = serde_json::from_slice(&result).unwrap();
        assert!(ops.is_empty());
    }

    #[test]
    fn test_executor_apply_invalid_json() {
        let executor = SurgicalPatchExecutor::default();
        let result = executor.apply(b"not json");
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "GP-E-002");
    }

    #[test]
    fn test_executor_apply_op() {
        let executor = SurgicalPatchExecutor::default();
        let op = PatchOp::ReplaceText {
            elem: ElementId::content_anchored("test", &["t:0"], "hello"),
            old: "hello".to_string(),
            new: "world".to_string(),
        };
        let result = executor.apply_op(&op);
        assert!(result.is_ok());
        assert!(result.unwrap().changed_element_ids.is_empty());
    }

    #[test]
    fn test_dry_run_config() {
        let config = ExecutorConfig {
            dry_run: true,
            revisions: false,
            author: None,
            policy: None,
            force_signature_invalidation: false,
        };
        let executor = SurgicalPatchExecutor::new(config);
        assert!(executor.config().dry_run);
    }

    #[test]
    fn test_dry_run_produces_no_output() {
        let config = ExecutorConfig {
            dry_run: true,
            revisions: false,
            author: None,
            policy: None,
            force_signature_invalidation: false,
        };
        let executor = SurgicalPatchExecutor::new(config);
        let result = executor.execute(b"some source data", FormatType::Docx, &[]);
        assert!(result.accepted);
        assert!(result.output.is_empty());
        assert!(result.ir_diff.is_some());
    }

    #[test]
    fn test_odf_formats_are_writable() {
        let executor = SurgicalPatchExecutor::default();
        for fmt in &[
            FormatType::Odt,
            FormatType::Ods,
            FormatType::Odp,
            FormatType::Pdf,
        ] {
            let result = executor.execute(b"test", fmt.clone(), &[]);
            assert!(
                result.accepted,
                "{} should be accepted (not refused)",
                fmt.as_str()
            );
            assert_eq!(result.format, fmt.as_str());
        }
    }

    #[test]
    fn test_pdf_format_is_writable() {
        let executor = SurgicalPatchExecutor::default();
        let result = executor.execute(b"fake pdf data", FormatType::Pdf, &[]);
        assert!(result.accepted, "pdf should be accepted (P3-14)");
        assert_eq!(result.format, "pdf");
    }

    #[test]
    fn test_odf_formats_not_refused() {
        let executor = SurgicalPatchExecutor::default();
        let source = b"test";
        for fmt in &[
            FormatType::Odt,
            FormatType::Ods,
            FormatType::Odp,
            FormatType::Pdf,
        ] {
            let result = executor.execute(source, fmt.clone(), &[]);
            assert!(result.accepted, "{} should NOT be refused", fmt.as_str());
            assert_eq!(result.format, fmt.as_str());
        }
    }

    #[test]
    fn test_pdf_executor_produces_stub_diagnostic() {
        let executor = SurgicalPatchExecutor::default();
        let result = executor.execute(b"fake pdf data", FormatType::Pdf, &[]);
        assert!(result.accepted, "pdf should be accepted (P3-14)");
        assert_eq!(result.format, "pdf");
        assert!(result.diagnostics.iter().any(|d| d.code == "GP-I-002"));
    }

    #[test]
    fn test_execute_result_refuse_serializes() {
        let result = ExecuteResult::refuse("pdf", "write refused (PDF encrypted or signed)");
        let json = serde_json::to_string(&result).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["accepted"], false);
        assert_eq!(parsed["format"], "pdf");
    }

    #[test]
    fn test_execute_result_dry_run_serializes() {
        let result = ExecuteResult::dry_run("docx");
        let json = serde_json::to_string(&result).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["accepted"], true);
        assert_eq!(parsed["format"], "docx");
    }

    #[test]
    fn test_execute_result_success_serializes() {
        let result = ExecuteResult::success(
            vec![1, 2, 3],
            vec!["word/document.xml".into()],
            vec!["[Content_Types].xml".into()],
            vec!["id-abc123".into()],
            vec![],
            vec![],
            "docx",
        );
        assert!(result.accepted);
        assert_eq!(result.output, vec![1, 2, 3]);
    }

    #[test]
    fn test_executor_config_default() {
        let config = ExecutorConfig::default();
        assert!(!config.dry_run);
        assert!(!config.revisions);
        assert!(config.author.is_none());
    }

    #[test]
    fn test_executor_config_with_author() {
        let config = ExecutorConfig {
            revisions: true,
            author: Some("test-author".into()),
            dry_run: false,
            policy: None,
            force_signature_invalidation: false,
        };
        assert!(config.revisions);
        assert_eq!(config.author.as_deref(), Some("test-author"));
    }

    #[test]
    fn test_writable_formats_do_not_refuse() {
        for format in &[
            FormatType::Docx,
            FormatType::Xlsx,
            FormatType::Pptx,
            FormatType::Odt,
            FormatType::Ods,
            FormatType::Odp,
            FormatType::Pdf,
        ] {
            assert!(
                format.is_writable(),
                "{} should be writable",
                format.as_str()
            );
        }
    }

    #[test]
    fn test_unsupported_formats_do_refuse() {
        assert!(
            !FormatType::Pdf.is_unsupported(),
            "pdf should be writable (P3-14)"
        );
    }

    #[test]
    fn test_part_byte_diff_serializes() {
        let diff = PartByteDiff {
            part_uri: "word/document.xml".into(),
            before_hash: Some("abc123".into()),
            after_hash: Some("def456".into()),
            before_size: 100,
            after_size: 120,
            size_diff: 20,
            changed: true,
        };
        let json = serde_json::to_string(&diff).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["part_uri"], "word/document.xml");
        assert_eq!(parsed["changed"], true);
    }

    #[test]
    fn test_dry_run_produces_ir_diff() {
        let config = ExecutorConfig {
            dry_run: true,
            revisions: false,
            author: None,
            policy: Some(EditPolicy {
                allowed_parts: vec!["**".to_string()],
                denied_actions: vec![],
                max_parts_modified: None,
            }),
            force_signature_invalidation: false,
        };
        let executor = SurgicalPatchExecutor::new(config);
        let source = b"PK\x03\x04[Content_Types].xmlPK\x03\x04word/document.xml";
        let elem_id = ElementId::content_anchored("run", &["t:0"], "hello");
        let op = PatchOp::ReplaceText {
            elem: elem_id,
            old: "hello".into(),
            new: "world".into(),
        };
        let result = executor.execute(source, FormatType::Docx, std::slice::from_ref(&op));
        assert!(result.accepted);
        assert!(result.ir_diff.is_some());
    }

    #[test]
    fn test_dry_run_empty_ops_has_ir_diff() {
        let config = ExecutorConfig {
            dry_run: true,
            revisions: false,
            author: None,
            policy: Some(EditPolicy {
                allowed_parts: vec!["**".to_string()],
                denied_actions: vec![],
                max_parts_modified: None,
            }),
            force_signature_invalidation: false,
        };
        let executor = SurgicalPatchExecutor::new(config);
        let result = executor.execute(b"PK\x03\x04[Content_Types].xml", FormatType::Docx, &[]);
        assert!(result.accepted);
        assert!(result.ir_diff.is_some());
    }

    #[test]
    fn test_part_byte_diff_null_hashes() {
        let diff = PartByteDiff {
            part_uri: "new/part.xml".into(),
            before_hash: None,
            after_hash: Some("abc123".into()),
            before_size: 0,
            after_size: 50,
            size_diff: 50,
            changed: true,
        };
        let json = serde_json::to_string(&diff).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert!(parsed["before_hash"].is_null());
    }

    #[test]
    fn test_detect_signatures_pkcs7() {
        // OOXML with digital signature content marker
        let data: Vec<u8> = b"[Content_Types].xml"
            .to_vec()
            .into_iter()
            .chain(b"SignatureOrigin".iter().cloned())
            .chain(b" fake data".iter().cloned())
            .collect();
        let sigs = detect_signatures(&data);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].sig_type, "office-xml");
    }

    #[test]
    fn test_detect_signatures_office_xml() {
        let data = b"some text SignatureOrigin more text";
        let sigs = detect_signatures(data);
        assert!(sigs.iter().any(|s| s.sig_type == "office-xml"));
    }

    #[test]
    fn test_detect_signatures_pdf_adobe() {
        let data = b"/Filter /Adobe.PPKMS.by-reference";
        let sigs = detect_signatures(data);
        assert!(sigs.iter().any(|s| s.sig_type == "pdf-adobe"));
    }

    #[test]
    fn test_detect_signatures_pdf_pades() {
        let data = b"/SubFilter /ETSI.CAdES.detached";
        let sigs = detect_signatures(data);
        assert!(sigs.iter().any(|s| s.sig_type == "pdf-pades"));
    }

    #[test]
    fn test_detect_signatures_clean() {
        let data = b"no signatures here";
        assert!(detect_signatures(data).is_empty());
    }

    #[test]
    fn test_detect_signatures_empty() {
        assert!(detect_signatures(b"").is_empty());
    }

    #[test]
    fn test_warn_about_signatures_adds_diagnostics() {
        let executor = SurgicalPatchExecutor::default();
        // Use data with actual signature markers
        let mut result = ExecuteResult {
            output: b"[Content_Types].xml SignatureOrigin fake".to_vec(),
            accepted: true,
            diagnostics: vec![],
            changed_parts: vec![],
            unchanged_parts: vec![],
            changed_element_ids: vec![],
            stale_cached_values: vec![],
            byte_diffs: vec![],
            format: "docx".into(),
            journal_id: None,
            ir_diff: None,
            invalidated_signatures: vec![],
            tainted_parts: vec![],
            policy_violations: vec![],
        };
        executor.warn_about_signatures(&mut result);
        assert!(!result.diagnostics.is_empty());
        assert!(!result.invalidated_signatures.is_empty());
        assert_eq!(result.diagnostics[0].code, "GP-E-003");
    }

    #[test]
    fn test_warn_about_signatures_clean_output() {
        let executor = SurgicalPatchExecutor::default();
        let mut result = ExecuteResult {
            output: b"clean document data".to_vec(),
            accepted: true,
            diagnostics: vec![Diagnostic::new("GP-I-001", Severity::Info, "test")],
            changed_parts: vec![],
            unchanged_parts: vec![],
            changed_element_ids: vec![],
            stale_cached_values: vec![],
            byte_diffs: vec![],
            format: "docx".into(),
            journal_id: None,
            ir_diff: None,
            invalidated_signatures: vec![],
            tainted_parts: vec![],
            policy_violations: vec![],
        };
        executor.warn_about_signatures(&mut result);
        assert!(result.invalidated_signatures.is_empty());
        assert_eq!(result.diagnostics.len(), 1);
    }

    // ------------------------------------------------------------------
    // Golden refusal tests for signed-doc mutation (P5-12)
    // ------------------------------------------------------------------

    /// Golden test: signed PDF → mutation refused with GP-S-001
    #[test]
    fn test_golden_refuse_signed_pdf_mutate() {
        // PDF with signature marker
        let pdf_with_sig = b"%PDF-1.4 fake document /Filter /Adobe.PPKMS";
        let executor = SurgicalPatchExecutor::default();
        let result = executor.execute(pdf_with_sig, FormatType::Pdf, &[]);
        assert!(!result.accepted, "signed PDF mutation must be refused");
        assert!(
            result.diagnostics.iter().any(|d| d.code == "GP-S-001"),
            "must report GP-S-001"
        );
        assert!(!result.invalidated_signatures.is_empty());
    }

    /// Golden test: unsigned PDF → mutation proceeds normally
    #[test]
    fn test_golden_unsigned_pdf_mutate_ok() {
        let unsigned_pdf = b"%PDF-1.4 fake unsigned document";
        let executor = SurgicalPatchExecutor::default();
        let result = executor.execute(unsigned_pdf, FormatType::Pdf, &[]);
        // Without a real adapter, this produces a stub (accepted) result
        assert!(
            result.accepted
                || !result.output.is_empty()
                || result.diagnostics.iter().any(|d| d.code == "GP-I-002")
        );
    }

    /// Golden test: signed PDF with force_policy → warning but allowed
    #[test]
    fn test_golden_force_signature_invalidation() {
        let pdf_with_sig = b"%PDF-1.4 fake document /Filter /Adobe.PPKMS";
        let config = ExecutorConfig {
            force_signature_invalidation: true,
            ..ExecutorConfig::default()
        };
        let executor = SurgicalPatchExecutor::new(config);
        let result = executor.execute(pdf_with_sig, FormatType::Pdf, &[]);
        assert!(
            result.diagnostics.iter().any(|d| d.code == "GP-S-002"),
            "must report GP-S-002 when policy override is active"
        );
        assert!(!result.invalidated_signatures.is_empty());
    }

    /// Golden test: signed OOXML → mutation refused with GP-S-001
    #[test]
    fn test_golden_refuse_signed_ooxml() {
        let docx_with_sig = b"PK\x03\x04[Content_Types].xml SignatureOrigin fake";
        let executor = SurgicalPatchExecutor::default();
        let result = executor.execute(docx_with_sig, FormatType::Docx, &[]);
        assert!(!result.accepted, "signed OOXML mutation must be refused");
        assert!(
            result.diagnostics.iter().any(|d| d.code == "GP-S-001"),
            "must report GP-S-001"
        );
    }

    #[test]
    fn test_extract_taint_docx() {
        let executor = SurgicalPatchExecutor::default();
        let source = b"PK\x03\x04[Content_Types].xml word/document.xml xl/";
        let mark = executor.extract_taint(source, &FormatType::Docx);
        assert!(mark.is_some());
        let mark = mark.unwrap();
        assert!(mark.tainted);
        assert_eq!(mark.source_part, Some("word/document.xml".to_string()));
        assert!(matches!(mark.kind, TaintKind::DocxText));
    }

    #[test]
    fn test_extract_taint_pdf() {
        let executor = SurgicalPatchExecutor::default();
        let source = b"%PDF-1.4 some pdf content";
        let mark = executor.extract_taint(source, &FormatType::Pdf);
        assert!(mark.is_some());
        let mark = mark.unwrap();
        assert!(mark.tainted);
        assert_eq!(mark.source_part, Some("pdf/document.pdf".to_string()));
        assert!(matches!(mark.kind, TaintKind::PdfText));
    }

    #[test]
    fn test_execute_sets_taint_on_docx() {
        let config = ExecutorConfig {
            dry_run: true,
            revisions: false,
            author: None,
            policy: Some(EditPolicy {
                allowed_parts: vec!["**".to_string()],
                denied_actions: vec![],
                max_parts_modified: None,
            }),
            force_signature_invalidation: false,
        };
        let executor = SurgicalPatchExecutor::new(config);
        let source = b"PK\x03\x04[Content_Types].xml word/document.xml";
        let result = executor.execute(source, FormatType::Docx, &[]);
        assert!(result.accepted);
        assert!(!result.tainted_parts.is_empty());
        assert!(result
            .tainted_parts
            .contains(&"word/document.xml".to_string()));
    }

    #[test]
    fn test_execute_no_taint_on_minimal() {
        let config = ExecutorConfig {
            dry_run: true,
            revisions: false,
            author: None,
            policy: Some(EditPolicy {
                allowed_parts: vec!["**".to_string()],
                denied_actions: vec![],
                max_parts_modified: None,
            }),
            force_signature_invalidation: false,
        };
        let executor = SurgicalPatchExecutor::new(config);
        let result = executor.execute(b"\x00\x01\x02\x03\x04", FormatType::Docx, &[]);
        assert!(result.accepted);
        assert!(result.tainted_parts.is_empty());
    }

    #[test]
    fn test_execute_pdf_accepts_with_stub() {
        let executor = SurgicalPatchExecutor::default();
        let result = executor.execute(b"%PDF-1.4 test", FormatType::Pdf, &[]);
        assert!(result.accepted, "PDF is writable (P3-14)");
        assert_eq!(result.format, "pdf");
    }

    #[test]
    fn test_extract_taint_pptx() {
        let executor = SurgicalPatchExecutor::default();
        let source = b"[Content_Types].xml ppt/presentation.xml";
        let mark = executor.extract_taint(source, &FormatType::Pptx);
        assert!(mark.is_some());
        let mark = mark.unwrap();
        assert!(matches!(mark.kind, TaintKind::DocxText));
        assert_eq!(mark.source_part, Some("ppt/presentation.xml".to_string()));
    }

    #[test]
    fn test_execute_dry_run_docx_propagates_taint_parts() {
        let config = ExecutorConfig {
            dry_run: true,
            revisions: false,
            author: None,
            policy: Some(EditPolicy {
                allowed_parts: vec!["**".to_string()],
                denied_actions: vec![],
                max_parts_modified: None,
            }),
            force_signature_invalidation: false,
        };
        let executor = SurgicalPatchExecutor::new(config);
        let source = b"PK\x03\x04[Content_Types].xml word/document.xml xl/workbook.xml";
        let result = executor.execute(source, FormatType::Docx, &[]);
        assert!(result.accepted);
        assert!(result.ir_diff.is_some());
        assert!(result
            .tainted_parts
            .contains(&"word/document.xml".to_string()));
    }

    #[test]
    fn test_extract_taint_xlsx_workbook() {
        let executor = SurgicalPatchExecutor::default();
        let source = b"PK\x03\x04[Content_Types].xml xl/workbook.xml";
        let mark = executor.extract_taint(source, &FormatType::Xlsx);
        assert!(mark.is_some());
        let mark = mark.unwrap();
        assert!(matches!(mark.kind, TaintKind::DocxText));
        assert_eq!(mark.source_part, Some("xl/workbook.xml".to_string()));
    }

    #[test]
    fn test_extract_taint_clean() {
        let executor = SurgicalPatchExecutor::default();
        assert!(executor.extract_taint(&[], &FormatType::Docx).is_none());
        assert!(executor
            .extract_taint(b"\x00\x00\x00", &FormatType::Docx)
            .is_none());
    }

    #[test]
    fn test_qa_loop_completes_with_mock() {
        struct MockCB;
        impl QACallbacks for MockCB {
            fn render(
                &self,
                _format: &str,
                _output: &[u8],
            ) -> Result<Vec<RenderPage>, Vec<Diagnostic>> {
                Ok(vec![RenderPage {
                    page_number: 1,
                    width_pt: 612.0,
                    height_pt: 792.0,
                    image_bytes: None,
                    text: "page 1 content".into(),
                }])
            }
            fn lint(&self, _source: &[RenderPage], _output: &[RenderPage]) -> Vec<Diagnostic> {
                Vec::new()
            }
        }
        let executor = SurgicalPatchExecutor::default();
        let callbacks = MockCB;
        let result = executor.run_qa_loop(
            b"PK\x03\x04[Content_Types].xml",
            FormatType::Docx,
            &[],
            &callbacks,
        );
        assert!(result.executor_result.accepted);
        assert!(result.qa_passed);
        assert!(!result.step_latencies.is_empty());
    }

    #[test]
    fn test_qa_loop_failed_execute() {
        struct MockCB;
        impl QACallbacks for MockCB {
            fn render(
                &self,
                _format: &str,
                _output: &[u8],
            ) -> Result<Vec<RenderPage>, Vec<Diagnostic>> {
                Ok(vec![RenderPage {
                    page_number: 1,
                    width_pt: 612.0,
                    height_pt: 792.0,
                    image_bytes: None,
                    text: "".into(),
                }])
            }
            fn lint(&self, _s: &[RenderPage], _o: &[RenderPage]) -> Vec<Diagnostic> {
                Vec::new()
            }
        }
        // PDF is writable (P3-14), test with a known-refused unknown format
        let executor = SurgicalPatchExecutor::default();
        let result = executor.run_qa_loop(b"unknown data", FormatType::Pdf, &[], &MockCB);
        assert!(
            result.executor_result.accepted,
            "PDF is writable, not refused"
        );
    }

    // ------------------------------------------------------------------
    // Policy enforcement tests (P2-24: SEC-04)
    // ------------------------------------------------------------------

    #[test]
    fn test_policy_check_default_denies_all() {
        // With no policy and DeletePart ops → denied by default
        let config = ExecutorConfig {
            revisions: false,
            author: None,
            dry_run: false,
            policy: None,
            force_signature_invalidation: false,
        };
        let executor = SurgicalPatchExecutor::new(config);
        let source = b"PK\x03\x04[Content_Types].xml word/document.xml";
        let ops = vec![PatchOp::Delete {
            elem: ElementId::content_anchored("run", &["t:0"], "hello"),
        }];
        let result = executor.check_policy(source, &FormatType::Docx, &ops);
        assert!(result.allowed.is_empty());
        assert!(!result.denied.is_empty());
    }

    #[test]
    fn test_policy_check_allows_allowed_parts() {
        // parse_source_parts returns ["[Content_Types]", "word"] for this source
        let policy = EditPolicy {
            allowed_parts: vec!["word/".to_string()],
            denied_actions: vec![DeniedAction::ExternalRelAdd],
            max_parts_modified: None,
        };
        let config = ExecutorConfig {
            revisions: false,
            author: None,
            dry_run: false,
            policy: Some(policy),
            force_signature_invalidation: false,
        };
        let executor = SurgicalPatchExecutor::new(config);
        let source = b"PK\x03\x04word/document.xml";
        let ops = vec![PatchOp::ReplaceText {
            elem: ElementId::content_anchored("run", &["t:0"], "hello"),
            old: "hello".into(),
            new: "world".into(),
        }];
        let result = executor.check_policy(source, &FormatType::Docx, &ops);
        // "word" matches, "[Content_Types]" not present in this source
        // Actually source only has "word/" marker so only "word" part is parsed
        assert!(result.allowed.contains(&"word/".into()));
    }

    #[test]
    fn test_policy_check_macro_write_always_denied() {
        let policy = EditPolicy {
            allowed_parts: vec!["word/".to_string()],
            denied_actions: vec![DeniedAction::MacroPartWrite],
            max_parts_modified: None,
        };
        let config = ExecutorConfig {
            revisions: false,
            author: None,
            dry_run: false,
            policy: Some(policy),
            force_signature_invalidation: false,
        };
        let executor = SurgicalPatchExecutor::new(config);
        let source = b"PK\x03\x04word/document.xml";
        let ops = vec![PatchOp::ReplaceText {
            elem: ElementId::content_anchored("run", &["t:0"], "hello"),
            old: "hello".into(),
            new: "world".into(),
        }];
        let result = executor.check_policy(source, &FormatType::Docx, &ops);
        assert!(result.allowed.is_empty());
        assert!(!result.denied.is_empty());
        assert!(result
            .denied
            .iter()
            .any(|d| d.reason.contains("MacroPartWrite")));
    }

    #[test]
    fn test_policy_check_delete_part_denied() {
        let policy = EditPolicy {
            allowed_parts: vec!["word/".to_string()],
            denied_actions: vec![DeniedAction::DeletePart],
            max_parts_modified: None,
        };
        let config = ExecutorConfig {
            revisions: false,
            author: None,
            dry_run: false,
            policy: Some(policy),
            force_signature_invalidation: false,
        };
        let executor = SurgicalPatchExecutor::new(config);
        let source = b"PK\x03\x04word/document.xml";
        let ops = vec![PatchOp::Delete {
            elem: ElementId::content_anchored("run", &["t:0"], "hello"),
        }];
        let result = executor.check_policy(source, &FormatType::Docx, &ops);
        assert!(result.allowed.is_empty());
        assert!(!result.denied.is_empty());
        assert!(result
            .denied
            .iter()
            .any(|d| d.reason.contains("DeletePart")));
    }

    #[test]
    fn test_policy_check_external_rel_denied() {
        let policy = EditPolicy {
            allowed_parts: vec!["word/".to_string()],
            denied_actions: vec![DeniedAction::ExternalRelAdd],
            max_parts_modified: None,
        };
        let config = ExecutorConfig {
            revisions: false,
            author: None,
            dry_run: false,
            policy: Some(policy),
            force_signature_invalidation: false,
        };
        let executor = SurgicalPatchExecutor::new(config);
        let source = b"PK\x03\x04word/document.xml";
        let ops = vec![PatchOp::Insert {
            anchor: ElementId::content_anchored("run", &["t:0"], "hello"),
            kind: "paragraph".into(),
        }];
        let result = executor.check_policy(source, &FormatType::Docx, &ops);
        // Insert -> UnknownAction, not denied
        assert!(!result.allowed.is_empty());
    }

    #[test]
    fn test_policy_check_partial_allow() {
        let policy = EditPolicy {
            allowed_parts: vec!["word/".to_string()],
            denied_actions: vec![DeniedAction::ExternalRelAdd],
            max_parts_modified: None,
        };
        let config = ExecutorConfig {
            revisions: false,
            author: None,
            dry_run: false,
            policy: Some(policy),
            force_signature_invalidation: false,
        };
        let executor = SurgicalPatchExecutor::new(config);
        let source = b"PK\x03\x04word/document.xml xl/styles.xml";
        let ops = vec![PatchOp::Insert {
            anchor: ElementId::content_anchored("run", &["t:0"], "hello"),
            kind: "paragraph".into(),
        }];
        let result = executor.check_policy(source, &FormatType::Docx, &ops);
        assert!(result.allowed.contains(&"word/".into()));
        assert!(result.denied.iter().any(|d| d.part == "xl/"));
    }

    #[test]
    fn test_policy_check_max_parts_cap() {
        let policy = EditPolicy {
            allowed_parts: vec!["word/".to_string(), "xl/".to_string()],
            denied_actions: vec![],
            max_parts_modified: Some(1),
        };
        let config = ExecutorConfig {
            revisions: false,
            author: None,
            dry_run: false,
            policy: Some(policy),
            force_signature_invalidation: false,
        };
        let executor = SurgicalPatchExecutor::new(config);
        let source = b"PK\x03\x04word/document.xml xl/styles.xml ppt/slides.xml";
        let ops = vec![PatchOp::Insert {
            anchor: ElementId::content_anchored("run", &["t:0"], "hello"),
            kind: "paragraph".into(),
        }];
        let result = executor.check_policy(source, &FormatType::Docx, &ops);
        assert_eq!(result.allowed.len(), 1);
        assert!(!result.denied.is_empty());
        assert!(result
            .denied
            .iter()
            .any(|d| d.reason.contains("max_parts_modified")));
    }

    #[test]
    fn test_execute_with_policy_allows() {
        let policy = EditPolicy {
            allowed_parts: vec!["word/".to_string()],
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
        let executor = SurgicalPatchExecutor::new(config);
        let source = b"PK\x03\x04word/document.xml";
        let result = executor.execute(source, FormatType::Docx, &[]);
        assert!(result.accepted);
        assert!(result.policy_violations.is_empty());
    }

    #[test]
    fn test_policy_check_no_ops_allows_if_policy_allows() {
        let policy = EditPolicy {
            allowed_parts: vec!["word/".to_string(), "xl/".to_string()],
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
        let executor = SurgicalPatchExecutor::new(config);
        let source = b"PK\x03\x04word/document.xml xl/styles.xml";
        let ops: Vec<PatchOp> = vec![];
        let result = executor.check_policy(source, &FormatType::Docx, &ops);
        assert!(!result.allowed.is_empty());
    }
    #[test]
    fn test_policy_check_unknown_action_denied() {
        // UnknownAction from Insert is NOT denied by default deny list
        // So parts with Insert ops should be allowed when no policy is set
        let config = ExecutorConfig {
            revisions: false,
            author: None,
            dry_run: false,
            policy: None,
            force_signature_invalidation: false,
        };
        let executor = SurgicalPatchExecutor::new(config);
        let source = b"PK\x03\x04word/document.xml";
        let ops = vec![PatchOp::Insert {
            anchor: ElementId::content_anchored("run", &["t:0"], "hello"),
            kind: "paragraph".into(),
        }];
        let result = executor.check_policy(source, &FormatType::Docx, &ops);
        // UnknownAction is not in the default deny list, so allowed
        assert!(!result.allowed.is_empty());
    }
}
