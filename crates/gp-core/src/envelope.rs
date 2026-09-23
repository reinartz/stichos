// SPDX-License-Identifier: MIT OR Apache-2.0
//! CTX-01 response envelope (P6-01; contracts C14 §4 + implementation guide §1).
//!
//! Every MCP/CLI tool response carries three envelope fields on top of its
//! tool-specific payload:
//!
//! ```json
//! {
//!   "...": "...",
//!   "__budget": { "tokens_est": 150, "elements_count": 100, "image_count": 0 },
//!   "diagnostics": [],
//!   "rendered_by": "stichos/0.1.0"
//! }
//! ```
//!
//! This module is the single home for the envelope shape so that Wave-C steps
//! (P6-02..P6-06) and the INV-2 schema gate share one definition instead of
//! re-declaring it per tool. The CLI-side `budget_enforcer` and `mcp/spec.rs`
//! wire-up belong to later Wave-C steps and are deliberately **not** touched
//! here; the gate function [`check_envelope_compliance`] is the gp-core core
//! they will call.
//!
//! # Token estimator (REN-02 reuse / calibration point)
//!
//! REN-02 (contract C8, implemented as `lint::budget::compute_visual_tokens`)
//! covers **visual** tokens: `ceil(w/28) * ceil(h/28)`. There is no existing
//! **text** token estimator anywhere in the workspace (`lint` and `gp-core`
//! were both searched for P6-01; only the visual estimator exists, and
//! `gp-core` cannot depend on `lint` without inverting the crate DAG).
//!
//! So this module provides:
//! - [`estimate_visual_tokens`]: a local re-statement of the REN-02 formula
//!   (same `TOKEN_GRID = 28` divisor; calibration parity is locked by
//!   [`self::tests::visual_estimator_matches_ren02_known_values`]), and
//! - [`estimate_text_tokens`]: the text-side companion heuristic —
//!   `ceil(chars / 4)` over Unicode scalar values (≈4 chars/token, the same
//!   rule the C14 §7.4 byte+token contract tests calibrate against).
//!
//! If a measured calibration pass later replaces the text heuristic, it lands
//! here, behind these function signatures.

use std::collections::BTreeMap;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::diagnostics::Diagnostic;

// ---------------------------------------------------------------------------
// RenderOutput (C14 §3.2/§3.3/§3.7)
// ---------------------------------------------------------------------------

/// How a tool returns rendered images.
///
/// Default is [`RenderOutput::Urls`]: images go to the temp-file store and the
/// response carries paths/URLs, never inline base64.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderOutput {
    /// Inline base64 image blocks (pre-C14 behavior; explicit opt-in).
    Base64,
    /// Temp-file URLs/paths with `expiresAt` (default).
    #[default]
    Urls,
    /// Both URLs and inline base64 for the same pages.
    Both,
}

impl std::fmt::Display for RenderOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderOutput::Base64 => write!(f, "base64"),
            RenderOutput::Urls => write!(f, "urls"),
            RenderOutput::Both => write!(f, "both"),
        }
    }
}

impl FromStr for RenderOutput {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "base64" => Ok(RenderOutput::Base64),
            "urls" => Ok(RenderOutput::Urls),
            "both" => Ok(RenderOutput::Both),
            other => Err(format!(
                "unknown render output mode '{}' (expected base64|urls|both)",
                other
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// InspectDetail (C14 §3.1)
// ---------------------------------------------------------------------------

/// How much detail `inspect_document` returns.
///
/// Default is [`InspectDetail::Summary`] (~150 tokens): page stats, element
/// counts, compact ID list. Full detail is always an explicit opt-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectDetail {
    /// Summary only: counts + IDs.
    #[default]
    Summary,
    /// Up to `max_elements` full element records.
    Elements,
    /// Full coordinates, styles, per-element fidelity budget (legacy output).
    All,
}

impl std::fmt::Display for InspectDetail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InspectDetail::Summary => write!(f, "summary"),
            InspectDetail::Elements => write!(f, "elements"),
            InspectDetail::All => write!(f, "all"),
        }
    }
}

impl FromStr for InspectDetail {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "summary" => Ok(InspectDetail::Summary),
            "elements" => Ok(InspectDetail::Elements),
            "all" => Ok(InspectDetail::All),
            other => Err(format!(
                "unknown inspect detail '{}' (expected summary|elements|all)",
                other
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// ResponseBudget (`__budget`, C14 §4)
// ---------------------------------------------------------------------------

/// Token-budget metadata present on **every** tool response as `__budget`.
///
/// The three core counters are always serialized. Tool-specific counters
/// (`operations_returned`/`operations_total` for capped diffs, per C14 §3.4;
/// `operations_count` for mutation previews, per C14 §3.5) ride along as
/// optional fields so the envelope stays one struct across all tools.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResponseBudget {
    /// Estimated token cost of this response (text heuristic + visual REN-02).
    pub tokens_est: usize,
    /// Number of elements / data items returned.
    pub elements_count: usize,
    /// Number of images referenced or embedded by this response.
    pub image_count: usize,
    /// Capped-list companion (C14 §3.4): how many ops are in this page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operations_returned: Option<usize>,
    /// Capped-list companion (C14 §3.4): how many ops exist in total.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operations_total: Option<usize>,
    /// Mutation-preview companion (C14 §3.5/§3.6): ops in the preview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operations_count: Option<usize>,
}

impl ResponseBudget {
    /// Build a budget with explicit counters.
    pub fn new(tokens_est: usize, elements_count: usize, image_count: usize) -> Self {
        Self {
            tokens_est,
            elements_count,
            image_count,
            operations_returned: None,
            operations_total: None,
            operations_count: None,
        }
    }

    /// Budget for a text-only response: tokens estimated from `text`.
    pub fn for_text(text: &str, elements_count: usize) -> Self {
        Self::new(estimate_text_tokens(text), elements_count, 0)
    }

    /// Budget for an image response: `tokens_est` is pure REN-02 visual cost.
    pub fn for_images(image_count: usize, visual_tokens: usize) -> Self {
        Self::new(visual_tokens, 0, image_count)
    }

    /// Attach capped-diff counters (C14 §3.4).
    pub fn with_operations_page(mut self, returned: usize, total: usize) -> Self {
        self.operations_returned = Some(returned);
        self.operations_total = Some(total);
        self
    }

    /// Attach the mutation-preview counter (C14 §3.5/§3.6).
    pub fn with_operations_count(mut self, count: usize) -> Self {
        self.operations_count = Some(count);
        self
    }
}

// ---------------------------------------------------------------------------
// PreviewSummary (shared preview shape, C14 §3.5/§3.6)
// ---------------------------------------------------------------------------

/// Maximum operations carried in any mutation preview (C14 §3.5).
pub const PREVIEW_MAX_OPS: usize = 20;

/// Shared preview shape for `dry_run: true` mutation responses.
///
/// Both `apply_patch` (C14 §3.5) and `edit_text`/`edit_cell`/`edit_structure`
/// (C14 §3.6) return this shape; tool-specific extras (e.g. `elementId`,
/// `oldText`/`newText`) stay in the tool payload, never here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewSummary {
    /// Human-readable token delta, e.g. `"+450"`.
    pub estimated_token_change: String,
    /// How many elements the full op list would affect.
    pub elements_affected: usize,
    /// One-liner for the LLM reasoning turn, e.g. `"Changed 2 paragraphs"`.
    pub diff_summary: String,
    /// At most [`PREVIEW_MAX_OPS`] operations (truncated on construction).
    pub operations: Vec<serde_json::Value>,
    /// True when ops were truncated.
    pub has_more: bool,
    /// Full op count before truncation.
    pub total_operations: usize,
}

impl PreviewSummary {
    /// Build a preview, truncating `operations` to [`PREVIEW_MAX_OPS`].
    pub fn new(
        estimated_token_change: impl Into<String>,
        elements_affected: usize,
        diff_summary: impl Into<String>,
        operations: Vec<serde_json::Value>,
    ) -> Self {
        let total_operations = operations.len();
        let has_more = total_operations > PREVIEW_MAX_OPS;
        let operations = operations.into_iter().take(PREVIEW_MAX_OPS).collect();
        Self {
            estimated_token_change: estimated_token_change.into(),
            elements_affected,
            diff_summary: diff_summary.into(),
            operations,
            has_more,
            total_operations,
        }
    }
}

// ---------------------------------------------------------------------------
// Token estimators (REN-02 reuse / calibration point — see module docs)
// ---------------------------------------------------------------------------

/// Chars-per-token divisor for the text heuristic (≈4 chars/token).
pub const CHARS_PER_TOKEN: usize = 4;

/// REN-02 visual-token grid divisor (contract C8; mirrors
/// `lint::budget::TOKEN_GRID`). Kept as a named constant here so the parity
/// with `lint` stays greppable from `gp-core`.
pub const TOKEN_GRID: usize = 28;

/// Estimate text tokens as `ceil(chars / 4)` over Unicode scalar values.
///
/// Empty text costs 0. This is a heuristic for budget metadata, not a model
/// tokenizer; C14 §7.4 contract tests bound summary size in bytes as well as
/// tokens so estimator drift is caught there (concept §8 risk table).
pub fn estimate_text_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    chars.div_ceil(CHARS_PER_TOKEN)
}

/// Estimate tokens for a compact-serialized JSON value.
pub fn estimate_json_tokens(value: &serde_json::Value) -> usize {
    estimate_text_tokens(&serde_json::to_string(value).unwrap_or_default())
}

/// Estimate visual tokens per REN-02: `ceil(w/28) * ceil(h/28)`.
///
/// Local re-statement of `lint::budget::compute_visual_tokens` (which
/// `gp-core` must not depend on — crate DAG). Parity is locked by unit test
/// against the published REN-02 known values below.
pub fn estimate_visual_tokens(width_px: usize, height_px: usize) -> usize {
    if width_px == 0 || height_px == 0 {
        return 0;
    }
    let tw = (width_px as f64 / TOKEN_GRID as f64).ceil() as usize;
    let th = (height_px as f64 / TOKEN_GRID as f64).ceil() as usize;
    tw * th
}

/// Default `rendered_by` value: `stichos/<crate-version>`.
pub fn default_rendered_by() -> String {
    format!("stichos/{}", env!("CARGO_PKG_VERSION"))
}

// ---------------------------------------------------------------------------
// Envelope construction + INV-2 schema gate (CTX-01 extension)
// ---------------------------------------------------------------------------

/// Typed envelope for handlers that want it: tool payload plus the three
/// mandatory C14 §4 fields.
///
/// `budget` serializes as the reserved `__budget` key so it cannot collide
/// with future tool fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvelopedResponse<T: Serialize> {
    /// Tool-specific payload.
    #[serde(flatten)]
    pub inner: T,
    /// Budget metadata (serialized as `__budget`).
    #[serde(rename = "__budget")]
    pub budget: ResponseBudget,
    /// C3-coded diagnostics.
    pub diagnostics: Vec<Diagnostic>,
    /// Engine identifier + version.
    pub rendered_by: String,
}

impl<T: Serialize> EnvelopedResponse<T> {
    /// Wrap a payload with budget, diagnostics, and provenance.
    pub fn new(
        inner: T,
        budget: ResponseBudget,
        diagnostics: Vec<Diagnostic>,
        rendered_by: impl Into<String>,
    ) -> Self {
        Self {
            inner,
            budget,
            diagnostics,
            rendered_by: rendered_by.into(),
        }
    }
}

/// Merge the envelope fields into a JSON tool response object.
///
/// Handlers build their tool-specific `inner` object, then call this to stamp
/// `__budget` + `diagnostics` (created empty when `None`) + `rendered_by`
/// onto it. Returns the object on success, or a `String` error when `inner`
/// is not a JSON object (a programming error in the handler).
pub fn envelope_json(
    mut inner: serde_json::Value,
    budget: &ResponseBudget,
    diagnostics: Option<Vec<Diagnostic>>,
    rendered_by: &str,
) -> Result<serde_json::Value, String> {
    let obj = inner
        .as_object_mut()
        .ok_or_else(|| "envelope_json: tool payload must be a JSON object".to_string())?;
    obj.insert(
        "__budget".to_string(),
        serde_json::to_value(budget)
            .map_err(|e| format!("envelope_json: budget serialize failed: {}", e))?,
    );
    if obj.get("diagnostics").is_none() {
        obj.insert(
            "diagnostics".to_string(),
            serde_json::to_value(diagnostics.unwrap_or_default())
                .map_err(|e| format!("envelope_json: diagnostics serialize failed: {}", e))?,
        );
    }
    obj.insert(
        "rendered_by".to_string(),
        serde_json::Value::String(rendered_by.to_string()),
    );
    Ok(inner)
}

/// INV-2 schema gate, CTX-01 extension: require `__budget` + `diagnostics` +
/// `rendered_by` on every tool response value.
///
/// Checks (all on the raw `serde_json::Value`, so one gate covers CLI and
/// MCP responses alike):
/// - `__budget` is an object with a numeric `tokens_est`;
/// - `diagnostics` is an array;
/// - `rendered_by` is a non-empty string.
///
/// Notes for the later wiring steps (P6-02..P6-06, which own
/// `cli/src/budget_enforcer.rs` and `mcp/spec.rs` and will call this):
/// the legacy CLI `visual_feedback`-as-provenance escape hatch is
/// deliberately **not** accepted here — C14 §4 names `rendered_by`, and the
/// concept §5 `__budget` ubiquity promise is "100 % of tool responses carry
/// `__budget` + `diagnostics` + `rendered_by`".
pub fn check_envelope_compliance(response: &serde_json::Value) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    match response.get("__budget") {
        Some(b) if b.is_object() => {
            let tokens_ok = b
                .get("tokens_est")
                .is_some_and(|v| v.is_u64() || v.is_i64());
            if !tokens_ok {
                errors.push("missing or non-numeric __budget.tokens_est (CTX-01)".to_string());
            }
        }
        _ => errors.push("missing __budget object (CTX-01)".to_string()),
    }

    if !response.get("diagnostics").is_some_and(|v| v.is_array()) {
        errors.push("missing diagnostics array (C3)".to_string());
    }

    if !response
        .get("rendered_by")
        .is_some_and(|v| v.as_str().is_some_and(|s| !s.is_empty()))
    {
        errors.push("missing rendered_by string (INV-2)".to_string());
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Validate a batch of named tool responses, returning per-tool errors.
///
/// Convenience for the Wave-C gate work: maps each response through
/// [`check_envelope_compliance`] and collects failures without short-circuit.
pub fn check_all_envelopes<'a>(
    responses: impl IntoIterator<Item = (&'a str, &'a serde_json::Value)>,
) -> BTreeMap<String, Vec<String>> {
    let mut failures = BTreeMap::new();
    for (tool, response) in responses {
        if let Err(errs) = check_envelope_compliance(response) {
            failures.insert(tool.to_string(), errs);
        }
    }
    failures
}

// ---------------------------------------------------------------------------
// Tests (P6-01 verify: round-trip, estimator, gate negative test)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Severity;

    fn sample_diag() -> Diagnostic {
        Diagnostic::new("GP-E-001", Severity::Info, "sample")
    }

    #[test]
    fn render_output_serde_roundtrip() {
        for (mode, wire) in [
            (RenderOutput::Base64, "\"base64\""),
            (RenderOutput::Urls, "\"urls\""),
            (RenderOutput::Both, "\"both\""),
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(json, wire, "wire form for {:?}", mode);
            let back: RenderOutput = serde_json::from_str(&json).unwrap();
            assert_eq!(back, mode);
            assert_eq!(mode.to_string(), wire.trim_matches('"'));
            assert_eq!(
                wire.trim_matches('"').parse::<RenderOutput>().unwrap(),
                mode
            );
        }
        assert_eq!(RenderOutput::default(), RenderOutput::Urls);
        assert!("inline".parse::<RenderOutput>().is_err());
    }

    #[test]
    fn inspect_detail_serde_roundtrip() {
        for (detail, wire) in [
            (InspectDetail::Summary, "\"summary\""),
            (InspectDetail::Elements, "\"elements\""),
            (InspectDetail::All, "\"all\""),
        ] {
            let json = serde_json::to_string(&detail).unwrap();
            assert_eq!(json, wire, "wire form for {:?}", detail);
            let back: InspectDetail = serde_json::from_str(&json).unwrap();
            assert_eq!(back, detail);
            assert_eq!(detail.to_string(), wire.trim_matches('"'));
            assert_eq!(
                wire.trim_matches('"').parse::<InspectDetail>().unwrap(),
                detail
            );
        }
        assert_eq!(InspectDetail::default(), InspectDetail::Summary);
        assert!("full".parse::<InspectDetail>().is_err());
    }

    #[test]
    fn response_budget_serde_roundtrip() {
        // Core counters only: optional companions are skipped on the wire.
        let bare = ResponseBudget::new(150, 100, 0);
        let json = serde_json::to_string(&bare).unwrap();
        assert!(json.contains("\"tokens_est\":150"));
        assert!(!json.contains("operations_returned"));
        let back: ResponseBudget = serde_json::from_str(&json).unwrap();
        assert_eq!(back, bare);

        // Capped-diff companions (C14 §3.4) survive the round-trip.
        let paged = ResponseBudget::new(1200, 20, 0).with_operations_page(20, 523);
        let json = serde_json::to_string(&paged).unwrap();
        assert!(json.contains("\"operations_returned\":20"));
        assert!(json.contains("\"operations_total\":523"));
        let back: ResponseBudget = serde_json::from_str(&json).unwrap();
        assert_eq!(back, paged);

        // Mutation-preview companion (C14 §3.5).
        let preview = ResponseBudget::new(300, 3, 0).with_operations_count(3);
        let back: ResponseBudget =
            serde_json::from_str(&serde_json::to_string(&preview).unwrap()).unwrap();
        assert_eq!(back, preview);

        // Constructors.
        assert_eq!(ResponseBudget::for_text("abcd", 7).tokens_est, 1);
        assert_eq!(ResponseBudget::for_images(2, 484).image_count, 2);
        assert_eq!(ResponseBudget::for_images(2, 484).tokens_est, 484);
    }

    #[test]
    fn enveloped_response_wire_shape() {
        let inner = serde_json::json!({"summary": {"pageCount": 1}});
        let env = EnvelopedResponse::new(
            inner,
            ResponseBudget::new(150, 100, 0),
            vec![sample_diag()],
            "stichos/0.1.0",
        );
        let json = serde_json::to_string(&env).unwrap();
        assert!(
            json.contains("\"__budget\""),
            "reserved key present: {}",
            json
        );
        assert!(json.contains("\"tokens_est\":150"));
        assert!(json.contains("\"diagnostics\""));
        assert!(json.contains("\"rendered_by\":\"stichos/0.1.0\""));
        assert!(json.contains("\"summary\""));
        let back: EnvelopedResponse<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.budget.tokens_est, 150);
        assert_eq!(back.rendered_by, "stichos/0.1.0");
    }

    #[test]
    fn envelope_json_stamps_fields() {
        let inner = serde_json::json!({"tool": "inspect_document"});
        let out = envelope_json(
            inner,
            &ResponseBudget::new(150, 100, 0),
            Some(vec![sample_diag()]),
            "stichos/0.1.0",
        )
        .unwrap();
        assert!(check_envelope_compliance(&out).is_ok());

        // Diagnostics default to [] when not supplied.
        let out = envelope_json(
            serde_json::json!({"tool": "doctor"}),
            &ResponseBudget::new(10, 0, 0),
            None,
            "stichos/0.1.0",
        )
        .unwrap();
        assert_eq!(out["diagnostics"], serde_json::json!([]));

        // Non-object payloads are a handler bug: explicit error.
        assert!(envelope_json(
            serde_json::json!([1, 2, 3]),
            &ResponseBudget::new(0, 0, 0),
            None,
            "stichos/0.1.0",
        )
        .is_err());
    }

    #[test]
    fn text_estimator_units() {
        assert_eq!(estimate_text_tokens(""), 0);
        assert_eq!(estimate_text_tokens("a"), 1);
        assert_eq!(estimate_text_tokens("abcd"), 1);
        assert_eq!(estimate_text_tokens("abcde"), 2);
        // C14 §3.1 summary shape (~150 tokens ⇒ ~600 chars of JSON).
        assert_eq!(estimate_text_tokens(&"x".repeat(600)), 150);
        // Unicode counts scalar values, not bytes.
        assert_eq!(estimate_text_tokens("éééé"), 1);
        // JSON estimator tracks compact serialization.
        assert_eq!(
            estimate_json_tokens(&serde_json::json!({"a": "abcd"})),
            estimate_text_tokens(r#"{"a":"abcd"}"#)
        );
        // Budget constructors reuse the estimator.
        assert_eq!(
            ResponseBudget::for_text("hello world", 1).tokens_est,
            estimate_text_tokens("hello world")
        );
    }

    #[test]
    fn visual_estimator_matches_ren02_known_values() {
        // REN-02 known values from lint (C8): ceil(w/28)*ceil(h/28).
        assert_eq!(estimate_visual_tokens(0, 0), 0);
        assert_eq!(estimate_visual_tokens(0, 100), 0);
        assert_eq!(estimate_visual_tokens(100, 0), 0);
        assert_eq!(estimate_visual_tokens(28, 28), 1);
        assert_eq!(estimate_visual_tokens(29, 28), 2);
        assert_eq!(estimate_visual_tokens(56, 56), 4);
        assert_eq!(estimate_visual_tokens(100, 100), 16);
        // Cropper reference: 300px box ⇒ 11*11 = 121; 2x zoom ⇒ 22*22 = 484.
        assert_eq!(estimate_visual_tokens(300, 300), 121);
        assert_eq!(estimate_visual_tokens(600, 600), 484);
    }

    #[test]
    fn gate_negative_unenveloped_fixture_fails() {
        // Deliberately unenveloped legacy tool response: must FAIL the gate.
        let legacy = serde_json::json!({
            "tool": "inspect_document",
            "path": "test.docx",
            "structure": {},
            "element_ids": []
        });
        let errors = check_envelope_compliance(&legacy).unwrap_err();
        assert_eq!(errors.len(), 3, "all three fields missing: {:?}", errors);
        assert!(errors.iter().any(|e| e.contains("__budget")));
        assert!(errors.iter().any(|e| e.contains("diagnostics")));
        assert!(errors.iter().any(|e| e.contains("rendered_by")));
    }

    #[test]
    fn gate_partial_envelopes_fail() {
        // __budget without tokens_est.
        let bad_budget = serde_json::json!({
            "__budget": {"elements_count": 1},
            "diagnostics": [],
            "rendered_by": "stichos/0.1.0"
        });
        let errors = check_envelope_compliance(&bad_budget).unwrap_err();
        assert!(errors.iter().any(|e| e.contains("tokens_est")));

        // diagnostics not an array.
        let bad_diags = serde_json::json!({
            "__budget": {"tokens_est": 1, "elements_count": 0, "image_count": 0},
            "diagnostics": {},
            "rendered_by": "stichos/0.1.0"
        });
        let errors = check_envelope_compliance(&bad_diags).unwrap_err();
        assert!(errors.iter().any(|e| e.contains("diagnostics")));

        // Empty rendered_by.
        let bad_provenance = serde_json::json!({
            "__budget": {"tokens_est": 1, "elements_count": 0, "image_count": 0},
            "diagnostics": [],
            "rendered_by": ""
        });
        let errors = check_envelope_compliance(&bad_provenance).unwrap_err();
        assert!(errors.iter().any(|e| e.contains("rendered_by")));
    }

    #[test]
    fn gate_enveloped_response_passes() {
        let ok = envelope_json(
            serde_json::json!({"tool": "render_document"}),
            &ResponseBudget::new(0, 0, 2),
            None,
            &default_rendered_by(),
        )
        .unwrap();
        assert!(check_envelope_compliance(&ok).is_ok());
        assert!(ok["rendered_by"].as_str().unwrap().starts_with("stichos/"));

        // Batch helper: one failure collected, passes omitted.
        let bad = serde_json::json!({"tool": "doctor"});
        let failures = check_all_envelopes([("render_document", &ok), ("doctor", &bad)]);
        assert_eq!(failures.len(), 1);
        assert!(failures.contains_key("doctor"));
    }

    #[test]
    fn preview_summary_caps_at_20_ops() {
        let ops: Vec<serde_json::Value> = (0..25)
            .map(|i| serde_json::json!({"op": "replace", "path": format!("/body/para[{}]", i)}))
            .collect();
        let preview = PreviewSummary::new("+450", 25, "Changed 25 paragraphs", ops);
        assert_eq!(preview.operations.len(), PREVIEW_MAX_OPS);
        assert!(preview.has_more);
        assert_eq!(preview.total_operations, 25);
        let json = serde_json::to_string(&preview).unwrap();
        let back: PreviewSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back, preview);

        let small = PreviewSummary::new(
            "+50",
            1,
            "Changed 1 paragraph",
            vec![serde_json::json!({"op": "replace"})],
        );
        assert!(!small.has_more);
        assert_eq!(small.total_operations, 1);
    }
}
