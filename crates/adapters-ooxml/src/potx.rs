// SPDX-License-Identifier: MIT OR Apache-2.0
//! POTX -> PPTX template conversion.
//!
//! Converts a PowerPoint template (`.potx`) into a working presentation (`.pptx`)
//! by rewriting the content-type override in `[Content_Types].xml` from
//! `presentationml.template.main+xml` to `presentationml.presentation.main+xml`.
//!
//! This is the only safe static conversion — PowerPoint always expects the
//! content type to match the usage.  python-pptx equivalent:
//! `zipfile` rewrite of `[Content_Types].xml`.
//!
//! **Important:** The template's sample slides must be deleted **through
//! PowerPoint COM** (not by stripping ZIP entries) to avoid orphan parts,
//! dangling relationships, and the zero-slide pitfall. See T4.2.

use gp_core::diagnostics::{Diagnostic, Severity};
use std::io::{Cursor, Read, Write};

/// Content-type for a PPTX presentation (used in `[Content_Types].xml`).
const PPTX_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml";

/// Content-type for a POTX template (used in `[Content_Types].xml`).
const POTX_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.template.main+xml";

/// Convert a `.potx` template bytes to a `.pptx` presentation bytes.
///
/// Rewrites `[Content_Types].xml` content-type override from
/// `presentationml.template.main+xml` to `presentationml.presentation.main+xml`.
/// All other ZIP entries are copied verbatim.
///
/// Acceptance criteria:
/// - python-pptx consumers accept the output (same ZIP structure, only content-type changed).
/// - `validate_package` (T1) passes on the output.
pub fn potx_to_pptx(bytes: &[u8]) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let mut input_archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| {
        vec![Diagnostic::new(
            "POTX-E-001",
            Severity::Error,
            format!("Failed to open POTX archive: {}", e),
        )]
    })?;

    // Collect all entries first.
    let entries: Vec<(String, Vec<u8>)> = (0..input_archive.len())
        .map(|i| {
            let mut entry = input_archive.by_index(i).map_err(|e| {
                vec![Diagnostic::new(
                    "POTX-E-002",
                    Severity::Error,
                    format!("Failed to read entry {}: {}", i, e),
                )]
            })?;
            let name = entry.name().to_string();
            let mut data = Vec::new();
            entry.read_to_end(&mut data).map_err(|e| {
                vec![Diagnostic::new(
                    "POTX-E-003",
                    Severity::Error,
                    format!("Failed to read entry {}: {}", name, e),
                )]
            })?;
            Ok::<(String, Vec<u8>), Vec<Diagnostic>>((name, data))
        })
        .collect::<Result<_, _>>()?;

    // Use Cursor<Vec<u8>> (owned) — avoids borrow issues.
    let stored_opts: zip::write::FileOptions<'static, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

    let cursor = Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    for (name, data) in &entries {
        if *name == "[Content_Types].xml" {
            let content = String::from_utf8_lossy(data);
            let rewritten = content.replace(POTX_CONTENT_TYPE, PPTX_CONTENT_TYPE);
            if rewritten == content {
                return Err(vec![Diagnostic::new(
                    "POTX-W-001",
                    Severity::Warning,
                    "[Content_Types].xml does not contain POTX content-type; may not be a valid .potx"
                        .to_string(),
                )]);
            }
            writer
                .start_file("[Content_Types].xml", stored_opts)
                .map_err(|e| {
                    vec![Diagnostic::new(
                        "POTX-E-004",
                        Severity::Error,
                        format!("Failed to write entry: {}", e),
                    )]
                })?;
            writer.write_all(rewritten.as_bytes()).map_err(|e| {
                vec![Diagnostic::new(
                    "POTX-E-005",
                    Severity::Error,
                    format!("Failed to write data: {}", e),
                )]
            })?;
        } else {
            writer.start_file(name, stored_opts).map_err(|e| {
                vec![Diagnostic::new(
                    "POTX-E-004",
                    Severity::Error,
                    format!("Failed to write entry: {}", e),
                )]
            })?;
            writer.write_all(data).map_err(|e| {
                vec![Diagnostic::new(
                    "POTX-E-005",
                    Severity::Error,
                    format!("Failed to write data: {}", e),
                )]
            })?;
        }
    }
    let cursor = writer.finish().map_err(|e| {
        vec![Diagnostic::new(
            "POTX-E-006",
            Severity::Error,
            format!("Failed to finish ZIP: {}", e),
        )]
    })?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    type FileOpts<'a> = zip::write::FileOptions<'a, ()>;

    fn opts() -> FileOpts<'static> {
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored)
    }

    fn create_potx_fixture() -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let o = opts();

        writer.start_file("[Content_Types].xml", o).unwrap();
        writer
            .write_all(
                b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
              <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
              <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
              <Override PartName=\"/ppt/presentation.xml\" \
                       ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.template.main+xml\"/>\
              <Override PartName=\"/ppt/slides/slide1.xml\" \
                       ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>\
              </Types>",
            )
            .unwrap();

        writer.start_file("_rels/.rels", o).unwrap();
        writer
            .write_all(
                b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
              <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/>\
              </Relationships>",
            )
            .unwrap();

        writer.start_file("ppt/presentation.xml", o).unwrap();
        writer
            .write_all(
                b"<p:presentation xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:sldIdLst><p:sldId id=\"256\" r:id=\"rId1\"/></p:sldIdLst></p:presentation>",
            )
            .unwrap();

        writer
            .start_file("ppt/_rels/presentation.xml.rels", o)
            .unwrap();
        writer
            .write_all(
                b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
              <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide1.xml\"/>\
              </Relationships>",
            )
            .unwrap();

        writer.start_file("ppt/slides/slide1.xml", o).unwrap();
        writer
            .write_all(b"<slide><cSld><nvGrpSpPr/><grpSpPr/></cSld></slide>")
            .unwrap();

        writer.finish().unwrap().into_inner()
    }

    /// test_potx_to_pptx_content_type_rewritten: POTX content-type -> PPTX content-type.
    #[test]
    fn test_potx_to_pptx_content_type_rewritten() {
        let potx = create_potx_fixture();
        let pptx = potx_to_pptx(&potx).expect("potx_to_pptx should succeed");

        let cursor = Cursor::new(&pptx);
        let mut archive = zip::ZipArchive::new(cursor).expect("output should be valid ZIP");
        let mut ct = String::new();
        archive
            .by_name("[Content_Types].xml")
            .expect("[Content_Types].xml should exist")
            .read_to_string(&mut ct)
            .expect("should read [Content_Types].xml");

        assert!(
            ct.contains(PPTX_CONTENT_TYPE),
            "Should contain PPTX content-type, got: {}",
            ct
        );
        assert!(
            !ct.contains(POTX_CONTENT_TYPE),
            "Should NOT contain POTX content-type, got: {}",
            ct
        );
    }

    /// test_potx_to_pptx_other_entries_unchanged: all non-content-type entries copied verbatim.
    #[test]
    fn test_potx_to_pptx_other_entries_unchanged() {
        let potx = create_potx_fixture();
        let pptx = potx_to_pptx(&potx).expect("potx_to_pptx should succeed");

        let cursor = Cursor::new(&pptx);
        let mut archive = zip::ZipArchive::new(cursor).expect("output should be valid ZIP");

        let entry_names: Vec<String> = (0..archive.len())
            .filter_map(|i| archive.by_index(i).ok().map(|e| e.name().to_string()))
            .collect();
        assert!(entry_names.iter().any(|n| n == "_rels/.rels"));
        assert!(entry_names.iter().any(|n| n == "ppt/presentation.xml"));
        assert!(entry_names
            .iter()
            .any(|n| n == "ppt/_rels/presentation.xml.rels"));
        assert!(entry_names.iter().any(|n| n == "ppt/slides/slide1.xml"));
    }

    /// test_potx_to_pptx_output_valid_opc: output passes basic OPC parse.
    #[test]
    fn test_potx_to_pptx_output_valid_opc() {
        let potx = create_potx_fixture();
        let pptx = potx_to_pptx(&potx).expect("potx_to_pptx should succeed");

        assert_eq!(&pptx[0..2], b"PK", "Output should have PK ZIP signature");

        let result = crate::opc::parse_opc(&pptx);
        assert!(
            result.is_ok(),
            "Output should parse as valid OPC: {:?}",
            result
        );
    }

    /// test_potx_to_pptx_validate_package_passes: output passes T1 structural validation.
    #[test]
    fn test_potx_to_pptx_validate_package_passes() {
        let potx = create_potx_fixture();
        let pptx = potx_to_pptx(&potx).expect("potx_to_pptx should succeed");

        let diags = crate::opc_validate::validate_package_bytes(&pptx);
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "Converted PPTX should pass OPC validation, got: {:?}",
            errors
        );
    }
}
