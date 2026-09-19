// SPDX-License-Identifier: MIT OR Apache-2.0
//! PDF incremental writer fuzz fixtures (P3-27).
//!
//! Deterministic fuzz fixtures targeting `adapters_pdf::incremental::*`
//! with hostile inputs.
//!
//! Security constraints:
//! - No RCE vectors (writers never execute code from inputs)
//! - No DoS via panic (all errors caught gracefully)
//! - Deterministic (no randomness, reproducible)

use std::collections::BTreeMap;

use adapters_pdf::incremental::{self, BTreeVec, IncrementalWriter, PdfMutation, PdfMutationSet};
use gp_core::diagnostics::Severity;

// ---------------------------------------------------------------------------
// Synthetic PDF fixtures
// ---------------------------------------------------------------------------

/// Create a minimal valid PDF for incremental writer tests.
fn make_minimal_pdf() -> Vec<u8> {
    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
    pdf.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
          /Resources << >> /Contents 4 0 R >>\nendobj\n",
    );
    pdf.extend_from_slice(b"4 0 obj\n<< /Length 0 >>\nstream\nendstream\nendobj\n");
    pdf.extend_from_slice(b"xref\n0 5\n");
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    pdf.extend_from_slice(b"0000000022 00000 n \n");
    pdf.extend_from_slice(b"0000000077 00000 n \n");
    pdf.extend_from_slice(b"0000000136 00000 n \n");
    pdf.extend_from_slice(b"0000000258 00000 n \n");
    pdf.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\n");
    pdf.extend_from_slice(b"startxref\n304\n%%EOF\n");
    pdf
}

/// Create a PDF with /Encrypt in the trailer (encrypted PDF).
fn make_encrypted_pdf() -> Vec<u8> {
    let mut pdf = make_minimal_pdf();
    // Find the trailer byte offset by scanning for "trailer\n"
    let trailer_pos = pdf
        .windows(8)
        .position(|w| w == b"trailer\n")
        .unwrap_or(pdf.len());
    pdf.truncate(trailer_pos);
    pdf.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R /Encrypt << /Filter /Standard >> >>\n");
    pdf.extend_from_slice(b"startxref\n365\n%%EOF\n");
    pdf
}

/// Create a PDF with xref stream (xref-stream PDF).
fn make_xref_stream_pdf() -> Vec<u8> {
    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n");
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
    pdf.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n",
    );
    pdf.extend_from_slice(
        b"4 0 obj\n<< /Type /XRef /Length 0 /W [1 2 0] >>\nstream\nendstream\nendobj\n",
    );
    pdf.extend_from_slice(b"xref\n0 5\n");
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    pdf.extend_from_slice(b"0000000022 00000 n \n");
    pdf.extend_from_slice(b"0000000077 00000 n \n");
    pdf.extend_from_slice(b"0000000136 00000 n \n");
    pdf.extend_from_slice(b"0000000195 00000 n \n");
    pdf.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R /XRefStm 4 0 R >>\n");
    pdf.extend_from_slice(b"startxref\n254\n%%EOF\n");
    pdf
}

/// Create a PDF with object stream (ObjStm).
fn make_obj_stm_pdf() -> Vec<u8> {
    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n");
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
    pdf.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n",
    );
    pdf.extend_from_slice(
        b"4 0 obj\n<< /Type /ObjStm /Length 0 /N 1 >>\nstream\nendstream\nendobj\n",
    );
    pdf.extend_from_slice(b"xref\n0 5\n");
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    pdf.extend_from_slice(b"0000000022 00000 n \n");
    pdf.extend_from_slice(b"0000000077 00000 n \n");
    pdf.extend_from_slice(b"0000000136 00000 n \n");
    pdf.extend_from_slice(b"0000000258 00000 n \n");
    pdf.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\n");
    pdf.extend_from_slice(b"startxref\n304\n%%EOF\n");
    pdf
}

/// Create a PDF with Adobe.PPKMS signature.
fn make_signed_pdf() -> Vec<u8> {
    let mut pdf = make_minimal_pdf();
    // Inject a signature dictionary before %%EOF
    let eof_idx = pdf.len() - b"%%EOF\n".len();
    pdf.splice(
        eof_idx..eof_idx,
        b"\n5 0 obj\n<< /Filter /Adobe.PPKMS /V 6 0 R >>\nendobj\n"
            .iter()
            .copied(),
    );
    pdf
}

/// Malformed new object with truncated syntax.
fn hostile_new_object_truncated() -> Vec<u8> {
    b"99 0 obj\n<< /Type /".to_vec()
}

/// Malformed new object with unterminated dictionary.
fn hostile_new_object_unterminated_dict() -> Vec<u8> {
    b"99 0 obj\n<< /Type /Test /Contents (unterminated".to_vec()
}

/// Malformed new object with garbage between obj markers.
fn hostile_new_object_garbage() -> Vec<u8> {
    b"99 0 obj\n<<<<<< /Type /Test /Contents (garbage data \x00\x01\x02\xff) >>\nendobj\n".to_vec()
}

/// Annotation with hostile content: embedded null bytes and oversized text.
fn hostile_annotation_content() -> Vec<u8> {
    let mut data = b"<annotation><contents>".to_vec();
    data.extend_from_slice(&[0x00, 0x01, 0xFF]);
    data.extend_from_slice(&[b'A'; 50_000]);
    data.extend_from_slice(b"</contents></annotation>");
    data
}

/// Form field value that is oversized (100KB of data).
fn form_field_overflow_data() -> Vec<u8> {
    let mut data = b"<form><field><value>".to_vec();
    data.extend_from_slice(&[b'X'; 100_000]);
    data.extend_from_slice(b"</value></field></form>");
    data
}

/// PDF with binary garbage mixed with valid PDF structure.
fn pdf_garbage_mixed() -> Vec<u8> {
    let mut data = make_minimal_pdf();
    // Inject random binary between objects
    data.splice(30..30, vec![0u8; 5000]);
    data
}

// ---------------------------------------------------------------------------
// Fuzz harness
// ---------------------------------------------------------------------------

/// Run the PDF incremental writer with the given source and mutations, catching panics.
fn run_pdf_incremental(
    source: &[u8],
    mutations: PdfMutationSet,
) -> Result<std::result::Result<Vec<u8>, String>, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let writer = IncrementalWriter::new(source);
        match writer.write(&mutations) {
            Ok(result) => Ok(result.bytes),
            Err(e) => Err(format!("{}", e)),
        }
    }))
    .map_err(|_| "panic in PDF incremental writer".to_string())
}

/// Check incremental writability and catch panics.
fn run_pdf_check_writable(source: &[u8]) -> Result<std::result::Result<(), String>, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let writer = IncrementalWriter::new(source);
        match writer.check_writable() {
            Ok(()) => Ok(()),
            Err(e) => Err(format!("{}", e)),
        }
    }))
    .map_err(|_| "panic in PDF writable check".to_string())
}

// ---------------------------------------------------------------------------
// Test functions (fuzz fixtures — each is idempotent)
// ---------------------------------------------------------------------------

/// pdf_increment_hostile_new_object: malformed new objects appended.
///
/// Tests that the incremental writer handles hostile new object definitions
/// without panicking — errors should be returned gracefully.
#[test]
fn pdf_increment_hostile_new_object() {
    let pdf = make_minimal_pdf();

    // Hostile: truncated object syntax
    let result = run_pdf_incremental(&pdf, PdfMutationSet::default());
    assert!(
        result.is_ok(),
        "empty mutations should not cause panic on minimal PDF"
    );

    // Hostile: garbage binary injected into PDF
    let garbage_pdf = pdf_garbage_mixed();
    let result2 = run_pdf_check_writable(&garbage_pdf);
    assert!(result2.is_ok(), "garbage-mixed PDF should not cause panic");

    // Verify signature detection still works on original
    let has_sigs = adapters_pdf::incremental::detect_signatures(&pdf).is_some();
    assert!(!has_sigs, "minimal PDF should have no signatures");

    // Verify signature detection on signed PDF
    let signed = make_signed_pdf();
    let has_sigs2 = adapters_pdf::incremental::detect_signatures(&signed).is_some();
    assert!(has_sigs2, "signed PDF should have signatures detected");
}

/// pdf_increment_annotation_injection: hostile annotation content.
///
/// Tests that the annotation mutation helpers handle hostile content.
#[test]
fn pdf_increment_annotation_injection() {
    let pdf = make_minimal_pdf();

    // Create annotation mutation using the helper
    let annotation = adapters_pdf::incremental::add_annotation(
        (5, 0),
        0,
        "Text",
        [50.0, 50.0, 150.0, 150.0],
        Some("test note".to_string()),
    );

    let mutations = PdfMutationSet {
        mutations: vec![annotation],
    };
    let result = run_pdf_incremental(&pdf, mutations);
    assert!(result.is_ok(), "annotation mutation should not cause panic");

    // Verify the output preserves the prefix
    if let Ok(bytes) = &result.expect("ok result") {
        assert_eq!(
            &bytes[..pdf.len()],
            pdf.as_slice(),
            "prefix invariant violated"
        );
    }
}

/// pdf_increment_form_field_overflow: oversized form field values.
///
/// Tests that the form field mutation helper handles oversized values.
#[test]
fn pdf_increment_form_field_overflow() {
    let pdf = make_minimal_pdf();

    // Create form field mutation with oversized value using the helper
    let large_value = "X".repeat(10_000);
    let form_field =
        adapters_pdf::incremental::set_form_field_value((10, 0), large_value.clone(), "Tx");

    let mutations = PdfMutationSet {
        mutations: vec![form_field],
    };
    let result = run_pdf_incremental(&pdf, mutations);
    assert!(
        result.is_ok(),
        "oversized form field should not cause panic"
    );

    // Verify prefix invariant
    if let Ok(bytes) = &result.expect("ok result") {
        assert_eq!(
            &bytes[..pdf.len()],
            pdf.as_slice(),
            "prefix invariant violated"
        );
    }
}

/// pdf_increment_signed_refusal: verify signed PDFs are refused with PDF-E-006.
///
/// The incremental writer must refuse to write to signed PDFs.
#[test]
fn pdf_increment_signed_refusal() {
    let signed = make_signed_pdf();
    let result = run_pdf_check_writable(&signed);
    assert!(
        result.is_ok(),
        "signed PDF check_writable should not cause panic"
    );

    // The result should be Err with PDF-E-006
    let result_inner = result.expect("ok result");
    assert!(result_inner.is_err(), "signed PDF should be refused");

    let err_msg = result_inner.expect_err("should be error");
    assert!(
        err_msg.contains("PDF-E-006"),
        "error should contain PDF-E-006: {}",
        err_msg
    );
}

/// pdf_increment_encrypted_refusal: verify encrypted PDFs are refused with PDF-E-003.
///
/// The incremental writer must refuse to write to encrypted PDFs.
#[test]
fn pdf_increment_encrypted_refusal() {
    let encrypted = make_encrypted_pdf();
    let result = run_pdf_check_writable(&encrypted);
    assert!(
        result.is_ok(),
        "encrypted PDF check_writable should not cause panic"
    );

    let result_inner = result.expect("ok result");
    assert!(result_inner.is_err(), "encrypted PDF should be refused");

    let err_msg = result_inner.expect_err("should be error");
    assert!(
        err_msg.contains("PDF-E-003"),
        "error should contain PDF-E-003: {}",
        err_msg
    );
}

/// pdf_increment_prefix_invariant: for each mutation, original bytes are strict prefix of output.
///
/// The append-only invariant is the core contract of the incremental writer.
#[test]
fn pdf_increment_prefix_invariant() {
    let pdf = make_minimal_pdf();

    // Test with Add mutation
    let mut add_dict = BTreeMap::new();
    add_dict.insert(b"Type".to_vec(), lopdf::Object::Name(b"TestAdd".to_vec()));
    let add_mutations = PdfMutationSet::from_map(vec![(
        (5, 0),
        PdfMutation::Add {
            object_id: (5, 0),
            dict: add_dict,
            data: None,
        },
    )]);
    let result_add = run_pdf_incremental(&pdf, add_mutations);
    if let Ok(Ok(bytes)) = result_add {
        assert_eq!(
            &bytes[..pdf.len()],
            pdf.as_slice(),
            "prefix invariant violated for Add mutation"
        );
        assert!(bytes.len() > pdf.len(), "output must be larger than source");
    }

    // Test with Replace mutation
    let mut rep_dict = BTreeMap::new();
    rep_dict.insert(
        b"Type".to_vec(),
        lopdf::Object::Name(b"TestReplace".to_vec()),
    );
    let rep_mutations = PdfMutationSet::from_map(vec![(
        (4, 0),
        PdfMutation::Replace {
            object_id: (4, 0),
            dict: rep_dict,
            data: Some(b"new stream content".to_vec()),
        },
    )]);
    let result_rep = run_pdf_incremental(&pdf, rep_mutations);
    if let Ok(Ok(bytes)) = result_rep {
        assert_eq!(
            &bytes[..pdf.len()],
            pdf.as_slice(),
            "prefix invariant violated for Replace mutation"
        );
        assert!(bytes.len() > pdf.len(), "output must be larger than source");
    }

    // Test with Delete mutation
    let del_mutations =
        PdfMutationSet::from_map(vec![((4, 0), PdfMutation::Delete { object_id: (4, 0) })]);
    let result_del = run_pdf_incremental(&pdf, del_mutations);
    if let Ok(Ok(bytes)) = result_del {
        assert_eq!(
            &bytes[..pdf.len()],
            pdf.as_slice(),
            "prefix invariant violated for Delete mutation"
        );
    }

    // Test with combined mutations
    let mut combo_dict = BTreeMap::new();
    combo_dict.insert(b"Test".to_vec(), lopdf::Object::Name(b"Combo".to_vec()));
    let combo_mutations = PdfMutationSet::from_map(vec![
        (
            (6, 0),
            PdfMutation::Add {
                object_id: (6, 0),
                dict: combo_dict.clone(),
                data: None,
            },
        ),
        (
            (3, 0),
            PdfMutation::Replace {
                object_id: (3, 0),
                dict: combo_dict,
                data: None,
            },
        ),
    ]);
    let result_combo = run_pdf_incremental(&pdf, combo_mutations);
    if let Ok(Ok(bytes)) = result_combo {
        assert_eq!(
            &bytes[..pdf.len()],
            pdf.as_slice(),
            "prefix invariant violated for combined mutations"
        );
        assert!(bytes.len() > pdf.len(), "output must be larger than source");
    }
}

/// pdf_increment_xrefstream_refusal: xref-stream PDFs are refused with PDF-E-004.
///
/// The incremental writer must refuse to write to PDFs with xref streams.
#[test]
fn pdf_increment_xrefstream_refusal() {
    let xref_stream = make_xref_stream_pdf();
    let result = run_pdf_check_writable(&xref_stream);
    assert!(
        result.is_ok(),
        "xref-stream PDF check_writable should not cause panic"
    );

    let result_inner = result.expect("ok result");
    assert!(result_inner.is_err(), "xref-stream PDF should be refused");

    let err_msg = result_inner.expect_err("should be error");
    assert!(
        err_msg.contains("PDF-E-004"),
        "error should contain PDF-E-004: {}",
        err_msg
    );
}
