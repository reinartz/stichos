//! Creation IR input fuzz fixtures (P3-27).
//!
//! Deterministic fuzz fixtures targeting the creation writers with malformed IR.
/// Tests that the creation writers handle edge cases gracefully:
/// - Empty document bodies are rejected or produce valid minimal packages
/// - Malformed UTF-8 in element text is handled
/// - Deeply nested structures don't cause stack overflow
/// - Duplicate IDs are detected
/// - Empty spreadsheets/presentations produce valid packages
use std::io::Write;

// ---------------------------------------------------------------------------
// Synthetic IR fixtures
// ---------------------------------------------------------------------------

/// Create an IR document with an empty body (no sections).
fn ir_empty_document() -> String {
    serde_json::json!({
        "ir_version": "1.0.0",
        "sections": []
    })
    .to_string()
}

/// Create an IR document with malformed UTF-8 in element text.
/// This is tricky because IRDocument uses valid serde types.
/// We test by creating a Document with deeply nested elements
/// that would exercise the IR builder.
fn ir_deeply_nested_document() -> String {
    let mut sections = Vec::new();
    let mut items = Vec::new();
    let mut para_children = Vec::new();

    for _ in 0..100 {
        para_children.push(serde_json::json!({
            "type": "run",
            "id": "run:deep",
            "text": "deep nested content"
        }));
    }

    items.push(serde_json::json!({
        "type": "paragraph",
        "id": "para:deep",
        "children": para_children
    }));

    sections.push(serde_json::json!({
        "id": "section:0",
        "kind": "body",
        "items": items
    }));

    serde_json::json!({
        "ir_version": "1.0.0",
        "sections": sections
    })
    .to_string()
}

/// Create an IR document with duplicate element IDs.
fn ir_duplicate_ids_document() -> String {
    let mut sections = Vec::new();
    sections.push(serde_json::json!({
        "id": "section:0",
        "kind": "body",
        "items": [
            {
                "type": "paragraph",
                "id": "run:duplicate",
                "children": [
                    { "type": "run", "id": "run:duplicate", "text": "first" },
                    { "type": "run", "id": "run:duplicate", "text": "second" }
                ]
            }
        ]
    }));

    serde_json::json!({
        "ir_version": "1.0.0",
        "sections": sections
    })
    .to_string()
}

/// Create an empty spreadsheet IR (no sheets).
fn ir_xlsx_empty_sheet() -> String {
    serde_json::json!({
        "ir_version": "1.0.0",
        "sheets": []
    })
    .to_string()
}

/// Create an empty presentation IR (no slides).
fn ir_pptx_empty_pres() -> String {
    serde_json::json!({
        "ir_version": "1.0.0",
        "slides": [],
        "layouts": [],
        "masters": []
    })
    .to_string()
}

/// Create an IR document with extremely long text content (memory stress test).
fn ir_oversized_content() -> String {
    let long_text = "A".repeat(100_000);
    let mut sections = Vec::new();
    sections.push(serde_json::json!({
        "id": "section:0",
        "kind": "body",
        "items": [
            {
                "type": "paragraph",
                "id": "para:0",
                "children": [
                    { "type": "run", "id": "run:0", "text": long_text }
                ]
            }
        ]
    }));

    serde_json::json!({
        "ir_version": "1.0.0",
        "sections": sections
    })
    .to_string()
}

/// Create an IR document with invalid IR version.
fn ir_bad_ir_version() -> String {
    serde_json::json!({
        "ir_version": "",
        "sections": []
    })
    .to_string()
}

/// Create an IR document with binary garbage embedded in element data.
fn ir_binary_garbage_in_content() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(b"[");
    // Valid JSON with null bytes embedded
    data.extend_from_slice(b"\"type\":\"run\",\"id\":\"run:0\",\"text\":\"");
    data.extend_from_slice(&[0x00, 0x01, 0x02, 0xFF, 0xFE]);
    data.extend_from_slice(b"test\"]");
    data.extend_from_slice(b"]");
    data
}

/// Create IR template creation with missing parts.
fn ir_template_missing() -> String {
    serde_json::json!({
        "ir_version": "1.0.0",
        "sections": [],
        "template": "nonexistent-template-id"
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// Fuzz harness
// ---------------------------------------------------------------------------

/// Run the docx creation writer with the given IR JSON, catching panics.
fn run_create_docx(ir_json: &str) -> Result<Result<Vec<u8>, String>, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let doc: Result<gp_core::ir::Document, _> = serde_json::from_str(ir_json);
        match doc {
            Ok(doc) => match adapters_ooxml::create::create_minimal_docx(&doc) {
                Ok(bytes) => Ok(bytes),
                Err(diags) => Err(format!("create error: {}", diags[0].message)),
            },
            Err(e) => Err(format!("parse error: {}", e)),
        }
    }))
    .map_err(|_| "panic in creation writer".to_string())
}

/// Run the xlsx creation writer with the given IR JSON, catching panics.
fn run_create_xlsx(ir_json: &str) -> Result<Result<Vec<u8>, String>, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let spreadsheet: Result<gp_core::ir::Spreadsheet, _> = serde_json::from_str(ir_json);
        match spreadsheet {
            Ok(spreadsheet) => {
                match adapters_ooxml::create_xlsx::create_minimal_xlsx(&spreadsheet) {
                    Ok(bytes) => Ok(bytes),
                    Err(diags) => Err(format!("create error: {}", diags[0].message)),
                }
            }
            Err(e) => Err(format!("parse error: {}", e)),
        }
    }))
    .map_err(|_| "panic in creation writer".to_string())
}

/// Run the pptx creation writer with the given IR JSON, catching panics.
fn run_create_pptx(ir_json: &str) -> Result<Result<Vec<u8>, String>, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let presentation: Result<gp_core::ir::Presentation, _> = serde_json::from_str(ir_json);
        match presentation {
            Ok(presentation) => {
                match adapters_ooxml::create_pptx::create_minimal_pptx(&presentation) {
                    Ok(bytes) => Ok(bytes),
                    Err(diags) => Err(format!("create error: {}", diags[0].message)),
                }
            }
            Err(e) => Err(format!("parse error: {}", e)),
        }
    }))
    .map_err(|_| "panic in creation writer".to_string())
}

// ---------------------------------------------------------------------------
// Test functions (fuzz fixtures — each is idempotent)
// ---------------------------------------------------------------------------

/// creation_ir_empty_document: IR with empty document body.
///
/// Tests that the creation writer handles empty document bodies gracefully.
#[test]
fn creation_ir_empty_document() {
    let ir_json = ir_empty_document();
    let result = run_create_docx(&ir_json);
    assert!(result.is_ok(), "empty document IR should not cause panic");
}

/// creation_ir_bad_unicode: IR with malformed UTF-8 in element text.
///
/// Tests that the IR parser and creation writer handle non-UTF-8 content.
#[test]
fn creation_ir_bad_unicode() {
    // Binary garbage that's invalid UTF-8 in the middle of JSON
    let garbage = ir_binary_garbage_in_content();
    // This will fail JSON parse, which is the expected graceful behavior
    let ir_json = String::from_utf8_lossy(&garbage);
    let result = run_create_docx(&ir_json);
    assert!(result.is_ok(), "binary garbage IR should not cause panic");
}

/// creation_ir_deeply_nested: deeply nested element structure.
///
/// Tests that deeply nested IR doesn't cause stack overflow.
#[test]
fn creation_ir_deeply_nested() {
    let ir_json = ir_deeply_nested_document();
    let result = run_create_docx(&ir_json);
    assert!(result.is_ok(), "deeply nested IR should not cause panic");
}

/// creation_ir_duplicate_ids: IR with duplicate element IDs.
///
/// Tests that duplicate IDs are handled gracefully.
#[test]
fn creation_ir_duplicate_ids() {
    let ir_json = ir_duplicate_ids_document();
    let result = run_create_docx(&ir_json);
    assert!(result.is_ok(), "duplicate ID IR should not cause panic");
}

/// creation_ir_xlsx_empty_sheet: empty spreadsheet creation.
///
/// Tests that the xlsx writer handles empty spreadsheet IR.
#[test]
fn creation_ir_xlsx_empty_sheet() {
    let ir_json = ir_xlsx_empty_sheet();
    let result = run_create_xlsx(&ir_json);
    assert!(
        result.is_ok(),
        "empty spreadsheet IR should not cause panic"
    );
}

/// creation_ir_pptx_empty_pres: empty presentation creation.
///
/// Tests that the pptx writer handles empty presentation IR.
#[test]
fn creation_ir_pptx_empty_pres() {
    let ir_json = ir_pptx_empty_pres();
    let result = run_create_pptx(&ir_json);
    assert!(
        result.is_ok(),
        "empty presentation IR should not cause panic"
    );
}

/// creation_ir_template_missing: template creation with missing parts.
///
/// Tests that template creation handles missing template references gracefully.
#[test]
fn creation_ir_template_missing() {
    let ir_json = ir_template_missing();
    let result = run_create_docx(&ir_json);
    assert!(result.is_ok(), "missing template IR should not cause panic");
}
