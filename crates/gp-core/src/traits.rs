// SPDX-License-Identifier: MIT OR Apache-2.0
//! Traits that adapters, engines, linters, and the patch executor must implement.
//!
//! These traits are the contract surface between crate families.

use crate::diagnostics::{Diagnostic, Severity};
use crate::ir::TextWithBbox;
use crate::patch::{ApplyResult, PatchOp};

/// A byte-surgical patch executor.
///
/// Phase 2 provides `SurgicalPatchExecutor` (in `gp_core::executor`) that
/// routes format-specific writers for docx/xlsx/pptx and refuses ODF/PDF
/// with explicit C3 codes.
pub trait PatchExecutor: Send + Sync {
    /// Error type for patch application failures.
    type Error;

    /// Apply a patch (JSON-encoded PatchOps) and return result bytes.
    fn apply(&self, patch: &[u8]) -> Result<Vec<u8>, Self::Error>;

    /// Apply a typed patch operation to an IR document.
    fn apply_op(&self, op: &PatchOp) -> Result<ApplyResult, Vec<Diagnostic>>;
}

/// Additional Codec methods
pub trait CodecBuilder: Send + Sync {
    /// Create a codec for a specific format variant.
    fn for_format(format: &str) -> Option<Box<dyn Codec>>;

    /// List all formats this builder can produce.
    fn supported_formats(&self) -> Vec<&'static str>;
}

/// Engine capability info returned by Engine::probe_info()
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EngineInfo {
    pub engine_id: String,
    pub version: Option<String>,
    pub fontset_hash: Option<String>,
    pub is_available: bool,
    pub capabilities: Vec<CapabilityEntry>,
}

/// Extended Engine trait
pub trait EngineExt: Engine {
    /// Get full engine info including fontset hash and capabilities.
    fn info(&self) -> EngineInfo;

    /// Get the fontset hash (used in render cache keys, INV-4).
    fn fontset_hash(&self) -> Option<String>;

    /// List supported conversions as (source, target) pairs.
    fn supported_conversions(&self) -> Vec<(String, String)>;
}

/// Batch linter: lint multiple pages/documents
pub trait BatchLinter: Linter {
    /// Lint multiple pages at once, returning per-page diagnostics.
    fn lint_batch(&self, pages: &[LinterPage]) -> Vec<PageDiagnostic>;
}

/// Per-page diagnostic from batch lint
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PageDiagnostic {
    pub page_number: usize,
    pub diagnostics: Vec<Diagnostic>,
}
pub trait Codec: Send + Sync {
    /// The format name, e.g. "docx", "xlsx", "pptx", "odt", "ods", "odp", "pdf".
    fn format_name(&self) -> &str;

    /// Read a document from bytes into IR.
    fn read(&self, data: &[u8]) -> Result<ReadResult, Vec<Diagnostic>>;

    /// Serialize IR back to the native format bytes (best-effort).
    fn write(&self, _ir: &[u8]) -> Result<Vec<u8>, Vec<Diagnostic>> {
        Err(vec![Diagnostic::new(
            "GP-T001",
            Severity::Info,
            "write not yet implemented",
        )])
    }
}

/// The result of a read operation, including IR and fidelity diagnostics.
#[derive(Debug, Clone)]
pub struct ReadResult {
    /// Format-specific IR bytes (serde-serialized JSON).
    pub ir: Vec<u8>,
    /// Diagnostics about features that could not be faithfully represented.
    pub diagnostics: Vec<Diagnostic>,
}

/// A rendering engine that can convert documents.
pub trait Engine: Send + Sync {
    /// Engine identifier, e.g. "libreoffice", "ms-office".
    fn engine_id(&self) -> &str;

    /// Engine version string.
    fn version(&self) -> Option<&str>;

    /// Check if this engine can convert a given source format to a target format.
    fn can_convert(&self, source: &str, target: &str) -> bool;

    /// Run a conversion. Returns bytes of the output document.
    fn convert(&self, input: &[u8], source: &str, target: &str)
        -> Result<Vec<u8>, Vec<Diagnostic>>;

    /// Check if this engine is available on the current system.
    fn is_available(&self) -> bool;
}

/// Probes engine capabilities by actually converting a test document.
pub trait Probe: Send + Sync {
    /// Probe a specific engine for its actual capabilities.
    fn probe(&self, engine: &dyn Engine) -> CapabilityMatrix;
}

/// A layout linter that checks IR/bboxes for layout issues.
pub trait Linter: Send + Sync {
    /// Run lint checks and return diagnostics.
    fn lint(&self, input: &[u8], pages: &[LinterPage]) -> Vec<Diagnostic>;
}

/// A page of layout data for the linter.
#[derive(Debug, Clone)]
pub struct LinterPage {
    pub page_number: usize,
    pub width_pt: f64,
    pub height_pt: f64,
    /// Text fragments with bounding boxes.
    pub text_bboxes: Vec<TextWithBbox>,
    /// Whether the page exists (vs is missing from render).
    pub exists: bool,
}

/// A capability matrix from probing.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CapabilityMatrix {
    pub engine: String,
    pub version: Option<String>,
    pub conversions: Vec<CapabilityEntry>,
}

/// A single capability entry (e.g. docx → pdf).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CapabilityEntry {
    pub source: String,
    pub target: String,
    pub works: bool,
}

/// Extract text with bounding boxes from PDF bytes.
///
/// This trait abstracts the PDF text extraction used by the MCP review
/// text-layer route. Implementations (e.g. `adapters-pdf`) provide the
/// actual parsing; the `mcp` crate depends only on this trait.
pub trait PdfTextSource: Send + Sync {
    /// Extract text with bounding boxes from raw PDF bytes.
    fn extract_text(&self, pdf_bytes: &[u8]) -> Result<Vec<TextWithBbox>, Vec<Diagnostic>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_matrix_serializes() {
        let matrix = CapabilityMatrix {
            engine: "test".into(),
            version: Some("1.0".into()),
            conversions: vec![CapabilityEntry {
                source: "docx".into(),
                target: "pdf".into(),
                works: true,
            }],
        };
        let json = serde_json::to_string(&matrix).unwrap();
        let back: CapabilityMatrix = serde_json::from_str(&json).unwrap();
        assert_eq!(matrix, back);
    }
}
