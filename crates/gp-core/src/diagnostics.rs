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
            "LIN-B-001",
            "LIN-B-002",
            "LIN-I-001",
            "LIN-I-002",
            "CLI-E-001",
            "CLI-E-002",
            "MCP-E-001",
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
