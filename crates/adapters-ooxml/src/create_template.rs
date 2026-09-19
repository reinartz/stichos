// SPDX-License-Identifier: MIT OR Apache-2.0
//! Template-based native creation: reference docx as base package.
//!
//! Takes an existing template `.docx` file as a reference package, preserves
//! style/theme/master parts **byte-for-byte**, and replaces content parts
//! (document.xml, headers, footers, numbering, media) with IR-derived content.
//!
//! # Template preservation
//!
//! The following parts are copied raw from the template:
//! - `word/styles.xml` — styles
//! - `word/theme/theme1.xml` — theme
//! - `word/fontTable.xml` — font table
//! - `word/webSettings.xml` — web settings
//! - `word/footnotes1.xml` — footnotes (if present)
//! - `word/endnotes1.xml` — endnotes (if present)
//! - `word/numbering.xml` — numbering definitions (if present)
//! - `word/proxy.docprops` — document properties
//!
//! The following parts are generated from IR:
//! - `word/document.xml` — main document body
//! - `word/headerN.xml` — headers
//! - `word/footerN.xml` — footers
//! - `word/_rels/document.xml.rels` — updated relationships
//!
//! Template drop is reported as informational diagnostics.

use gp_core::diagnostics::{Diagnostic, Severity};
use gp_core::ir::*;
use std::collections::{BTreeMap, HashSet};
use std::io::{Cursor, Write};

use crate::create;

// ===========================================================================
// Template part classification
// ===========================================================================

/// Parts that should be preserved byte-for-byte from the template.
fn is_preserve_part(uri: &str) -> bool {
    matches!(
        uri,
        "word/styles.xml"
            | "word/theme/theme1.xml"
            | "word/fontTable.xml"
            | "word/webSettings.xml"
            | "word/footnotes1.xml"
            | "word/endnotes1.xml"
            | "word/numbering.xml"
            | "word/proxy.docprops"
    )
}

/// Parts that should always be regenerated from IR.
#[allow(dead_code)]
fn is_replace_part(uri: &str) -> bool {
    matches!(
        uri,
        "word/document.xml"
            | "word/header1.xml"
            | "word/header2.xml"
            | "word/header3.xml"
            | "word/footer1.xml"
            | "word/footer2.xml"
            | "word/footer3.xml"
            | "word/_rels/document.xml.rels"
    )
}

/// Check if a URI looks like a media part.
fn is_media_part(uri: &str) -> bool {
    uri.starts_with("word/media/")
}

/// Check if a URI is a rels file.
#[allow(dead_code)]
fn is_rels_part(uri: &str) -> bool {
    uri.ends_with(".rels")
}

// ===========================================================================
// Template drop budget
// ===========================================================================

/// A diagnostic describing a template element that is not used by the IR.
pub struct TemplateDrop {
    /// The template part URI.
    pub part_uri: String,
    /// Description of what was dropped.
    pub description: String,
}

/// Analyze which template parts are not used by the IR document.
///
/// This produces informational diagnostics about template elements that
/// are present in the template but not referenced by the IR content.
fn compute_template_drops(
    template_pkg: &crate::opc::OpcPackage,
    doc: &Document,
) -> Vec<TemplateDrop> {
    let mut drops: Vec<TemplateDrop> = Vec::new();

    // Collect which parts are actually used
    let mut used_parts: HashSet<String> = HashSet::new();
    used_parts.insert("word/document.xml".to_string());

    // Check sections for headers/footers
    let mut has_footnotes = false;
    let mut has_endnotes = false;
    let mut has_numbering = false;

    for section in &doc.sections {
        if !section.headers.is_empty() {
            used_parts.insert("word/header1.xml".to_string());
        }
        if !section.footers.is_empty() {
            used_parts.insert("word/footer1.xml".to_string());
        }
        if !section.footnotes.is_empty() {
            has_footnotes = true;
        }
        if !section.endnotes.is_empty() {
            has_endnotes = true;
        }
        for item in &section.items {
            match item {
                SectionItem::List(_) => {
                    has_numbering = true;
                    used_parts.insert("word/numbering.xml".to_string());
                }
                SectionItem::Drawing(_) => {}
                _ => {}
            }
        }
    }

    // Check for images in the document
    let mut has_images = false;
    for section in &doc.sections {
        for item in &section.items {
            if matches!(item, SectionItem::Drawing(_)) {
                has_images = true;
                break;
            }
        }
        if has_images {
            break;
        }
    }

    // Check template parts against what's used
    for part in &template_pkg.parts {
        let uri = &part.uri;

        // Check preserve-identical parts
        if is_preserve_part(uri) && !used_parts.contains(uri.as_str()) {
            // These parts exist in template but aren't needed by IR content.
            // However, styles.xml, theme, and fontTable are typically needed
            // by any Word document even with empty content.
            match uri.as_str() {
                "word/footnotes1.xml" if !has_footnotes => {
                    drops.push(TemplateDrop {
                        part_uri: uri.clone(),
                        description: "Template footnotes not referenced by IR document".to_string(),
                    });
                }
                "word/endnotes1.xml" if !has_endnotes => {
                    drops.push(TemplateDrop {
                        part_uri: uri.clone(),
                        description: "Template endnotes not referenced by IR document".to_string(),
                    });
                }
                "word/numbering.xml" if !has_numbering => {
                    drops.push(TemplateDrop {
                        part_uri: uri.clone(),
                        description: "Template numbering definitions not needed — no lists in IR"
                            .to_string(),
                    });
                }
                "word/webSettings.xml" | "word/proxy.docprops" => {
                    // These are metadata files that are generally safe to keep
                }
                _ => {
                    // Styles, theme, fontTable are always used
                }
            }
        }

        // Report unused media parts
        if is_media_part(uri) && !has_images {
            drops.push(TemplateDrop {
                part_uri: uri.clone(),
                description: "Template media part not referenced by IR document".to_string(),
            });
        }
    }

    drops
}

// ===========================================================================
// Public API
// ===========================================================================

/// Create a docx from an IR `Document` using a template as the base package.
///
/// The template is parsed as an OPC package. Parts that should be preserved
/// byte-for-byte (styles, theme, fontTable, etc.) are copied from the template.
/// Content parts (document.xml, headers, footers) are replaced with IR-derived
/// content.
///
/// Returns a diagnostic-aware result: the output bytes plus any template drop
/// warnings.
pub fn create_from_template(
    template_bytes: &[u8],
    doc: &Document,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    // Parse the template package
    let template_pkg = crate::opc::parse_opc(template_bytes)?;

    // Compute template drops (informational)
    let _drops = compute_template_drops(&template_pkg, doc);

    // Build the output parts
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();

    // Track which template parts we've already copied
    let mut template_uri_set: HashSet<String> = HashSet::new();
    for part in &template_pkg.parts {
        template_uri_set.insert(part.uri.clone());
    }

    // 1. Preserve template parts that should stay byte-identical
    for part in &template_pkg.parts {
        if is_preserve_part(&part.uri) {
            parts.push((part.uri.clone(), part.data.clone()));
        }
    }

    // 2. Generate content parts from IR
    let mut diags: Vec<Diagnostic> = Vec::new();

    // Collect media parts from document
    let mut media_parts: Vec<(String, Vec<u8>, String)> = Vec::new();
    let mut hyperlinks: Vec<(String, String)> = Vec::new();
    let mut rel_counter: usize = 5;

    for section in &doc.sections {
        for item in &section.items {
            match item {
                SectionItem::Drawing(d) => {
                    let (uri, data, mime) = create::build_media_part(d);
                    media_parts.push((uri, data, mime));
                }
                SectionItem::Paragraph(p) => {
                    for run in &p.runs {
                        if let Some(url) = detect_hyperlink(&run.text) {
                            hyperlinks.push((format!("rId{}", rel_counter), url));
                            rel_counter += 1;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // 3. Build [Content_Types].xml (may need to merge with template types)
    let content_types_xml = build_content_types_for_template(&media_parts, &template_pkg);
    parts.push(("[Content_Types].xml".to_string(), content_types_xml));

    // 4. Build _rels/.rels (from template or minimal default)
    let rels_xml = build_package_rels_from_template(&template_pkg);
    parts.push(("_rels/.rels".to_string(), rels_xml));

    // 5. Build word/document.xml from IR
    let has_headers = doc.sections.iter().any(|s| !s.headers.is_empty());
    let has_footers = doc.sections.iter().any(|s| !s.footers.is_empty());
    let document_xml = create::build_document_xml(
        doc,
        &mut diags,
        &hyperlinks,
        has_headers,
        has_footers,
        &media_parts,
    );
    parts.push(("word/document.xml".to_string(), document_xml));

    // 6. Build word/_rels/document.xml.rels from IR
    let has_footnotes = doc.sections.iter().any(|s| !s.footnotes.is_empty());
    let has_endnotes = doc.sections.iter().any(|s| !s.endnotes.is_empty());
    let rels_data = create::build_document_rels(
        &media_parts,
        &hyperlinks,
        has_headers,
        has_footers,
        has_footnotes,
        has_endnotes,
        rel_counter,
    );
    parts.push(("word/_rels/document.xml.rels".to_string(), rels_data));

    // 7. Preserve word/settings.xml from template if present, otherwise use default
    if let Some(setting_part) = template_pkg
        .parts
        .iter()
        .find(|p| p.uri == "word/settings.xml")
    {
        parts.push(("word/settings.xml".to_string(), setting_part.data.clone()));
    } else {
        parts.push(("word/settings.xml".to_string(), create::build_settings()));
    }

    // 8. Header/footer parts from IR (only if the IR defines them)
    let mut header_items: Vec<String> = Vec::new();
    let mut footer_items: Vec<String> = Vec::new();

    for section in &doc.sections {
        for h in &section.headers {
            header_items.push(create::build_header_xml(h));
        }
        for f in &section.footers {
            footer_items.push(create::build_footer_xml(f));
        }
    }

    if !header_items.is_empty() {
        parts.push((
            "word/header1.xml".to_string(),
            create::build_header_part_xml(&header_items.join("\n")),
        ));
    }
    if !footer_items.is_empty() {
        parts.push((
            "word/footer1.xml".to_string(),
            create::build_footer_part_xml(&footer_items.join("\n")),
        ));
    }

    // 9. Media parts
    for (uri, data, _mime) in &media_parts {
        parts.push((uri.clone(), data.clone()));
    }

    for diag in &diags {
        if diag.severity == Severity::Warning {
            eprintln!("CreateFromTemplate: {}", diag.message);
        }
    }

    // Package into OPC ZIP
    Ok(pack_opc(&parts))
}

/// Create a brand-new docx from an IR `Document`, falling back to default parts.
pub fn create_minimal_docx(doc: &Document) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let mut diags: Vec<Diagnostic> = Vec::new();
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();

    // Collect media parts and hyperlinks from document
    let mut media_parts: Vec<(String, Vec<u8>, String)> = Vec::new();
    let mut hyperlinks: Vec<(String, String)> = Vec::new();
    let mut rel_counter: usize = 5;

    for section in &doc.sections {
        for item in &section.items {
            match item {
                SectionItem::Drawing(d) => {
                    let (uri, data, mime) = create::build_media_part(d);
                    media_parts.push((uri, data, mime));
                }
                SectionItem::Paragraph(p) => {
                    for run in &p.runs {
                        if let Some(url) = detect_hyperlink(&run.text) {
                            hyperlinks.push((format!("rId{}", rel_counter), url));
                            rel_counter += 1;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    parts.push((
        "[Content_Types].xml".to_string(),
        create::build_content_types(&media_parts),
    ));
    parts.push(("_rels/.rels".to_string(), create::build_rels()));

    // Collect header/footer data
    let mut header_items: Vec<String> = Vec::new();
    let mut footer_items: Vec<String> = Vec::new();
    let mut has_headers = false;
    let mut has_footers = false;

    for section in &doc.sections {
        for h in &section.headers {
            has_headers = true;
            header_items.push(create::build_header_xml(h));
        }
        for f in &section.footers {
            has_footers = true;
            footer_items.push(create::build_footer_xml(f));
        }
    }

    let document_xml = create::build_document_xml(
        doc,
        &mut diags,
        &hyperlinks,
        has_headers,
        has_footers,
        &media_parts,
    );
    parts.push(("word/document.xml".to_string(), document_xml));

    let has_footnotes = doc.sections.iter().any(|s| !s.footnotes.is_empty());
    let has_endnotes = doc.sections.iter().any(|s| !s.endnotes.is_empty());
    let rels_data = create::build_document_rels(
        &media_parts,
        &hyperlinks,
        has_headers,
        has_footers,
        has_footnotes,
        has_endnotes,
        rel_counter,
    );
    parts.push(("word/_rels/document.xml.rels".to_string(), rels_data));

    // Standard parts
    parts.push(("word/styles.xml".to_string(), create::build_styles()));
    parts.push(("word/settings.xml".to_string(), create::build_settings()));
    parts.push(("word/fontTable.xml".to_string(), create::build_font_table()));
    parts.push(("word/theme/theme1.xml".to_string(), create::build_theme()));

    let has_lists = doc
        .sections
        .iter()
        .any(|s| s.items.iter().any(|i| matches!(i, SectionItem::List(_))));
    if has_lists {
        parts.push((
            "word/numbering.xml".to_string(),
            create::build_numbering_xml(doc),
        ));
    }

    if has_headers {
        parts.push((
            "word/header1.xml".to_string(),
            create::build_header_part_xml(&header_items.join("\n")),
        ));
    }
    if has_footers {
        parts.push((
            "word/footer1.xml".to_string(),
            create::build_footer_part_xml(&footer_items.join("\n")),
        ));
    }

    // Package
    Ok(pack_opc(&parts))
}

// ===========================================================================
// OPC packaging
// ===========================================================================

/// Pack a list of parts into an OPC ZIP package.
fn pack_opc(parts: &[(String, Vec<u8>)]) -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut zip_writer = zip::ZipWriter::new(cursor);
    let stored_opts: zip::write::FileOptions<'static, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (uri, data) in parts {
        let _ = zip_writer.start_file(uri, stored_opts);
        let _ = zip_writer.write_all(data);
    }
    let cursor = zip_writer.finish().expect("ZIP finish");
    cursor.into_inner()
}

// ===========================================================================
// XML helpers
// ===========================================================================

/// Escape XML special characters in text content.
#[allow(dead_code)]
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Detect if text looks like a URL hyperlink.
fn detect_hyperlink(text: &str) -> Option<String> {
    let t = text.trim();
    if t.starts_with("http://") || t.starts_with("https://") || t.starts_with("mailto:") {
        Some(t.to_string())
    } else {
        None
    }
}

// ===========================================================================
// Content types for template-based creation
// ===========================================================================

/// Build [Content_Types].xml that preserves template type overrides
/// and adds any new ones needed for IR-derived content.
fn build_content_types_for_template(
    media_parts: &[(String, Vec<u8>, String)],
    template_pkg: &crate::opc::OpcPackage,
) -> Vec<u8> {
    // Collect existing overrides from template content types
    let mut existing: BTreeMap<String, String> = BTreeMap::new();

    // Also try to parse from the template's [Content_Types].xml
    let template_content_types = template_pkg
        .parts
        .iter()
        .find(|p| p.uri == "[Content_Types].xml")
        .map(|p| parse_content_types_xml(&p.data))
        .unwrap_or_default();

    for (k, v) in &template_content_types {
        existing.insert(k.clone(), v.clone());
    }

    // Merge with the default content types
    let defaults = build_default_content_types(media_parts);
    for (k, v) in defaults {
        existing.entry(k).or_insert(v);
    }

    // Serialize back to XML
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
    );

    // Add Default extension entries first
    for (ext, ct) in &existing {
        if ext.starts_with('/') && ext.len() <= 10 {
            xml.push_str(&format!(
                "\n<Default Extension=\"{}\" ContentType=\"{}\"/>",
                ext.strip_prefix('/').unwrap(),
                ct
            ));
        }
    }

    // Add Override entries
    for (uri, ct) in &existing {
        if !uri.starts_with('/') || uri.len() > 10 {
            xml.push_str(&format!(
                "\n<Override PartName=\"{}\" ContentType=\"{}\"/>",
                uri, ct
            ));
        }
    }

    xml.push_str("\n</Types>");
    xml.into_bytes()
}

/// Parse content types XML into a BTreeMap.
fn parse_content_types_xml(data: &[u8]) -> BTreeMap<String, String> {
    let mut result: BTreeMap<String, String> = BTreeMap::new();
    let content = String::from_utf8_lossy(data);

    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.contains("<Override") && !trimmed.contains("<Default") {
            continue;
        }

        if trimmed.contains("<Override") {
            let path = extract_attr(trimmed, "PartName").or_else(|| extract_attr(trimmed, "Path"));
            if let (Some(path), Some(ct)) = (path, extract_attr(trimmed, "ContentType")) {
                let normalized = path.strip_prefix('/').unwrap_or(&path).to_string();
                result.insert(normalized, ct);
            }
        }

        if trimmed.contains("<Default") {
            if let (Some(ext), Some(ct)) = (
                extract_attr(trimmed, "Extension"),
                extract_attr(trimmed, "ContentType"),
            ) {
                result.insert(format!("/{}", ext), ct);
            }
        }
    }

    result
}

/// Extract an attribute value from an XML tag string.
fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let attr_pat = format!("{}=\"", attr);
    let start = tag.find(&attr_pat)?;
    let after_eq = &tag[start + attr_pat.len()..];
    let end = after_eq.find('"')?;
    Some(after_eq[..end].to_string())
}

/// Build the default set of content types needed for a Word document.
fn build_default_content_types(
    media_parts: &[(String, Vec<u8>, String)],
) -> BTreeMap<String, String> {
    let mut result: BTreeMap<String, String> = BTreeMap::new();

    // Defaults
    result.insert(
        "/rels".to_string(),
        "application/vnd.openxmlformats-package.relationships+xml".to_string(),
    );
    result.insert("/xml".to_string(), "application/xml".to_string());

    // Image defaults
    for (_uri, _data, mime) in media_parts {
        let ext = match mime.as_str() {
            "image/png" => "/png",
            "image/jpeg" | "image/jpg" => "/jpeg",
            "image/gif" => "/gif",
            "image/bmp" => "/bmp",
            "image/webp" => "/webp",
            "image/tiff" | "image/tif" => "/tif",
            "image/svg+xml" => "/svg",
            _ => continue,
        };
        if !result.contains_key(ext) {
            result.insert(ext.to_string(), mime.clone());
        }
    }

    // Overrides for core parts
    result.insert(
        "word/document.xml".to_string(),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
            .to_string(),
    );
    result.insert(
        "word/styles.xml".to_string(),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml".to_string(),
    );
    result.insert(
        "word/settings.xml".to_string(),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml".to_string(),
    );
    result.insert(
        "word/fontTable.xml".to_string(),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml".to_string(),
    );
    result.insert(
        "word/theme/theme1.xml".to_string(),
        "application/vnd.openxmlformats-officedocument.theme+xml".to_string(),
    );
    result.insert(
        "word/numbering.xml".to_string(),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml".to_string(),
    );
    result.insert(
        "word/header1.xml".to_string(),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml".to_string(),
    );
    result.insert(
        "word/footer1.xml".to_string(),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml".to_string(),
    );
    result.insert(
        "word/footnotes1.xml".to_string(),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml".to_string(),
    );
    result.insert(
        "word/endnotes1.xml".to_string(),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml".to_string(),
    );

    // Media overrides
    for (uri, _data, _mime) in media_parts {
        let ext = uri.rsplit('.').next().unwrap_or("bin");
        let ct = match ext {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "bmp" => "image/bmp",
            "webp" => "image/webp",
            "tif" | "tiff" => "image/tiff",
            "svg" => "image/svg+xml",
            _ => "application/octet-stream",
        };
        result.insert(uri.clone(), ct.to_string());
    }

    result
}

/// Build package-level _rels/.rels from template or minimal default.
fn build_package_rels_from_template(template_pkg: &crate::opc::OpcPackage) -> Vec<u8> {
    // Try to preserve template _rels/.rels if present
    if let Some(rels_part) = template_pkg.parts.iter().find(|p| p.uri == "_rels/.rels") {
        return rels_part.data.clone();
    }

    // Minimal default
    let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;
    xml.as_bytes().to_vec()
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use gp_core::ir::*;

    use super::{compute_template_drops, create_from_template, create_minimal_docx};
    use crate::opc::parse_opc;

    fn read_zip_entry_str(archive: &mut zip::ZipArchive<Cursor<&Vec<u8>>>, name: &str) -> String {
        let mut buf = Vec::new();
        archive
            .by_name(name)
            .unwrap_or_else(|_| panic!("Entry '{}' should exist", name))
            .read_to_end(&mut buf)
            .unwrap_or_else(|_| panic!("Should read entry '{}'", name));
        String::from_utf8(buf).expect("Entry should be valid UTF-8")
    }

    /// Create a minimal template docx for testing.
    fn make_minimal_template() -> Vec<u8> {
        // Use create_minimal_docx to generate a valid template
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "template"),
                kind: SectionKind::Body,
                items: vec![],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            revisions: vec![],
            template: None,
        };
        create_minimal_docx(&doc).expect("should create template")
    }

    /// test_template_preserves_styles — create from template → styles.xml hash matches template.
    #[test]
    fn test_template_preserves_styles() {
        let template_bytes = make_minimal_template();

        // Read the styles.xml from the template
        let template_cursor = Cursor::new(&template_bytes);
        let mut template_archive = zip::ZipArchive::new(template_cursor).expect("valid template");
        let mut template_styles_buf = Vec::new();
        template_archive
            .by_name("word/styles.xml")
            .expect("template should have styles.xml")
            .read_to_end(&mut template_styles_buf)
            .expect("should read styles");

        // Create from template
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "test"),
                kind: SectionKind::Body,
                items: vec![SectionItem::Paragraph(Paragraph {
                    id: ElementId::content_anchored("paragraph", &["para:0"], "test"),
                    runs: vec![Run {
                        id: ElementId::content_anchored("run", &["run:0"], "test"),
                        text: "Hello from template".to_string(),
                        bold: Some(true),
                        italic: None,
                        font_name: None,
                        font_size_pt: None,
                        underline: None,
                        strikethrough: None,
                        style_name: Some("Heading 1".to_string()),
                    }],
                    page_number: None,
                    style_name: Some("Heading 1".to_string()),
                    numbering_info: None,
                })],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            revisions: vec![],
            template: None,
        };

        let result =
            create_from_template(&template_bytes, &doc).expect("should create from template");
        let cursor = Cursor::new(&result);
        let mut archive = zip::ZipArchive::new(cursor).expect("valid result");

        // Read styles.xml from result
        let mut result_styles_buf = Vec::new();
        archive
            .by_name("word/styles.xml")
            .expect("result should have styles.xml")
            .read_to_end(&mut result_styles_buf)
            .expect("should read styles");

        // Styles should be byte-identical
        assert_eq!(
            template_styles_buf, result_styles_buf,
            "styles.xml should be byte-identical to template"
        );
    }

    /// test_template_preserves_theme — create from template → theme1.xml hash matches template.
    #[test]
    fn test_template_preserves_theme() {
        let template_bytes = make_minimal_template();

        // Read theme from template
        let template_cursor = Cursor::new(&template_bytes);
        let mut template_archive = zip::ZipArchive::new(template_cursor).expect("valid template");
        let mut template_theme_buf = Vec::new();
        template_archive
            .by_name("word/theme/theme1.xml")
            .expect("template should have theme")
            .read_to_end(&mut template_theme_buf)
            .expect("should read theme");

        // Create from template
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "test"),
                kind: SectionKind::Body,
                items: vec![],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            revisions: vec![],
            template: None,
        };

        let result =
            create_from_template(&template_bytes, &doc).expect("should create from template");
        let cursor = Cursor::new(&result);
        let mut archive = zip::ZipArchive::new(cursor).expect("valid result");

        let mut result_theme_buf = Vec::new();
        archive
            .by_name("word/theme/theme1.xml")
            .expect("result should have theme")
            .read_to_end(&mut result_theme_buf)
            .expect("should read theme");

        assert_eq!(
            template_theme_buf, result_theme_buf,
            "theme1.xml should be byte-identical to template"
        );
    }

    /// test_template_replaces_document — create from template → document.xml from IR.
    #[test]
    fn test_template_replaces_document() {
        let template_bytes = make_minimal_template();

        // IR with content
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "test"),
                kind: SectionKind::Body,
                items: vec![SectionItem::Paragraph(Paragraph {
                    id: ElementId::content_anchored("paragraph", &["para:0"], "test"),
                    runs: vec![Run {
                        id: ElementId::content_anchored("run", &["run:0"], "test"),
                        text: "Template replaced content".to_string(),
                        bold: Some(true),
                        italic: None,
                        font_name: None,
                        font_size_pt: None,
                        underline: None,
                        strikethrough: None,
                        style_name: None,
                    }],
                    page_number: None,
                    style_name: None,
                    numbering_info: None,
                })],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            revisions: vec![],
            template: None,
        };

        let result =
            create_from_template(&template_bytes, &doc).expect("should create from template");
        let cursor = Cursor::new(&result);
        let mut archive = zip::ZipArchive::new(cursor).expect("valid result");

        let doc_xml = read_zip_entry_str(&mut archive, "word/document.xml");
        assert!(
            doc_xml.contains("Template replaced content"),
            "document.xml should contain IR content"
        );
        assert!(doc_xml.contains("<w:b/>"), "Should have bold formatting");
    }

    /// test_template_budget_report — template has unused styles → budget reports drops.
    #[test]
    fn test_template_budget_report() {
        let template_bytes = make_minimal_template();

        // Parse template to get parts
        let template_pkg = parse_opc(&template_bytes).expect("valid template");

        // Document with no lists → numbering.xml should be reported as unused drop
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "test"),
                kind: SectionKind::Body,
                items: vec![],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            revisions: vec![],
            template: None,
        };

        // Force numbering in template to test drop reporting
        // Add a numbering part to template
        let template_pkg_with_numbering = crate::opc::OpcPackage {
            parts: {
                let mut parts = template_pkg.parts.clone();
                parts.push(crate::opc::Part {
                    uri: "word/numbering.xml".to_string(),
                    content_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml".to_string(),
                    data: b"<?xml version=\"1.0\"?><w:numbering/>".to_vec(),
                });
                parts
            },
            content_types: template_pkg.content_types.clone(),
            relationships: template_pkg.relationships.clone(),
            diagnostics: template_pkg.diagnostics.clone(),
        };

        let drops = compute_template_drops(&template_pkg_with_numbering, &doc);

        // Should report numbering.xml as unused
        assert!(
            drops.iter().any(|d| d.part_uri == "word/numbering.xml"),
            "Should report numbering.xml as unused drop"
        );
    }

    /// test_template_roundtrip — IR → template create → read back → verify IR structure.
    #[test]
    fn test_template_roundtrip() {
        let template_bytes = make_minimal_template();

        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "roundtrip"),
                kind: SectionKind::Body,
                items: vec![
                    SectionItem::Paragraph(Paragraph {
                        id: ElementId::content_anchored("paragraph", &["para:0"], "p1"),
                        runs: vec![Run {
                            id: ElementId::content_anchored("run", &["run:0"], "p1"),
                            text: "First paragraph".to_string(),
                            bold: None,
                            italic: None,
                            font_name: None,
                            font_size_pt: None,
                            underline: None,
                            strikethrough: None,
                            style_name: None,
                        }],
                        page_number: None,
                        style_name: None,
                        numbering_info: None,
                    }),
                    SectionItem::Paragraph(Paragraph {
                        id: ElementId::content_anchored("paragraph", &["para:1"], "p2"),
                        runs: vec![Run {
                            id: ElementId::content_anchored("run", &["run:1"], "p2"),
                            text: "Second paragraph".to_string(),
                            bold: Some(true),
                            italic: Some(true),
                            font_name: None,
                            font_size_pt: None,
                            underline: None,
                            strikethrough: None,
                            style_name: None,
                        }],
                        page_number: None,
                        style_name: None,
                        numbering_info: None,
                    }),
                ],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            revisions: vec![],
            template: None,
        };

        let result =
            create_from_template(&template_bytes, &doc).expect("should create from template");

        // Verify the result is a valid OPC package
        let cursor = Cursor::new(&result);
        let mut archive = zip::ZipArchive::new(cursor).expect("valid result");

        // Core parts must exist
        assert!(archive.by_name("[Content_Types].xml").is_ok());
        assert!(archive.by_name("_rels/.rels").is_ok());
        assert!(archive.by_name("word/document.xml").is_ok());

        // Content must be present
        let doc_xml = read_zip_entry_str(&mut archive, "word/document.xml");
        assert!(doc_xml.contains("First paragraph"));
        assert!(doc_xml.contains("Second paragraph"));
        assert!(doc_xml.contains("<w:b/>"));
        assert!(doc_xml.contains("<w:i/>"));

        // Styles must be preserved from template
        let mut template_styles = Vec::new();
        let tmpl_cursor = Cursor::new(&template_bytes);
        let mut tmpl_archive = zip::ZipArchive::new(tmpl_cursor).expect("valid template");
        tmpl_archive
            .by_name("word/styles.xml")
            .expect("template has styles")
            .read_to_end(&mut template_styles)
            .expect("read styles");

        let mut result_styles = Vec::new();
        archive
            .by_name("word/styles.xml")
            .expect("result has styles")
            .read_to_end(&mut result_styles)
            .expect("read styles");

        assert_eq!(
            template_styles, result_styles,
            "styles must be preserved byte-for-byte"
        );
    }

    /// test_template_with_headers_footers — headers/footers from IR replace template parts.
    #[test]
    fn test_template_with_headers_footers() {
        let template_bytes = make_minimal_template();

        let header = Header {
            id: ElementId::content_anchored("header", &["header:0"], "h"),
            content: vec![SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("paragraph", &["para:0"], "hdr"),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["run:0"], "hdr"),
                    text: "Template header".to_string(),
                    bold: None,
                    italic: None,
                    font_name: None,
                    font_size_pt: None,
                    underline: None,
                    strikethrough: None,
                    style_name: None,
                }],
                page_number: None,
                style_name: None,
                numbering_info: None,
            })],
            is_even_page: false,
        };

        let footer = Footer {
            id: ElementId::content_anchored("footer", &["footer:0"], "f"),
            content: vec![SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("paragraph", &["para:0"], "ftr"),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["run:0"], "ftr"),
                    text: "Template footer page".to_string(),
                    bold: None,
                    italic: None,
                    font_name: None,
                    font_size_pt: None,
                    underline: None,
                    strikethrough: None,
                    style_name: None,
                }],
                page_number: None,
                style_name: None,
                numbering_info: None,
            })],
            is_even_page: false,
        };

        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "hf"),
                kind: SectionKind::Body,
                items: vec![],
                headers: vec![header],
                footers: vec![footer],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            revisions: vec![],
            template: None,
        };

        let result =
            create_from_template(&template_bytes, &doc).expect("should create from template");
        let cursor = Cursor::new(&result);
        let mut archive = zip::ZipArchive::new(cursor).expect("valid result");

        assert!(
            archive.by_name("word/header1.xml").is_ok(),
            "Should have header"
        );
        assert!(
            archive.by_name("word/footer1.xml").is_ok(),
            "Should have footer"
        );

        let header_xml = read_zip_entry_str(&mut archive, "word/header1.xml");
        assert!(
            header_xml.contains("Template header"),
            "Header should contain IR content"
        );

        let footer_xml = read_zip_entry_str(&mut archive, "word/footer1.xml");
        assert!(
            footer_xml.contains("Template footer page"),
            "Footer should contain IR content"
        );
    }

    /// test_minimal_docx_fallback — create_minimal_docx works as before.
    #[test]
    fn test_minimal_docx_fallback() {
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "test"),
                kind: SectionKind::Body,
                items: vec![],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            revisions: vec![],
            template: None,
        };
        let result = create_minimal_docx(&doc).expect("should create minimal docx");
        assert!(!result.is_empty());

        let cursor = Cursor::new(&result);
        let mut archive = zip::ZipArchive::new(cursor).expect("valid ZIP");
        assert!(archive.by_name("word/document.xml").is_ok());
        assert!(archive.by_name("word/styles.xml").is_ok());
        assert!(archive.by_name("word/theme/theme1.xml").is_ok());
    }

    /// test_template_preserves_font_table — fontTable.xml byte-identical.
    #[test]
    fn test_template_preserves_font_table() {
        let template_bytes = make_minimal_template();

        // Read font table from template
        let template_cursor = Cursor::new(&template_bytes);
        let mut template_archive = zip::ZipArchive::new(template_cursor).expect("valid template");
        let mut template_fonttable = Vec::new();
        template_archive
            .by_name("word/fontTable.xml")
            .expect("template has fontTable")
            .read_to_end(&mut template_fonttable)
            .expect("read fontTable");

        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "test"),
                kind: SectionKind::Body,
                items: vec![],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            revisions: vec![],
            template: None,
        };

        let result =
            create_from_template(&template_bytes, &doc).expect("should create from template");
        let cursor = Cursor::new(&result);
        let mut archive = zip::ZipArchive::new(cursor).expect("valid result");

        let mut result_fonttable = Vec::new();
        archive
            .by_name("word/fontTable.xml")
            .expect("result has fontTable")
            .read_to_end(&mut result_fonttable)
            .expect("read fontTable");

        assert_eq!(
            template_fonttable, result_fonttable,
            "fontTable.xml should be byte-identical to template"
        );
    }
}
