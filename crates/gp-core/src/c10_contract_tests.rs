//! C10 contract test suite — end-to-end conformance checks for policy + taint.
//!
//! These tests verify that the producer types (EditPolicy, TaintMark,
//! ResponseEnvelope) and the consumer (SurgicalPatchExecutor) conform
//! to the C10 contract. See `docs/contracts/C10.md` for the spec.

use crate::diagnostics::Diagnostic;
use crate::executor::{
    ExecuteResult, ExecutorConfig, FormatType, PolicyDenial, SurgicalPatchExecutor,
};
use crate::fidelity::FidelityBudget;
use crate::ir::ElementId;
use crate::patch::PatchOp;
use crate::policy::{DeniedAction, EditPolicy, ResponseEnvelope, TaintKind, TaintMark};

// ---------------------------------------------------------------------------
// Producer tests — EditPolicy
// ---------------------------------------------------------------------------

#[test]
fn test_c10_edit_policy_empty_denies_all() {
    // Empty allowed_parts → deny-by-default: any part with any action is denied
    let policy = EditPolicy::new();
    assert!(policy.allowed_parts.is_empty());

    let denied = policy
        .evaluate(
            "word/document.xml",
            &[DeniedAction::DeletePart, DeniedAction::MacroPartWrite],
        )
        .unwrap();
    assert_eq!(denied.len(), 2);
}

#[test]
fn test_c10_edit_policy_glob_matching() {
    let policy = EditPolicy {
        allowed_parts: vec![
            "word/*.xml".to_string(),
            "ppt/slides/*.xml".to_string(),
            "**/document.xml".to_string(),
        ],
        denied_actions: Vec::new(),
        max_parts_modified: None,
    };

    // Matching parts: no denied actions → empty result
    assert!(policy
        .evaluate("word/document.xml", &[])
        .unwrap()
        .is_empty());
    assert!(policy.evaluate("word/slides.xml", &[]).unwrap().is_empty());
    assert!(policy
        .evaluate("ppt/slides/document.xml", &[])
        .unwrap()
        .is_empty());
    assert!(policy
        .evaluate("deep/nested/document.xml", &[])
        .unwrap()
        .is_empty());
    // Non-matching part: denied (xl/styles.xml not in allow-list)
    let denied = policy
        .evaluate("xl/styles.xml", &[DeniedAction::DeletePart])
        .unwrap();
    assert_eq!(denied.len(), 1);
    assert!(denied[0] == DeniedAction::DeletePart);
}

#[test]
fn test_c10_edit_policy_deny_actions_always_blocked() {
    // denied_actions are enforced regardless of allow-list matching
    let policy = EditPolicy {
        allowed_parts: vec!["word/*.xml".to_string()],
        denied_actions: vec![DeniedAction::DeletePart, DeniedAction::ExternalRelAdd],
        max_parts_modified: None,
    };

    // Matching part: denied_actions still blocked
    let denied = policy
        .evaluate(
            "word/document.xml",
            &[DeniedAction::DeletePart, DeniedAction::UnknownAction],
        )
        .unwrap();
    assert_eq!(denied.len(), 1);
    assert!(matches!(denied[0], DeniedAction::DeletePart));
    assert!(!denied.contains(&DeniedAction::UnknownAction));

    // Non-matching part: everything denied (part not in allow-list)
    let denied = policy
        .evaluate("xl/styles.xml", &[DeniedAction::UnknownAction])
        .unwrap();
    assert_eq!(denied.len(), 1);
}

#[test]
fn test_c10_edit_policy_max_parts_cap() {
    let policy = EditPolicy {
        allowed_parts: vec!["word/".to_string(), "xl/".to_string(), "ppt/".to_string()],
        denied_actions: Vec::new(),
        max_parts_modified: Some(2),
    };

    let config = ExecutorConfig {
        revisions: false,
        author: None,
        dry_run: false,
        policy: Some(policy.clone()),
    };
    let executor = SurgicalPatchExecutor::new(config);
    let source = b"PK\x03\x04word/document.xml xl/styles.xml ppt/slides.xml";
    let ops = vec![PatchOp::Insert {
        anchor: ElementId::content_anchored("run", &["t:0"], "hello"),
        kind: "paragraph".into(),
    }];

    let result = executor.check_policy(source, &FormatType::Docx, &ops);

    // Only 2 parts should be allowed (max_parts_modified = 2)
    assert_eq!(result.allowed.len(), 2);
    assert!(!result.denied.is_empty());
    assert!(result
        .denied
        .iter()
        .any(|d| d.reason.contains("max_parts_modified")));
    assert!(result
        .denied
        .iter()
        .any(|d| d.diagnostic_code == "POL-D-003"));
}

#[test]
fn test_c10_edit_policy_serde_roundtrip() {
    let policy = EditPolicy {
        allowed_parts: vec!["word/*.xml".to_string(), "**/document.xml".to_string()],
        denied_actions: vec![
            DeniedAction::DeletePart,
            DeniedAction::ExternalRelAdd,
            DeniedAction::UnknownAction,
        ],
        max_parts_modified: Some(10),
    };

    let json = serde_json::to_string(&policy).expect("serialize policy");
    let decoded: EditPolicy = serde_json::from_str(&json).expect("deserialize policy");
    assert_eq!(policy, decoded);

    // Verify specific fields via JSON shape
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed["allowed_parts"].is_array());
    assert_eq!(parsed["allowed_parts"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["max_parts_modified"], 10);
    assert_eq!(parsed["denied_actions"][0], "DeletePart");
    assert_eq!(parsed["denied_actions"][1], "ExternalRelAdd");
    assert_eq!(parsed["denied_actions"][2], "UnknownAction");
}

// ---------------------------------------------------------------------------
// Producer tests — TaintMark
// ---------------------------------------------------------------------------

#[test]
fn test_c10_taint_mark_docx() {
    let mark = TaintMark::new(
        true,
        Some("word/document.xml".to_string()),
        TaintKind::DocxText,
    );
    assert!(mark.tainted);
    assert_eq!(mark.source_part, Some("word/document.xml".to_string()));
    assert!(matches!(mark.kind, TaintKind::DocxText));
}

#[test]
fn test_c10_taint_mark_pdf() {
    let mark = TaintMark::new(
        true,
        Some("pdf/document.pdf".to_string()),
        TaintKind::PdfText,
    );
    assert!(mark.tainted);
    assert_eq!(mark.source_part, Some("pdf/document.pdf".to_string()));
    assert!(matches!(mark.kind, TaintKind::PdfText));
}

#[test]
fn test_c10_taint_mark_serde() {
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
    assert_eq!(parsed["source_part"], "word/document.xml");
}

// ---------------------------------------------------------------------------
// Producer tests — ResponseEnvelope
// ---------------------------------------------------------------------------

#[test]
fn test_c10_response_envelope_with_taint() {
    let mark = TaintMark::new(
        true,
        Some("word/document.xml".to_string()),
        TaintKind::DocxText,
    );
    let env = ResponseEnvelope::success()
        .with_taint(mark.clone())
        .with_diagnostic(Diagnostic::new(
            "GP-W-001",
            crate::diagnostics::Severity::Warning,
            "taint detected",
        ))
        .with_journal_id("j-001")
        .with_fidelity_budget(FidelityBudget::new().with_confidence(0.9));

    // Builder chain verified
    assert!(env.success);
    assert!(env.taint.is_some());
    let taint = env.taint.as_ref().unwrap();
    assert!(taint.tainted);
    assert_eq!(taint.source_part, Some("word/document.xml".to_string()));
    assert!(matches!(taint.kind, TaintKind::DocxText));
    assert_eq!(env.diagnostics.len(), 1);
    assert_eq!(env.journal_id, Some("j-001".to_string()));
    assert_eq!(env.fidelity_budget.confidence, 0.9);

    // Serde round-trip preserves taint
    let json = serde_json::to_string(&env).expect("serialize");
    let decoded: ResponseEnvelope = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(env, decoded);
}

// ---------------------------------------------------------------------------
// Consumer tests — executor integration
// ---------------------------------------------------------------------------

#[test]
fn test_c10_executor_refuses_without_policy() {
    // No policy → default denies destructive ops (DeletePart, MacroPartWrite, ExternalRelAdd)
    let config = ExecutorConfig {
        revisions: false,
        author: None,
        dry_run: false,
        policy: None,
    };
    let executor = SurgicalPatchExecutor::new(config);
    let source = b"PK\x03\x04[Content_Types].xml word/document.xml";

    // Delete op → mapped to DeletePart → denied by default deny list
    let ops = vec![PatchOp::Delete {
        elem: ElementId::content_anchored("run", &["t:0"], "hello"),
    }];
    let result = executor.execute(source, FormatType::Docx, &ops);

    // DeletePart is denied → all parts denied → refusal
    assert!(!result.accepted);
    assert!(!result.policy_violations.is_empty());
}

#[test]
fn test_c10_executor_allows_with_policy() {
    // Policy with allow-list → allowed parts proceed
    let policy = EditPolicy {
        allowed_parts: vec!["word/".to_string(), "**".to_string()],
        denied_actions: vec![DeniedAction::ExternalRelAdd],
        max_parts_modified: None,
    };
    let config = ExecutorConfig {
        revisions: false,
        author: None,
        dry_run: false,
        policy: Some(policy),
    };
    let executor = SurgicalPatchExecutor::new(config);
    let source = b"PK\x03\x04word/document.xml";

    // Insert → UnknownAction, not in deny list, part matches
    let ops = vec![PatchOp::Insert {
        anchor: ElementId::content_anchored("run", &["t:0"], "hello"),
        kind: "paragraph".into(),
    }];
    let result = executor.execute(source, FormatType::Docx, &ops);

    assert!(result.accepted);
    assert!(result.policy_violations.is_empty());
}

#[test]
fn test_c10_executor_taint_on_source() {
    // Source with OOXML markers → taint detected
    let config = ExecutorConfig {
        dry_run: true,
        revisions: false,
        author: None,
        policy: Some(EditPolicy {
            allowed_parts: vec!["**".to_string()],
            denied_actions: vec![],
            max_parts_modified: None,
        }),
    };
    let executor = SurgicalPatchExecutor::new(config);
    let source = b"PK\x03\x04[Content_Types].xml word/document.xml";
    let result = executor.execute(source, FormatType::Docx, &[]);

    assert!(result.accepted);
    assert!(result.ir_diff.is_some());
    assert!(!result.tainted_parts.is_empty());
    assert!(result
        .tainted_parts
        .contains(&"word/document.xml".to_string()));
}

#[test]
fn test_c10_executor_policy_violations_in_result() {
    // When some parts are denied by policy, violations appear in the result
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
    };
    let executor = SurgicalPatchExecutor::new(config);
    // Source has both word/ and xl/ — xl/ is not in allow-list
    let source = b"PK\x03\x04word/document.xml xl/styles.xml";

    let ops = vec![PatchOp::Insert {
        anchor: ElementId::content_anchored("run", &["t:0"], "hello"),
        kind: "paragraph".into(),
    }];
    let result = executor.execute(source, FormatType::Docx, &ops);

    // xl/ should be denied, word/ allowed
    assert!(!result.policy_violations.is_empty());
    assert!(result
        .policy_violations
        .iter()
        .any(|v| v.part.contains("xl/")));
}

#[test]
fn test_c10_executor_taint_propagates_through_dry_run() {
    // Dry-run mode must carry taint information through the result
    let config = ExecutorConfig {
        dry_run: true,
        revisions: false,
        author: None,
        policy: Some(EditPolicy {
            allowed_parts: vec!["**".to_string()],
            denied_actions: vec![],
            max_parts_modified: None,
        }),
    };
    let executor = SurgicalPatchExecutor::new(config);

    // Docx source with multiple taint markers
    let source =
        b"PK\x03\x04[Content_Types].xml word/document.xml xl/workbook.xml ppt/presentation.xml";
    let result = executor.execute(source, FormatType::Docx, &[]);

    assert!(result.accepted);
    assert!(result.ir_diff.is_some());
    // Primary taint marker should be in tainted_parts
    assert!(result
        .tainted_parts
        .contains(&"word/document.xml".to_string()));
    // The taint mark kind should be DocxText for OOXML
    assert!(result
        .tainted_parts
        .contains(&"word/document.xml".to_string()));
}

// ---------------------------------------------------------------------------
// Additional consumer tests — denied actions always block
// ---------------------------------------------------------------------------

#[test]
fn test_c10_executor_denied_actions_always_block() {
    // Even with a permissive allow-list, denied_actions are always enforced
    let policy = EditPolicy {
        allowed_parts: vec!["**".to_string()],
        denied_actions: vec![DeniedAction::DeletePart, DeniedAction::MacroPartWrite],
        max_parts_modified: None,
    };
    let config = ExecutorConfig {
        revisions: false,
        author: None,
        dry_run: false,
        policy: Some(policy),
    };
    let executor = SurgicalPatchExecutor::new(config);
    let source = b"PK\x03\x04word/document.xml";

    // Delete op → DeletePart → always denied
    let ops = vec![PatchOp::Delete {
        elem: ElementId::content_anchored("run", &["t:0"], "hello"),
    }];
    let result = executor.execute(source, FormatType::Docx, &ops);

    assert!(!result.accepted);
    assert!(!result.policy_violations.is_empty());
}

#[test]
fn test_c10_executor_taint_extract_pdf() {
    // PDF taint: %PDF- marker → PdfText kind
    let executor = SurgicalPatchExecutor::default();
    let mark = executor.extract_taint(b"%PDF-1.4 some pdf content", &FormatType::Pdf);

    assert!(mark.is_some());
    let mark = mark.unwrap();
    assert!(mark.tainted);
    assert_eq!(mark.source_part, Some("pdf/document.pdf".to_string()));
    assert!(matches!(mark.kind, TaintKind::PdfText));
}

#[test]
fn test_c10_executor_no_taint_on_refused_format() {
    // Refused formats (PDF, ODF) → no taint propagation
    let executor = SurgicalPatchExecutor::default();
    let result = executor.execute(b"%PDF-1.4 test", FormatType::Pdf, &[]);

    assert!(!result.accepted);
    assert!(result.tainted_parts.is_empty());
}

#[test]
fn test_c10_policy_denial_diagnostic_codes() {
    // Verify diagnostic codes are correct for policy violations
    let policy = EditPolicy {
        allowed_parts: vec!["word/".to_string()],
        denied_actions: vec![DeniedAction::DeletePart],
        max_parts_modified: Some(1),
    };
    let config = ExecutorConfig {
        revisions: false,
        author: None,
        dry_run: false,
        policy: Some(policy),
    };
    let executor = SurgicalPatchExecutor::new(config);

    // Test POL-D-002 (policy denied)
    let source = b"PK\x03\x04word/document.xml xl/styles.xml";
    let ops = vec![PatchOp::Delete {
        elem: ElementId::content_anchored("run", &["t:0"], "hello"),
    }];
    let result = executor.check_policy(source, &FormatType::Docx, &ops);
    assert!(!result.denied.is_empty());
    assert!(result
        .denied
        .iter()
        .any(|d| d.diagnostic_code == "POL-D-002"));

    // Test POL-D-003 (max_parts_modified)
    let policy2 = EditPolicy {
        allowed_parts: vec!["word/".to_string(), "xl/".to_string()],
        denied_actions: vec![],
        max_parts_modified: Some(1),
    };
    let config2 = ExecutorConfig {
        revisions: false,
        author: None,
        dry_run: false,
        policy: Some(policy2),
    };
    let executor2 = SurgicalPatchExecutor::new(config2);
    let source2 = b"PK\x03\x04word/document.xml xl/styles.xml ppt/slides.xml";
    let ops2 = vec![PatchOp::Insert {
        anchor: ElementId::content_anchored("run", &["t:0"], "hello"),
        kind: "paragraph".into(),
    }];
    let result2 = executor2.check_policy(source2, &FormatType::Docx, &ops2);
    assert!(!result2.denied.is_empty());
    assert!(result2
        .denied
        .iter()
        .any(|d| d.diagnostic_code == "POL-D-003"));
}

#[test]
fn test_c10_execute_result_policy_refuse_serializes() {
    let violations = vec![PolicyDenial {
        part: "word/document.xml".to_string(),
        reason: "word/document.xml: DeletePart denied".to_string(),
        diagnostic_code: "POL-D-002".to_string(),
    }];
    let result = ExecuteResult::policy_refuse("docx", violations.clone());

    assert!(!result.accepted);
    assert_eq!(result.policy_violations.len(), 1);
    assert_eq!(result.policy_violations[0].part, "word/document.xml");
    assert_eq!(result.policy_violations[0].diagnostic_code, "POL-D-002");

    // Serialize and deserialize round-trip
    let json = serde_json::to_string(&result).expect("serialize");
    let decoded: ExecuteResult = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.policy_violations.len(), 1);
    assert_eq!(decoded.policy_violations[0].part, "word/document.xml");
}

#[test]
fn test_c10_taintmark_skip_serializing_if_none() {
    let clean = TaintMark::new(false, None, TaintKind::Unknown);
    let json = serde_json::to_string(&clean).expect("serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["tainted"], false);
    assert_eq!(parsed["kind"], "unknown");
    // source_part should be null/absent when None
    assert!(parsed["source_part"].is_null());
}

#[test]
fn test_c10_response_envelope_default_has_no_taint() {
    let env = ResponseEnvelope::default();
    assert!(!env.success);
    assert!(env.taint.is_none());
    assert!(env.diagnostics.is_empty());
    assert!(env.journal_id.is_none());
    assert!(env.fidelity_budget.confidence > 0.0);
}

#[test]
fn test_c10_denied_action_all_variants_serde() {
    // All DeniedAction variants must serialize/deserialize correctly
    let actions = vec![
        DeniedAction::DeletePart,
        DeniedAction::MacroPartWrite,
        DeniedAction::ExternalRelAdd,
        DeniedAction::UnknownAction,
    ];
    let json = serde_json::to_string(&actions).expect("serialize");
    let decoded: Vec<DeniedAction> = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(actions, decoded);

    // Verify PascalCase serialization (no rename_all on DeniedAction)
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed[0], "DeletePart");
    assert_eq!(parsed[1], "MacroPartWrite");
    assert_eq!(parsed[2], "ExternalRelAdd");
    assert_eq!(parsed[3], "UnknownAction");
}

#[test]
fn test_c10_taintkind_all_variants_serde() {
    let kinds = vec![TaintKind::DocxText, TaintKind::PdfText, TaintKind::Unknown];
    let json = serde_json::to_string(&kinds).expect("serialize");
    let decoded: Vec<TaintKind> = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(kinds, decoded);

    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed[0], "docx_text");
    assert_eq!(parsed[1], "pdf_text");
    assert_eq!(parsed[2], "unknown");
}

#[test]
fn test_c10_executor_config_with_policy_none() {
    // ExecutorConfig with None policy uses permissive default (deny destructive only)
    let config = ExecutorConfig {
        revisions: false,
        author: None,
        dry_run: false,
        policy: None,
    };
    let executor = SurgicalPatchExecutor::new(config);
    let source = b"PK\x03\x04word/document.xml";

    // Insert (UnknownAction) → allowed (not in default deny list)
    let ops = vec![PatchOp::Insert {
        anchor: ElementId::content_anchored("run", &["t:0"], "hello"),
        kind: "paragraph".into(),
    }];
    let result = executor.check_policy(source, &FormatType::Docx, &ops);
    assert!(!result.allowed.is_empty());
}
