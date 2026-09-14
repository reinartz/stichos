//! Edit policy and taint propagation types.
//!
//! `EditPolicy` is a part-class allow-list with destructive-op default-deny.
//! `TaintMark` propagates document-taint information through the IR response
//! envelope so tooling can surface taint to the user.
//!
//! See also: `docs/contracts/C10.md`

use serde::{Deserialize, Serialize};

use crate::diagnostics::Diagnostic;
use crate::fidelity::FidelityBudget;

// ---------------------------------------------------------------------------
// DeniedAction
// ---------------------------------------------------------------------------

/// Actions that are always denied regardless of the allow-list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeniedAction {
    /// Deleting an OPC part.
    DeletePart,
    /// Writing to a macro-containing part.
    MacroPartWrite,
    /// Adding an external relationship.
    ExternalRelAdd,
    /// Catch-all for unknown actions.
    UnknownAction,
}

// ---------------------------------------------------------------------------
// EditPolicy
// ---------------------------------------------------------------------------

/// Policy governing which part classes and actions are permitted.
///
/// - `allowed_parts` uses glob-like patterns for matching part paths.
/// - `denied_actions` is an always-denied list.
/// - `max_parts_modified` is an optional safety cap.
///
/// **Empty policy means deny-by-default**: no parts are allowed and all
/// actions in `denied_actions` are blocked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditPolicy {
    /// Glob-like patterns for allowed parts.
    pub allowed_parts: Vec<String>,
    /// Actions that are always denied.
    pub denied_actions: Vec<DeniedAction>,
    /// Maximum number of parts that may be modified in a single edit.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_parts_modified: Option<usize>,
}

impl Default for EditPolicy {
    fn default() -> Self {
        Self {
            allowed_parts: Vec::new(),
            denied_actions: vec![
                DeniedAction::DeletePart,
                DeniedAction::MacroPartWrite,
                DeniedAction::ExternalRelAdd,
            ],
            max_parts_modified: None,
        }
    }
}

impl EditPolicy {
    /// Create a new deny-by-default policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check whether a specific denied action is permitted.
    ///
    /// Returns `true` only if the action is **not** in the denied list.
    pub fn is_allowed(&self, action: &DeniedAction) -> bool {
        !self.denied_actions.contains(action)
    }

    /// Evaluate a part name against this policy.
    ///
    /// Returns a `Vec` of actions that were denied. An empty vec means the
    /// part and all requested actions are allowed.
    ///
    /// # Denial rules
    /// 1. If `allowed_parts` is empty, **every** part is denied (deny-by-default).
    /// 2. If the part does not match any pattern in `allowed_parts`, it is denied.
    /// 3. Any action present in `denied_actions` is always denied.
    pub fn evaluate(
        &self,
        part: &str,
        actions: &[DeniedAction],
    ) -> Result<Vec<DeniedAction>, Diagnostic> {
        let mut denied: Vec<DeniedAction> = Vec::new();

        // Rule 1: empty allow-list = deny everything
        if self.allowed_parts.is_empty() {
            return Ok(actions.to_vec());
        }

        // Rule 2: check glob-like match
        let part_allowed = self.allowed_parts.iter().any(|pattern| {
            glob_match(pattern, part)
        });

        if !part_allowed {
            // Return all actions as denied
            return Ok(actions.to_vec());
        }

        // Rule 3: deny listed actions
        for action in actions {
            if !self.is_allowed(action) {
                denied.push(action.clone());
            }
        }

        Ok(denied)
    }
}

/// Simple glob-like pattern matching for part paths.
///
/// Supports:
/// - `*` matches any sequence of characters within a path segment
/// - `**` matches any sequence of characters across path boundaries
/// - `?` matches a single character
/// - Literal characters match themselves
fn glob_match(pattern: &str, text: &str) -> bool {
    glob_match_inner(pattern.as_bytes(), 0, text.as_bytes(), 0)
}

fn glob_match_inner(
    pattern: &[u8],
    pi: usize,
    text: &[u8],
    ti: usize,
) -> bool {
    let mut p = pi;
    let mut t = ti;
    let plen = pattern.len();
    let tlen = text.len();

    while p < plen || t < tlen {
        // Check for ** (match anything including /)
        if p + 1 < plen && pattern[p] == b'*' && pattern[p + 1] == b'*' {
            // Skip trailing * as well
            let mut np = p + 2;
            while np < plen && pattern[np] == b'*' {
                np += 1;
            }
            // Try matching the rest of the pattern at every position
            for skip in 0..=(tlen - t) {
                if glob_match_inner(pattern, np, text, t + skip) {
                    return true;
                }
            }
            return false;
        }

        // Check for * (match anything except /)
        if p < plen && pattern[p] == b'*' {
            let mut np = p + 1;
            while np < plen && pattern[np] == b'*' {
                np += 1;
            }
            for skip in 0..=(tlen - t) {
                if text[t..t + skip].contains(&b'/') {
                    break;
                }
                if glob_match_inner(pattern, np, text, t + skip) {
                    return true;
                }
            }
            return false;
        }

        // Check for ? (match single char, not /)
        if p < plen && t < tlen && pattern[p] == b'?' && text[t] != b'/' {
            p += 1;
            t += 1;
            continue;
        }

        // Literal match
        if p < plen && t < tlen && pattern[p] == text[t] {
            p += 1;
            t += 1;
            continue;
        }

        return false;
    }

    p >= plen
}

// ---------------------------------------------------------------------------
// TaintKind
// ---------------------------------------------------------------------------

/// The kind of taint attached to an IR response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaintKind {
    /// Text extracted from a .docx source.
    DocxText,
    /// Text extracted from a PDF source.
    PdfText,
    /// Unclassified taint source.
    Unknown,
}

// ---------------------------------------------------------------------------
// TaintMark
// ---------------------------------------------------------------------------

/// A taint marker that propagates through the IR response envelope.
///
/// Carries the taint source part and kind so tooling can surface this
/// to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaintMark {
    /// Whether the response is tainted.
    pub tainted: bool,
    /// The part that introduced the taint.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub source_part: Option<String>,
    /// The kind of taint.
    pub kind: TaintKind,
}

impl TaintMark {
    /// Create a new taint mark.
    pub fn new(tainted: bool, source_part: Option<String>, kind: TaintKind) -> Self {
        Self {
            tainted,
            source_part,
            kind,
        }
    }
}

// ---------------------------------------------------------------------------
// ResponseEnvelope
// ---------------------------------------------------------------------------

/// The IR response envelope carrying taint, diagnostics, and fidelity info.
///
/// Every tool/executor response is wrapped in this type so that clients
/// can inspect taint and fidelity budget information alongside the result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    /// Whether the operation succeeded.
    pub success: bool,
    /// Taint information, if any.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub taint: Option<TaintMark>,
    /// Diagnostics from the operation.
    pub diagnostics: Vec<Diagnostic>,
    /// Optional journal entry ID.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub journal_id: Option<String>,
    /// Fidelity budget for the operation.
    pub fidelity_budget: FidelityBudget,
}

impl Default for ResponseEnvelope {
    fn default() -> Self {
        Self {
            success: false,
            taint: None,
            diagnostics: Vec::new(),
            journal_id: None,
            fidelity_budget: FidelityBudget::new(),
        }
    }
}

impl ResponseEnvelope {
    /// Create a new successful envelope with no taint.
    pub fn success() -> Self {
        Self {
            success: true,
            taint: None,
            diagnostics: Vec::new(),
            journal_id: None,
            fidelity_budget: FidelityBudget::new(),
        }
    }

    /// Create a new failed envelope.
    pub fn failure() -> Self {
        Self {
            success: false,
            taint: None,
            diagnostics: Vec::new(),
            journal_id: None,
            fidelity_budget: FidelityBudget::new(),
        }
    }

    /// Mark this envelope as tainted.
    pub fn with_taint(mut self, taint: TaintMark) -> Self {
        self.taint = Some(taint);
        self
    }

    /// Add a diagnostic.
    pub fn with_diagnostic(mut self, diag: Diagnostic) -> Self {
        self.diagnostics.push(diag);
        self
    }

    /// Set the journal ID.
    pub fn with_journal_id(mut self, id: impl Into<String>) -> Self {
        self.journal_id = Some(id.into());
        self
    }

    /// Set the fidelity budget.
    pub fn with_fidelity_budget(mut self, budget: FidelityBudget) -> Self {
        self.fidelity_budget = budget;
        self
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Severity;

    // ------------------------------------------------------------------
    // Type tests
    // ------------------------------------------------------------------

    #[test]
    fn test_denied_action_serde() {
        let actions = vec![
            DeniedAction::DeletePart,
            DeniedAction::MacroPartWrite,
            DeniedAction::ExternalRelAdd,
            DeniedAction::UnknownAction,
        ];
        let json = serde_json::to_string(&actions).expect("serialize");
        let decoded: Vec<DeniedAction> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(actions, decoded);
    }

    #[test]
    fn test_edit_policy_default_is_deny_all() {
        let policy = EditPolicy::new();
        // Empty allowed_parts → deny-by-default
        let denied = policy.evaluate("word/document.xml", &[]).unwrap();
        assert!(denied.is_empty()); // no actions requested, but part is also not allowed

        // With actions, they should all be denied
        let denied = policy
            .evaluate("word/document.xml", &[DeniedAction::DeletePart])
            .unwrap();
        assert_eq!(denied.len(), 1);
    }

    #[test]
    fn test_deny_by_default_test() {
        // Empty policy → nothing allowed (deny-by-default)
        let policy = EditPolicy::default();
        assert!(policy.allowed_parts.is_empty());

        // Any part with any action should be denied
        let denied = policy
            .evaluate(
                "word/document.xml",
                &[DeniedAction::DeletePart, DeniedAction::MacroPartWrite],
            )
            .unwrap();
        // All actions are denied because the part is not in the allow-list
        assert_eq!(denied.len(), 2);
    }

    #[test]
    fn test_is_allowed() {
        let mut policy = EditPolicy::default();
        assert!(!policy.is_allowed(&DeniedAction::DeletePart));
        assert!(!policy.is_allowed(&DeniedAction::MacroPartWrite));
        assert!(!policy.is_allowed(&DeniedAction::ExternalRelAdd));
        // UnknownAction is NOT in the default deny list
        assert!(policy.is_allowed(&DeniedAction::UnknownAction));

        // Allow DeletePart
        policy
            .denied_actions
            .retain(|a| *a != DeniedAction::DeletePart);
        assert!(policy.is_allowed(&DeniedAction::DeletePart));
        assert!(!policy.is_allowed(&DeniedAction::MacroPartWrite));

        // Add UnknownAction to deny list
        policy.denied_actions.push(DeniedAction::UnknownAction);
        assert!(policy.is_allowed(&DeniedAction::DeletePart));
        assert!(!policy.is_allowed(&DeniedAction::MacroPartWrite));
        assert!(!policy.is_allowed(&DeniedAction::UnknownAction));
    }

    #[test]
    fn test_allow_list_policy() {
        let policy = EditPolicy {
            allowed_parts: vec!["word/*.xml".to_string(), "ppt/slides/*.xml".to_string()],
            denied_actions: vec![DeniedAction::ExternalRelAdd],
            max_parts_modified: Some(10),
        };

        // Matching part: only denied_actions are denied
        let denied = policy
            .evaluate(
                "word/document.xml",
                &[
                    DeniedAction::DeletePart,
                    DeniedAction::ExternalRelAdd,
                ],
            )
            .unwrap();
        assert_eq!(denied.len(), 1);
        assert!(matches!(denied[0], DeniedAction::ExternalRelAdd));

        // Non-matching part: everything denied
        let denied = policy
            .evaluate("xl/styles.xml", &[DeniedAction::DeletePart])
            .unwrap();
        assert_eq!(denied.len(), 1);

        // Matching part, no denied actions → empty
        let denied = policy
            .evaluate("word/document.xml", &[])
            .unwrap();
        assert!(denied.is_empty());

        // Glob ** matches across path
        let policy2 = EditPolicy {
            allowed_parts: vec!["**/document.xml".to_string()],
            denied_actions: Vec::new(),
            max_parts_modified: None,
        };
        let denied = policy2.evaluate("ppt/slides/document.xml", &[]).unwrap();
        assert!(denied.is_empty());
    }

    #[test]
    fn test_glob_match() {
        assert!(glob_match("word/*.xml", "word/document.xml"));
        assert!(glob_match("word/*.xml", "word/slides.xml"));
        assert!(!glob_match("word/*.xml", "xl/styles.xml"));
        assert!(glob_match("**/document.xml", "word/document.xml"));
        assert!(glob_match("**/document.xml", "ppt/slides/document.xml"));
        assert!(glob_match("doc?.xml", "doc1.xml"));
        assert!(glob_match("doc?.xml", "docA.xml"));
        assert!(!glob_match("doc?.xml", "doc12.xml"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", ""));
    }

    #[test]
    fn test_taint_mark_serde() {
        let mark = TaintMark::new(
            true,
            Some("word/document.xml".to_string()),
            TaintKind::DocxText,
        );
        let json = serde_json::to_string(&mark).expect("serialize");
        let decoded: TaintMark = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(mark, decoded);

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["tainted"], true);
        assert_eq!(parsed["kind"], "docx_text");
        assert!(parsed["source_part"].is_string());
    }

    #[test]
    fn test_taint_kind_serde() {
        let json = serde_json::to_string(&TaintKind::DocxText).unwrap();
        assert_eq!(json, "\"docx_text\"");
        let json = serde_json::to_string(&TaintKind::PdfText).unwrap();
        assert_eq!(json, "\"pdf_text\"");
        let json = serde_json::to_string(&TaintKind::Unknown).unwrap();
        assert_eq!(json, "\"unknown\"");
    }

    #[test]
    fn test_taint_propagation() {
        // TaintMark serializes correctly with source_part
        let mark = TaintMark::new(
            true,
            Some("slide1.xml".to_string()),
            TaintKind::PdfText,
        );
        let json = serde_json::to_string(&mark).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["tainted"], true);
        assert_eq!(parsed["source_part"], "slide1.xml");
        assert_eq!(parsed["kind"], "pdf_text");

        // Untainted mark omits source_part
        let clean = TaintMark::new(false, None, TaintKind::Unknown);
        let json = serde_json::to_string(&clean).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["tainted"], false);
        assert!(parsed["source_part"].is_null());
    }

    #[test]
    fn test_response_envelope_serde() {
        let env = ResponseEnvelope {
            success: true,
            taint: Some(TaintMark::new(
                true,
                Some("word/document.xml".to_string()),
                TaintKind::DocxText,
            )),
            diagnostics: vec![Diagnostic::new(
                "GP-W-001",
                Severity::Warning,
                "macro detected",
            )],
            journal_id: Some("j-001".to_string()),
            fidelity_budget: FidelityBudget::new().with_confidence(0.9),
        };
        let json = serde_json::to_string(&env).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["success"], true);
        assert_eq!(parsed["taint"]["tainted"], true);
        assert_eq!(parsed["taint"]["kind"], "docx_text");
        assert_eq!(parsed["journal_id"], "j-001");
        assert_eq!(parsed["fidelity_budget"]["confidence"], 0.9);

        // Round-trip
        let decoded: ResponseEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(env, decoded);
    }

    #[test]
    fn test_response_envelope_builder() {
        let env = ResponseEnvelope::success()
            .with_taint(TaintMark::new(true, None, TaintKind::Unknown))
            .with_diagnostic(Diagnostic::new(
                "GP-W-001",
                Severity::Warning,
                "test",
            ))
            .with_journal_id("j-42")
            .with_fidelity_budget(FidelityBudget::new().with_confidence(0.5));

        assert!(env.success);
        assert!(env.taint.as_ref().unwrap().tainted);
        assert_eq!(env.diagnostics.len(), 1);
        assert_eq!(env.journal_id, Some("j-42".to_string()));
        assert_eq!(env.fidelity_budget.confidence, 0.5);
    }

    #[test]
    fn test_response_envelope_default() {
        let env = ResponseEnvelope::default();
        assert!(!env.success);
        assert!(env.taint.is_none());
        assert!(env.diagnostics.is_empty());
        assert!(env.journal_id.is_none());
    }

    #[test]
    fn test_empty_policy_evaluates_to_all_denied() {
        let policy = EditPolicy::default();
        let denied = policy.evaluate("any/part.xml", &[DeniedAction::DeletePart]).unwrap();
        assert_eq!(denied.len(), 1);
    }

    #[test]
    fn test_max_parts_modified_in_policy() {
        let policy = EditPolicy {
            allowed_parts: vec!["word/*.xml".to_string()],
            denied_actions: Vec::new(),
            max_parts_modified: Some(5),
        };
        assert_eq!(policy.max_parts_modified, Some(5));

        let json = serde_json::to_string(&policy).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["max_parts_modified"], 5);
    }
}
