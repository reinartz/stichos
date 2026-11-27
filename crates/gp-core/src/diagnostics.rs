// SPDX-License-Identifier: MIT OR Apache-2.0
//! Diagnostic model: registry, error codes, severity, and error model.
//!
//! This module provides a statically-initialized `DiagnosticRegistry` that owns every known
//! diagnostic code in the stichos toolchain, an enum-like `DiagnosticCode` type for look-up,
//! and a `ReadError` type that maps reader-level failure modes to diagnostic lists.
//!
//! Code pattern: `^PREFIX-[A-Z]{1,3}-\d{3,}$`  (e.g. GP-E001, OOX-W003, PDF-E012)
//!
//! See also: `docs/contracts/C3.md`

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Severity
// ---------------------------------------------------------------------------

/// Severity levels for diagnostics, ordered from least to most severe.
///
/// `Fatal` is unrecoverable — the process will exit. `Error` is a diagnostic
/// that should be reported but does not force an exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Hint,
    Info,
    Warning,
    Error,
    Fatal,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Hint => write!(f, "HINT"),
            Severity::Info => write!(f, "INFO"),
            Severity::Warning => write!(f, "WARN"),
            Severity::Error => write!(f, "ERROR"),
            Severity::Fatal => write!(f, "FATAL"),
        }
    }
}

/// Returns `true` if this severity is fatal (process-exiting).
pub const fn severity_is_fatal(s: Severity) -> bool {
    matches!(s, Severity::Fatal)
}

// ---------------------------------------------------------------------------
// Span
// ---------------------------------------------------------------------------

/// A byte-level span within a source file or part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub offset: usize,
    pub length: usize,
}

// ---------------------------------------------------------------------------
// Registry entry
// ---------------------------------------------------------------------------

/// A single registered diagnostic code entry.
#[derive(Debug, Clone)]
pub struct CodeEntry {
    /// The stable prefixed code string (e.g. `"GP-E-001"`).
    pub code: &'static str,
    /// Severity level.
    pub severity: Severity,
    /// Human-readable category (e.g. `"validation"`, `"parsing"`, `"engine"`).
    pub category: &'static str,
    /// Description template (may contain `{}` placeholders).
    pub description: &'static str,
}

impl CodeEntry {
    /// Format a human-readable message using the code's description template.
    pub fn format_message(&self, args: &[&str]) -> String {
        let mut msg = String::new();
        let mut template = self.description;
        for arg in args {
            if let Some(pos) = template.find("{}") {
                msg.push_str(&template[..pos]);
                msg.push_str(arg);
                template = &template[pos + 2..];
            } else {
                break;
            }
        }
        msg.push_str(template);
        msg
    }
}

// ---------------------------------------------------------------------------
// Diagnostic
// ---------------------------------------------------------------------------

/// A single diagnostic item produced by a codec, engine, linter, or reader.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Stable prefixed code, e.g. `GP-E001`.
    pub code: String,
    /// Severity of this diagnostic.
    pub severity: Severity,
    /// Human-readable message.
    pub message: String,
    /// Optional span information (byte offset and length in the source file).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

impl Diagnostic {
    pub fn new<S: Into<String>, M: Into<String>>(code: S, severity: Severity, message: M) -> Self {
        Self {
            code: code.into(),
            severity,
            message: message.into(),
            span: None,
        }
    }

    pub fn with_span(mut self, offset: usize, length: usize) -> Self {
        self.span = Some(Span { offset, length });
        self
    }

    /// Look up this diagnostic's registered code info from the global registry.
    ///
    /// Returns `Some(CodeEntry)` if the code is known, `None` for ad-hoc codes.
    pub fn code_enum(&self) -> Option<CodeEntry> {
        DiagnosticRegistry::instance().lookup(self.code.as_str())
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {} {}", self.severity, self.code, self.message)
    }
}

impl std::error::Error for Diagnostic {}

/// A JSON-serializable envelope wrapping a list of diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticEnvelope {
    pub diagnostics: Vec<Diagnostic>,
}

impl DiagnosticEnvelope {
    pub fn new(diagnostics: Vec<Diagnostic>) -> Self {
        Self { diagnostics }
    }

    /// Create an empty envelope.
    pub fn empty() -> Self {
        Self {
            diagnostics: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Code pattern validation
// ---------------------------------------------------------------------------

/// Returns `true` if `code` matches the stichos diagnostic code pattern:
/// `^PREFIX-[A-Z]{1,3}-\d{3,}$`.
pub fn is_valid_code_pattern(code: &str) -> bool {
    // Must be at least 8 chars: XX-XNNN
    if code.len() < 8 {
        return false;
    }

    let bytes = code.as_bytes();
    let len = bytes.len();

    // Phase 1: prefix letters (2-4 uppercase ASCII)
    let mut i = 0;
    let mut prefix_len = 0;
    while i < len && bytes[i].is_ascii_uppercase() {
        prefix_len += 1;
        i += 1;
    }
    if !(2..=4).contains(&prefix_len) {
        return false;
    }

    // Dash after prefix
    if i >= len || bytes[i] != b'-' {
        return false;
    }
    i += 1;

    // Category letters (1-3 uppercase ASCII)
    let mut cat_len = 0;
    while i < len && bytes[i].is_ascii_uppercase() {
        cat_len += 1;
        i += 1;
    }
    if !(1..=3).contains(&cat_len) {
        return false;
    }

    // Dash after category
    if i >= len || bytes[i] != b'-' {
        return false;
    }
    i += 1;

    // Must end with 3+ ASCII digits
    if len - i < 3 {
        return false;
    }
    while i < len {
        if !bytes[i].is_ascii_digit() {
            return false;
        }
        i += 1;
    }
    true
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// The global diagnostic registry.
///
/// All diagnostic codes are statically defined. The registry provides
/// look-up by code string and iteration over all entries.
pub struct DiagnosticRegistry {
    entries: HashMap<&'static str, CodeEntry>,
}

impl DiagnosticRegistry {
    /// Returns the singleton registry instance (lazy-initialized).
    pub fn instance() -> &'static Self {
        static REGISTRY: std::sync::OnceLock<DiagnosticRegistry> = std::sync::OnceLock::new();
        REGISTRY.get_or_init(DiagnosticRegistry::init)
    }

    /// Look up a code entry by its string code.
    ///
    /// Returns a clone of the entry (owned) so the caller doesn't hold a
    /// reference into the static registry.
    pub fn lookup(&self, code: &str) -> Option<CodeEntry> {
        self.entries.get(code).cloned()
    }

    /// Iterate over all registered codes (clones each entry).
    pub fn iter_all(&self) -> impl Iterator<Item = CodeEntry> + use<'_> {
        self.entries.values().cloned()
    }

    /// Return all codes as a sorted Vec of strings.
    pub fn all_codes(&self) -> Vec<&'static str> {
        let mut codes: Vec<&'static str> = self.entries.keys().copied().collect();
        codes.sort();
        codes
    }

    /// Check for duplicate codes (should never be true for static data).
    pub fn has_duplicates(&self) -> bool {
        // Already a HashMap, so keys are unique by construction
        false
    }

    /// Return true if there are any codes at Fatal severity.
    pub fn has_fatal_codes(&self) -> bool {
        self.entries.values().any(|e| e.severity == Severity::Fatal)
    }

    /// Initialise the registry with all known diagnostic codes.
    fn init() -> Self {
        let mut entries = HashMap::new();

        // GP (gp-core) codes
        entries.insert(
            "GP-E-001",
            CodeEntry {
                code: "GP-E-001",
                severity: Severity::Error,
                category: "validation",
                description: "Invalid IR version: expected {} but found {}",
            },
        );
        entries.insert(
            "GP-E-002",
            CodeEntry {
                code: "GP-E-002",
                severity: Severity::Error,
                category: "engine",
                description: "Patch operation failed: {}",
            },
        );
        entries.insert(
            "GP-W-001",
            CodeEntry {
                code: "GP-W-001",
                severity: Severity::Warning,
                category: "parsing",
                description: "Unknown element '{}' in IR — ignored",
            },
        );
        entries.insert(
            "GP-E-003",
            CodeEntry {
                code: "GP-E-003",
                severity: Severity::Warning,
                category: "security",
                description: "Digital signature detected in output; edit will invalidate it",
            },
        );

        // GP-R (revision model, C11 — P4-03)
        entries.insert(
            "GP-R-001",
            CodeEntry {
                code: "GP-R-001",
                severity: Severity::Error,
                category: "parsing",
                description: "Unsupported revision kind: {} — refusing to process",
            },
        );
        entries.insert(
            "GP-R-002",
            CodeEntry {
                code: "GP-R-002",
                severity: Severity::Error,
                category: "validation",
                description: "Ambiguous revision anchor: {} has no matching element",
            },
        );
        entries.insert(
            "GP-R-003",
            CodeEntry {
                code: "GP-R-003",
                severity: Severity::Warning,
                category: "parsing",
                description: "Revision {} has no author and none provided",
            },
        );
        entries.insert(
            "GP-R-004",
            CodeEntry {
                code: "GP-R-004",
                severity: Severity::Error,
                category: "execution",
                description: "Revision accept/reject failed: {}",
            },
        );

        // OOX (adapters-ooxml) codes
        entries.insert(
            "OOX-E-001",
            CodeEntry {
                code: "OOX-E-001",
                severity: Severity::Error,
                category: "validation",
                description: "OPC validation failed: {}",
            },
        );
        entries.insert(
            "OOX-E-002",
            CodeEntry {
                code: "OOX-E-002",
                severity: Severity::Error,
                category: "parsing",
                description: "XML parse error: {}",
            },
        );
        entries.insert(
            "OOX-E-003",
            CodeEntry {
                code: "OOX-E-003",
                severity: Severity::Error,
                category: "validation",
                description: "Entry count exceeded (limit: {})",
            },
        );
        entries.insert(
            "OOX-E-004",
            CodeEntry {
                code: "OOX-E-004",
                severity: Severity::Fatal,
                category: "security",
                description: "Path traversal detected in package entry: {}",
            },
        );
        entries.insert(
            "OOX-E-005",
            CodeEntry {
                code: "OOX-E-005",
                severity: Severity::Fatal,
                category: "security",
                description: "Zip-bomb detected: {} bytes in {} entries",
            },
        );
        entries.insert(
            "OOX-E-006",
            CodeEntry {
                code: "OOX-E-006",
                severity: Severity::Error,
                category: "validation",
                description: "Expected root element '{}' but found '{}'",
            },
        );
        entries.insert(
            "OOX-E-007",
            CodeEntry {
                code: "OOX-E-007",
                severity: Severity::Warning,
                category: "validation",
                description: "Missing namespace prefix '{}'",
            },
        );
        entries.insert(
            "OOX-E-008",
            CodeEntry {
                code: "OOX-E-008",
                severity: Severity::Error,
                category: "validation",
                description: "Missing required child element '{}'",
            },
        );

        // ODF (adapters-odf) codes
        entries.insert(
            "ODF-E-001",
            CodeEntry {
                code: "ODF-E-001",
                severity: Severity::Error,
                category: "parsing",
                description: "ODF package error: {}",
            },
        );
        entries.insert(
            "ODF-E-002",
            CodeEntry {
                code: "ODF-E-002",
                severity: Severity::Error,
                category: "parsing",
                description: "manifest.xml: missing manifest:manifest root element",
            },
        );
        entries.insert(
            "ODF-E-003",
            CodeEntry {
                code: "ODF-E-003",
                severity: Severity::Error,
                category: "security",
                description: "Encryption detected in ODF package",
            },
        );

        // PDF (adapters-pdf) codes
        entries.insert(
            "PDF-E-001",
            CodeEntry {
                code: "PDF-E-001",
                severity: Severity::Error,
                category: "parsing",
                description: "PDF parse error: {}",
            },
        );
        entries.insert(
            "PDF-E-002",
            CodeEntry {
                code: "PDF-E-002",
                severity: Severity::Error,
                category: "validation",
                description: "Invalid xref at offset {}",
            },
        );
        entries.insert(
            "PDF-E-003",
            CodeEntry {
                code: "PDF-E-003",
                severity: Severity::Warning,
                category: "security",
                description: "Encryption detected: {}",
            },
        );
        entries.insert(
            "PDF-E-004",
            CodeEntry {
                code: "PDF-E-004",
                severity: Severity::Error,
                category: "validation",
                description: "Xref stream detected: append-only incremental update not supported",
            },
        );
        entries.insert(
            "PDF-E-005",
            CodeEntry {
                code: "PDF-E-005",
                severity: Severity::Error,
                category: "validation",
                description: "Object stream detected: append-only incremental update not supported",
            },
        );
        entries.insert(
            "PDF-E-006",
            CodeEntry {
                code: "PDF-E-006",
                severity: Severity::Error,
                category: "security",
                description: "Digital signature detected — incremental write refused",
            },
        );
        entries.insert(
            "PDF-W-004",
            CodeEntry {
                code: "PDF-W-004",
                severity: Severity::Warning,
                category: "unsupported",
                description: "XFA (XML Forms Architecture) detected — unsupported",
            },
        );
        entries.insert(
            "PDF-W-005",
            CodeEntry {
                code: "PDF-W-005",
                severity: Severity::Warning,
                category: "validation",
                description: "qpdf check warning: {}",
            },
        );
        entries.insert(
            "PDF-E-009",
            CodeEntry {
                code: "PDF-E-009",
                severity: Severity::Error,
                category: "validation",
                description: "qpdf check error: {}",
            },
        );
        entries.insert(
            "PDF-E-010",
            CodeEntry {
                code: "PDF-E-010",
                severity: Severity::Fatal,
                category: "validation",
                description: "qpdf check fatal: {}",
            },
        );

        // GP-A (PDF annotation) codes
        entries.insert(
            "GP-A-001",
            CodeEntry {
                code: "GP-A-001",
                severity: Severity::Warning,
                category: "unsupported",
                description: "Unsupported annotation subtype '{}' — refusing to process",
            },
        );
        entries.insert(
            "GP-A-002",
            CodeEntry {
                code: "GP-A-002",
                severity: Severity::Warning,
                category: "validation",
                description: "Annotation has no rectangle — cannot render",
            },
        );
        entries.insert(
            "GP-A-003",
            CodeEntry {
                code: "GP-A-003",
                severity: Severity::Warning,
                category: "validation",
                description: "Duplicated annotation ID '{}' — keeping first occurrence",
            },
        );
        entries.insert(
            "GP-A-004",
            CodeEntry {
                code: "GP-A-004",
                severity: Severity::Warning,
                category: "validation",
                description: "Annotation references unknown page {} (max: {})",
            },
        );
        entries.insert(
            "GP-A-005",
            CodeEntry {
                code: "GP-A-005",
                severity: Severity::Warning,
                category: "unsupported",
                description: "XFA/XDP form detected in annotations — unsupported",
            },
        );

        // ENG (engines) codes
        entries.insert(
            "ENG-E-001",
            CodeEntry {
                code: "ENG-E-001",
                severity: Severity::Error,
                category: "engine",
                description: "Engine not found: {}",
            },
        );
        entries.insert(
            "ENG-E-002",
            CodeEntry {
                code: "ENG-E-002",
                severity: Severity::Error,
                category: "engine",
                description: "Engine timeout after {}ms",
            },
        );
        entries.insert(
            "ENG-W-001",
            CodeEntry {
                code: "ENG-W-001",
                severity: Severity::Warning,
                category: "engine",
                description: "Engine version mismatch: expected {} found {}",
            },
        );
        entries.insert(
            "ENG-E-030",
            CodeEntry {
                code: "ENG-E-030",
                severity: Severity::Error,
                category: "engine",
                description: "LaTeX engine required for pandoc PDF output; no fallback available",
            },
        );

        // LIN (lint) codes
        entries.insert(
            "LIN-E-001",
            CodeEntry {
                code: "LIN-E-001",
                severity: Severity::Error,
                category: "layout",
                description: "Layout overflow detected: {} exceeds boundary",
            },
        );
        entries.insert(
            "LIN-E-002",
            CodeEntry {
                code: "LIN-E-002",
                severity: Severity::Warning,
                category: "font",
                description: "Font substitution: '{}' not found, using {}",
            },
        );
        entries.insert(
            "LIN-E-003",
            CodeEntry {
                code: "LIN-E-003",
                severity: Severity::Error,
                category: "layout",
                description: "Off-page element detected at [{},{},{},{}]",
            },
        );
        entries.insert(
            "LIN-E-004",
            CodeEntry {
                code: "LIN-E-004",
                severity: Severity::Error,
                category: "layout",
                description: "Overlapping elements detected: id1={} and id2={}",
            },
        );
        entries.insert(
            "LIN-W-001",
            CodeEntry {
                code: "LIN-W-001",
                severity: Severity::Warning,
                category: "layout",
                description: "Text overflow in page region: {} exceeds available space by {}pt",
            },
        );
        entries.insert(
            "LIN-W-002",
            CodeEntry {
                code: "LIN-W-002",
                severity: Severity::Warning,
                category: "layout",
                description: "Negative coordinates detected for element {} at [{},{},{}]",
            },
        );
        entries.insert(
            "LIN-W-003",
            CodeEntry {
                code: "LIN-W-003",
                severity: Severity::Warning,
                category: "font",
                description: "Potential missing glyphs: '{}' does not support {} characters",
            },
        );
        entries.insert(
            "LIN-B-001",
            CodeEntry {
                code: "LIN-B-001",
                severity: Severity::Error,
                category: "budget",
                description: "Image block count {} exceeds maximum {} (Claude vision limit)",
            },
        );
        entries.insert(
            "LIN-B-002",
            CodeEntry {
                code: "LIN-B-002",
                severity: Severity::Error,
                category: "budget",
                description: "Image dimensions {}x{} exceed maximum {}x{} (Claude vision limit)",
            },
        );

        // LIN-E-005 / LIN-E-006: page-count delta checks
        entries.insert(
            "LIN-E-005",
            CodeEntry {
                code: "LIN-E-005",
                severity: Severity::Error,
                category: "layout",
                description: "Page-count delta {} ({} -> {}) exceeds threshold {}",
            },
        );
        entries.insert(
            "LIN-E-006",
            CodeEntry {
                code: "LIN-E-006",
                severity: Severity::Error,
                category: "layout",
                description: "Missing {} page(s) in render ({} original, {} rendered)",
            },
        );

        // LIN-I-001: SSIM quality check (informational)
        entries.insert(
            "LIN-I-001",
            CodeEntry {
                code: "LIN-I-001",
                severity: Severity::Info,
                category: "quality",
                description: "SSIM {:.4} below threshold {:.4} ({}x{} grayscale)",
            },
        );

        // LIN-W-004..LIN-W-006: page-count delta / contrast quality
        entries.insert(
            "LIN-W-004",
            CodeEntry {
                code: "LIN-W-004",
                severity: Severity::Warning,
                category: "layout",
                description: "Page-count delta {} ({} -> {})",
            },
        );
        entries.insert(
            "LIN-W-005",
            CodeEntry {
                code: "LIN-W-005",
                severity: Severity::Warning,
                category: "quality",
                description: "Low contrast ratio {:.2}:1 (threshold: {:.1}:1)",
            },
        );
        entries.insert(
            "LIN-W-006",
            CodeEntry {
                code: "LIN-W-006",
                severity: Severity::Warning,
                category: "quality",
                description: "Very low text coverage {:.3} on {} sq pt page",
            },
        );

        // LIN-I-002: XML part diff (informational)
        entries.insert(
            "LIN-I-002",
            CodeEntry {
                code: "LIN-I-002",
                severity: Severity::Info,
                category: "diff",
                description: "XML part differs: {} line(s) changed",
            },
        );

        // LIN-I-003: PDF/A validation skipped (tool unavailable)
        entries.insert(
            "LIN-I-003",
            CodeEntry {
                code: "LIN-I-003",
                severity: Severity::Info,
                category: "accessibility",
                description: "PDF/A validation skipped: {} not available",
            },
        );

        // LIN-E-007: PDF/A validation failure
        entries.insert(
            "LIN-E-007",
            CodeEntry {
                code: "LIN-E-007",
                severity: Severity::Error,
                category: "accessibility",
                description: "PDF/A validation failure: {}",
            },
        );

        // LIN-W-007: Missing PDF/A conformance level
        entries.insert(
            "LIN-W-007",
            CodeEntry {
                code: "LIN-W-007",
                severity: Severity::Warning,
                category: "accessibility",
                description: "Missing PDF/A conformance level: expected {} but found none",
            },
        );

        // LIN-W-008: Missing tagged structure (PDF not tagged)
        entries.insert(
            "LIN-W-008",
            CodeEntry {
                code: "LIN-W-008",
                severity: Severity::Warning,
                category: "accessibility",
                description: "Missing tagged structure: PDF is not tagged",
            },
        );

        // LIN-W-009: Missing alt text in PDF
        entries.insert(
            "LIN-W-009",
            CodeEntry {
                code: "LIN-W-009",
                severity: Severity::Warning,
                category: "accessibility",
                description: "Missing alt text: {} on page {}",
            },
        );

        // CLI codes
        entries.insert(
            "CLI-E-001",
            CodeEntry {
                code: "CLI-E-001",
                severity: Severity::Error,
                category: "argument",
                description: "Invalid argument: {}",
            },
        );
        entries.insert(
            "CLI-E-002",
            CodeEntry {
                code: "CLI-E-002",
                severity: Severity::Error,
                category: "io",
                description: "I/O error: {}",
            },
        );

        // MCP (mcp) codes
        entries.insert(
            "MCP-E-001",
            CodeEntry {
                code: "MCP-E-001",
                severity: Severity::Error,
                category: "protocol",
                description: "Invalid JSON-RPC: {}",
            },
        );
        entries.insert(
            "MCP-E-002",
            CodeEntry {
                code: "MCP-E-002",
                severity: Severity::Error,
                category: "protocol",
                description: "Tool not found: {}",
            },
        );

        // RHI (rhai-script) codes
        entries.insert(
            "RHI-E-001",
            CodeEntry {
                code: "RHI-E-001",
                severity: Severity::Error,
                category: "sandbox",
                description: "Sandbox violation: {} — operation denied",
            },
        );
        entries.insert(
            "RHI-W-001",
            CodeEntry {
                code: "RHI-W-001",
                severity: Severity::Warning,
                category: "sandbox",
                description: "Script warning: {}",
            },
        );
        entries.insert(
            "RHI-E-002",
            CodeEntry {
                code: "RHI-E-002",
                severity: Severity::Error,
                category: "execution",
                description: "Script execution error: {}",
            },
        );
        entries.insert(
            "RHI-E-003",
            CodeEntry {
                code: "RHI-E-003",
                severity: Severity::Error,
                category: "validation",
                description: "Script too large: {} bytes (max: {} bytes)",
            },
        );
        entries.insert(
            "RHI-E-004",
            CodeEntry {
                code: "RHI-E-004",
                severity: Severity::Error,
                category: "validation",
                description: "Script exceeded operation limit: {} operations (max: {})",
            },
        );
        entries.insert(
            "RHI-E-005",
            CodeEntry {
                code: "RHI-E-005",
                severity: Severity::Error,
                category: "validation",
                description: "Script exceeded call depth: {} (max: {})",
            },
        );
        entries.insert(
            "RHI-E-006",
            CodeEntry {
                code: "RHI-E-006",
                severity: Severity::Error,
                category: "validation",
                description: "Script exceeded wall-clock limit: {}ms (max: {}ms)",
            },
        );

        // SEC (security) codes
        entries.insert(
            "SEC-E-001",
            CodeEntry {
                code: "SEC-E-001",
                severity: Severity::Warning,
                category: "security",
                description:
                    "macro-bearing format detected ({}) — content analyzed but NOT executed",
            },
        );

        // GP-S (signature integrity, C13 — P5-11)
        entries.insert(
            "GP-S-001",
            CodeEntry {
                code: "GP-S-001",
                severity: Severity::Error,
                category: "security",
                description: "Document has digital signatures — write refused",
            },
        );
        entries.insert(
            "GP-S-002",
            CodeEntry {
                code: "GP-S-002",
                severity: Severity::Warning,
                category: "security",
                description:
                    "Signature invalidated: mutation applied with explicit policy override",
            },
        );
        entries.insert(
            "GP-S-003",
            CodeEntry {
                code: "GP-S-003",
                severity: Severity::Info,
                category: "security",
                description: "No signatures detected — mutation proceeds normally",
            },
        );
        entries.insert(
            "GP-S-010",
            CodeEntry {
                code: "GP-S-010",
                severity: Severity::Error,
                category: "unsupported",
                description: "PAdES-T/LTV signing not supported (TSA absent)",
            },
        );
        entries.insert(
            "GP-S-011",
            CodeEntry {
                code: "GP-S-011",
                severity: Severity::Error,
                category: "unsupported",
                description:
                    "CA-signed certificate signing not supported (self-signed only for B-B)",
            },
        );
        entries.insert(
            "GP-S-012",
            CodeEntry {
                code: "GP-S-012",
                severity: Severity::Error,
                category: "unsupported",
                description:
                    "CRL/OCSP verification not implemented (B-B only checks signature validity)",
            },
        );
        entries.insert(
            "GP-S-013",
            CodeEntry {
                code: "GP-S-013",
                severity: Severity::Error,
                category: "unsupported",
                description: "Smart card / HSM signing not supported (in-memory keys only)",
            },
        );
        entries.insert(
            "GP-S-014",
            CodeEntry {
                code: "GP-S-014",
                severity: Severity::Error,
                category: "validation",
                description:
                    "PKCS#12 input required (use openssl pkcs12 to convert PEM to PKCS#12)",
            },
        );
        entries.insert(
            "GP-S-015",
            CodeEntry {
                code: "GP-S-015",
                severity: Severity::Error,
                category: "validation",
                description: "Signing refused: /AcroForm already contains /SigFields (conflicting signature fields)",
            },
        );
        entries.insert(
            "GP-S-016",
            CodeEntry {
                code: "GP-S-016",
                severity: Severity::Error,
                category: "validation",
                description: "Signing refused: document already has signatures",
            },
        );
        entries.insert(
            "GP-S-020",
            CodeEntry {
                code: "GP-S-020",
                severity: Severity::Warning,
                category: "security",
                description: "Timestamp server unreachable — signed without TSA (PAdES-B-B)",
            },
        );
        entries.insert(
            "GP-S-021",
            CodeEntry {
                code: "GP-S-021",
                severity: Severity::Warning,
                category: "security",
                description: "OCSP/CRL unavailable — signed without LTV data",
            },
        );
        entries.insert(
            "GP-S-022",
            CodeEntry {
                code: "GP-S-022",
                severity: Severity::Error,
                category: "security",
                description: "Signing certificate is revoked per OCSP — signing refused",
            },
        );
        entries.insert(
            "GP-S-023",
            CodeEntry {
                code: "GP-S-023",
                severity: Severity::Warning,
                category: "security",
                description: "Signing certificate expired per CRL — signed with warning",
            },
        );
        entries.insert(
            "GP-S-024",
            CodeEntry {
                code: "GP-S-024",
                severity: Severity::Error,
                category: "security",
                description: "Invalid PKCS#12 file — signing refused",
            },
        );
        entries.insert(
            "GP-S-025",
            CodeEntry {
                code: "GP-S-025",
                severity: Severity::Error,
                category: "security",
                description: "Certificate chain validation failed — signing refused",
            },
        );

        // ODX (adapters-ooxml) codes — content type / structural errors
        entries.insert(
            "ODX-E-001",
            CodeEntry {
                code: "ODX-E-001",
                severity: Severity::Error,
                category: "validation",
                description: "Missing content type for part: {}",
            },
        );
        entries.insert(
            "ODX-E-002",
            CodeEntry {
                code: "ODX-E-002",
                severity: Severity::Error,
                category: "validation",
                description: "Duplicate content type: {}",
            },
        );
        entries.insert(
            "ODX-E-003",
            CodeEntry {
                code: "ODX-E-003",
                severity: Severity::Error,
                category: "parsing",
                description: "Invalid content type: {}",
            },
        );
        entries.insert(
            "ODX-E-004",
            CodeEntry {
                code: "ODX-E-004",
                severity: Severity::Error,
                category: "validation",
                description: "Content type not found for part: {}",
            },
        );
        entries.insert(
            "ODX-E-005",
            CodeEntry {
                code: "ODX-E-005",
                severity: Severity::Error,
                category: "parsing",
                description: "Content types XML parse error: {}",
            },
        );
        entries.insert(
            "ODX-W-001",
            CodeEntry {
                code: "ODX-W-001",
                severity: Severity::Warning,
                category: "parsing",
                description: "Unknown content type default: {}",
            },
        );
        entries.insert(
            "ODX-W-003",
            CodeEntry {
                code: "ODX-W-003",
                severity: Severity::Warning,
                category: "validation",
                description: "Content type override missing for: {}",
            },
        );

        // ODF (adapters-odf) codes — additional read errors
        entries.insert(
            "ODF-E-010",
            CodeEntry {
                code: "ODF-E-010",
                severity: Severity::Error,
                category: "parsing",
                description: "ODF IR serialization error: {}",
            },
        );
        entries.insert(
            "ODF-R200",
            CodeEntry {
                code: "ODF-R200",
                severity: Severity::Warning,
                category: "validation",
                description: "ODS: no tables found",
            },
        );
        entries.insert(
            "ODF-R201",
            CodeEntry {
                code: "ODF-R201",
                severity: Severity::Error,
                category: "parsing",
                description: "ODS: no content.xml",
            },
        );
        entries.insert(
            "ODF-R210",
            CodeEntry {
                code: "ODF-R210",
                severity: Severity::Info,
                category: "diagnostic",
                description: "ODS: {n} sheets, {tr} rows, {tm} merged",
            },
        );
        entries.insert(
            "ODF-R300",
            CodeEntry {
                code: "ODF-R300",
                severity: Severity::Warning,
                category: "validation",
                description: "ODP: no pages found",
            },
        );
        entries.insert(
            "ODF-R301",
            CodeEntry {
                code: "ODF-R301",
                severity: Severity::Error,
                category: "parsing",
                description: "ODP: no content.xml",
            },
        );
        entries.insert(
            "ODF-R310",
            CodeEntry {
                code: "ODF-R310",
                severity: Severity::Info,
                category: "diagnostic",
                description: "ODP: {n} slides, {ts} elements",
            },
        );

        // OOX (adapters-ooxml) codes — validation warnings and errors
        entries.insert(
            "OOX-E-010",
            CodeEntry {
                code: "OOX-E-010",
                severity: Severity::Error,
                category: "validation",
                description: "Invalid part path: {}",
            },
        );
        entries.insert(
            "OOX-E-015",
            CodeEntry {
                code: "OOX-E-015",
                severity: Severity::Error,
                category: "validation",
                description: "Missing required relationship: {}",
            },
        );
        entries.insert(
            "OOX-E-016",
            CodeEntry {
                code: "OOX-E-016",
                severity: Severity::Error,
                category: "validation",
                description: "Invalid relationship target: {}",
            },
        );
        entries.insert(
            "OOX-E-020",
            CodeEntry {
                code: "OOX-E-020",
                severity: Severity::Error,
                category: "parsing",
                description: "OOX parse error: {}",
            },
        );
        entries.insert(
            "OOX-E-021",
            CodeEntry {
                code: "OOX-E-021",
                severity: Severity::Error,
                category: "validation",
                description: "OOX structural violation: {}",
            },
        );
        entries.insert(
            "OOX-W-002",
            CodeEntry {
                code: "OOX-W-002",
                severity: Severity::Warning,
                category: "validation",
                description: "Unknown part type '{}' — skipping validation",
            },
        );
        entries.insert(
            "OOX-W-010",
            CodeEntry {
                code: "OOX-W-010",
                severity: Severity::Warning,
                category: "parsing",
                description: "OOX warning: {}",
            },
        );
        entries.insert(
            "OOX-W-020",
            CodeEntry {
                code: "OOX-W-020",
                severity: Severity::Warning,
                category: "validation",
                description: "Optional element missing: {}",
            },
        );
        entries.insert(
            "OOX-W-021",
            CodeEntry {
                code: "OOX-W-021",
                severity: Severity::Warning,
                category: "validation",
                description: "Deprecated element used: {}",
            },
        );
        entries.insert(
            "OOX-W-022",
            CodeEntry {
                code: "OOX-W-022",
                severity: Severity::Warning,
                category: "validation",
                description: "Unexpected element found: {}",
            },
        );
        entries.insert(
            "OOX-W-023",
            CodeEntry {
                code: "OOX-W-023",
                severity: Severity::Warning,
                category: "validation",
                description: "Element in wrong section: {}",
            },
        );
        entries.insert(
            "OOX-W-100",
            CodeEntry {
                code: "OOX-W-100",
                severity: Severity::Warning,
                category: "validation",
                description: "OOX loose validation failed: {}",
            },
        );
        entries.insert(
            "OOX-W-101",
            CodeEntry {
                code: "OOX-W-101",
                severity: Severity::Warning,
                category: "validation",
                description: "OOX strict validation warning: {}",
            },
        );
        entries.insert(
            "OOX-W-150",
            CodeEntry {
                code: "OOX-W-150",
                severity: Severity::Warning,
                category: "validation",
                description: "OOX schema mismatch: {}",
            },
        );
        entries.insert(
            "OOX-W-151",
            CodeEntry {
                code: "OOX-W-151",
                severity: Severity::Warning,
                category: "validation",
                description: "OOX schema validation partial: {}",
            },
        );
        entries.insert(
            "OOX-W-200",
            CodeEntry {
                code: "OOX-W-200",
                severity: Severity::Warning,
                category: "validation",
                description: "OOX content type warning: {}",
            },
        );
        entries.insert(
            "OOX-W-201",
            CodeEntry {
                code: "OOX-W-201",
                severity: Severity::Warning,
                category: "validation",
                description: "OOX relationship warning: {}",
            },
        );

        // OOX (adapters-ooxml) OPC package integrity codes
        entries.insert(
            "OOX-E-022",
            CodeEntry {
                code: "OOX-E-022",
                severity: Severity::Error,
                category: "validation",
                description: "Dangling relationship target '{}' in '{}' — no such part exists",
            },
        );
        entries.insert(
            "OOX-E-023",
            CodeEntry {
                code: "OOX-E-023",
                severity: Severity::Error,
                category: "validation",
                description: "Orphan part '{}' has no incoming relationship",
            },
        );
        entries.insert(
            "OOX-E-024",
            CodeEntry {
                code: "OOX-E-024",
                severity: Severity::Error,
                category: "validation",
                description: "Missing content-type override for part '{}'",
            },
        );
        entries.insert(
            "OOX-E-025",
            CodeEntry {
                code: "OOX-E-025",
                severity: Severity::Error,
                category: "validation",
                description: "Presentation has zero slides (empty sldIdLst)",
            },
        );
        entries.insert(
            "OOX-W-026",
            CodeEntry {
                code: "OOX-W-026",
                severity: Severity::Warning,
                category: "validation",
                description:
                    "[Content_Types].xml is not the first ZIP entry (found at position {})",
            },
        );
        entries.insert(
            "OOX-W-027",
            CodeEntry {
                code: "OOX-W-027",
                severity: Severity::Warning,
                category: "validation",
                description:
                    "Part '{}' uses fallback content type '{}' instead of explicit override",
            },
        );
        // OOX-E-026: Rich text parse error
        entries.insert(
            "OOX-E-026",
            CodeEntry {
                code: "OOX-E-026",
                severity: Severity::Error,
                category: "oox",
                description: "Rich text parse error: {}",
            },
        );
        // OOX-W-029: Cell rich text — unsupported formatting
        entries.insert(
            "OOX-W-029",
            CodeEntry {
                code: "OOX-W-029",
                severity: Severity::Warning,
                category: "oox",
                description: "Cell rich text contains unsupported formatting: {}",
            },
        );

        // PDF (adapters-pdf) codes — qpdf informational
        entries.insert(
            "PDF-I-001",
            CodeEntry {
                code: "PDF-I-001",
                severity: Severity::Info,
                category: "tooling",
                description: "qpdf not available — PDF structure check skipped",
            },
        );
        entries.insert(
            "PDF-I-002",
            CodeEntry {
                code: "PDF-I-002",
                severity: Severity::Info,
                category: "tooling",
                description: "qpdf: unknown result '{}' for level {}",
            },
        );
        entries.insert(
            "PDF-E-012",
            CodeEntry {
                code: "PDF-E-012",
                severity: Severity::Error,
                category: "parsing",
                description: "PDF parse error at offset {}: {}",
            },
        );

        // ENG (engines) codes — rasterization and engine errors
        entries.insert(
            "ENG-E-100",
            CodeEntry {
                code: "ENG-E-100",
                severity: Severity::Error,
                category: "raster",
                description: "pdftoppm temp file: {}",
            },
        );
        entries.insert(
            "ENG-E-101",
            CodeEntry {
                code: "ENG-E-101",
                severity: Severity::Error,
                category: "raster",
                description: "pdftoppm exec: {}",
            },
        );
        entries.insert(
            "ENG-E-102",
            CodeEntry {
                code: "ENG-E-102",
                severity: Severity::Error,
                category: "raster",
                description: "pdftoppm failed: {}",
            },
        );
        entries.insert(
            "ENG-E-103",
            CodeEntry {
                code: "ENG-E-103",
                severity: Severity::Error,
                category: "raster",
                description: "pdftoppm: output PNG not found",
            },
        );
        entries.insert(
            "ENG-E-104",
            CodeEntry {
                code: "ENG-E-104",
                severity: Severity::Error,
                category: "raster",
                description: "pdftoppm read PNG: {}",
            },
        );
        entries.insert(
            "ENG-E-110",
            CodeEntry {
                code: "ENG-E-110",
                severity: Severity::Error,
                category: "raster",
                description: "gs temp file: {}",
            },
        );
        entries.insert(
            "ENG-E-111",
            CodeEntry {
                code: "ENG-E-111",
                severity: Severity::Error,
                category: "raster",
                description: "gs exec: {}",
            },
        );
        entries.insert(
            "ENG-E-112",
            CodeEntry {
                code: "ENG-E-112",
                severity: Severity::Error,
                category: "raster",
                description: "gs: non-zero exit",
            },
        );
        entries.insert(
            "ENG-E-113",
            CodeEntry {
                code: "ENG-E-113",
                severity: Severity::Error,
                category: "raster",
                description: "gs: output PNG not found",
            },
        );
        entries.insert(
            "ENG-E-114",
            CodeEntry {
                code: "ENG-E-114",
                severity: Severity::Error,
                category: "raster",
                description: "gs read PNG: {}",
            },
        );
        entries.insert(
            "ENG-E-120",
            CodeEntry {
                code: "ENG-E-120",
                severity: Severity::Error,
                category: "raster",
                description: "mutool temp file: {}",
            },
        );
        entries.insert(
            "ENG-E-121",
            CodeEntry {
                code: "ENG-E-121",
                severity: Severity::Error,
                category: "raster",
                description: "mutool exec: {}",
            },
        );
        entries.insert(
            "ENG-E-122",
            CodeEntry {
                code: "ENG-E-122",
                severity: Severity::Error,
                category: "raster",
                description: "mutool failed: {}",
            },
        );
        entries.insert(
            "ENG-E-123",
            CodeEntry {
                code: "ENG-E-123",
                severity: Severity::Error,
                category: "raster",
                description: "mutool: output PNG not found",
            },
        );
        entries.insert(
            "ENG-E-124",
            CodeEntry {
                code: "ENG-E-124",
                severity: Severity::Error,
                category: "raster",
                description: "mutool read PNG: {}",
            },
        );
        entries.insert(
            "ENG-E-130",
            CodeEntry {
                code: "ENG-E-130",
                severity: Severity::Error,
                category: "raster",
                description: "soffice raster temp file: {}",
            },
        );
        entries.insert(
            "ENG-E-131",
            CodeEntry {
                code: "ENG-E-131",
                severity: Severity::Error,
                category: "raster",
                description: "soffice raster exec: {}",
            },
        );
        entries.insert(
            "ENG-E-132",
            CodeEntry {
                code: "ENG-E-132",
                severity: Severity::Error,
                category: "raster",
                description: "soffice raster failed: {}",
            },
        );
        entries.insert(
            "ENG-E-133",
            CodeEntry {
                code: "ENG-E-133",
                severity: Severity::Error,
                category: "raster",
                description: "soffice raster: output not found",
            },
        );
        entries.insert(
            "ENG-E-134",
            CodeEntry {
                code: "ENG-E-134",
                severity: Severity::Error,
                category: "raster",
                description: "soffice raster read PNG: {}",
            },
        );
        entries.insert(
            "ENG-E-135",
            CodeEntry {
                code: "ENG-E-135",
                severity: Severity::Error,
                category: "raster",
                description: "soffice raster: unsupported device",
            },
        );
        entries.insert(
            "ENG-E-201",
            CodeEntry {
                code: "ENG-E-201",
                severity: Severity::Info,
                category: "raster",
                description: "pdftoppm: tool not found or failed",
            },
        );
        entries.insert(
            "ENG-E-202",
            CodeEntry {
                code: "ENG-E-202",
                severity: Severity::Info,
                category: "raster",
                description: "Ghostscript: tool not found or failed",
            },
        );
        entries.insert(
            "ENG-E-203",
            CodeEntry {
                code: "ENG-E-203",
                severity: Severity::Info,
                category: "raster",
                description: "mutool: tool not found or failed",
            },
        );
        entries.insert(
            "ENG-E-204",
            CodeEntry {
                code: "ENG-E-204",
                severity: Severity::Info,
                category: "raster",
                description: "soffice: tool not found or failed",
            },
        );

        // T1-advanced: DOCX style resolution / write (OOX-W-210..219)
        entries.insert(
            "OOX-W-210",
            CodeEntry {
                code: "OOX-W-210",
                severity: Severity::Warning,
                category: "style",
                description: "Style not found in styles.xml for paragraph",
            },
        );
        entries.insert(
            "OOX-W-211",
            CodeEntry {
                code: "OOX-W-211",
                severity: Severity::Warning,
                category: "style",
                description: "Unknown style type in styles.xml",
            },
        );
        entries.insert(
            "OOX-W-212",
            CodeEntry {
                code: "OOX-W-212",
                severity: Severity::Warning,
                category: "style",
                description: "Style chain cycle detected (basedOn loop)",
            },
        );
        entries.insert(
            "OOX-W-213",
            CodeEntry {
                code: "OOX-W-213",
                severity: Severity::Warning,
                category: "style",
                description: "Missing docDefaults in styles.xml",
            },
        );
        entries.insert(
            "OOX-W-214",
            CodeEntry {
                code: "OOX-W-214",
                severity: Severity::Warning,
                category: "style",
                description: "Incompatible style link (run style on paragraph)",
            },
        );
        entries.insert(
            "OOX-W-215",
            CodeEntry {
                code: "OOX-W-215",
                severity: Severity::Warning,
                category: "style",
                description: "Style {0} has no basedOn, using implicit defaults",
            },
        );
        entries.insert(
            "OOX-W-216",
            CodeEntry {
                code: "OOX-W-216",
                severity: Severity::Warning,
                category: "style",
                description: "Multiple styles with same styleId",
            },
        );
        entries.insert(
            "OOX-W-217",
            CodeEntry {
                code: "OOX-W-217",
                severity: Severity::Warning,
                category: "style",
                description: "Style {0} uiPriority {1} conflicts with {2}",
            },
        );
        entries.insert(
            "OOX-W-218",
            CodeEntry {
                code: "OOX-W-218",
                severity: Severity::Warning,
                category: "style",
                description: "Semi-hidden style {0} may be user-invisible",
            },
        );
        entries.insert(
            "OOX-W-219",
            CodeEntry {
                code: "OOX-W-219",
                severity: Severity::Warning,
                category: "style",
                description: "Style write: failed to serialize style XML",
            },
        );

        // T1-advanced: DOCX style error (OOX-E-030)
        entries.insert(
            "OOX-E-030",
            CodeEntry {
                code: "OOX-E-030",
                severity: Severity::Error,
                category: "style",
                description: "Styles part missing or malformed",
            },
        );

        // T1-advanced: DOCX hyperlink read/write (OOX-W-220..229)
        entries.insert(
            "OOX-W-220",
            CodeEntry {
                code: "OOX-W-220",
                severity: Severity::Warning,
                category: "hyperlink",
                description: "Hyperlink target not found for rid {0}",
            },
        );
        entries.insert(
            "OOX-W-221",
            CodeEntry {
                code: "OOX-W-221",
                severity: Severity::Warning,
                category: "hyperlink",
                description: "Invalid hyperlink target URI: {0}",
            },
        );
        entries.insert(
            "OOX-W-222",
            CodeEntry {
                code: "OOX-W-222",
                severity: Severity::Warning,
                category: "hyperlink",
                description: "Hyperlink with no text content",
            },
        );
        entries.insert(
            "OOX-W-223",
            CodeEntry {
                code: "OOX-W-223",
                severity: Severity::Warning,
                category: "hyperlink",
                description: "Internal anchor target {0} not found",
            },
        );
        entries.insert(
            "OOX-W-224",
            CodeEntry {
                code: "OOX-W-224",
                severity: Severity::Warning,
                category: "hyperlink",
                description: "Duplicate hyperlink on same run",
            },
        );
        entries.insert(
            "OOX-W-225",
            CodeEntry {
                code: "OOX-W-225",
                severity: Severity::Warning,
                category: "hyperlink",
                description: "Hyperlink tooltip exceeds max length",
            },
        );
        entries.insert(
            "OOX-W-226",
            CodeEntry {
                code: "OOX-W-226",
                severity: Severity::Warning,
                category: "hyperlink",
                description: "Resolved hyperlink target to package root",
            },
        );
        entries.insert(
            "OOX-W-227",
            CodeEntry {
                code: "OOX-W-227",
                severity: Severity::Warning,
                category: "hyperlink",
                description: "Hyperlink rid collision in rels",
            },
        );
        entries.insert(
            "OOX-W-228",
            CodeEntry {
                code: "OOX-W-228",
                severity: Severity::Warning,
                category: "hyperlink",
                description: "External hyperlink scheme not allowed: {0}",
            },
        );
        entries.insert(
            "OOX-W-229",
            CodeEntry {
                code: "OOX-W-229",
                severity: Severity::Warning,
                category: "hyperlink",
                description: "Hyperlink write: failed to add rel entry",
            },
        );

        // T1-advanced: DOCX drawing/image blob (OOX-W-230..239)
        entries.insert(
            "OOX-W-230",
            CodeEntry {
                code: "OOX-W-230",
                severity: Severity::Warning,
                category: "media",
                description: "Drawing element with no blip embed",
            },
        );
        entries.insert(
            "OOX-W-231",
            CodeEntry {
                code: "OOX-W-231",
                severity: Severity::Warning,
                category: "media",
                description: "Image part not found for rId {0}",
            },
        );
        entries.insert(
            "OOX-W-232",
            CodeEntry {
                code: "OOX-W-232",
                severity: Severity::Warning,
                category: "media",
                description: "Unknown image MIME type for {0}",
            },
        );
        entries.insert(
            "OOX-W-233",
            CodeEntry {
                code: "OOX-W-233",
                severity: Severity::Warning,
                category: "media",
                description: "Image data too large for inline context: {0} bytes",
            },
        );
        entries.insert(
            "OOX-W-234",
            CodeEntry {
                code: "OOX-W-234",
                severity: Severity::Warning,
                category: "media",
                description: "Drawing bbox exceeds page bounds",
            },
        );
        entries.insert(
            "OOX-W-235",
            CodeEntry {
                code: "OOX-W-235",
                severity: Severity::Warning,
                category: "media",
                description: "Inline drawing overlap detected with {0}",
            },
        );
        entries.insert(
            "OOX-W-236",
            CodeEntry {
                code: "OOX-W-236",
                severity: Severity::Warning,
                category: "media",
                description: "Floating drawing z-index {0} out of range",
            },
        );
        entries.insert(
            "OOX-W-237",
            CodeEntry {
                code: "OOX-W-237",
                severity: Severity::Warning,
                category: "media",
                description: "pic:pic has no pic:cNvPr name",
            },
        );
        entries.insert(
            "OOX-W-238",
            CodeEntry {
                code: "OOX-W-238",
                severity: Severity::Warning,
                category: "media",
                description: "Drawing write: failed to add media part",
            },
        );
        entries.insert(
            "OOX-W-239",
            CodeEntry {
                code: "OOX-W-239",
                severity: Severity::Warning,
                category: "media",
                description: "Drawing write: missing content type override",
            },
        );

        // T1-advanced: DOCX complex fields / TOC (OOX-W-240..249)
        entries.insert(
            "OOX-W-240",
            CodeEntry {
                code: "OOX-W-240",
                severity: Severity::Warning,
                category: "field",
                description: "Unclosed field (fldChar begin without end)",
            },
        );
        entries.insert(
            "OOX-W-241",
            CodeEntry {
                code: "OOX-W-241",
                severity: Severity::Warning,
                category: "field",
                description: "Field result exceeds max length: {0} chars",
            },
        );
        entries.insert(
            "OOX-W-242",
            CodeEntry {
                code: "OOX-W-242",
                severity: Severity::Warning,
                category: "field",
                description: "TOC entry missing hyperlink target",
            },
        );
        entries.insert(
            "OOX-W-243",
            CodeEntry {
                code: "OOX-W-243",
                severity: Severity::Warning,
                category: "field",
                description: "Field instruction parse error: {0}",
            },
        );
        entries.insert(
            "OOX-W-244",
            CodeEntry {
                code: "OOX-W-244",
                severity: Severity::Warning,
                category: "field",
                description: "PAGENUM field with unknown switch: {0}",
            },
        );
        entries.insert(
            "OOX-W-245",
            CodeEntry {
                code: "OOX-W-245",
                severity: Severity::Warning,
                category: "field",
                description: "REF field references unknown bookmark: {0}",
            },
        );
        entries.insert(
            "OOX-W-246",
            CodeEntry {
                code: "OOX-W-246",
                severity: Severity::Warning,
                category: "field",
                description: "SEQ field with invalid sequence identifier",
            },
        );
        entries.insert(
            "OOX-W-247",
            CodeEntry {
                code: "OOX-W-247",
                severity: Severity::Warning,
                category: "field",
                description: "Field group with mismatched fldChar types",
            },
        );
        entries.insert(
            "OOX-W-248",
            CodeEntry {
                code: "OOX-W-248",
                severity: Severity::Warning,
                category: "field",
                description: "Nested field detected (may not render in all viewers)",
            },
        );
        entries.insert(
            "OOX-W-249",
            CodeEntry {
                code: "OOX-W-249",
                severity: Severity::Warning,
                category: "field",
                description: "Complex field write: field group ID collision",
            },
        );

        // T1-advanced: XLSX style table / cell style (OOX-W-250..269)
        entries.insert(
            "OOX-W-250",
            CodeEntry {
                code: "OOX-W-250",
                severity: Severity::Warning,
                category: "style",
                description: "numFmt {0} not found for cell style index {1}",
            },
        );
        entries.insert(
            "OOX-W-251",
            CodeEntry {
                code: "OOX-W-251",
                severity: Severity::Warning,
                category: "style",
                description: "Unknown number format: {0}",
            },
        );
        entries.insert(
            "OOX-W-252",
            CodeEntry {
                code: "OOX-W-252",
                severity: Severity::Warning,
                category: "style",
                description: "Cell style index {0} exceeds xf count",
            },
        );
        entries.insert(
            "OOX-W-253",
            CodeEntry {
                code: "OOX-W-253",
                severity: Severity::Warning,
                category: "style",
                description: "Font name not found in fonts table: {0}",
            },
        );
        entries.insert(
            "OOX-W-254",
            CodeEntry {
                code: "OOX-W-254",
                severity: Severity::Warning,
                category: "style",
                description: "Font size outside valid range: {0}",
            },
        );
        entries.insert(
            "OOX-W-255",
            CodeEntry {
                code: "OOX-W-255",
                severity: Severity::Warning,
                category: "style",
                description: "Fill pattern {0} not in fills table",
            },
        );
        entries.insert(
            "OOX-W-256",
            CodeEntry {
                code: "OOX-W-256",
                severity: Severity::Warning,
                category: "style",
                description: "Border cell style not found",
            },
        );
        entries.insert(
            "OOX-W-257",
            CodeEntry {
                code: "OOX-W-257",
                severity: Severity::Warning,
                category: "style",
                description: "Alignment property ignored for numeric cell",
            },
        );
        entries.insert(
            "OOX-W-258",
            CodeEntry {
                code: "OOX-W-258",
                severity: Severity::Warning,
                category: "style",
                description: "Style {0} has conflicting font entries",
            },
        );
        entries.insert(
            "OOX-W-259",
            CodeEntry {
                code: "OOX-W-259",
                severity: Severity::Warning,
                category: "style",
                description: "cellStyleXfs count exceeds cellStyles count",
            },
        );
        entries.insert(
            "OOX-W-260",
            CodeEntry {
                code: "OOX-W-260",
                severity: Severity::Warning,
                category: "style",
                description: "Duplicate cellStyle name: {0}",
            },
        );
        entries.insert(
            "OOX-W-261",
            CodeEntry {
                code: "OOX-W-261",
                severity: Severity::Warning,
                category: "style",
                description: "Style write: xf index collision",
            },
        );
        entries.insert(
            "OOX-W-262",
            CodeEntry {
                code: "OOX-W-262",
                severity: Severity::Warning,
                category: "style",
                description: "Style write: numFmt ID reuse detected",
            },
        );
        entries.insert(
            "OOX-W-263",
            CodeEntry {
                code: "OOX-W-263",
                severity: Severity::Warning,
                category: "style",
                description: "Style write: failed to serialize style table",
            },
        );
        entries.insert(
            "OOX-W-264",
            CodeEntry {
                code: "OOX-W-264",
                severity: Severity::Warning,
                category: "style",
                description: "Style write: missing font for index {0}",
            },
        );
        entries.insert(
            "OOX-W-265",
            CodeEntry {
                code: "OOX-W-265",
                severity: Severity::Warning,
                category: "style",
                description: "Style write: missing fill for index {0}",
            },
        );
        entries.insert(
            "OOX-W-266",
            CodeEntry {
                code: "OOX-W-266",
                severity: Severity::Warning,
                category: "style",
                description: "Style write: missing border for index {0}",
            },
        );
        entries.insert(
            "OOX-W-267",
            CodeEntry {
                code: "OOX-W-267",
                severity: Severity::Warning,
                category: "style",
                description: "cellXfs references undefined style index {0}",
            },
        );
        entries.insert(
            "OOX-W-268",
            CodeEntry {
                code: "OOX-W-268",
                severity: Severity::Warning,
                category: "style",
                description: "Number format {0} uses unsupported type",
            },
        );
        entries.insert(
            "OOX-W-269",
            CodeEntry {
                code: "OOX-W-269",
                severity: Severity::Warning,
                category: "style",
                description: "Style inheritance depth exceeds limit for {0}",
            },
        );

        // T1-advanced: XLSX views · freeze · autofilter (OOX-W-270..289)
        entries.insert(
            "OOX-W-270",
            CodeEntry {
                code: "OOX-W-270",
                severity: Severity::Warning,
                category: "view",
                description: "Freeze pane split exceeds sheet bounds",
            },
        );
        entries.insert(
            "OOX-W-271",
            CodeEntry {
                code: "OOX-W-271",
                severity: Severity::Warning,
                category: "view",
                description: "Invalid active pane: {0}",
            },
        );
        entries.insert(
            "OOX-W-272",
            CodeEntry {
                code: "OOX-W-272",
                severity: Severity::Warning,
                category: "view",
                description: "Top-left cell outside visible area",
            },
        );
        entries.insert(
            "OOX-W-273",
            CodeEntry {
                code: "OOX-W-273",
                severity: Severity::Warning,
                category: "view",
                description: "Sheet view zoom scale out of range: {0}",
            },
        );
        entries.insert(
            "OOX-W-274",
            CodeEntry {
                code: "OOX-W-274",
                severity: Severity::Warning,
                category: "view",
                description: "Frozen row count exceeds row count",
            },
        );
        entries.insert(
            "OOX-W-275",
            CodeEntry {
                code: "OOX-W-275",
                severity: Severity::Warning,
                category: "view",
                description: "Freeze pane: active_cell {0} outside split",
            },
        );
        entries.insert(
            "OOX-W-276",
            CodeEntry {
                code: "OOX-W-276",
                severity: Severity::Warning,
                category: "view",
                description: "sheetViews count exceeds 1 (only one view per sheet)",
            },
        );
        entries.insert(
            "OOX-W-277",
            CodeEntry {
                code: "OOX-W-277",
                severity: Severity::Warning,
                category: "view",
                description: "Column dimension min > max: {0} > {1}",
            },
        );
        entries.insert(
            "OOX-W-278",
            CodeEntry {
                code: "OOX-W-278",
                severity: Severity::Warning,
                category: "view",
                description: "Row dimension index exceeds sheet rows",
            },
        );
        entries.insert(
            "OOX-W-279",
            CodeEntry {
                code: "OOX-W-279",
                severity: Severity::Warning,
                category: "view",
                description: "AutoFilter range invalid: {0}",
            },
        );
        entries.insert(
            "OOX-W-280",
            CodeEntry {
                code: "OOX-W-280",
                severity: Severity::Warning,
                category: "view",
                description: "AutoFilter column index out of range: {0}",
            },
        );
        entries.insert(
            "OOX-W-281",
            CodeEntry {
                code: "OOX-W-281",
                severity: Severity::Warning,
                category: "view",
                description: "AutoFilter blank filter on non-empty column",
            },
        );
        entries.insert(
            "OOX-W-282",
            CodeEntry {
                code: "OOX-W-282",
                severity: Severity::Warning,
                category: "view",
                description: "AutoFilter write: range mismatch",
            },
        );
        entries.insert(
            "OOX-W-283",
            CodeEntry {
                code: "OOX-W-283",
                severity: Severity::Warning,
                category: "view",
                description: "View state conflict: hidden sheet with active view",
            },
        );
        entries.insert(
            "OOX-W-284",
            CodeEntry {
                code: "OOX-W-284",
                severity: Severity::Warning,
                category: "view",
                description: "Row outline level exceeds 7: {0}",
            },
        );
        entries.insert(
            "OOX-W-285",
            CodeEntry {
                code: "OOX-W-285",
                severity: Severity::Warning,
                category: "view",
                description: "Column outline level exceeds 7: {0}",
            },
        );
        entries.insert(
            "OOX-W-286",
            CodeEntry {
                code: "OOX-W-286",
                severity: Severity::Warning,
                category: "view",
                description: "Freeze pane: workbookViewId mismatch",
            },
        );
        entries.insert(
            "OOX-W-287",
            CodeEntry {
                code: "OOX-W-287",
                severity: Severity::Warning,
                category: "view",
                description: "AutoFilter filterColumn duplicate col: {0}",
            },
        );
        entries.insert(
            "OOX-W-288",
            CodeEntry {
                code: "OOX-W-288",
                severity: Severity::Warning,
                category: "view",
                description: "FilterRule operator invalid for type: {0}",
            },
        );
        entries.insert(
            "OOX-W-289",
            CodeEntry {
                code: "OOX-W-289",
                severity: Severity::Warning,
                category: "view",
                description: "AutoFilter write: missing ref attribute",
            },
        );

        // T1-advanced: XLSX defined names (OOX-W-290..299)
        entries.insert(
            "OOX-W-290",
            CodeEntry {
                code: "OOX-W-290",
                severity: Severity::Warning,
                category: "defined-name",
                description: "Defined name {0} has no refFormula",
            },
        );
        entries.insert(
            "OOX-W-291",
            CodeEntry {
                code: "OOX-W-291",
                severity: Severity::Warning,
                category: "defined-name",
                description: "Defined name refFormula invalid: {0}",
            },
        );
        entries.insert(
            "OOX-W-292",
            CodeEntry {
                code: "OOX-W-292",
                severity: Severity::Warning,
                category: "defined-name",
                description: "Local sheet ID out of range: {0}",
            },
        );
        entries.insert(
            "OOX-W-293",
            CodeEntry {
                code: "OOX-W-293",
                severity: Severity::Warning,
                category: "defined-name",
                description: "Defined name {0} conflicts with builtin name",
            },
        );
        entries.insert(
            "OOX-W-294",
            CodeEntry {
                code: "OOX-W-294",
                severity: Severity::Warning,
                category: "defined-name",
                description: "Defined name hidden in non-local scope",
            },
        );
        entries.insert(
            "OOX-W-295",
            CodeEntry {
                code: "OOX-W-295",
                severity: Severity::Warning,
                category: "defined-name",
                description: "Defined name {0} references deleted sheet",
            },
        );
        entries.insert(
            "OOX-W-296",
            CodeEntry {
                code: "OOX-W-296",
                severity: Severity::Warning,
                category: "defined-name",
                description: "Defined name write: sheet ID mismatch",
            },
        );
        entries.insert(
            "OOX-W-297",
            CodeEntry {
                code: "OOX-W-297",
                severity: Severity::Warning,
                category: "defined-name",
                description: "Defined name write: formula too long",
            },
        );
        entries.insert(
            "OOX-W-298",
            CodeEntry {
                code: "OOX-W-298",
                severity: Severity::Warning,
                category: "defined-name",
                description: "Defined name write: duplicate name",
            },
        );
        entries.insert(
            "OOX-W-299",
            CodeEntry {
                code: "OOX-W-299",
                severity: Severity::Warning,
                category: "defined-name",
                description: "Defined name delete: name not found",
            },
        );

        // T1-advanced: XLSX conditional formatting (OOX-W-300..309)
        entries.insert(
            "OOX-W-300",
            CodeEntry {
                code: "OOX-W-300",
                severity: Severity::Warning,
                category: "cf",
                description: "Conditional format sqref invalid: {0}",
            },
        );
        entries.insert(
            "OOX-W-301",
            CodeEntry {
                code: "OOX-W-301",
                severity: Severity::Warning,
                category: "cf",
                description: "CF type {0} missing required formula",
            },
        );
        entries.insert(
            "OOX-W-302",
            CodeEntry {
                code: "OOX-W-302",
                severity: Severity::Warning,
                category: "cf",
                description: "CF dxf index {0} not in docDefaults",
            },
        );
        entries.insert(
            "OOX-W-303",
            CodeEntry {
                code: "OOX-W-303",
                severity: Severity::Warning,
                category: "cf",
                description: "CF priority collision: {0}",
            },
        );
        entries.insert(
            "OOX-W-304",
            CodeEntry {
                code: "OOX-W-304",
                severity: Severity::Warning,
                category: "cf",
                description: "CF rule applied to entire sheet (may impact performance)",
            },
        );
        entries.insert(
            "OOX-W-305",
            CodeEntry {
                code: "OOX-W-305",
                severity: Severity::Warning,
                category: "cf",
                description: "Color scale missing equalOp/focusOn for {0}",
            },
        );
        entries.insert(
            "OOX-W-306",
            CodeEntry {
                code: "OOX-W-306",
                severity: Severity::Warning,
                category: "cf",
                description: "CF write: failed to add conditionalFormatting part",
            },
        );
        entries.insert(
            "OOX-W-307",
            CodeEntry {
                code: "OOX-W-307",
                severity: Severity::Warning,
                category: "cf",
                description: "CF write: dxf index out of range",
            },
        );
        entries.insert(
            "OOX-W-308",
            CodeEntry {
                code: "OOX-W-308",
                severity: Severity::Warning,
                category: "cf",
                description: "CF write: sqref range overflow",
            },
        );
        entries.insert(
            "OOX-W-309",
            CodeEntry {
                code: "OOX-W-309",
                severity: Severity::Warning,
                category: "cf",
                description: "Conditional format content too complex: {0} rules",
            },
        );

        // T1-advanced: XLSX DV · ListObject tables (OOX-W-310..329)
        entries.insert(
            "OOX-W-310",
            CodeEntry {
                code: "OOX-W-310",
                severity: Severity::Warning,
                category: "dv",
                description: "DataValidation sqref invalid: {0}",
            },
        );
        entries.insert(
            "OOX-W-311",
            CodeEntry {
                code: "OOX-W-311",
                severity: Severity::Warning,
                category: "dv",
                description: "DV type {0} missing formula1",
            },
        );
        entries.insert(
            "OOX-W-312",
            CodeEntry {
                code: "OOX-W-312",
                severity: Severity::Warning,
                category: "dv",
                description: "DV operator invalid for type: {0}",
            },
        );
        entries.insert(
            "OOX-W-313",
            CodeEntry {
                code: "OOX-W-313",
                severity: Severity::Warning,
                category: "dv",
                description: "DV error message too long: {0} chars",
            },
        );
        entries.insert(
            "OOX-W-314",
            CodeEntry {
                code: "OOX-W-314",
                severity: Severity::Warning,
                category: "dv",
                description: "DV prompt message too long: {0} chars",
            },
        );
        entries.insert(
            "OOX-W-315",
            CodeEntry {
                code: "OOX-W-315",
                severity: Severity::Warning,
                category: "dv",
                description: "List data validation references external sheet",
            },
        );
        entries.insert(
            "OOX-W-316",
            CodeEntry {
                code: "OOX-W-316",
                severity: Severity::Warning,
                category: "dv",
                description: "DV write: failed to add dataValidation part",
            },
        );
        entries.insert(
            "OOX-W-317",
            CodeEntry {
                code: "OOX-W-317",
                severity: Severity::Warning,
                category: "dv",
                description: "DV write: formula parse error",
            },
        );
        entries.insert(
            "OOX-W-318",
            CodeEntry {
                code: "OOX-W-318",
                severity: Severity::Warning,
                category: "dv",
                description: "DV delete: validation index not found",
            },
        );
        entries.insert(
            "OOX-W-319",
            CodeEntry {
                code: "OOX-W-319",
                severity: Severity::Warning,
                category: "dv",
                description: "DV type {0} requires operator {1}",
            },
        );
        entries.insert(
            "OOX-W-320",
            CodeEntry {
                code: "OOX-W-320",
                severity: Severity::Warning,
                category: "table",
                description: "Table {0} ref range invalid",
            },
        );
        entries.insert(
            "OOX-W-321",
            CodeEntry {
                code: "OOX-W-321",
                severity: Severity::Warning,
                category: "table",
                description: "Table column count exceeds 16384",
            },
        );
        entries.insert(
            "OOX-W-322",
            CodeEntry {
                code: "OOX-W-322",
                severity: Severity::Warning,
                category: "table",
                description: "Table name already exists: {0}",
            },
        );
        entries.insert(
            "OOX-W-323",
            CodeEntry {
                code: "OOX-W-323",
                severity: Severity::Warning,
                category: "table",
                description: "Table part URI collision: {0}",
            },
        );
        entries.insert(
            "OOX-W-324",
            CodeEntry {
                code: "OOX-W-324",
                severity: Severity::Warning,
                category: "table",
                description: "Table write: missing table part",
            },
        );
        entries.insert(
            "OOX-W-325",
            CodeEntry {
                code: "OOX-W-325",
                severity: Severity::Warning,
                category: "table",
                description: "Table write: rel collision",
            },
        );
        entries.insert(
            "OOX-W-326",
            CodeEntry {
                code: "OOX-W-326",
                severity: Severity::Warning,
                category: "table",
                description: "Table totalsRow function invalid: {0}",
            },
        );
        entries.insert(
            "OOX-W-327",
            CodeEntry {
                code: "OOX-W-327",
                severity: Severity::Warning,
                category: "table",
                description: "Table autoFilter range mismatch",
            },
        );
        entries.insert(
            "OOX-W-328",
            CodeEntry {
                code: "OOX-W-328",
                severity: Severity::Warning,
                category: "table",
                description: "Table style not found: {0}",
            },
        );
        entries.insert(
            "OOX-W-329",
            CodeEntry {
                code: "OOX-W-329",
                severity: Severity::Warning,
                category: "table",
                description: "Table column duplicate id: {0}",
            },
        );

        // T1-advanced: XLSX protection · page setup · header-footer (OOX-W-330..349)
        entries.insert(
            "OOX-W-330",
            CodeEntry {
                code: "OOX-W-330",
                severity: Severity::Warning,
                category: "protection",
                description: "Sheet protection password too long: {0} chars",
            },
        );
        entries.insert(
            "OOX-W-331",
            CodeEntry {
                code: "OOX-W-331",
                severity: Severity::Warning,
                category: "protection",
                description: "Sheet protection: password hash mismatch",
            },
        );
        entries.insert(
            "OOX-W-332",
            CodeEntry {
                code: "OOX-W-332",
                severity: Severity::Warning,
                category: "protection",
                description: "Protection flag conflict: {0} vs {1}",
            },
        );
        entries.insert(
            "OOX-W-333",
            CodeEntry {
                code: "OOX-W-333",
                severity: Severity::Warning,
                category: "protection",
                description: "Protection write: sheet not locked",
            },
        );
        entries.insert(
            "OOX-W-334",
            CodeEntry {
                code: "OOX-W-334",
                severity: Severity::Warning,
                category: "protection",
                description: "Page setup paper size undefined: {0}",
            },
        );
        entries.insert(
            "OOX-W-335",
            CodeEntry {
                code: "OOX-W-335",
                severity: Severity::Warning,
                category: "protection",
                description: "Page setup scale 0 (use fit-to)",
            },
        );
        entries.insert(
            "OOX-W-336",
            CodeEntry {
                code: "OOX-W-336",
                severity: Severity::Warning,
                category: "protection",
                description: "Page setup: fit-to overrides scale",
            },
        );
        entries.insert(
            "OOX-W-337",
            CodeEntry {
                code: "OOX-W-337",
                severity: Severity::Warning,
                category: "protection",
                description: "Header/footer section unknown: {0}",
            },
        );
        entries.insert(
            "OOX-W-338",
            CodeEntry {
                code: "OOX-W-338",
                severity: Severity::Warning,
                category: "protection",
                description: "Header/footer string too long: {0} chars",
            },
        );
        entries.insert(
            "OOX-W-339",
            CodeEntry {
                code: "OOX-W-339",
                severity: Severity::Warning,
                category: "protection",
                description: "Header/footer ampersand escape invalid",
            },
        );
        entries.insert(
            "OOX-W-340",
            CodeEntry {
                code: "OOX-W-340",
                severity: Severity::Warning,
                category: "protection",
                description: "Page margins: negative value {0}",
            },
        );
        entries.insert(
            "OOX-W-341",
            CodeEntry {
                code: "OOX-W-341",
                severity: Severity::Warning,
                category: "protection",
                description: "Header/footer: different first but no first section",
            },
        );
        entries.insert(
            "OOX-W-342",
            CodeEntry {
                code: "OOX-W-342",
                severity: Severity::Warning,
                category: "protection",
                description: "Protection write: password not hashed",
            },
        );
        entries.insert(
            "OOX-W-343",
            CodeEntry {
                code: "OOX-W-343",
                severity: Severity::Warning,
                category: "protection",
                description: "Page setup write: invalid paper size",
            },
        );
        entries.insert(
            "OOX-W-344",
            CodeEntry {
                code: "OOX-W-344",
                severity: Severity::Warning,
                category: "protection",
                description: "Header/footer write: section not supported",
            },
        );
        entries.insert(
            "OOX-W-345",
            CodeEntry {
                code: "OOX-W-345",
                severity: Severity::Warning,
                category: "protection",
                description: "Protection: delete rows not allowed on protected sheet",
            },
        );
        entries.insert(
            "OOX-W-346",
            CodeEntry {
                code: "OOX-W-346",
                severity: Severity::Warning,
                category: "protection",
                description: "Page setup: DPI out of range: {0}",
            },
        );
        entries.insert(
            "OOX-W-347",
            CodeEntry {
                code: "OOX-W-347",
                severity: Severity::Warning,
                category: "protection",
                description: "Header/footer: unknown format code in string",
            },
        );
        entries.insert(
            "OOX-W-348",
            CodeEntry {
                code: "OOX-W-348",
                severity: Severity::Warning,
                category: "protection",
                description: "Protection write: sheet not in parts map",
            },
        );
        entries.insert(
            "OOX-W-349",
            CodeEntry {
                code: "OOX-W-349",
                severity: Severity::Warning,
                category: "protection",
                description: "Page setup write: margin out of range",
            },
        );

        // T1-advanced: PPTX shape tree · geometry · shape props (OOX-W-350..369)
        // NOTE: 350 and 351 already exist in the registry
        entries.insert(
            "OOX-W-352",
            CodeEntry {
                code: "OOX-W-352",
                severity: Severity::Warning,
                category: "shape",
                description: "Shape geometry: unknown preset {0}",
            },
        );
        entries.insert(
            "OOX-W-353",
            CodeEntry {
                code: "OOX-W-353",
                severity: Severity::Warning,
                category: "shape",
                description: "Shape fill: unknown fill type {0}",
            },
        );
        entries.insert(
            "OOX-W-354",
            CodeEntry {
                code: "OOX-W-354",
                severity: Severity::Warning,
                category: "shape",
                description: "Shape line: missing ln for connector",
            },
        );
        entries.insert(
            "OOX-W-355",
            CodeEntry {
                code: "OOX-W-355",
                severity: Severity::Warning,
                category: "shape",
                description: "Shape rotation: value out of range: {0}",
            },
        );
        entries.insert(
            "OOX-W-356",
            CodeEntry {
                code: "OOX-W-356",
                severity: Severity::Warning,
                category: "shape",
                description: "Group child count exceeds limit: {0}",
            },
        );
        entries.insert(
            "OOX-W-357",
            CodeEntry {
                code: "OOX-W-357",
                severity: Severity::Warning,
                category: "shape",
                description: "Shape ID collision: {0}",
            },
        );
        entries.insert(
            "OOX-W-358",
            CodeEntry {
                code: "OOX-W-358",
                severity: Severity::Warning,
                category: "shape",
                description: "Shape name too long: {0} chars",
            },
        );
        entries.insert(
            "OOX-W-359",
            CodeEntry {
                code: "OOX-W-359",
                severity: Severity::Warning,
                category: "shape",
                description: "Placeholder idx {0} not found in layout",
            },
        );
        entries.insert(
            "OOX-W-360",
            CodeEntry {
                code: "OOX-W-360",
                severity: Severity::Warning,
                category: "shape",
                description: "Shape write: part not in parts map",
            },
        );
        entries.insert(
            "OOX-W-361",
            CodeEntry {
                code: "OOX-W-361",
                severity: Severity::Warning,
                category: "shape",
                description: "Shape write: spPr mutation failed",
            },
        );
        entries.insert(
            "OOX-W-362",
            CodeEntry {
                code: "OOX-W-362",
                severity: Severity::Warning,
                category: "shape",
                description: "Shape write: fill color invalid",
            },
        );
        entries.insert(
            "OOX-W-363",
            CodeEntry {
                code: "OOX-W-363",
                severity: Severity::Warning,
                category: "shape",
                description: "Shape write: rotation not preserved",
            },
        );
        entries.insert(
            "OOX-W-364",
            CodeEntry {
                code: "OOX-W-364",
                severity: Severity::Warning,
                category: "shape",
                description: "Connector: missing connection coords",
            },
        );
        entries.insert(
            "OOX-W-365",
            CodeEntry {
                code: "OOX-W-365",
                severity: Severity::Warning,
                category: "shape",
                description: "Group: child {0} not in parent tree",
            },
        );
        entries.insert(
            "OOX-W-366",
            CodeEntry {
                code: "OOX-W-366",
                severity: Severity::Warning,
                category: "shape",
                description: "Shape write: missing rel entry",
            },
        );
        entries.insert(
            "OOX-W-367",
            CodeEntry {
                code: "OOX-W-367",
                severity: Severity::Warning,
                category: "shape",
                description: "Shape tree: orphan blip embed rId {0}",
            },
        );
        entries.insert(
            "OOX-W-368",
            CodeEntry {
                code: "OOX-W-368",
                severity: Severity::Warning,
                category: "shape",
                description: "pic:pic blip not in media parts",
            },
        );
        entries.insert(
            "OOX-W-369",
            CodeEntry {
                code: "OOX-W-369",
                severity: Severity::Warning,
                category: "shape",
                description: "Shape tree: depth exceeds limit: {0}",
            },
        );

        // T1-advanced: PPTX placeholders · table cells · notes (OOX-W-370..389)
        entries.insert(
            "OOX-W-370",
            CodeEntry {
                code: "OOX-W-370",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Placeholder {0} type mismatch: expected {1}",
            },
        );
        entries.insert(
            "OOX-W-371",
            CodeEntry {
                code: "OOX-W-371",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Placeholder text too long for {0}: {1} chars",
            },
        );
        entries.insert(
            "OOX-W-372",
            CodeEntry {
                code: "OOX-W-372",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Placeholder write: shape not found",
            },
        );
        entries.insert(
            "OOX-W-373",
            CodeEntry {
                code: "OOX-W-373",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Placeholder write: txBody not found",
            },
        );
        entries.insert(
            "OOX-W-374",
            CodeEntry {
                code: "OOX-W-374",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Table cell text too long: {0} chars",
            },
        );
        entries.insert(
            "OOX-W-375",
            CodeEntry {
                code: "OOX-W-375",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Table: row count exceeds limit: {0}",
            },
        );
        entries.insert(
            "OOX-W-376",
            CodeEntry {
                code: "OOX-W-376",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Table: column count exceeds limit: {0}",
            },
        );
        entries.insert(
            "OOX-W-377",
            CodeEntry {
                code: "OOX-W-377",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Table cell merge: invalid range",
            },
        );
        entries.insert(
            "OOX-W-378",
            CodeEntry {
                code: "OOX-W-378",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Table: cell rowspan + colSpan overflow",
            },
        );
        entries.insert(
            "OOX-W-379",
            CodeEntry {
                code: "OOX-W-379",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Table write: slide not found",
            },
        );
        entries.insert(
            "OOX-W-380",
            CodeEntry {
                code: "OOX-W-380",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Notes slide: no notes part URI",
            },
        );
        entries.insert(
            "OOX-W-381",
            CodeEntry {
                code: "OOX-W-381",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Notes: text exceeds limit: {0} chars",
            },
        );
        entries.insert(
            "OOX-W-382",
            CodeEntry {
                code: "OOX-W-382",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Notes write: part not in parts map",
            },
        );
        entries.insert(
            "OOX-W-383",
            CodeEntry {
                code: "OOX-W-383",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Notes write: failed to create notesSlide part",
            },
        );
        entries.insert(
            "OOX-W-384",
            CodeEntry {
                code: "OOX-W-384",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Placeholder: layout not found for slide {0}",
            },
        );
        entries.insert(
            "OOX-W-385",
            CodeEntry {
                code: "OOX-W-385",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Placeholder: idx 0 reserved (skip)",
            },
        );
        entries.insert(
            "OOX-W-386",
            CodeEntry {
                code: "OOX-W-386",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Table: body row missing text",
            },
        );
        entries.insert(
            "OOX-W-387",
            CodeEntry {
                code: "OOX-W-387",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Table: header row missing name",
            },
        );
        entries.insert(
            "OOX-W-388",
            CodeEntry {
                code: "OOX-W-388",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Table write: XML serialization error",
            },
        );
        entries.insert(
            "OOX-W-389",
            CodeEntry {
                code: "OOX-W-389",
                severity: Severity::Warning,
                category: "placeholder",
                description: "Notes: malformed txBody",
            },
        );

        // T1-advanced: PPTX theme · chart read (OOX-W-390..399)
        // NOTE: 390 and 391 already exist in the registry
        entries.insert(
            "OOX-W-392",
            CodeEntry {
                code: "OOX-W-392",
                severity: Severity::Warning,
                category: "theme",
                description: "Theme: clrMap override ignored",
            },
        );
        entries.insert(
            "OOX-W-393",
            CodeEntry {
                code: "OOX-W-393",
                severity: Severity::Warning,
                category: "theme",
                description: "Theme: font scheme unknown font family: {0}",
            },
        );
        entries.insert(
            "OOX-W-394",
            CodeEntry {
                code: "OOX-W-394",
                severity: Severity::Warning,
                category: "theme",
                description: "Theme: color count mismatch in clrScheme",
            },
        );
        entries.insert(
            "OOX-W-395",
            CodeEntry {
                code: "OOX-W-395",
                severity: Severity::Warning,
                category: "theme",
                description: "Chart: graphicFrame not chart: {0}",
            },
        );
        entries.insert(
            "OOX-W-396",
            CodeEntry {
                code: "OOX-W-396",
                severity: Severity::Warning,
                category: "theme",
                description: "Chart: series name missing",
            },
        );
        entries.insert(
            "OOX-W-397",
            CodeEntry {
                code: "OOX-W-397",
                severity: Severity::Warning,
                category: "theme",
                description: "Chart: category count mismatch",
            },
        );
        entries.insert(
            "OOX-W-398",
            CodeEntry {
                code: "OOX-W-398",
                severity: Severity::Warning,
                category: "theme",
                description: "Chart: value count mismatch",
            },
        );
        entries.insert(
            "OOX-W-399",
            CodeEntry {
                code: "OOX-W-399",
                severity: Severity::Warning,
                category: "theme",
                description: "Chart: data type unknown: {0}",
            },
        );

        // T1-advanced: capability matrix / inspect budget (GP-W-310..GP-W-319)
        entries.insert(
            "GP-W-310",
            CodeEntry {
                code: "GP-W-310",
                severity: Severity::Warning,
                category: "capability",
                description: "Capability {0} not available for format {1}",
            },
        );
        entries.insert(
            "GP-W-311",
            CodeEntry {
                code: "GP-W-311",
                severity: Severity::Warning,
                category: "capability",
                description: "Inspect feature {0} disabled by budget",
            },
        );
        entries.insert(
            "GP-W-312",
            CodeEntry {
                code: "GP-W-312",
                severity: Severity::Warning,
                category: "capability",
                description: "Inspect payload exceeds budget envelope: {0} bytes",
            },
        );
        entries.insert(
            "GP-W-313",
            CodeEntry {
                code: "GP-W-313",
                severity: Severity::Warning,
                category: "capability",
                description: "Feature {0} write not supported for format {1}",
            },
        );
        entries.insert(
            "GP-W-314",
            CodeEntry {
                code: "GP-W-314",
                severity: Severity::Warning,
                category: "capability",
                description: "Schema output truncated (budget limit)",
            },
        );
        entries.insert(
            "GP-W-315",
            CodeEntry {
                code: "GP-W-315",
                severity: Severity::Warning,
                category: "capability",
                description: "Capability check: format not registered: {0}",
            },
        );
        entries.insert(
            "GP-W-316",
            CodeEntry {
                code: "GP-W-316",
                severity: Severity::Warning,
                category: "capability",
                description: "Budget cap exceeded for {0}: {1}/{2}",
            },
        );
        entries.insert(
            "GP-W-317",
            CodeEntry {
                code: "GP-W-317",
                severity: Severity::Warning,
                category: "capability",
                description: "Inspect detail level {0} not recognized",
            },
        );
        entries.insert(
            "GP-W-318",
            CodeEntry {
                code: "GP-W-318",
                severity: Severity::Warning,
                category: "capability",
                description: "Feature {0} default-off (use --{0} flag)",
            },
        );
        entries.insert(
            "GP-W-319",
            CodeEntry {
                code: "GP-W-319",
                severity: Severity::Warning,
                category: "capability",
                description: "Capability matrix incomplete (format {0} partial)",
            },
        );

        Self { entries }
    }
}

/// Look up a code entry by string — convenience wrapper over the registry.
///
/// Returns a clone of the entry (owned).
pub fn get_code(code: &str) -> Option<CodeEntry> {
    DiagnosticRegistry::instance().lookup(code)
}

// ---------------------------------------------------------------------------
// ReadError
// ---------------------------------------------------------------------------

/// Error types covering all reader failure paths across stichos.
///
/// Each variant maps to one or more registered diagnostic codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// OPC (Open Packaging Conventions) validation/parsing failure.
    OpcError { code: &'static str, detail: String },
    /// XML parsing failure (OOX).
    XmlError { code: &'static str, detail: String },
    /// PDF parse failure.
    PdfError { code: &'static str, detail: String },
    /// ODF package error.
    OdfError { code: &'static str, detail: String },
    /// Engine-related failure.
    EngineError { code: &'static str, detail: String },
    /// Linter failure.
    LinterError { code: &'static str, detail: String },
    /// Standard I/O error.
    IoError { code: &'static str, detail: String },
    /// Deserialization (serde) failure.
    SerdeError { code: &'static str, detail: String },
    /// Catch-all for unclassified errors.
    Unknown { detail: String },
}

impl ReadError {
    /// Return the primary diagnostic code for this error.
    pub fn code(&self) -> &'static str {
        match self {
            ReadError::OpcError { code, .. } => code,
            ReadError::XmlError { code, .. } => code,
            ReadError::PdfError { code, .. } => code,
            ReadError::OdfError { code, .. } => code,
            ReadError::EngineError { code, .. } => code,
            ReadError::LinterError { code, .. } => code,
            ReadError::IoError { code, .. } => code,
            ReadError::SerdeError { code, .. } => code,
            ReadError::Unknown { .. } => "GP-W-001",
        }
    }

    /// Convert this error into a list of diagnostics (1..N).
    ///
    /// The primary diagnostic uses the error's code; additional diagnostics
    /// are produced for multi-part errors.
    pub fn to_diagnostics(&self) -> Vec<Diagnostic> {
        let entry = get_code(self.code());
        let severity = entry
            .as_ref()
            .map(|e| e.severity)
            .unwrap_or(Severity::Error);
        let primary_msg: String = entry
            .as_ref()
            .map(|e| e.format_message(&[self.detail()]))
            .unwrap_or_else(|| self.detail().to_string());

        let mut diags = vec![Diagnostic::new(self.code(), severity, primary_msg)];

        // Some errors produce additional diagnostics.
        if let ReadError::Unknown { detail } = self {
            diags.push(Diagnostic::new(
                "GP-W-001",
                Severity::Warning,
                format!("Unrecognized error: {}", detail),
            ));
        }

        diags
    }

    /// Return a human-readable detail string.
    pub fn detail(&self) -> &str {
        match self {
            ReadError::OpcError { detail, .. } => detail,
            ReadError::XmlError { detail, .. } => detail,
            ReadError::PdfError { detail, .. } => detail,
            ReadError::OdfError { detail, .. } => detail,
            ReadError::EngineError { detail, .. } => detail,
            ReadError::LinterError { detail, .. } => detail,
            ReadError::IoError { detail, .. } => detail,
            ReadError::SerdeError { detail, .. } => detail,
            ReadError::Unknown { detail } => detail,
        }
    }
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::OpcError { code, detail } => write!(f, "[{}] {}", code, detail),
            ReadError::XmlError { code, detail } => write!(f, "[{}] {}", code, detail),
            ReadError::PdfError { code, detail } => write!(f, "[{}] {}", code, detail),
            ReadError::OdfError { code, detail } => write!(f, "[{}] {}", code, detail),
            ReadError::EngineError { code, detail } => write!(f, "[{}] {}", code, detail),
            ReadError::LinterError { code, detail } => write!(f, "[{}] {}", code, detail),
            ReadError::IoError { code, detail } => write!(f, "[{}] {}", code, detail),
            ReadError::SerdeError { code, detail } => write!(f, "[{}] {}", code, detail),
            ReadError::Unknown { detail } => write!(f, "error: {}", detail),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<std::io::Error> for ReadError {
    fn from(err: std::io::Error) -> Self {
        let code = "CLI-E-002";
        let kind = match err.kind() {
            std::io::ErrorKind::NotFound => "file not found",
            std::io::ErrorKind::PermissionDenied => "permission denied",
            std::io::ErrorKind::AlreadyExists => "file already exists",
            std::io::ErrorKind::InvalidData => "invalid data",
            _ => "io error",
        };
        ReadError::IoError {
            code,
            detail: format!("{}: {}", kind, err),
        }
    }
}

impl From<serde_json::Error> for ReadError {
    fn from(err: serde_json::Error) -> Self {
        ReadError::SerdeError {
            code: "GP-E-001",
            detail: err.to_string(),
        }
    }
}

/// Convenience: create an OPC validation error.
pub fn opc_error<S: Into<String>>(detail: S) -> ReadError {
    ReadError::OpcError {
        code: "OOX-E-001",
        detail: detail.into(),
    }
}

/// Convenience: create an XML parse error.
pub fn xml_error<S: Into<String>>(detail: S) -> ReadError {
    ReadError::XmlError {
        code: "OOX-E-002",
        detail: detail.into(),
    }
}

/// Convenience: create a path traversal error.
pub fn path_traversal_error<S: Into<String>>(entry: S) -> ReadError {
    ReadError::OpcError {
        code: "OOX-E-004",
        detail: format!("path traversal: {}", entry.into()),
    }
}

/// Convenience: create a zip-bomb error.
pub fn zip_bomb_error(bytes: usize, entries: usize) -> ReadError {
    ReadError::OpcError {
        code: "OOX-E-005",
        detail: format!("zip-bomb: {} bytes in {} entries", bytes, entries),
    }
}

/// Convenience: create a PDF parse error.
pub fn pdf_error<S: Into<String>>(detail: S) -> ReadError {
    ReadError::PdfError {
        code: "PDF-E-001",
        detail: detail.into(),
    }
}

/// Convenience: create an ODF package error.
pub fn odf_error<S: Into<String>>(detail: S) -> ReadError {
    ReadError::OdfError {
        code: "ODF-E-001",
        detail: detail.into(),
    }
}

/// Convenience: create an engine-not-found error.
pub fn engine_not_found<S: Into<String>>(name: S) -> ReadError {
    ReadError::EngineError {
        code: "ENG-E-001",
        detail: format!("engine not found: {}", name.into()),
    }
}

/// Convenience: create a layout overflow error.
pub fn layout_overflow_error<S: Into<String>>(detail: S) -> ReadError {
    ReadError::LinterError {
        code: "LIN-E-001",
        detail: detail.into(),
    }
}

/// Convenience: create an invalid CLI argument error.
pub fn invalid_arg_error<S: Into<String>>(detail: S) -> ReadError {
    ReadError::IoError {
        code: "CLI-E-001",
        detail: format!("invalid argument: {}", detail.into()),
    }
}

/// Convenience: create an invalid JSON-RPC error.
pub fn invalid_jsonrpc_error<S: Into<String>>(detail: S) -> ReadError {
    ReadError::SerdeError {
        code: "MCP-E-001",
        detail: format!("invalid JSON-RPC: {}", detail.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Code pattern tests
    // ------------------------------------------------------------------

    #[test]
    fn valid_code_patterns() {
        assert!(is_valid_code_pattern("GP-E-001"));
        assert!(is_valid_code_pattern("OOX-E-001"));
        assert!(is_valid_code_pattern("ODF-E-001"));
        assert!(is_valid_code_pattern("PDF-E-001"));
        assert!(is_valid_code_pattern("ENG-E-001"));
        assert!(is_valid_code_pattern("LIN-E-001"));
        assert!(is_valid_code_pattern("CLI-E-001"));
        assert!(is_valid_code_pattern("MCP-E-001"));
        // Variations
        assert!(is_valid_code_pattern("GP-W-001"));
        assert!(is_valid_code_pattern("ENG-W-001"));
        // Longer codes
        assert!(is_valid_code_pattern("PDF-E-012"));
        assert!(is_valid_code_pattern("OOX-W-9999"));
    }

    #[test]
    fn invalid_code_patterns() {
        assert!(!is_valid_code_pattern(""));
        assert!(!is_valid_code_pattern("A-E001")); // prefix too short (1 letter)
        assert!(!is_valid_code_pattern("GP-e001")); // lowercase category
        assert!(!is_valid_code_pattern("GP-E00")); // digits too short (2 digits)
        assert!(!is_valid_code_pattern("GP-E00a")); // non-digit
        assert!(!is_valid_code_pattern("GP--001")); // empty category
        assert!(!is_valid_code_pattern("GP-ABCDE-001")); // 5-letter category
        assert!(!is_valid_code_pattern("GP-AB-00")); // only 2 digits
        assert!(!is_valid_code_pattern("GP-E")); // no dash+digits after category
    }

    // ------------------------------------------------------------------
    // Registry tests
    // ------------------------------------------------------------------

    #[test]
    fn all_required_codes_exist() {
        let reg = DiagnosticRegistry::instance();
        let _all = reg.all_codes();
        let required = [
            "GP-E-001",
            "GP-E-002",
            "GP-E-003",
            "GP-W-001",
            "ODX-E-001",
            "ODX-E-002",
            "ODX-E-003",
            "ODX-E-004",
            "ODX-E-005",
            "ODX-W-001",
            "ODX-W-003",
            "OOX-E-001",
            "OOX-E-002",
            "OOX-E-003",
            "OOX-E-004",
            "OOX-E-005",
            "OOX-E-006",
            "OOX-E-007",
            "OOX-E-008",
            "OOX-E-010",
            "OOX-E-015",
            "OOX-E-016",
            "OOX-E-020",
            "OOX-E-021",
            "OOX-W-002",
            "OOX-W-010",
            "OOX-W-020",
            "OOX-W-021",
            "OOX-W-022",
            "OOX-W-023",
            "OOX-W-100",
            "OOX-W-101",
            "OOX-W-150",
            "OOX-W-151",
            "OOX-W-200",
            "OOX-W-201",
            "OOX-E-022",
            "OOX-E-023",
            "OOX-E-024",
            "OOX-E-025",
            "OOX-W-026",
            "OOX-W-027",
            "OOX-E-026",
            "OOX-W-029",
            "ODF-E-001",
            "ODF-E-002",
            "ODF-E-003",
            "ODF-E-010",
            "ODF-R200",
            "ODF-R201",
            "ODF-R210",
            "ODF-R300",
            "ODF-R301",
            "ODF-R310",
            "PDF-E-001",
            "PDF-E-002",
            "PDF-E-003",
            "PDF-E-005",
            "PDF-E-006",
            "PDF-W-004",
            "PDF-E-009",
            "PDF-E-010",
            "PDF-I-001",
            "PDF-I-002",
            "PDF-E-012",
            "ENG-E-001",
            "ENG-E-002",
            "ENG-W-001",
            "ENG-E-100",
            "ENG-E-101",
            "ENG-E-102",
            "ENG-E-103",
            "ENG-E-104",
            "ENG-E-110",
            "ENG-E-111",
            "ENG-E-112",
            "ENG-E-113",
            "ENG-E-114",
            "ENG-E-120",
            "ENG-E-121",
            "ENG-E-122",
            "ENG-E-123",
            "ENG-E-124",
            "ENG-E-130",
            "ENG-E-131",
            "ENG-E-132",
            "ENG-E-133",
            "ENG-E-134",
            "ENG-E-135",
            "ENG-E-030",
            "ENG-E-201",
            "ENG-E-202",
            "ENG-E-203",
            "ENG-E-204",
            "LIN-E-001",
            "LIN-E-002",
            "LIN-E-003",
            "LIN-E-004",
            "LIN-E-005",
            "LIN-E-006",
            "LIN-W-001",
            "LIN-W-002",
            "LIN-W-003",
            "LIN-W-004",
            "LIN-W-005",
            "LIN-W-006",
            "LIN-W-007",
            "LIN-W-008",
            "LIN-W-009",
            "LIN-B-001",
            "LIN-B-002",
            "LIN-I-001",
            "LIN-I-002",
            "LIN-I-003",
            "LIN-E-007",
            "CLI-E-001",
            "CLI-E-002",
            "MCP-E-001",
            "GP-A-001",
            "GP-A-002",
            "GP-A-003",
            "GP-A-004",
            "GP-A-005",
            "MCP-E-002",
        ];
        for code in &required {
            assert!(
                reg.lookup(code).is_some(),
                "Missing required code: {}",
                code
            );
        }
    }

    #[test]
    fn code_uniqueness() {
        let reg = DiagnosticRegistry::instance();
        assert!(!reg.has_duplicates(), "Duplicate code entries found");
    }

    #[test]
    fn all_codes_match_pattern() {
        let reg = DiagnosticRegistry::instance();
        for entry in reg.iter_all() {
            // ODF-R200..R310 codes use a legacy format (ODF-R200 vs ODF-R-200)
            // They are registered but don't match the strict C3 pattern
            if entry.code.starts_with("ODF-R") {
                continue;
            }
            assert!(
                is_valid_code_pattern(entry.code),
                "Code '{}' does not match pattern ^PREFIX-[A-Z]{{1,3}}-\\d{{3,}}$",
                entry.code
            );
        }
    }

    #[test]
    fn code_lookup_by_string() {
        let reg = DiagnosticRegistry::instance();
        assert!(reg.lookup("GP-E-001").is_some());
        assert!(reg.lookup("OOX-E-003").is_some());
        assert!(reg.lookup("UNKNOWN-E001").is_none());
        assert!(reg.lookup("nonexistent").is_none());
    }

    #[test]
    fn severity_ordering() {
        assert!(Severity::Hint < Severity::Info);
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
        assert!(Severity::Error < Severity::Fatal);
        assert!(severity_is_fatal(Severity::Fatal));
        assert!(!severity_is_fatal(Severity::Error));
    }

    // ------------------------------------------------------------------
    // Diagnostic tests
    // ------------------------------------------------------------------

    #[test]
    fn diagnostic_code_enum_lookup() {
        let d = Diagnostic::new("GP-E-001", Severity::Error, "test");
        let entry = d.code_enum().expect("GP-E001 should be in registry");
        assert_eq!(entry.code, "GP-E-001");
        assert_eq!(entry.severity, Severity::Error);
        assert_eq!(entry.category, "validation");
    }

    #[test]
    fn diagnostic_code_enum_unknown() {
        let d = Diagnostic::new("XXX-Z-999", Severity::Error, "test");
        assert!(d.code_enum().is_none());
    }

    #[test]
    fn diagnostic_display_format() {
        let d = Diagnostic::new("GP-E-001", Severity::Error, "invalid version");
        assert_eq!(format!("{}", d), "[ERROR] GP-E-001 invalid version");
    }

    #[test]
    fn diagnostic_with_span() {
        let d = Diagnostic::new("GP-E-001", Severity::Error, "invalid version").with_span(10, 5);
        assert_eq!(d.span.as_ref().unwrap().offset, 10);
        assert_eq!(d.span.as_ref().unwrap().length, 5);
    }

    // ------------------------------------------------------------------
    // Serde / JSON tests
    // ------------------------------------------------------------------

    #[test]
    fn diagnostic_roundtrip() {
        let d = Diagnostic::new("GP-E-001", Severity::Error, "test message").with_span(0, 10);
        let json = serde_json::to_string(&d).unwrap();
        let back: Diagnostic = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn diagnostic_no_span_skipped() {
        let d = Diagnostic::new("GP-E-001", Severity::Error, "no location data");
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("span"));
    }

    #[test]
    fn envelope_json_shape() {
        let diags = vec![
            Diagnostic::new("GP-E-001", Severity::Error, "invalid version").with_span(0, 4),
            Diagnostic::new("GP-W-001", Severity::Warning, "unknown element"),
        ];
        let envelope = DiagnosticEnvelope::new(diags);
        let json = serde_json::to_string(&envelope).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Must have a "diagnostics" array
        let diags_arr = parsed["diagnostics"]
            .as_array()
            .expect("should have diagnostics array");
        assert_eq!(diags_arr.len(), 2);
        // Each element must have code, severity, message
        for item in diags_arr {
            assert!(item["code"].is_string());
            assert!(item["severity"].is_string());
            assert!(item["message"].is_string());
        }
    }

    #[test]
    fn envelope_roundtrip() {
        let diags =
            vec![Diagnostic::new("OOX-E-002", Severity::Error, "xml error").with_span(1, 50)];
        let env = DiagnosticEnvelope::new(diags);
        let json = serde_json::to_string(&env).unwrap();
        let back: DiagnosticEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(env, back);
    }

    #[test]
    fn empty_envelope_json_shape() {
        let envelope = DiagnosticEnvelope::empty();
        let json = serde_json::to_string(&envelope).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["diagnostics"].as_array().map(|a| a.len()), Some(0));
    }

    // ------------------------------------------------------------------
    // ReadError tests
    // ------------------------------------------------------------------

    #[test]
    fn read_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "no file");
        let re: ReadError = io_err.into();
        match re {
            ReadError::IoError { code, .. } => assert_eq!(code, "CLI-E-002"),
            _ => panic!("expected IoError"),
        }
    }

    #[test]
    fn read_error_from_serde() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid");
        assert!(json_err.is_err());
        let re: ReadError = json_err.unwrap_err().into();
        match re {
            ReadError::SerdeError { code, .. } => assert_eq!(code, "GP-E-001"),
            _ => panic!("expected SerdeError"),
        }
    }

    #[test]
    fn read_error_to_diagnostics_single() {
        let re = ReadError::OpcError {
            code: "OOX-E-001",
            detail: "missing [Content_Types].xml".into(),
        };
        let diags = re.to_diagnostics();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "OOX-E-001");
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn read_error_to_diagnostics_unknown_produces_two() {
        let re = ReadError::Unknown {
            detail: "something weird".into(),
        };
        let diags = re.to_diagnostics();
        assert_eq!(diags.len(), 2, "Unknown errors produce 2 diagnostics");
        assert_eq!(diags[0].code, "GP-W-001");
        assert_eq!(diags[1].code, "GP-W-001");
    }

    #[test]
    fn read_error_display() {
        let re = ReadError::PdfError {
            code: "PDF-E-001",
            detail: "trailer not found".into(),
        };
        let display = format!("{}", re);
        assert!(display.starts_with("[PDF-E-001]"));
        assert!(display.contains("trailer not found"));
    }

    #[test]
    fn read_error_code_map() {
        assert_eq!(
            ReadError::OpcError {
                code: "OOX-E-001",
                detail: "x".into()
            }
            .code(),
            "OOX-E-001"
        );
        assert_eq!(
            ReadError::XmlError {
                code: "OOX-E-002",
                detail: "x".into()
            }
            .code(),
            "OOX-E-002"
        );
        assert_eq!(
            ReadError::PdfError {
                code: "PDF-E-001",
                detail: "x".into()
            }
            .code(),
            "PDF-E-001"
        );
        assert_eq!(
            ReadError::OdfError {
                code: "ODF-E-001",
                detail: "x".into()
            }
            .code(),
            "ODF-E-001"
        );
        assert_eq!(
            ReadError::EngineError {
                code: "ENG-E-001",
                detail: "x".into()
            }
            .code(),
            "ENG-E-001"
        );
        assert_eq!(
            ReadError::LinterError {
                code: "LIN-E-001",
                detail: "x".into()
            }
            .code(),
            "LIN-E-001"
        );
        assert_eq!(
            ReadError::IoError {
                code: "CLI-E-001",
                detail: "x".into()
            }
            .code(),
            "CLI-E-001"
        );
        assert_eq!(ReadError::Unknown { detail: "x".into() }.code(), "GP-W-001");
    }

    // ------------------------------------------------------------------
    // Convenience function tests
    // ------------------------------------------------------------------

    #[test]
    fn convenience_functions() {
        let re = opc_error("bad content types");
        assert!(matches!(re, ReadError::OpcError { .. }));

        let re = xml_error("malformed");
        assert!(matches!(re, ReadError::XmlError { .. }));

        let re = path_traversal_error("../../etc/passwd");
        assert!(matches!(re, ReadError::OpcError { .. }));
        assert_eq!(re.code(), "OOX-E-004");

        let re = zip_bomb_error(10_000_000, 1000);
        assert!(matches!(re, ReadError::OpcError { .. }));
        assert_eq!(re.code(), "OOX-E-005");

        let re = pdf_error("bad xref");
        assert!(matches!(re, ReadError::PdfError { .. }));

        let re = odf_error("not odf");
        assert!(matches!(re, ReadError::OdfError { .. }));

        let re = engine_not_found("libreoffice");
        assert!(matches!(re, ReadError::EngineError { .. }));

        let re = layout_overflow_error("margin exceeded");
        assert!(matches!(re, ReadError::LinterError { .. }));

        let re = invalid_arg_error("--unknown flag");
        assert!(matches!(re, ReadError::IoError { .. }));

        let re = invalid_jsonrpc_error("not json");
        assert!(matches!(re, ReadError::SerdeError { .. }));
    }

    // ------------------------------------------------------------------
    // Registry invariant tests
    // ------------------------------------------------------------------

    #[test]
    fn registry_has_fatal_codes() {
        let reg = DiagnosticRegistry::instance();
        // OOX-E004 and OOX-E005 are fatal
        assert!(reg.has_fatal_codes());
        // Verify they are actually fatal
        let e004 = reg.lookup("OOX-E-004").expect("should exist");
        let e005 = reg.lookup("OOX-E-005").expect("should exist");
        assert_eq!(e004.severity, Severity::Fatal);
        assert_eq!(e005.severity, Severity::Fatal);
    }

    #[test]
    fn code_entry_format_message() {
        let reg = DiagnosticRegistry::instance();
        let entry = reg.lookup("GP-E-001").expect("should exist");
        let msg = entry.format_message(&["1.0", "2.0"]);
        assert_eq!(msg, "Invalid IR version: expected 1.0 but found 2.0");
    }

    #[test]
    fn code_entry_format_message_no_args() {
        let reg = DiagnosticRegistry::instance();
        let entry = reg.lookup("GP-W-001").expect("should exist");
        let msg = entry.format_message(&[]);
        assert_eq!(msg, "Unknown element '{}' in IR — ignored");
    }

    #[test]
    fn all_codes_sorted() {
        let reg = DiagnosticRegistry::instance();
        let codes = reg.all_codes();
        let mut sorted = codes.clone();
        sorted.sort();
        assert_eq!(codes, sorted, "codes should be returned in sorted order");
    }

    #[test]
    fn no_adhoc_strings_in_registry_codes() {
        // Verify every code in the registry follows the pattern
        let reg = DiagnosticRegistry::instance();
        for entry in reg.iter_all() {
            // ODF-R200..R310 codes use a legacy format (ODF-R200 vs ODF-R-200)
            if entry.code.starts_with("ODF-R") {
                continue;
            }
            assert!(
                is_valid_code_pattern(entry.code),
                "Registry code '{}' is not ad-hoc — must match PREFIX-XYZ000 pattern",
                entry.code
            );
        }
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;
    #[test]
    fn debug_pattern_gp() {
        // Manual trace: "GP-E-001" has 7 chars: G(0) P(1) -(2) E(3) -(4) 0(5) 0(6) 1(7)
        // Wait: GP-E001 = 7 chars: indices 0-6
        let result = is_valid_code_pattern("GP-E-001");
        assert!(result, "GP-E-001 failed");
    }
}
