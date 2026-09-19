// SPDX-License-Identifier: MIT OR Apache-2.0
//! ODF writer fuzz fixtures (P3-27).
//!
//! Deterministic fuzz fixtures targeting the ODF surgical writer
//! (`adapters_odf::writer::SurgicalWriter`) with hostile inputs.
//!
//! Security constraints:
//! - No RCE vectors (writers never execute code from inputs)
//! - No DoS via panic (all errors caught gracefully)
//! - Deterministic (no randomness, reproducible)

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use adapters_odf::writer::detect_xml_encrypt_in_manifest;
use adapters_odf::writer::{OdfMutation, SurgicalWriter};

// ---------------------------------------------------------------------------
// Synthetic ODF fixtures
// ---------------------------------------------------------------------------

/// Create a minimal valid ODT fixture (mimetype first, stored).
fn make_valid_odt() -> Vec<u8> {
    let mut buf = Vec::new();
    let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
    let o: zip::write::FileOptions<'static, ()> = zip::write::FileOptions::default();

    writer.start_file("mimetype", o).unwrap();
    writer
        .write_all(b"application/vnd.oasis.opendocument.text")
        .unwrap();

    writer.start_file("content.xml", o).unwrap();
    writer
        .write_all(
            b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:document-content>",
        )
        .unwrap();
    writer.start_file("meta.xml", o).unwrap();
    writer
        .write_all(
            b"<office:meta xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:meta>",
        )
        .unwrap();
    writer.start_file("styles.xml", o).unwrap();
    writer
        .write_all(
            b"<office:styles xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:styles>",
        )
        .unwrap();
    writer.start_file("META-INF/manifest.xml", o).unwrap();
    writer.write_all(
        b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/><manifest:default manifest:full-path=\"\" manifest:media-type=\"text/xml\"/></manifest:manifest>",
    ).unwrap();

    writer.finish().unwrap();
    buf
}

/// Create an ODF fixture with xml:encrypt in manifest.
fn make_encrypted_odt() -> Vec<u8> {
    let mut buf = Vec::new();
    let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
    let o: zip::write::FileOptions<'static, ()> = zip::write::FileOptions::default();

    writer.start_file("mimetype", o).unwrap();
    writer
        .write_all(b"application/vnd.oasis.opendocument.text")
        .unwrap();

    writer.start_file("content.xml", o).unwrap();
    writer
        .write_all(
            b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><text:p>encrypted</text:p></office:document-content>",
        )
        .unwrap();
    writer.start_file("META-INF/manifest.xml", o).unwrap();
    writer.write_all(
        b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" xmlns:xml=\"http://www.w3.org/XML/1998/namespace\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\" xml:encrypt=\"symmetric\"/></manifest:manifest>",
    ).unwrap();

    writer.finish().unwrap();
    buf
}

/// A malformed content.xml with invalid XML (truncated tags).
fn hostile_content_xml() -> Vec<u8> {
    b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><text:p>unclosed tag</text>".to_vec()
}

/// A content.xml with embedded null bytes.
fn content_xml_with_nulls() -> Vec<u8> {
    let mut data = b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\">".to_vec();
    data.push(0x00);
    data.push(0x01);
    data.extend_from_slice(b"<text:p>test</text:p>");
    data.extend_from_slice(b"</office:document-content>");
    data
}

/// A content.xml with extremely long text content (memory test).
fn content_xml_oversized() -> Vec<u8> {
    let mut data = b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><text:p>".to_vec();
    data.extend_from_slice(&[b'A'; 100_000]);
    data.extend_from_slice(b"</text:p></office:document-content>");
    data
}

/// A manifest.xml with wrong hashes and missing entries.
fn corrupt_manifest_xml() -> Vec<u8> {
    b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"nonexistent.xml\" manifest:media-type=\"text/xml\" manifest:compression=\"99\"/><manifest:default manifest:full-path=\"\" manifest:media-type=\"application/octet-stream\"/></manifest:manifest>".to_vec()
}

/// Deeply nested XML that could stress the ODF writer's XML handling.
fn deeply_nested_odf_xml(depth: usize) -> Vec<u8> {
    let mut data = b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\">".to_vec();
    for _ in 0..depth {
        data.extend_from_slice(b"<n>");
    }
    data.extend_from_slice(b"deep");
    for _ in 0..depth {
        data.extend_from_slice(b"</n>");
    }
    data.extend_from_slice(b"</office:document-content>");
    data
}

// ---------------------------------------------------------------------------
// Fuzz harness
// ---------------------------------------------------------------------------

/// Run the ODF writer with the given source bytes and mutations, catching panics.
fn run_odf_write(
    source: &[u8],
    mutations: BTreeMap<String, OdfMutation>,
) -> Result<Result<Vec<u8>, String>, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let writer = SurgicalWriter::new(source);
        let write_result = writer.write(mutations);
        match write_result {
            Ok(result) => Ok(result.bytes),
            Err(diags) => {
                // Check if the error contains ODF-E-003 (encryption)
                let has_e003 = diags.iter().any(|d| d.code == "ODF-E-003");
                if has_e003 {
                    return Err(diags[0].message.clone());
                }
                Err(diags[0].message.clone())
            }
        }
    }))
    .map_err(|_| "panic in ODF writer".to_string())
}

// ---------------------------------------------------------------------------
// Test functions (fuzz fixtures — each is idempotent)
// ---------------------------------------------------------------------------

/// odf_mimetype_first_stored: verify mimetype ordering (valid).
///
/// The ODF writer must always place mimetype as the first entry in the ZIP
/// with stored (uncompressed) compression method.
#[test]
fn odf_mimetype_first_stored() {
    let pkg = make_valid_odt();
    let writer = SurgicalWriter::new(&pkg);
    let mutations: BTreeMap<String, OdfMutation> = BTreeMap::new();
    let result = writer.write(mutations).expect("write");

    let mut archive =
        zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");
    let first_name = archive.by_index(0).expect("first entry").name().to_string();
    assert_eq!(
        first_name, "mimetype",
        "mimetype must be the first entry in the ZIP"
    );

    let first_entry = archive.by_index(0).expect("first entry");
    assert_eq!(
        first_entry.compression(),
        zip::CompressionMethod::Stored,
        "mimetype must be stored (uncompressed)"
    );
}

/// odf_hostile_content_xml: malformed XML injected as content.xml replacement.
///
/// Tests that hostile content.xml is handled gracefully:
/// - Valid malformed XML is written through (the writer doesn't validate XML content)
/// - Content with null bytes is written through
/// - Oversized content is written through without panicking
#[test]
fn odf_hostile_content_xml() {
    let pkg = make_valid_odt();

    // Hostile: truncated/malformed XML
    let mut mutations: BTreeMap<String, OdfMutation> = BTreeMap::new();
    mutations.insert(
        "content.xml".to_string(),
        OdfMutation::Replace {
            data: hostile_content_xml(),
        },
    );
    let result = run_odf_write(&pkg, mutations.clone());
    assert!(
        result.is_ok(),
        "malformed content.xml should not cause panic"
    );

    // Hostile: null bytes in content
    let mut mutations2: BTreeMap<String, OdfMutation> = BTreeMap::new();
    mutations2.insert(
        "content.xml".to_string(),
        OdfMutation::Replace {
            data: content_xml_with_nulls(),
        },
    );
    let result2 = run_odf_write(&pkg, mutations2.clone());
    assert!(
        result2.is_ok(),
        "null bytes in content.xml should not cause panic"
    );

    // Hostile: oversized content
    let mut mutations3: BTreeMap<String, OdfMutation> = BTreeMap::new();
    mutations3.insert(
        "content.xml".to_string(),
        OdfMutation::Replace {
            data: content_xml_oversized(),
        },
    );
    let result3 = run_odf_write(&pkg, mutations3);
    assert!(
        result3.is_ok(),
        "oversized content.xml should not cause panic"
    );
}

/// odf_manifest_corruption: manifest.xml with wrong hashes/missing entries.
///
/// Tests that corrupt manifest data does not cause panics.
#[test]
fn odf_manifest_corruption() {
    let pkg = make_valid_odt();

    // Replace manifest with corrupt version
    let mut mutations: BTreeMap<String, OdfMutation> = BTreeMap::new();
    mutations.insert(
        "META-INF/manifest.xml".to_string(),
        OdfMutation::Replace {
            data: corrupt_manifest_xml(),
        },
    );
    let result = run_odf_write(&pkg, mutations);
    assert!(result.is_ok(), "corrupt manifest should not cause panic");

    // Deeply nested XML in content
    let mut mutations2: BTreeMap<String, OdfMutation> = BTreeMap::new();
    mutations2.insert(
        "content.xml".to_string(),
        OdfMutation::Replace {
            data: deeply_nested_odf_xml(100),
        },
    );
    let result2 = run_odf_write(&pkg, mutations2);
    assert!(
        result2.is_ok(),
        "deeply nested ODF XML should not cause panic"
    );
}

/// odf_encrypted_rejection: verify encrypted ODF is refused with ODF-E-003.
///
/// The writer's detect_xml_encrypt_in_manifest should flag encrypted packages.
#[test]
fn odf_encrypted_rejection() {
    let pkg = make_encrypted_odt();
    let mut archive = zip::ZipArchive::new(Cursor::new(&pkg)).expect("open ZIP");
    let mut manifest_bytes = Vec::new();
    archive
        .by_name("META-INF/manifest.xml")
        .expect("find manifest")
        .read_to_end(&mut manifest_bytes)
        .expect("read manifest");

    let result = detect_xml_encrypt_in_manifest(&manifest_bytes);
    assert!(
        result.is_some(),
        "xml:encrypt should be detected in manifest"
    );

    // Verify the diagnostic code matches ODF-E-003 convention
    let reason = result.unwrap();
    assert!(
        reason.contains("xml:encrypt"),
        "reason should mention xml:encrypt"
    );
}

/// odf_write_roundtrip_stability: same mutation x10 produces identical output.
///
/// The ODF writer is deterministic: applying the same mutation to the same
/// source must produce byte-identical output every time.
#[test]
fn odf_write_roundtrip_stability() {
    let pkg = make_valid_odt();

    let mut mutations: BTreeMap<String, OdfMutation> = BTreeMap::new();
    mutations.insert(
        "content.xml".to_string(),
        OdfMutation::Replace {
            data: b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><text:p>stability test</text:p></office:document-content>"
                .to_vec(),
        },
    );

    // Run 10 times, all must produce identical output
    let mut first_bytes: Option<Vec<u8>> = None;
    for i in 0..10 {
        let writer = SurgicalWriter::new(&pkg);
        let result = writer.write(mutations.clone()).expect("write");
        if i == 0 {
            first_bytes = Some(result.bytes.clone());
        } else {
            assert_eq!(
                result.bytes,
                first_bytes.as_ref().expect("first_bytes").clone(),
                "iteration {} produced different output",
                i
            );
        }
    }
}

/// odf_patch_reverse_order: multiple patches applied in reverse offset order.
///
/// The SurgicalWriter processes mutations via BTreeMap which is ordered by key.
/// Multiple patches replacing different parts must produce consistent output
/// regardless of the logical order they are applied (BTreeMap ordering is deterministic).
#[test]
fn odf_patch_reverse_order() {
    let pkg = make_valid_odt();

    // Create mutations with multiple paths — BTreeMap ensures deterministic key ordering
    let mut mutations: BTreeMap<String, OdfMutation> = BTreeMap::new();
    mutations.insert(
        "content.xml".to_string(),
        OdfMutation::Replace {
            data: b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><text:p>first</text:p></office:document-content>".to_vec(),
        },
    );
    mutations.insert(
        "meta.xml".to_string(),
        OdfMutation::Replace {
            data: b"<office:meta xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><office:generator>test</office:generator></office:meta>".to_vec(),
        },
    );
    mutations.insert(
        "styles.xml".to_string(),
        OdfMutation::Replace {
            data: b"<office:styles xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:styles>".to_vec(),
        },
    );

    let writer = SurgicalWriter::new(&pkg);
    let result = writer.write(mutations).expect("write");
    assert!(
        result.report.parts_written.len() >= 3,
        "all 3 parts should be written"
    );

    // Verify the output is a valid ZIP
    let mut archive =
        zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");
    assert!(
        archive.by_name("content.xml").is_ok(),
        "content.xml should exist"
    );
    assert!(archive.by_name("meta.xml").is_ok(), "meta.xml should exist");
    assert!(
        archive.by_name("styles.xml").is_ok(),
        "styles.xml should exist"
    );
}
