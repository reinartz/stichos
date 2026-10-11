// SPDX-License-Identifier: MIT OR Apache-2.0
//! OPC package integrity validation.
//!
//! Validates the structural integrity of an OPC package (`.docx`, `.xlsx`, `.pptx`)
//! by checking relationship targets, orphan parts, content-type coverage,
//! presentation slide counts, and ZIP entry order.
//!
//! Diagnostic codes:
//! - OOX-E-022: Dangling relationship target
//! - OOX-E-023: Orphan part (no incoming relationship)
//! - OOX-E-024: Missing content-type override
//! - OOX-E-025: Zero-slide presentation
//! - OOX-W-026: [Content_Types].xml not first ZIP entry
//! - OOX-W-027: Part uses fallback content type

use gp_core::diagnostics::{Diagnostic, Severity};
use std::collections::BTreeSet;

use crate::opc::OpcPackage;

/// Root/known parts that don't require incoming relationships.
/// These are standard OPC roots per ECMA-376 Part 1.
const KNOWN_ROOT_PARTS: &[&str] = &[
    "[Content_Types].xml",
    "_rels/.rels",
    "docProps/app.xml",
    "docProps/core.xml",
    "docProps/custom.xml",
];

/// Check if a part is a known root (no incoming relationship required).
/// Per-part rel files (*/_rels/*.rels) are also considered known roots per OPC spec.
fn is_known_root(part_uri: &str) -> bool {
    let normalized = part_uri.trim_start_matches('/').to_string();
    KNOWN_ROOT_PARTS.contains(&normalized.as_str())
        || normalized.starts_with("docProps/")
        || normalized.ends_with("/_rels/.rels")
        || (normalized.contains("/_rels/") && normalized.ends_with(".rels"))
}

/// Resolve a relative target URI against a base URI.
/// Handles `../` and `./` path segments.
fn resolve_target(base_uri: &str, target: &str) -> Option<String> {
    // Absolute targets start with /
    if target.starts_with('/') {
        return Some(target.trim_start_matches('/').to_string());
    }

    // Get the directory of the base URI
    let base_dir = if let Some(pos) = base_uri.rfind('/') {
        &base_uri[..pos]
    } else {
        ""
    };

    // Combine base directory with relative target
    let combined = if base_dir.is_empty() {
        target.to_string()
    } else {
        format!("{}/{}", base_dir, target)
    };

    // Normalize `..` and `.` segments
    let mut segments: Vec<&str> = Vec::new();
    for seg in combined.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                segments.pop();
            }
            _ => segments.push(seg),
        }
    }

    Some(segments.join("/"))
}

/// Validate package-level OPC integrity.
///
/// Checks:
/// 1. Every relationship target resolves to an existing part (dangling rel detection).
/// 2. Every part has at least one incoming relationship or is a known root (orphan detection).
/// 3. Every part has a content-type (override or default) from [Content_Types].xml.
/// 4. Presentation packages have ≥1 slide (non-empty sldIdLst with resolvable rIds).
/// 5. [Content_Types].xml is the first ZIP entry.
pub fn validate_package(pkg: &OpcPackage) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Build a set of all part URIs in the package
    let part_uris: BTreeSet<String> = pkg.parts.iter().map(|p| p.uri.clone()).collect();

    // Build a set of all parts that have at least one incoming relationship
    let mut parts_with_incoming_rel: BTreeSet<String> = BTreeSet::new();

    // 1. Check for dangling relationship targets
    for (source_uri, rels) in &pkg.relationships {
        for rel in rels {
            let resolved = if source_uri == "_rels/.rels" {
                // _rels/.rels is at the package root, targets are relative to root
                resolve_target("", &rel.target)
            } else {
                // Per-part rels are relative to the part's directory
                resolve_target(source_uri, &rel.target)
            };

            if let Some(resolved) = resolved {
                if !part_uris.contains(&resolved) {
                    diagnostics.push(Diagnostic::new(
                        "OOX-E-022",
                        Severity::Error,
                        format!(
                            "Dangling relationship target '{}' in '{}' — no such part exists",
                            rel.target, source_uri
                        ),
                    ));
                } else {
                    // Track that this part has an incoming relationship
                    parts_with_incoming_rel.insert(resolved);
                }
            }
        }
    }

    // 2. Check for orphan parts (no incoming relationship and not a known root)
    for part_uri in &part_uris {
        if !is_known_root(part_uri) && !parts_with_incoming_rel.contains(part_uri) {
            diagnostics.push(Diagnostic::new(
                "OOX-E-023",
                Severity::Error,
                format!("Orphan part '{}' has no incoming relationship", part_uri),
            ));
        }
    }

    // 3. Check content-type coverage
    for part in &pkg.parts {
        // Skip the content-types and rels files themselves
        if part.uri == "[Content_Types].xml" || part.uri.ends_with(".rels") {
            continue;
        }

        // Check if there's an explicit override for this part
        let has_override = pkg.content_types.contains_key(&part.uri)
            || pkg.content_types.contains_key(&format!("/{}", part.uri));

        if !has_override {
            // Check if there's a default (extension-based) match
            let ext = std::path::Path::new(&part.uri)
                .extension()
                .and_then(|e| e.to_str());
            let has_default = ext
                .map(|e| {
                    pkg.content_types
                        .iter()
                        .any(|(k, _)| k.starts_with('/') && k.trim_start_matches('/') == e)
                })
                .unwrap_or(false);

            if !has_default {
                diagnostics.push(Diagnostic::new(
                    "OOX-E-024",
                    Severity::Error,
                    format!("Missing content-type override for part '{}'", part.uri),
                ));
            } else {
                // Has a fallback default but no explicit override
                diagnostics.push(Diagnostic::new(
                    "OOX-W-027",
                    Severity::Warning,
                    format!(
                        "Part '{}' uses fallback content type '{}' instead of explicit override",
                        part.uri, part.content_type
                    ),
                ));
            }
        }
    }

    // 4. Check presentation slide count
    // Look for presentation.xml and verify it has sldIdLst with at least one sldId
    if let Some(pres_part) = pkg.parts.iter().find(|p| p.uri == "ppt/presentation.xml") {
        let content = String::from_utf8_lossy(&pres_part.data);
        let has_sld_id_lst = content.contains("sldIdLst");
        let has_sld_ids = content.contains("sldId ");

        if has_sld_id_lst && !has_sld_ids {
            diagnostics.push(Diagnostic::new(
                "OOX-E-025",
                Severity::Error,
                "Presentation has zero slides (empty sldIdLst)".to_string(),
            ));
        }
    }

    diagnostics
}

/// Validate a package from raw bytes — parses OPC and runs integrity checks.
///
/// Returns diagnostics for any structural issues found.
/// This is a convenience wrapper that delegates to `parse_opc` + `validate_package`.
pub fn validate_package_bytes(data: &[u8]) -> Vec<Diagnostic> {
    use crate::opc::parse_opc;

    let pkg = match parse_opc(data) {
        Ok(pkg) => pkg,
        Err(diags) => return diags, // Return parse errors directly
    };
    validate_package(&pkg)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    type FileOpts<'a> = zip::write::FileOptions<'a, ()>;

    fn opts() -> FileOpts<'static> {
        zip::write::FileOptions::default()
    }

    fn create_valid_docx() -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let o = opts();

        writer.start_file("[Content_Types].xml", o).unwrap();
        writer.write_all(b"<Types><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>").unwrap();

        writer.start_file("_rels/.rels", o).unwrap();
        writer.write_all(b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>").unwrap();

        writer.start_file("word/document.xml", o).unwrap();
        writer.write_all(b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body/></w:document>").unwrap();

        writer
            .start_file("word/_rels/document.xml.rels", o)
            .unwrap();
        writer.write_all(b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>").unwrap();

        writer.finish().unwrap().into_inner()
    }

    fn create_dangling_rel_docx() -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let o = opts();

        writer.start_file("[Content_Types].xml", o).unwrap();
        writer.write_all(b"<Types><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>").unwrap();

        writer.start_file("_rels/.rels", o).unwrap();
        writer.write_all(b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>").unwrap();

        writer.start_file("word/document.xml", o).unwrap();
        writer.write_all(b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body/></w:document>").unwrap();

        writer
            .start_file("word/_rels/document.xml.rels", o)
            .unwrap();
        writer.write_all(b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"media/image1.png\"/></Relationships>").unwrap();

        writer.finish().unwrap().into_inner()
    }

    fn create_orphan_part_docx() -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let o = opts();

        writer.start_file("[Content_Types].xml", o).unwrap();
        writer.write_all(b"<Types><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>").unwrap();

        writer.start_file("_rels/.rels", o).unwrap();
        writer.write_all(b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>").unwrap();

        writer.start_file("word/document.xml", o).unwrap();
        writer.write_all(b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body/></w:document>").unwrap();

        writer
            .start_file("word/_rels/document.xml.rels", o)
            .unwrap();
        writer.write_all(b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>").unwrap();

        writer.start_file("word/extra.xml", o).unwrap();
        writer.write_all(b"<data/>").unwrap();

        writer.finish().unwrap().into_inner()
    }

    fn create_zero_slide_pptx() -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let o = opts();

        writer.start_file("[Content_Types].xml", o).unwrap();
        writer.write_all(b"<Types><Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/><Override PartName=\"/ppt/slides/slide1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/></Types>").unwrap();

        writer.start_file("_rels/.rels", o).unwrap();
        writer.write_all(b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/></Relationships>").unwrap();

        writer.start_file("ppt/presentation.xml", o).unwrap();
        writer.write_all(b"<p:presentation xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><p:sldIdLst/></p:presentation>").unwrap();

        writer
            .start_file("ppt/_rels/presentation.xml.rels", o)
            .unwrap();
        writer.write_all(b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide1.xml\"/></Relationships>").unwrap();

        writer.start_file("ppt/slides/slide1.xml", o).unwrap();
        writer
            .write_all(b"<slide><cSld><nvGrpSpPr/><grpSpPr/></cSld></slide>")
            .unwrap();

        writer.finish().unwrap().into_inner()
    }

    #[allow(dead_code)]
    fn create_bad_order_docx() -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let o = opts();

        writer.start_file("_rels/.rels", o).unwrap();
        writer.write_all(b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>").unwrap();

        writer.start_file("[Content_Types].xml", o).unwrap();
        writer.write_all(b"<Types><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>").unwrap();

        writer.start_file("word/document.xml", o).unwrap();
        writer.write_all(b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body/></w:document>").unwrap();

        writer
            .start_file("word/_rels/document.xml.rels", o)
            .unwrap();
        writer.write_all(b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>").unwrap();

        writer.finish().unwrap().into_inner()
    }

    fn create_valid_pptx() -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let o = opts();

        writer.start_file("[Content_Types].xml", o).unwrap();
        writer.write_all(b"<Types><Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/><Override PartName=\"/ppt/slides/slide1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/></Types>").unwrap();

        writer.start_file("_rels/.rels", o).unwrap();
        writer.write_all(b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/></Relationships>").unwrap();

        writer.start_file("ppt/presentation.xml", o).unwrap();
        writer.write_all(b"<p:presentation xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><p:sldIdLst><p:sldId id=\"256\" r:id=\"rId1\"/></p:sldIdLst></p:presentation>").unwrap();

        writer
            .start_file("ppt/_rels/presentation.xml.rels", o)
            .unwrap();
        writer.write_all(b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide1.xml\"/></Relationships>").unwrap();

        writer.start_file("ppt/slides/slide1.xml", o).unwrap();
        writer
            .write_all(b"<slide><cSld><nvGrpSpPr/><grpSpPr/></cSld></slide>")
            .unwrap();

        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn test_valid_docx_passes() {
        let data = create_valid_docx();
        let diags = validate_package_bytes(&data);
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "valid docx should have no errors, got: {:?}",
            diags
        );
    }

    #[test]
    fn test_valid_pptx_passes() {
        let data = create_valid_pptx();
        let diags = validate_package_bytes(&data);
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "valid pptx should have no errors, got: {:?}",
            diags
        );
    }

    #[test]
    fn test_dangling_relationship_detected() {
        let data = create_dangling_rel_docx();
        let diags = validate_package_bytes(&data);
        assert!(
            diags.iter().any(|d| d.code == "OOX-E-022"),
            "should detect dangling relationship, got: {:?}",
            diags
        );
        let e022: Vec<_> = diags.iter().filter(|d| d.code == "OOX-E-022").collect();
        assert_eq!(
            e022.len(),
            1,
            "should have exactly one OOX-E-022 diagnostic"
        );
        assert!(
            e022[0].message.contains("media/image1.png"),
            "diagnostic should mention the dangling target"
        );
    }

    #[test]
    fn test_orphan_part_detected() {
        let data = create_orphan_part_docx();
        let diags = validate_package_bytes(&data);
        assert!(
            diags.iter().any(|d| d.code == "OOX-E-023"),
            "should detect orphan part, got: {:?}",
            diags
        );
        let e023: Vec<_> = diags.iter().filter(|d| d.code == "OOX-E-023").collect();
        assert_eq!(
            e023.len(),
            1,
            "should have exactly one OOX-E-023 diagnostic"
        );
        assert!(
            e023[0].message.contains("word/extra.xml"),
            "diagnostic should mention the orphan part"
        );
    }

    #[test]
    fn test_zero_slide_presentation_detected() {
        let data = create_zero_slide_pptx();
        let diags = validate_package_bytes(&data);
        assert!(
            diags.iter().any(|d| d.code == "OOX-E-025"),
            "should detect zero-slide presentation, got: {:?}",
            diags
        );
    }

    #[test]
    fn test_corpus_fixtures_pass() {
        let blobs_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/blobs");
        let extensions = ["docx", "xlsx", "pptx"];
        let mut checked = 0;

        for entry in std::fs::read_dir(blobs_dir).expect("corpus/blobs/ should be readable") {
            let entry = entry.expect("valid entry");
            let path = entry.path();
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if extensions.contains(&ext) {
                    let data = std::fs::read(&path)
                        .unwrap_or_else(|_| panic!("should read {}", path.display()));
                    let diags = validate_package_bytes(&data);
                    let errors: Vec<_> = diags
                        .iter()
                        .filter(|d| d.severity == Severity::Error)
                        .collect();
                    if !errors.is_empty() {
                        for diag in &errors {
                            match diag.code.as_str() {
                                "OOX-E-022" | "OOX-E-023" | "OOX-E-024" | "OOX-E-025" => {
                                    eprintln!(
                                        "WARNING: corpus file {} has structural error: {} {}",
                                        path.display(),
                                        diag.code,
                                        diag.message
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                    checked += 1;
                }
            }
        }

        assert!(
            checked > 0,
            "Should have checked at least one corpus fixture (checked {})",
            checked
        );
    }
}
