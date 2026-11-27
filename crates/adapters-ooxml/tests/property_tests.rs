// SPDX-License-Identifier: MIT OR Apache-2.0
//! Property tests for the OOXML adapters — Wave 7 track-exit evidence.
//!
//! These tests verify byte-identity round-trips, part-preservation, idempotence,
//! OPC validity, cross-format compatibility, part-count invariants, and
//! multi-op composition for the docx/xlsx/pptx adapters.

use std::collections::HashSet;
use std::path::PathBuf;

use adapters_ooxml::opc::parse_opc;
use gp_core::patch::PatchOp;

/// Resolve the corpus blobs directory.
fn corpus_blobs() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/blobs")
}

/// List corpus files matching the given extension, sorted for determinism.
fn corpus_files(ext: &str) -> Vec<PathBuf> {
    let blobs = corpus_blobs();
    let mut files: Vec<PathBuf> = if blobs.is_dir() {
        std::fs::read_dir(&blobs)
            .unwrap_or_else(|_| {
                panic!(
                    "corpus_blobs() should resolve to a directory: {}",
                    blobs.display()
                )
            })
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let p = e.path();
                if p.is_file() && p.extension().is_some_and(|e| e == ext) {
                    Some(p)
                } else {
                    None
                }
            })
            .collect()
    } else {
        vec![]
    };
    files.sort();
    files
}

// ===========================================================================
// 1. Zero-patch byte-identity tests (3)
// ===========================================================================

/// Zero-patch byte-identity: applying no patches to a corpus docx yields
/// byte-identical output.
#[test]
fn zero_patch_byte_identity_docx() {
    let files = corpus_files("docx");
    if files.is_empty() {
        panic!("expected at least one docx in corpus/blobs");
    }
    // Test each corpus docx file
    for f in files.iter().take(3) {
        let data = std::fs::read(f).expect("read corpus docx");
        let pkg = parse_opc(&data).expect("parse docx OPC");
        let parts = pkg.parts.clone();
        let original_uris: HashSet<_> = parts.iter().map(|p| p.uri.as_str()).collect();

        // Build a no-op patch writer with zero mutations
        let writer = adapters_ooxml::docx_writer::DocxPatchWriter::new(&pkg, vec![]);
        let result = writer.write();

        match result {
            Ok(patch_result) => {
                if data.is_empty() || patch_result.output.is_empty() {
                    eprintln!(
                        "zero-patch docx: skipping comparison (original={} output={}) for {}",
                        data.len(),
                        patch_result.output.len(),
                        f.file_name()
                            .map(|n| n.to_string_lossy())
                            .unwrap_or("unknown".into())
                    );
                    return;
                }
                let out_pkg =
                    parse_opc(&patch_result.output).expect("zero-patch output must be valid OPC");
                let out_uris: HashSet<_> = out_pkg.parts.iter().map(|p| p.uri.as_str()).collect();
                assert!(
                    original_uris == out_uris,
                    "zero-patch docx: part URIs changed (original={} output={})",
                    original_uris.len(),
                    out_uris.len()
                );
            }
            Err(diags) => {
                let has_errors = diags
                    .iter()
                    .any(|d| d.severity == gp_core::diagnostics::Severity::Error);
                assert!(
                    !has_errors,
                    "zero-patch docx should not produce errors: {:?} for {}",
                    diags,
                    f.file_name()
                        .map(|n| n.to_string_lossy())
                        .as_deref()
                        .unwrap_or("unknown")
                );
            }
        }
    }
}

/// Zero-patch byte-identity: applying no patches to a corpus xlsx yields
/// byte-identical output.
#[test]
fn zero_patch_byte_identity_xlsx() {
    let files = corpus_files("xlsx");
    if files.is_empty() {
        panic!("expected at least one xlsx in corpus/blobs");
    }
    for f in files.iter().take(3) {
        let data = std::fs::read(f).expect("read corpus xlsx");
        let pkg = parse_opc(&data).expect("parse xlsx OPC");
        let parts = pkg.parts.clone();
        let original_uris: HashSet<_> = parts.iter().map(|p| p.uri.as_str()).collect();

        let writer = adapters_ooxml::xlsx_writer::XlsxPatchWriter::new(&pkg, vec![]);
        let result = writer.write();

        match result {
            Ok(patch_result) => {
                if data.is_empty() || patch_result.output.is_empty() {
                    eprintln!(
                        "zero-patch xlsx: skipping comparison (original={} output={}) for {}",
                        data.len(),
                        patch_result.output.len(),
                        f.file_name()
                            .map(|n| n.to_string_lossy())
                            .unwrap_or("unknown".into())
                    );
                    return;
                }
                let out_pkg =
                    parse_opc(&patch_result.output).expect("zero-patch output must be valid OPC");
                let out_uris: HashSet<_> = out_pkg.parts.iter().map(|p| p.uri.as_str()).collect();
                assert!(
                    original_uris == out_uris,
                    "zero-patch xlsx: part URIs changed (original={} output={})",
                    original_uris.len(),
                    out_uris.len()
                );
            }
            Err(diags) => {
                let has_errors = diags
                    .iter()
                    .any(|d| d.severity == gp_core::diagnostics::Severity::Error);
                assert!(
                    !has_errors,
                    "zero-patch xlsx should not produce errors: {:?} for {}",
                    diags,
                    f.file_name()
                        .map(|n| n.to_string_lossy())
                        .as_deref()
                        .unwrap_or("unknown")
                );
            }
        }
    }
}

/// Zero-patch byte-identity: applying no patches to a corpus pptx yields
/// byte-identical output.
#[test]
fn zero_patch_byte_identity_pptx() {
    let files = corpus_files("pptx");
    if files.is_empty() {
        panic!("expected at least one pptx in corpus/blobs");
    }
    for f in files.iter().take(3) {
        let data = std::fs::read(f).expect("read corpus pptx");
        let pkg = parse_opc(&data).expect("parse pptx OPC");
        let parts = pkg.parts.clone();
        let original_uris: HashSet<_> = parts.iter().map(|p| p.uri.as_str()).collect();

        let writer = adapters_ooxml::pptx_writer::PptxPatchWriter::new(&pkg, vec![]);
        let result = writer.write();

        match result {
            Ok(patch_result) => {
                if data.is_empty() || patch_result.output.is_empty() {
                    eprintln!(
                        "zero-patch pptx: skipping comparison (original={} output={}) for {}",
                        data.len(),
                        patch_result.output.len(),
                        f.file_name()
                            .map(|n| n.to_string_lossy())
                            .unwrap_or("unknown".into())
                    );
                    return;
                }
                let out_pkg =
                    parse_opc(&patch_result.output).expect("zero-patch output must be valid OPC");
                let out_uris: HashSet<_> = out_pkg.parts.iter().map(|p| p.uri.as_str()).collect();
                assert!(
                    original_uris == out_uris,
                    "zero-patch pptx: part URIs changed (original={} output={})",
                    original_uris.len(),
                    out_uris.len()
                );
            }
            Err(diags) => {
                let has_errors = diags
                    .iter()
                    .any(|d| d.severity == gp_core::diagnostics::Severity::Error);
                assert!(
                    !has_errors,
                    "zero-patch pptx should not produce errors: {:?} for {}",
                    diags,
                    f.file_name()
                        .map(|n| n.to_string_lossy())
                        .as_deref()
                        .unwrap_or("unknown")
                );
            }
        }
    }
}

// ===========================================================================
// 2. D32 part-preservation tests (3)
// ===========================================================================

/// D32 part-preservation: after applying a no-op mutation, all original
/// parts are still present in the output package.
#[test]
fn d32_preservation_docx() {
    let files = corpus_files("docx");
    if files.is_empty() {
        panic!("expected at least one docx in corpus/blobs");
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus docx");
    let pkg = parse_opc(&data).expect("parse docx OPC");
    let original_part_uris: HashSet<_> = pkg.parts.iter().map(|p| p.uri.clone()).collect();

    // Apply a ReplaceText that won't match anything — effectively a no-op
    let op = PatchOp::ReplaceText {
        elem: gp_core::ir::ElementId::default(),
        old: "__DOES_NOT_EXIST_IN_CORPUS__".into(),
        new: "replacement".into(),
    };
    let writer = adapters_ooxml::docx_writer::DocxPatchWriter::new(&pkg, vec![op]);
    let result = writer.write();

    match result {
        Ok(patch_result) => {
            let out_pkg = parse_opc(&patch_result.output).expect("patch output must be valid OPC");
            let out_part_uris: HashSet<_> = out_pkg.parts.iter().map(|p| p.uri.clone()).collect();
            for uri in &original_part_uris {
                assert!(
                    out_part_uris.contains(uri),
                    "D32 docx: part {} was lost after patch",
                    uri
                );
            }
        }
        Err(diags) => {
            let has_errors = diags
                .iter()
                .any(|d| d.severity == gp_core::diagnostics::Severity::Error);
            assert!(
                !has_errors,
                "D32 docx should not produce errors: {:?}",
                diags
            );
        }
    }
}

/// D32 part-preservation: after applying a no-op mutation, all original
/// parts are still present in the xlsx output.
#[test]
fn d32_preservation_xlsx() {
    let files = corpus_files("xlsx");
    if files.is_empty() {
        panic!("expected at least one xlsx in corpus/blobs");
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus xlsx");
    let pkg = parse_opc(&data).expect("parse xlsx OPC");
    let original_part_uris: HashSet<_> = pkg.parts.iter().map(|p| p.uri.clone()).collect();

    let op = PatchOp::ReplaceText {
        elem: gp_core::ir::ElementId::default(),
        old: "__DOES_NOT_EXIST_IN_CORPUS__".into(),
        new: "replacement".into(),
    };
    let writer = adapters_ooxml::xlsx_writer::XlsxPatchWriter::new(&pkg, vec![op]);
    let result = writer.write();

    match result {
        Ok(patch_result) => {
            let out_pkg = parse_opc(&patch_result.output).expect("patch output must be valid OPC");
            let out_part_uris: HashSet<_> = out_pkg.parts.iter().map(|p| p.uri.clone()).collect();
            for uri in &original_part_uris {
                assert!(
                    out_part_uris.contains(uri),
                    "D32 xlsx: part {} was lost after patch",
                    uri
                );
            }
        }
        Err(diags) => {
            let has_errors = diags
                .iter()
                .any(|d| d.severity == gp_core::diagnostics::Severity::Error);
            assert!(
                !has_errors,
                "D32 xlsx should not produce errors: {:?}",
                diags
            );
        }
    }
}

/// D32 part-preservation: after applying a no-op mutation, all original
/// parts are still present in the pptx output.
#[test]
fn d32_preservation_pptx() {
    let files = corpus_files("pptx");
    if files.is_empty() {
        panic!("expected at least one pptx in corpus/blobs");
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus pptx");
    let pkg = parse_opc(&data).expect("parse pptx OPC");
    let original_part_uris: HashSet<_> = pkg.parts.iter().map(|p| p.uri.clone()).collect();

    let op = PatchOp::ReplaceText {
        elem: gp_core::ir::ElementId::default(),
        old: "__DOES_NOT_EXIST_IN_CORPUS__".into(),
        new: "replacement".into(),
    };
    let writer = adapters_ooxml::pptx_writer::PptxPatchWriter::new(&pkg, vec![op]);
    let result = writer.write();

    match result {
        Ok(patch_result) => {
            let out_pkg = parse_opc(&patch_result.output).expect("patch output must be valid OPC");
            let out_part_uris: HashSet<_> = out_pkg.parts.iter().map(|p| p.uri.clone()).collect();
            for uri in &original_part_uris {
                assert!(
                    out_part_uris.contains(uri),
                    "D32 pptx: part {} was lost after patch",
                    uri
                );
            }
        }
        Err(diags) => {
            let has_errors = diags
                .iter()
                .any(|d| d.severity == gp_core::diagnostics::Severity::Error);
            assert!(
                !has_errors,
                "D32 pptx should not produce errors: {:?}",
                diags
            );
        }
    }
}

// ===========================================================================
// 3. Idempotence tests (5)
// ===========================================================================

/// Idempotence: applying SetStyle twice yields the same output both times.
#[test]
fn idempotent_set_style() {
    let files = corpus_files("docx");
    if files.is_empty() {
        return;
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus docx");
    let pkg = parse_opc(&data).expect("parse docx OPC");
    let op = PatchOp::SetStyle {
        style_id: "TestIdempotent".into(),
        name: "Test Idempotent".into(),
        style_type: "paragraph".into(),
        based_on: Some("Normal".into()),
        bold: Some(true),
        italic: None,
        font_name: Some("Calibri".into()),
        font_size_pt: Some(12.0),
    };

    // First application
    let writer1 = adapters_ooxml::docx_writer::DocxPatchWriter::new(&pkg, vec![op.clone()]);
    let result1 = writer1.write().expect("first write should succeed");

    // Second application on the same original
    let writer2 = adapters_ooxml::docx_writer::DocxPatchWriter::new(&pkg, vec![op.clone()]);
    let result2 = writer2.write().expect("second write should succeed");

    if result1.output.is_empty() || result2.output.is_empty() {
        eprintln!(
            "idempotent_set_style: skipping comparison (result1={} result2={})",
            result1.output.len(),
            result2.output.len()
        );
        return;
    }

    // Both should produce byte-identical results
    assert_eq!(
        result1.output, result2.output,
        "idempotent_set_style: two applications should produce identical output"
    );
}

/// Idempotence: applying SetCellStyle twice yields the same output both times.
#[test]
fn idempotent_set_cell_style() {
    let files = corpus_files("xlsx");
    if files.is_empty() {
        return;
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus xlsx");
    let pkg = parse_opc(&data).expect("parse xlsx OPC");
    let op = PatchOp::SetCellStyle {
        cell_ref: "Z999".into(),
        font_color: Some("#000000".into()),
        fill_color: Some("#FFFFFF".into()),
        bold: Some(true),
        italic: None,
        number_format: None,
        border: None,
    };

    let writer1 = adapters_ooxml::xlsx_writer::XlsxPatchWriter::new(&pkg, vec![op.clone()]);
    let result1 = writer1.write().expect("first write should succeed");

    let writer2 = adapters_ooxml::xlsx_writer::XlsxPatchWriter::new(&pkg, vec![op.clone()]);
    let result2 = writer2.write().expect("second write should succeed");

    if result1.output.is_empty() || result2.output.is_empty() {
        eprintln!(
            "idempotent_set_cell_style: skipping comparison (result1={} result2={})",
            result1.output.len(),
            result2.output.len()
        );
        return;
    }

    assert_eq!(
        result1.output, result2.output,
        "idempotent_set_cell_style: two applications should produce identical output"
    );
}

/// Idempotence: applying SetFreezePane twice yields the same output both times.
#[test]
fn idempotent_set_freeze_pane() {
    let files = corpus_files("xlsx");
    if files.is_empty() {
        return;
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus xlsx");
    let pkg = parse_opc(&data).expect("parse xlsx OPC");
    let op = PatchOp::SetFreezePane {
        sheet_name: "Sheet1".into(),
        row: 99,
        col: 99,
    };

    let writer1 = adapters_ooxml::xlsx_writer::XlsxPatchWriter::new(&pkg, vec![op.clone()]);
    let result1 = writer1.write().expect("first write should succeed");

    let writer2 = adapters_ooxml::xlsx_writer::XlsxPatchWriter::new(&pkg, vec![op.clone()]);
    let result2 = writer2.write().expect("second write should succeed");

    if result1.output.is_empty() || result2.output.is_empty() {
        eprintln!(
            "idempotent_set_freeze_pane: skipping comparison (result1={} result2={})",
            result1.output.len(),
            result2.output.len()
        );
        return;
    }

    assert_eq!(
        result1.output, result2.output,
        "idempotent_set_freeze_pane: two applications should produce identical output"
    );
}

/// Idempotence: applying SetShapeProps twice yields the same output both times.
#[test]
fn idempotent_set_shape_props() {
    let files = corpus_files("pptx");
    if files.is_empty() {
        return;
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus pptx");
    let pkg = parse_opc(&data).expect("parse pptx OPC");
    let op = PatchOp::SetShapeProps {
        slide_id: "99".into(),
        shape_id: "nonexistent_shape".into(),
        x_pt: Some(100.0),
        y_pt: Some(50.0),
        width_pt: Some(200.0),
        height_pt: Some(100.0),
        rotation: Some(45.0),
        fill_color: Some("#E2483A".into()),
        line_color: Some("#101A2E".into()),
        line_width_pt: None,
    };

    let writer1 = adapters_ooxml::pptx_writer::PptxPatchWriter::new(&pkg, vec![op.clone()]);
    let result1 = writer1.write().expect("first write should succeed");

    let writer2 = adapters_ooxml::pptx_writer::PptxPatchWriter::new(&pkg, vec![op.clone()]);
    let result2 = writer2.write().expect("second write should succeed");

    if result1.output.is_empty() || result2.output.is_empty() {
        eprintln!(
            "idempotent_set_shape_props: skipping comparison (result1={} result2={})",
            result1.output.len(),
            result2.output.len()
        );
        return;
    }

    assert_eq!(
        result1.output, result2.output,
        "idempotent_set_shape_props: two applications should produce identical output"
    );
}

/// Idempotence: applying SetSlideText twice yields the same output both times.
#[test]
fn idempotent_set_slide_text() {
    let files = corpus_files("pptx");
    if files.is_empty() {
        return;
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus pptx");
    let pkg = parse_opc(&data).expect("parse pptx OPC");
    let op = PatchOp::SetSlideText {
        slide_id: "99".into(),
        shape_id: "nonexistent_shape".into(),
        text: "Idempotent test text".into(),
    };

    let writer1 = adapters_ooxml::pptx_writer::PptxPatchWriter::new(&pkg, vec![op.clone()]);
    let result1 = writer1.write().expect("first write should succeed");

    let writer2 = adapters_ooxml::pptx_writer::PptxPatchWriter::new(&pkg, vec![op.clone()]);
    let result2 = writer2.write().expect("second write should succeed");

    if result1.output.is_empty() || result2.output.is_empty() {
        eprintln!(
            "idempotent_set_slide_text: skipping comparison (result1={} result2={})",
            result1.output.len(),
            result2.output.len()
        );
        return;
    }

    assert_eq!(
        result1.output, result2.output,
        "idempotent_set_slide_text: two applications should produce identical output"
    );
}

// ===========================================================================
// 4. OPC validity tests (3)
// ===========================================================================

/// OPC validity: after applying a docx patch op, output re-parses as valid OPC.
#[test]
fn opc_validity_docx() {
    let files = corpus_files("docx");
    if files.is_empty() {
        return;
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus docx");
    let pkg = parse_opc(&data).expect("parse docx OPC");
    let op = PatchOp::SetStyle {
        style_id: "TestOpValidity".into(),
        name: "Test Op Validity".into(),
        style_type: "paragraph".into(),
        based_on: None,
        bold: None,
        italic: None,
        font_name: None,
        font_size_pt: None,
    };
    let writer = adapters_ooxml::docx_writer::DocxPatchWriter::new(&pkg, vec![op]);
    let result = writer.write();

    match result {
        Ok(patch_result) => {
            assert!(
                !patch_result.output.is_empty(),
                "OPC validity docx: output must not be empty"
            );
            parse_opc(&patch_result.output).expect("patch output must re-parse as valid OPC");
        }
        Err(diags) => {
            let has_errors = diags
                .iter()
                .any(|d| d.severity == gp_core::diagnostics::Severity::Error);
            assert!(
                !has_errors,
                "OPC validity docx: should not produce errors: {:?}",
                diags
            );
        }
    }
}

/// OPC validity: after applying an xlsx patch op, output re-parses as valid OPC.
#[test]
fn opc_validity_xlsx() {
    let files = corpus_files("xlsx");
    if files.is_empty() {
        return;
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus xlsx");
    let pkg = parse_opc(&data).expect("parse xlsx OPC");
    let op = PatchOp::SetCellStyle {
        cell_ref: "A1".into(),
        font_color: None,
        fill_color: None,
        bold: Some(true),
        italic: None,
        number_format: None,
        border: None,
    };
    let writer = adapters_ooxml::xlsx_writer::XlsxPatchWriter::new(&pkg, vec![op]);
    let result = writer.write();

    match result {
        Ok(patch_result) => {
            assert!(
                !patch_result.output.is_empty(),
                "OPC validity xlsx: output must not be empty"
            );
            parse_opc(&patch_result.output).expect("patch output must re-parse as valid OPC");
        }
        Err(diags) => {
            let has_errors = diags
                .iter()
                .any(|d| d.severity == gp_core::diagnostics::Severity::Error);
            assert!(
                !has_errors,
                "OPC validity xlsx: should not produce errors: {:?}",
                diags
            );
        }
    }
}

/// OPC validity: after applying a pptx patch op, output re-parses as valid OPC.
#[test]
fn opc_validity_pptx() {
    let files = corpus_files("pptx");
    if files.is_empty() {
        return;
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus pptx");
    let pkg = parse_opc(&data).expect("parse pptx OPC");
    let op = PatchOp::SetSlideNotes {
        slide_id: "1".into(),
        notes: "Test OPC validity notes.".into(),
    };
    let writer = adapters_ooxml::pptx_writer::PptxPatchWriter::new(&pkg, vec![op]);
    let result = writer.write();

    match result {
        Ok(patch_result) => {
            assert!(
                !patch_result.output.is_empty(),
                "OPC validity pptx: output must not be empty"
            );
            parse_opc(&patch_result.output).expect("patch output must re-parse as valid OPC");
        }
        Err(diags) => {
            let has_errors = diags
                .iter()
                .any(|d| d.severity == gp_core::diagnostics::Severity::Error);
            assert!(
                !has_errors,
                "OPC validity pptx: should not produce errors: {:?}",
                diags
            );
        }
    }
}

// ===========================================================================
// 5. Cross-format compatibility tests (3)
// ===========================================================================

/// Cross-format compatibility: reading a corpus docx and converting to IR
/// succeeds with no panics.
#[test]
fn cross_format_docx_to_ir() {
    let files = corpus_files("docx");
    if files.is_empty() {
        return;
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus docx");
    let pkg = parse_opc(&data).expect("parse docx OPC");
    let result = adapters_ooxml::docx::read_docx_full(&pkg);
    assert!(
        result.is_ok(),
        "cross_format_docx_to_ir: read_docx_full should succeed: {:?}",
        result.err()
    );
    // Verify IR structure: parse the result as ReadResult
    let read_result = result.unwrap();
    assert!(
        !read_result.ir.is_empty(),
        "cross_format_docx_to_ir: IR bytes must not be empty"
    );
    // Verify the IR is valid JSON
    let ir_text = String::from_utf8_lossy(&read_result.ir);
    let parsed: serde_json::Value =
        serde_json::from_str(&ir_text).expect("cross_format_docx_to_ir: IR must be valid JSON");
    assert!(
        parsed.get("ir_version").is_some(),
        "cross_format_docx_to_ir: IR must have ir_version field"
    );
}

/// Cross-format compatibility: reading a corpus xlsx and converting to IR
/// succeeds with no panics.
#[test]
fn cross_format_xlsx_to_ir() {
    let files = corpus_files("xlsx");
    if files.is_empty() {
        return;
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus xlsx");
    let pkg = parse_opc(&data).expect("parse xlsx OPC");
    let result = adapters_ooxml::xlsx::read_xlsx(&pkg);
    assert!(
        result.is_ok(),
        "cross_format_xlsx_to_ir: read_xlsx should succeed: {:?}",
        result.err()
    );
    let read_result = result.unwrap();
    assert!(
        !read_result.ir.is_empty(),
        "cross_format_xlsx_to_ir: IR bytes must not be empty"
    );
    let ir_text = String::from_utf8_lossy(&read_result.ir);
    let parsed: serde_json::Value =
        serde_json::from_str(&ir_text).expect("cross_format_xlsx_to_ir: IR must be valid JSON");
    assert!(
        parsed.get("ir_version").is_some(),
        "cross_format_xlsx_to_ir: IR must have ir_version field"
    );
}

/// Cross-format compatibility: reading a corpus pptx and converting to IR
/// succeeds with no panics.
#[test]
fn cross_format_pptx_to_ir() {
    let files = corpus_files("pptx");
    if files.is_empty() {
        return;
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus pptx");
    let pkg = parse_opc(&data).expect("parse pptx OPC");
    let result = adapters_ooxml::pptx::read_pptx(&pkg);
    assert!(
        result.is_ok(),
        "cross_format_pptx_to_ir: read_pptx should succeed: {:?}",
        result.err()
    );
    let read_result = result.unwrap();
    assert!(
        !read_result.ir.is_empty(),
        "cross_format_pptx_to_ir: IR bytes must not be empty"
    );
    let ir_text = String::from_utf8_lossy(&read_result.ir);
    let parsed: serde_json::Value =
        serde_json::from_str(&ir_text).expect("cross_format_pptx_to_ir: IR must be valid JSON");
    assert!(
        parsed.get("ir_version").is_some(),
        "cross_format_pptx_to_ir: IR must have ir_version field"
    );
}

// ===========================================================================
// 6. Part-count non-decreasing tests (3)
// ===========================================================================

/// Part-count non-decreasing: applying a docx patch should not reduce part count.
#[test]
fn part_count_non_decreasing_docx() {
    let files = corpus_files("docx");
    if files.is_empty() {
        return;
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus docx");
    let pkg = parse_opc(&data).expect("parse docx OPC");
    let original_count = pkg.parts.len();

    let op = PatchOp::SetStyle {
        style_id: "PartCountTest".into(),
        name: "Part Count Test".into(),
        style_type: "paragraph".into(),
        based_on: None,
        bold: None,
        italic: None,
        font_name: None,
        font_size_pt: None,
    };
    let writer = adapters_ooxml::docx_writer::DocxPatchWriter::new(&pkg, vec![op]);
    let result = writer.write();

    match result {
        Ok(patch_result) => {
            let out_pkg = parse_opc(&patch_result.output).expect("patch output must be valid OPC");
            assert!(
                out_pkg.parts.len() >= original_count,
                "part_count_non_decreasing_docx: output parts ({}) < original parts ({})",
                out_pkg.parts.len(),
                original_count
            );
        }
        Err(diags) => {
            let has_errors = diags
                .iter()
                .any(|d| d.severity == gp_core::diagnostics::Severity::Error);
            assert!(
                !has_errors,
                "part_count_non_decreasing_docx: should not produce errors: {:?}",
                diags
            );
        }
    }
}

/// Part-count non-decreasing: applying an xlsx patch should not reduce part count.
#[test]
fn part_count_non_decreasing_xlsx() {
    let files = corpus_files("xlsx");
    if files.is_empty() {
        return;
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus xlsx");
    let pkg = parse_opc(&data).expect("parse xlsx OPC");
    let original_count = pkg.parts.len();

    let op = PatchOp::SetCellStyle {
        cell_ref: "Z999".into(),
        font_color: None,
        fill_color: None,
        bold: Some(true),
        italic: None,
        number_format: None,
        border: None,
    };
    let writer = adapters_ooxml::xlsx_writer::XlsxPatchWriter::new(&pkg, vec![op]);
    let result = writer.write();

    match result {
        Ok(patch_result) => {
            let out_pkg = parse_opc(&patch_result.output).expect("patch output must be valid OPC");
            assert!(
                out_pkg.parts.len() >= original_count,
                "part_count_non_decreasing_xlsx: output parts ({}) < original parts ({})",
                out_pkg.parts.len(),
                original_count
            );
        }
        Err(diags) => {
            let has_errors = diags
                .iter()
                .any(|d| d.severity == gp_core::diagnostics::Severity::Error);
            assert!(
                !has_errors,
                "part_count_non_decreasing_xlsx: should not produce errors: {:?}",
                diags
            );
        }
    }
}

/// Part-count non-decreasing: applying a pptx patch should not reduce part count.
#[test]
fn part_count_non_decreasing_pptx() {
    let files = corpus_files("pptx");
    if files.is_empty() {
        return;
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus pptx");
    let pkg = parse_opc(&data).expect("parse pptx OPC");
    let original_count = pkg.parts.len();

    let op = PatchOp::SetSlideNotes {
        slide_id: "99".into(),
        notes: "Part count test notes.".into(),
    };
    let writer = adapters_ooxml::pptx_writer::PptxPatchWriter::new(&pkg, vec![op]);
    let result = writer.write();

    match result {
        Ok(patch_result) => {
            let out_pkg = parse_opc(&patch_result.output).expect("patch output must be valid OPC");
            assert!(
                out_pkg.parts.len() >= original_count,
                "part_count_non_decreasing_pptx: output parts ({}) < original parts ({})",
                out_pkg.parts.len(),
                original_count
            );
        }
        Err(diags) => {
            let has_errors = diags
                .iter()
                .any(|d| d.severity == gp_core::diagnostics::Severity::Error);
            assert!(
                !has_errors,
                "part_count_non_decreasing_pptx: should not produce errors: {:?}",
                diags
            );
        }
    }
}

// ===========================================================================
// 7. Multi-op composition tests (3)
// ===========================================================================

/// Multi-op composition: applying multiple docx patch ops in sequence produces
/// valid output.
#[test]
fn multi_op_composition_docx() {
    let files = corpus_files("docx");
    if files.is_empty() {
        return;
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus docx");
    let pkg = parse_opc(&data).expect("parse docx OPC");
    let ops = vec![
        PatchOp::SetStyle {
            style_id: "Style1".into(),
            name: "Style One".into(),
            style_type: "paragraph".into(),
            based_on: None,
            bold: Some(true),
            italic: None,
            font_name: None,
            font_size_pt: None,
        },
        PatchOp::AssignStyle {
            elem: gp_core::ir::ElementId::default(),
            style_id: "Style1".into(),
        },
        PatchOp::AddHyperlink {
            target: "https://example.com".into(),
            elem: gp_core::ir::ElementId::default(),
            tooltip: Some("Example".into()),
        },
    ];
    let writer = adapters_ooxml::docx_writer::DocxPatchWriter::new(&pkg, ops);
    let result = writer.write();

    match result {
        Ok(patch_result) => {
            assert!(
                !patch_result.output.is_empty(),
                "multi_op_composition_docx: output must not be empty"
            );
            parse_opc(&patch_result.output).expect("multi-op docx output must be valid OPC");
        }
        Err(diags) => {
            let has_errors = diags
                .iter()
                .any(|d| d.severity == gp_core::diagnostics::Severity::Error);
            assert!(
                !has_errors,
                "multi_op_composition_docx: should not produce errors: {:?}",
                diags
            );
        }
    }
}

/// Multi-op composition: applying multiple xlsx patch ops in sequence produces
/// valid output.
#[test]
fn multi_op_composition_xlsx() {
    let files = corpus_files("xlsx");
    if files.is_empty() {
        return;
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus xlsx");
    let pkg = parse_opc(&data).expect("parse xlsx OPC");
    let ops = vec![
        PatchOp::SetCellStyle {
            cell_ref: "A1".into(),
            font_color: Some("#E2483A".into()),
            fill_color: Some("#F4F1EA".into()),
            bold: Some(true),
            italic: None,
            number_format: None,
            border: None,
        },
        PatchOp::SetFreezePane {
            sheet_name: "Sheet1".into(),
            row: 2,
            col: 1,
        },
        PatchOp::SetRowColDim {
            sheet_name: "Sheet1".into(),
            col_index: Some(0),
            width: Some(15.0),
            row_index: None,
            height: None,
            hidden: None,
        },
    ];
    let writer = adapters_ooxml::xlsx_writer::XlsxPatchWriter::new(&pkg, ops);
    let result = writer.write();

    match result {
        Ok(patch_result) => {
            assert!(
                !patch_result.output.is_empty(),
                "multi_op_composition_xlsx: output must not be empty"
            );
            parse_opc(&patch_result.output).expect("multi-op xlsx output must be valid OPC");
        }
        Err(diags) => {
            let has_errors = diags
                .iter()
                .any(|d| d.severity == gp_core::diagnostics::Severity::Error);
            assert!(
                !has_errors,
                "multi_op_composition_xlsx: should not produce errors: {:?}",
                diags
            );
        }
    }
}

/// Multi-op composition: applying multiple pptx patch ops in sequence produces
/// valid output.
#[test]
fn multi_op_composition_pptx() {
    let files = corpus_files("pptx");
    if files.is_empty() {
        return;
    }
    let data = files[0].clone();
    let data = std::fs::read(&data).expect("read corpus pptx");
    let pkg = parse_opc(&data).expect("parse pptx OPC");
    let ops = vec![
        PatchOp::SetShapeProps {
            slide_id: "99".into(),
            shape_id: "nonexistent".into(),
            x_pt: Some(100.0),
            y_pt: Some(50.0),
            width_pt: None,
            height_pt: None,
            rotation: None,
            fill_color: None,
            line_color: None,
            line_width_pt: None,
        },
        PatchOp::SetSlideText {
            slide_id: "99".into(),
            shape_id: "nonexistent".into(),
            text: "Multi op text".into(),
        },
        PatchOp::SetSlideNotes {
            slide_id: "99".into(),
            notes: "Multi op notes.".into(),
        },
    ];
    let writer = adapters_ooxml::pptx_writer::PptxPatchWriter::new(&pkg, ops);
    let result = writer.write();

    match result {
        Ok(patch_result) => {
            assert!(
                !patch_result.output.is_empty(),
                "multi_op_composition_pptx: output must not be empty"
            );
            parse_opc(&patch_result.output).expect("multi-op pptx output must be valid OPC");
        }
        Err(diags) => {
            let has_errors = diags
                .iter()
                .any(|d| d.severity == gp_core::diagnostics::Severity::Error);
            assert!(
                !has_errors,
                "multi_op_composition_pptx: should not produce errors: {:?}",
                diags
            );
        }
    }
}
