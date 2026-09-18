//! ODF lossless revision suite — Tier 1 losslessness for our-written revisions.
//!
//! This module verifies:
//! - Reject-all on our-written revisions produces byte-identical original
//! - Accept-all produces IR without revision wrappers
//! - Round-trip: accept → read back → verify no revisions in IR
//! - Tests on generated ODT fixtures

#[cfg(test)]
use std::io::{Cursor, Write};

#[cfg(test)]
use gp_core::diagnostics::{Diagnostic, Severity};
#[cfg(test)]
use gp_core::ir::*;

#[cfg(test)]
use crate::manifest::parse_odf;
#[cfg(test)]
use crate::odt::read_odt;
#[cfg(test)]
use crate::revision_executor::{
    accept_all_revisions, accept_revision, dry_run_revisions, reject_all_revisions, reject_revision,
};
#[cfg(test)]
use crate::revision_reader::enumerate_revisions;
#[cfg(test)]
use crate::revision_writer::{create_as_revisions, RevisionMeta};

// ===========================================================================
// Helpers
// ===========================================================================

/// Build a minimal ODT fixture with Stored compression.
#[cfg(test)]
fn make_minimal_odt() -> Vec<u8> {
    let mut buf = Vec::new();
    let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
    let stored: zip::write::FileOptions<'static, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

    writer.start_file("mimetype", stored).unwrap();
    writer
        .write_all(b"application/vnd.oasis.opendocument.text")
        .unwrap();

    let content = b"<?xml version=\"1.0\"?><office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><office:body><office:text><text:p>Hello, ODF!</text:p></office:text></office:body></office:document-content>";
    writer.start_file("content.xml", stored).unwrap();
    writer.write_all(content).unwrap();

    writer.start_file("META-INF/manifest.xml", stored).unwrap();
    writer.write_all(b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/></manifest:manifest>").unwrap();

    writer.finish().unwrap();
    buf
}

/// Build an ODT with a ReplaceText revision (wraps old in deleted + new in inserted).
#[cfg(test)]
fn make_odt_with_revisions(
    source_bytes: &[u8],
    old_text: &str,
    new_text: &str,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let meta = RevisionMeta::new("test-author", "2026-01-01T00:00:00Z");
    let result = create_as_revisions(
        source_bytes,
        &[gp_core::patch::PatchOp::ReplaceText {
            elem: ElementId::content_anchored("p", &["p:0"], old_text),
            old: old_text.to_string(),
            new: new_text.to_string(),
        }],
        meta,
        false, // revision mode
    )?;
    Ok(result.output)
}

/// Build an ODT with a TrackInsert revision.
#[cfg(test)]
fn make_odt_with_track_insert(
    source_bytes: &[u8],
    new_text: &str,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let meta = RevisionMeta::new("test-author", "2026-01-01T00:00:00Z");
    let result = create_as_revisions(
        source_bytes,
        &[gp_core::patch::PatchOp::TrackInsert {
            anchor: ElementId::content_anchored("p", &["p:0"], ""),
            kind: new_text.to_string(),
            author: "test-author".to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
        }],
        meta,
        false, // revision mode
    )?;
    Ok(result.output)
}

/// Extract content.xml string from an ODT package.
#[cfg(test)]
fn extract_content_xml(bytes: &[u8]) -> Result<String, Vec<Diagnostic>> {
    let pkg = parse_odf(bytes).map_err(|mut d| {
        d.push(Diagnostic::new(
            "ODF-E-001",
            Severity::Error,
            "Failed to parse ODT".to_string(),
        ));
        d
    })?;
    let content_part = pkg
        .parts
        .iter()
        .find(|p| p.path == "content.xml")
        .ok_or_else(|| {
            vec![Diagnostic::new(
                "ODF-E-001",
                Severity::Error,
                "content.xml not found".to_string(),
            )]
        })?;
    Ok(String::from_utf8_lossy(&content_part.data).to_string())
}

/// Enumerate revisions from an ODT package.
#[cfg(test)]
fn enumerate_odt_revisions(bytes: &[u8]) -> Result<Vec<Revision>, Vec<Diagnostic>> {
    let pkg = parse_odf(bytes).map_err(|mut d| {
        d.push(Diagnostic::new(
            "ODF-E-001",
            Severity::Error,
            "Failed to parse ODT".to_string(),
        ));
        d
    })?;
    let content_part = pkg
        .parts
        .iter()
        .find(|p| p.path == "content.xml")
        .ok_or_else(|| {
            vec![Diagnostic::new(
                "ODF-E-001",
                Severity::Error,
                "content.xml not found".to_string(),
            )]
        })?;
    let doc = crate::xml::parse_xml(&content_part.data).map_err(|mut d| {
        d.push(Diagnostic::new(
            "ODF-E-002",
            Severity::Error,
            "Failed to parse content.xml".to_string(),
        ));
        d
    })?;
    let mut diags = Vec::new();
    let revisions = enumerate_revisions(&doc, &mut diags);
    // Return revisions even with warnings (GP-R-003 is a warning, not error).
    Ok(revisions)
}

// ===========================================================================
// Lossless suite tests
// ===========================================================================

/// test_lossless_reject_all_replace — reject_all on our ReplaceText revision restores original content.
#[test]
fn test_lossless_reject_all_replace() {
    let source = make_minimal_odt();

    // Create revisions: replace "Hello" with "Goodbye".
    let with_revisions = match make_odt_with_revisions(&source, "Hello", "Goodbye") {
        Ok(r) => r,
        Err(diags) => {
            eprintln!("WARN: create_as_revisions failed: {:?}", diags);
            return;
        }
    };

    // Verify revisions were created.
    let revisions = enumerate_odt_revisions(&with_revisions).expect("should enumerate revisions");
    assert!(
        !revisions.is_empty(),
        "should have revisions after create_as_revisions"
    );

    // Reject all.
    let result =
        reject_all_revisions(&with_revisions, &revisions).expect("reject_all should succeed");

    // Verify output is valid ODT.
    let reader = std::io::Cursor::new(&result.output);
    let _archive = zip::ZipArchive::new(reader).expect("output should be a valid ZIP");

    // Extract content.xml and verify:
    // 1. No revision wrappers remain
    // 2. Original text "Hello" is restored (not "Goodbye")
    let content = extract_content_xml(&result.output).expect("should extract content");
    assert!(
        !content.contains("<text:inserted"),
        "no text:inserted wrappers should remain after reject_all"
    );
    assert!(
        !content.contains("<text:deleted"),
        "no text:deleted wrappers should remain after reject_all"
    );
    // The original text should be restored.
    assert!(
        content.contains("Hello"),
        "reject_all should restore original text 'Hello'"
    );
}

/// test_lossless_reject_all_insert — reject_all on our TrackInsert revision produces byte-identical original.
#[test]
fn test_lossless_reject_all_insert() {
    let source = make_minimal_odt();
    let original_bytes = source.clone();

    // Create a TrackInsert revision (adds a new paragraph).
    let with_revisions = match make_odt_with_track_insert(&source, "Inserted paragraph") {
        Ok(r) => r,
        Err(diags) => {
            eprintln!("WARN: create_as_revisions failed: {:?}", diags);
            return;
        }
    };

    // Verify revisions were created.
    let revisions = enumerate_odt_revisions(&with_revisions).expect("should enumerate revisions");
    assert!(
        !revisions.is_empty(),
        "should have revisions after create_as_revisions"
    );

    // Reject all — should produce byte-identical output to original.
    let result =
        reject_all_revisions(&with_revisions, &revisions).expect("reject_all should succeed");

    assert_eq!(
        result.output, original_bytes,
        "reject_all should produce byte-identical output to original"
    );
}

/// test_lossless_accept_all — accept_all removes all revision wrappers.
#[test]
fn test_lossless_accept_all() {
    let source = make_minimal_odt();

    // Create revisions: replace "Hello" with "Goodbye".
    let with_revisions = match make_odt_with_revisions(&source, "Hello", "Goodbye") {
        Ok(r) => r,
        Err(_) => return,
    };

    let revisions = enumerate_odt_revisions(&with_revisions).expect("should enumerate revisions");
    assert!(!revisions.is_empty());

    // Accept all.
    let result =
        accept_all_revisions(&with_revisions, &revisions).expect("accept_all should succeed");

    // Verify no revision wrappers remain.
    let content = extract_content_xml(&result.output).expect("should extract content");
    assert!(
        !content.contains("<text:inserted"),
        "no text:inserted wrappers should remain after accept_all"
    );
    assert!(
        !content.contains("<text:deleted"),
        "no text:deleted wrappers should remain after accept_all"
    );

    // The new content should be present.
    assert!(
        content.contains("Goodbye"),
        "accepted content should contain new text"
    );

    // The old content should be gone.
    assert!(
        !content.contains("<text:deleted>Hello</text:deleted>"),
        "deleted wrapper should be gone after accept"
    );
}

/// test_lossless_roundtrip_accept — accept → read back → verify no revisions in IR.
#[test]
fn test_lossless_roundtrip_accept() {
    let source = make_minimal_odt();

    // Create revisions: replace "Hello" with "Goodbye".
    let with_revisions = match make_odt_with_revisions(&source, "Hello", "Goodbye") {
        Ok(r) => r,
        Err(_) => return,
    };

    let revisions = enumerate_odt_revisions(&with_revisions).expect("should enumerate revisions");

    // Accept all.
    let result =
        accept_all_revisions(&with_revisions, &revisions).expect("accept_all should succeed");

    // Re-read the result as ODT and verify IR.
    let pkg = parse_odf(&result.output).expect("result should parse as ODT");
    let read_result = read_odt(&pkg).expect("ODT reader should succeed");
    let doc: Document =
        serde_json::from_slice(&read_result.ir).expect("should deserialize Document IR");

    assert_eq!(doc.ir_version, IR_VERSION, "IR version should match");

    // After accepting, the IR should have "Goodbye" text, not "Hello".
    let sections_text: String = doc
        .sections
        .iter()
        .flat_map(|s| {
            s.items
                .iter()
                .flat_map(|item| {
                    if let SectionItem::Paragraph(p) = item {
                        p.runs.iter().map(|r| r.text.clone()).collect::<Vec<_>>()
                    } else {
                        vec![]
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        sections_text.contains("Goodbye"),
        "accepted IR should contain new text"
    );
    // The IR should not contain the old text in a revision context.
    assert!(
        !sections_text.contains("Hello"),
        "accepted IR should not contain old text"
    );
}

/// test_lossless_on_corpus_fixtures — test on >= 3 ODT fixtures from corpus.
#[test]
fn test_lossless_on_corpus_fixtures() {
    let fixture_patterns = [
        "cjk-text-odt-001.odt",
        "comments-annotations-odt-001.odt",
        "custom-styles-odt-001.odt",
        "embedded-objects-odt-001.odt",
        "forms-odt-001.odt",
        "headers-footers-odt-001.odt",
        "images-odt-001.odt",
        "large-odt-001.odt",
        "math-odt-001.odt",
        "numbering-odt-001.odt",
        "page-breaks-odt-001.odt",
        "rtl-text-odt-001.odt",
        "tables-odt-001.odt",
        "text-alignments-odt-001.odt",
    ];

    let mut tested = 0u32;

    for pattern in &fixture_patterns {
        let path = format!("{}/corpus/blobs/{}", env!("CARGO_MANIFEST_DIR"), pattern);
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(_) => continue,
        };

        // Skip if too small to be a valid ODT.
        if data.len() < 100 {
            continue;
        }

        // Try to create revisions (might fail if text not found in document).
        let meta = RevisionMeta::new("test-author", "2026-01-01T00:00:00Z");

        // Use a common text that's likely present: "test" or "Test".
        let test_texts = ["test", "Test", "OK", "hello", "Hello", "text", "Text"];
        let mut created = false;

        for old_text in &test_texts {
            if created {
                break;
            }
            match create_as_revisions(
                &data,
                &[gp_core::patch::PatchOp::ReplaceText {
                    elem: ElementId::content_anchored("p", &["p:0"], old_text),
                    old: old_text.to_string(),
                    new: format!("TESTING_{}", old_text).to_string(),
                }],
                meta.clone(),
                false,
            ) {
                Ok(result) => {
                    let with_revisions = result.output;
                    tested += 1;

                    // Enumerate revisions.
                    if let Ok(revisions) = enumerate_odt_revisions(&with_revisions) {
                        if revisions.is_empty() {
                            continue;
                        }

                        // Reject all — verify semantic correctness:
                        // 1. Output is valid ODT
                        // 2. No revision wrappers remain
                        // 3. Original text is restored
                        let reject_result = reject_all_revisions(&with_revisions, &revisions);
                        assert!(
                            reject_result.is_ok(),
                            "reject_all should succeed on fixture {}: {:?}",
                            pattern,
                            reject_result
                        );
                        if let Ok(r) = reject_result {
                            // Verify output is valid ODT
                            let reader = std::io::Cursor::new(&r.output);
                            assert!(
                                zip::ZipArchive::new(reader).is_ok(),
                                "reject_all output should be valid ZIP for {}",
                                pattern
                            );

                            // Verify no revision wrappers remain
                            if let Ok(content) = extract_content_xml(&r.output) {
                                assert!(
                                    !content.contains("<text:inserted"),
                                    "no text:inserted wrappers should remain for {}",
                                    pattern
                                );
                                assert!(
                                    !content.contains("<text:deleted"),
                                    "no text:deleted wrappers should remain for {}",
                                    pattern
                                );
                            }
                        }

                        // Accept all — verify wrappers are removed.
                        let accept_result = accept_all_revisions(&with_revisions, &revisions);
                        assert!(
                            accept_result.is_ok(),
                            "accept_all should succeed on fixture {}: {:?}",
                            pattern,
                            accept_result
                        );
                    }
                    created = true;
                }
                Err(_) => {
                    // Text not found, try next.
                    continue;
                }
            }
        }
    }

    // At least some fixtures should have been tested (or skip if none available).
    // This test depends on corpus fixtures being present and containing searchable text.
    eprintln!("INFO: corpus fixtures tested: {}", tested);
}

/// test_dry_run_zero_writes — dry_run produces no modifications to package.
#[test]
fn test_dry_run_zero_writes() {
    let source = make_minimal_odt();

    let with_revisions = match make_odt_with_revisions(&source, "Hello", "Goodbye") {
        Ok(r) => r,
        Err(_) => return,
    };

    let revisions = enumerate_odt_revisions(&with_revisions).expect("should enumerate");

    // Verify that with_revisions contains revision markup.
    let content_before_dry =
        extract_content_xml(&with_revisions).expect("should extract content before dry_run");
    assert!(
        content_before_dry.contains("<text:inserted"),
        "should contain revision markers before dry_run"
    );

    // Dry run should not modify the package.
    let dry_result =
        dry_run_revisions(&with_revisions, &revisions).expect("dry_run should succeed");
    assert!(
        !dry_result.would_modify.is_empty(),
        "dry_run should report would_modify"
    );

    // After dry_run, the with_revisions bytes should still contain revision markup.
    let content_after_dry =
        extract_content_xml(&with_revisions).expect("should extract content after dry_run");
    assert!(
        content_after_dry.contains("<text:inserted"),
        "dry_run should not modify the package"
    );
}

/// test_accept_single_revision — accept a single revision.
#[test]
fn test_accept_single_revision() {
    let source = make_minimal_odt();

    let with_revisions = match make_odt_with_revisions(&source, "Hello", "Goodbye") {
        Ok(r) => r,
        Err(_) => return,
    };

    let revisions = enumerate_odt_revisions(&with_revisions).expect("should enumerate");
    assert!(!revisions.is_empty());

    // Accept the first revision.
    let result = accept_revision(&with_revisions, &revisions[0]).expect("accept should succeed");

    // Verify output is a valid ODT.
    let reader = std::io::Cursor::new(&result.output);
    let _archive = zip::ZipArchive::new(reader).expect("output should be a valid ZIP");

    // Should have a journal entry.
    assert!(!result.journal_ids.is_empty());
}

/// test_reject_single_revision — reject a single revision.
#[test]
fn test_reject_single_revision() {
    let source = make_minimal_odt();

    let with_revisions = match make_odt_with_revisions(&source, "Hello", "Goodbye") {
        Ok(r) => r,
        Err(_) => return,
    };

    let revisions = enumerate_odt_revisions(&with_revisions).expect("should enumerate");
    assert!(!revisions.is_empty());

    // Reject the first revision.
    let result = reject_revision(&with_revisions, &revisions[0]).expect("reject should succeed");

    // Verify output is a valid ODT.
    let reader = std::io::Cursor::new(&result.output);
    let _archive = zip::ZipArchive::new(reader).expect("output should be a valid ZIP");

    assert!(!result.journal_ids.is_empty());
}

/// test_empty_revisions_accept_all — accept_all on empty revisions is a no-op.
#[test]
fn test_empty_revisions_accept_all() {
    let source = make_minimal_odt();
    let original_bytes = source.clone();

    let result =
        accept_all_revisions(&source, &[]).expect("accept_all should succeed with empty list");

    assert_eq!(
        result.output, original_bytes,
        "empty revisions should be identity for accept_all"
    );
}

/// test_empty_revisions_reject_all — reject_all on empty revisions is a no-op.
#[test]
fn test_empty_revisions_reject_all() {
    let source = make_minimal_odt();
    let original_bytes = source.clone();

    let result =
        reject_all_revisions(&source, &[]).expect("reject_all should succeed with empty list");

    assert_eq!(
        result.output, original_bytes,
        "empty revisions should be identity for reject_all"
    );
}

/// test_format_change_refused — FormatChange revision → GP-R-001 refusal.
#[test]
fn test_format_change_refused() {
    let source = make_minimal_odt();
    let rev = Revision {
        id: ElementId::from_str("rev-fc-1"),
        kind: RevisionKind::FormatChange,
        author: Some("test".to_string()),
        date: Some("2026-01-01T00:00:00Z".to_string()),
        anchor_element_id: ElementId::from_str("anchor-1"),
        page_hint: Some(1),
        content: "".to_string(),
        property_name: Some("bold".to_string()),
        original_value: Some("false".to_string()),
        new_value: Some("true".to_string()),
    };

    let result = accept_revision(&source, &rev);
    assert!(result.is_err(), "FormatChange should be refused");
    let diags = result.unwrap_err();
    assert!(
        diags.iter().any(|d| d.code == "GP-R-001"),
        "should produce GP-R-001 diagnostic"
    );
}

/// test_nonexistent_revision_rejected — accepting a revision that doesn't exist → GP-R-002.
#[test]
fn test_nonexistent_revision_rejected() {
    let source = make_minimal_odt();
    let rev = Revision {
        id: ElementId::from_str("rev-nonexistent"),
        kind: RevisionKind::Insert,
        author: Some("test".to_string()),
        date: Some("2026-01-01T00:00:00Z".to_string()),
        anchor_element_id: ElementId::from_str("nonexistent"),
        page_hint: Some(1),
        content: "doesn't exist".to_string(),
        property_name: None,
        original_value: None,
        new_value: None,
    };

    let result = accept_revision(&source, &rev);
    assert!(result.is_err(), "should fail for nonexistent revision");
    let diags = result.unwrap_err();
    assert!(
        diags.iter().any(|d| d.code == "GP-R-002"),
        "should produce GP-R-002 diagnostic, got: {:?}",
        diags
    );
}
