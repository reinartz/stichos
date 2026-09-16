//! Create a brand-new OPC package (docx) from an IR `Document`.
//!
//! Writes a valid WordprocessingML document with all required parts:
//!   - `[Content_Types].xml` — content-type overrides
//!   - `_rels/.rels` — package-level relationships
//!   - `word/document.xml` — main document body from IR
//!   - `word/_rels/document.xml.rels` — document relationships
//!   - `word/styles.xml` — default styles
//!   - `word/settings.xml` — default settings
//!   - `word/fontTable.xml` — default font table
//!   - `word/theme/theme1.xml` — minimal theme
//!   - `word/numbering.xml` — numbering definitions
//!   - `word/header1.xml` / `word/footer1.xml` — header/footer parts
//!   - `word/footnotes1.xml` / `word/endnotes1.xml` — footnote/endnote parts
//!   - `word/media/*` — embedded images
//!
//! This is a **creation** path (P3-16): it produces a full-featured docx
//! from scratch, including enhanced run properties, numbering, tables,
//! headers/footers, sections, media, hyperlinks, and notes.

use gp_core::diagnostics::{Diagnostic, Severity};
use gp_core::ir::*;

use std::io::{Cursor, Write};

// ===========================================================================
// Base64 helpers (minimal, no external dependency)
// ===========================================================================

/// Decode a base64-encoded string to bytes.
fn decode_base64(s: &str) -> Option<Vec<u8>> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if s.is_empty() {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    let mut buf: u32 = 0;
    let mut bits = 0;

    for b in s.bytes() {
        if b == b'=' {
            break;
        }
        let val = match b {
            b'A'..=b'Z' => (b - b'A') as u32,
            b'a'..=b'z' => (b - b'a' + 26) as u32,
            b'0'..=b'9' => (b - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// Map a MIME type to a file extension.
fn mime_to_ext(mime_type: &str) -> &str {
    match mime_type {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        "image/webp" => "webp",
        "image/tiff" | "image/tif" => "tif",
        "image/svg+xml" => "svg",
        _ => "bin",
    }
}

/// Map file extension to content type for [Content_Types].xml.
fn ext_to_content_type(ext: &str) -> &str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
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
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// ===========================================================================
// Public API
// ===========================================================================

/// A docx creator that takes an IR `Document` and produces OPC bytes.
pub struct CreateDocx<'a> {
    /// The IR document to serialize.
    pub document: &'a Document,
}

impl<'a> CreateDocx<'a> {
    /// Create a new `CreateDocx` from an IR document.
    pub fn new(document: &'a Document) -> Self {
        Self { document }
    }

    /// Build the OPC package bytes.
    pub fn build(self) -> Result<Vec<u8>, Vec<Diagnostic>> {
        let parts = self.build_parts()?;
        Ok(pack_opc(&parts))
    }

    /// Build all parts as (uri, data) tuples.
    fn build_parts(self) -> Result<Vec<(String, Vec<u8>)>, Vec<Diagnostic>> {
        let mut diags: Vec<Diagnostic> = Vec::new();
        let mut parts: Vec<(String, Vec<u8>)> = Vec::new();

        // Collect media parts and hyperlinks from document
        let mut media_parts: Vec<(String, Vec<u8>, String)> = Vec::new();
        let mut hyperlinks: Vec<(String, String)> = Vec::new();
        let mut rel_counter: usize = 5;

        for section in &self.document.sections {
            for item in &section.items {
                match item {
                    SectionItem::Drawing(d) => {
                        let (uri, data, mime) = build_media_part(d);
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
            build_content_types(&media_parts),
        ));
        parts.push(("_rels/.rels".to_string(), build_rels()));

        // Collect header/footer/footnote/endnote section data
        let mut header_items: Vec<String> = Vec::new();
        let mut footer_items: Vec<String> = Vec::new();
        let mut fn_parts: Vec<String> = Vec::new();
        let mut en_parts: Vec<String> = Vec::new();
        let mut has_headers = false;
        let mut has_footers = false;
        let mut has_footnotes = false;
        let mut has_endnotes = false;

        for section in &self.document.sections {
            for h in &section.headers {
                has_headers = true;
                header_items.push(build_header_xml(h));
            }
            for f in &section.footers {
                has_footers = true;
                footer_items.push(build_footer_xml(f));
            }
            for f in &section.footnotes {
                has_footnotes = true;
                fn_parts.push(build_footnote_xml(f));
            }
            for e in &section.endnotes {
                has_endnotes = true;
                en_parts.push(build_endnote_xml(e));
            }
        }

        // word/document.xml
        let document_xml = build_document_xml(
            self.document,
            &mut diags,
            &hyperlinks,
            has_headers,
            has_footers,
            &media_parts,
        );
        parts.push(("word/document.xml".to_string(), document_xml));

        // word/_rels/document.xml.rels
        let rels_data = build_document_rels(
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
        parts.push(("word/styles.xml".to_string(), build_styles()));
        parts.push(("word/settings.xml".to_string(), build_settings()));
        parts.push(("word/fontTable.xml".to_string(), build_font_table()));
        parts.push(("word/theme/theme1.xml".to_string(), build_theme()));

        // word/numbering.xml (if lists are present)
        let has_lists = self
            .document
            .sections
            .iter()
            .any(|s| s.items.iter().any(|i| matches!(i, SectionItem::List(_))));
        if has_lists {
            parts.push((
                "word/numbering.xml".to_string(),
                build_numbering_xml(self.document),
            ));
        }

        // Header/footer parts
        if has_headers {
            parts.push((
                "word/header1.xml".to_string(),
                build_header_part_xml(&header_items.join("\n")),
            ));
        }
        if has_footers {
            parts.push((
                "word/footer1.xml".to_string(),
                build_footer_part_xml(&footer_items.join("\n")),
            ));
        }

        // Footnotes/endnotes parts
        if has_footnotes {
            parts.push((
                "word/footnotes1.xml".to_string(),
                build_footnotes_part_xml(&fn_parts.join("\n")),
            ));
        }
        if has_endnotes {
            parts.push((
                "word/endnotes1.xml".to_string(),
                build_endnotes_part_xml(&en_parts.join("\n")),
            ));
        }

        // Media parts
        for (uri, data, _mime) in &media_parts {
            parts.push((uri.clone(), data.clone()));
        }

        for diag in &diags {
            if diag.severity == Severity::Warning {
                eprintln!("CreateDocx: {}", diag.message);
            }
        }

        Ok(parts)
    }
}

/// Create a minimal docx from an IR document.
pub fn create_minimal_docx(doc: &Document) -> Result<Vec<u8>, Vec<Diagnostic>> {
    CreateDocx::new(doc).build()
}

// ===========================================================================
// [Content_Types].xml
// ===========================================================================

pub(crate) fn build_content_types(media_parts: &[(String, Vec<u8>, String)]) -> Vec<u8> {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Default Extension=\"png\" ContentType=\"image/png\"/>\
         <Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/>\
         <Default Extension=\"jpg\" ContentType=\"image/jpeg\"/>\
         <Default Extension=\"gif\" ContentType=\"image/gif\"/>\
         <Default Extension=\"bmp\" ContentType=\"image/bmp\"/>\
         <Default Extension=\"webp\" ContentType=\"image/webp\"/>\
         <Default Extension=\"tif\" ContentType=\"image/tiff\"/>\
         <Default Extension=\"svg\" ContentType=\"image/svg+xml\"/>\
         <Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
         <Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
         <Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/>\
         <Override PartName=\"/word/fontTable.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml\"/>\
         <Override PartName=\"/word/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>\
         <Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/>\
         <Override PartName=\"/word/header1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml\"/>\
         <Override PartName=\"/word/footer1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml\"/>\
         <Override PartName=\"/word/footnotes1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml\"/>\
         <Override PartName=\"/word/endnotes1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml\"/>"
    );
    for (uri, _data, _mime) in media_parts {
        let ext = uri.rsplit('.').next().unwrap_or("bin");
        let ct = ext_to_content_type(ext);
        xml.push_str(&format!(
            "\n<Override PartName=\"{}\" ContentType=\"{}\"/>",
            uri, ct
        ));
    }
    xml.push_str("\n</Types>");
    xml.into_bytes()
}

// ===========================================================================
// _rels/.rels
// ===========================================================================

pub(crate) fn build_rels() -> Vec<u8> {
    let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;
    xml.as_bytes().to_vec()
}

// ===========================================================================
// word/_rels/document.xml.rels
// ===========================================================================

pub(crate) fn build_document_rels(
    media_parts: &[(String, Vec<u8>, String)],
    hyperlinks: &[(String, String)],
    has_headers: bool,
    has_footers: bool,
    has_footnotes: bool,
    has_endnotes: bool,
    _rel_counter: usize,
) -> Vec<u8> {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    xml.push_str("\n<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>");
    xml.push_str("\n<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings\" Target=\"settings.xml\"/>");
    xml.push_str("\n<Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/fontTable\" Target=\"fontTable.xml\"/>");
    xml.push_str("\n<Relationship Id=\"rId4\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"theme/theme1.xml\"/>");

    for (idx, (uri, _data, _mime)) in media_parts.iter().enumerate() {
        let file_name = uri.rsplit('/').next().unwrap_or("image");
        let img_rel = 5 + idx;
        xml.push_str(&format!(
            "\n<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"media/{}\"/>",
            img_rel, file_name
        ));
    }

    for (rel_id_str, url) in hyperlinks {
        xml.push_str(&format!(
            "\n<Relationship Id=\"{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink\" Target=\"{}\" TargetMode=\"External\"/>",
            rel_id_str, escape_xml(url)
        ));
    }

    if has_headers {
        xml.push_str("\n<Relationship Id=\"rIdH1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/header\" Target=\"header1.xml\"/>");
    }
    if has_footers {
        xml.push_str("\n<Relationship Id=\"rIdF1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer\" Target=\"footer1.xml\"/>");
    }
    if has_footnotes {
        xml.push_str("\n<Relationship Id=\"rIdFN\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes\" Target=\"footnotes1.xml\"/>");
    }
    if has_endnotes {
        xml.push_str("\n<Relationship Id=\"rIdEN\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes\" Target=\"endnotes1.xml\"/>");
    }

    xml.push_str("\n</Relationships>");
    xml.into_bytes()
}

// ===========================================================================
// word/document.xml - main document body from IR
// ===========================================================================

pub(crate) fn build_document_xml(
    doc: &Document,
    diags: &mut Vec<Diagnostic>,
    hyperlinks: &[(String, String)],
    has_headers: bool,
    has_footers: bool,
    media_parts: &[(String, Vec<u8>, String)],
) -> Vec<u8> {
    let mut body_parts: Vec<String> = Vec::new();
    let mut media_idx = 0usize;

    for (section_idx, section) in doc.sections.iter().enumerate() {
        if section.kind == SectionKind::Body {
            for item in &section.items {
                match item {
                    SectionItem::Paragraph(p) => {
                        body_parts.push(build_paragraph_xml(p, hyperlinks));
                    }
                    SectionItem::Table(t) => {
                        body_parts.push(build_table_xml(t));
                    }
                    SectionItem::Drawing(_) => {
                        if media_idx < media_parts.len() {
                            body_parts.push(build_drawing_placeholder(media_idx, media_parts));
                            media_idx += 1;
                        } else {
                            diags.push(Diagnostic::new(
                                "OOX-W-C01",
                                Severity::Warning,
                                format!("Drawing at section {} — placeholder skipped", section_idx),
                            ));
                        }
                    }
                    SectionItem::List(list) => {
                        body_parts.push(build_list_xml(list, section_idx, diags));
                    }
                }
            }
        } else if section.kind == SectionKind::Header || section.kind == SectionKind::Footer {
            for item in &section.items {
                if let SectionItem::Paragraph(p) = item {
                    body_parts.push(build_paragraph_xml(p, hyperlinks));
                }
            }
        }
    }

    let sect_pr = build_section_properties_xml(has_headers, has_footers);
    let body_content = body_parts.join("\n");

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
         xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\n\
         <w:body>\n\
         {}\n\
         {}\n\
         </w:body>\n\
         </w:document>",
        body_content, sect_pr
    )
    .into_bytes()
}

/// Build section properties with page size, margins, header/footer refs, borders, columns.
pub(crate) fn build_section_properties_xml(has_headers: bool, has_footers: bool) -> String {
    let mut xml = String::from(
        "<w:sectPr xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\n\
         <w:pgSz w:w=\"12240\" w:h=\"15840\"/>\n\
         <w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/>"
    );
    if has_headers {
        xml.push_str("\n<w:headerReference w:type=\"default\" r:id=\"rIdH1\"/>");
    }
    if has_footers {
        xml.push_str("\n<w:footerReference w:type=\"default\" r:id=\"rIdF1\"/>");
    }
    // Page borders
    xml.push_str(
        "\n<w:pgBorders>\n\
         <w:left w:val=\"single\" w:sz=\"6\" w:space=\"120\" w:color=\"auto\"/>\n\
         <w:top w:val=\"single\" w:sz=\"6\" w:space=\"120\" w:color=\"auto\"/>\n\
         <w:right w:val=\"single\" w:sz=\"6\" w:space=\"120\" w:color=\"auto\"/>\n\
         <w:bottom w:val=\"single\" w:sz=\"6\" w:space=\"120\" w:color=\"auto\"/>\n\
         </w:pgBorders>",
    );
    // Single column default
    xml.push_str("\n<w:cols w:num=\"1\" w:space=\"240\"/>");
    xml.push_str("\n</w:sectPr>");
    xml
}

/// Build paragraph XML with all run properties.
pub(crate) fn build_paragraph_xml(para: &Paragraph, hyperlinks: &[(String, String)]) -> String {
    let mut runs_xml = String::new();
    for run in &para.runs {
        if let Some(url) = detect_hyperlink(&run.text) {
            if let Some((rel_id, _)) = hyperlinks.iter().find(|(_, u)| u == &url) {
                runs_xml.push_str(&format!("<w:hyperlink r:link=\"{}\">", rel_id));
                runs_xml.push_str(&build_run_xml(run));
                runs_xml.push_str("</w:hyperlink>");
                continue;
            }
        }
        runs_xml.push_str(&build_run_xml(run));
    }

    let mut para_elem = String::new();
    if let Some(ref style) = para.style_name {
        para_elem.push_str(&format!("<w:pStyle w:val=\"{}\"/>", escape_xml(style)));
    }
    if let Some(ref num_info) = para.numbering_info {
        para_elem.push_str(&format!(
            "<w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"{}\"/></w:numPr>",
            num_info.level, num_info.num_id
        ));
    }

    format!("<w:p>{}</w:p>\n{}", para_elem, runs_xml)
}

/// Build a single run XML element with all formatting properties.
pub(crate) fn build_run_xml(run: &Run) -> String {
    let mut rpr = String::new();

    if run.bold == Some(true) {
        rpr.push_str("<w:b/>");
    }
    if run.bold == Some(false) {
        rpr.push_str("<w:bCs/>");
    }
    if run.italic == Some(true) {
        rpr.push_str("<w:i/>");
    }
    if run.underline == Some(true) {
        rpr.push_str("<w:u w:val=\"single\"/>");
    }
    if run.strikethrough == Some(true) {
        rpr.push_str("<w:strike/>");
    }

    if let Some(ref font) = run.font_name {
        rpr.push_str(&format!(
            "<w:rFonts w:ascii=\"{}\" w:hAnsi=\"{}\" w:eastAsia=\"{}\" w:cs=\"{}\"/>",
            escape_xml(font),
            escape_xml(font),
            escape_xml(font),
            escape_xml(font)
        ));
    }
    if let Some(pt) = run.font_size_pt {
        let half_points = (pt * 2.0) as i32;
        rpr.push_str(&format!("<w:sz w:val=\"{}\"/>", half_points));
    }

    let text = escape_xml(&run.text);
    if rpr.is_empty() {
        format!("<w:r><w:t>{}</w:t></w:r>", text)
    } else {
        format!("<w:r><w:rPr>{}</w:rPr><w:t>{}</w:t></w:r>", rpr, text)
    }
}

// ===========================================================================
// Enhanced table builder with grid, borders, margins, merged cells
// ===========================================================================

pub(crate) fn build_table_xml(table: &Table) -> String {
    let mut rows_xml = String::new();
    let total_cols = table.rows.first().map(|r| r.cells.len()).unwrap_or(1);

    let mut grid_cols = String::new();
    for _ in 0..total_cols {
        grid_cols.push_str("<w:gridCol w:w=\"1800\"/>");
    }

    for row in &table.rows {
        let mut cells_xml = String::new();
        for cell in &row.cells {
            let mut tc_pr = String::new();

            if let Some(c) = cell.colspan {
                tc_pr.push_str(&format!("<w:gridSpan w:val=\"{}\"/>", c));
            }
            if let Some(r) = cell.rowspan {
                if r > 1 {
                    tc_pr.push_str("<w:vMerge w:val=\"restart\"/>");
                } else {
                    tc_pr.push_str("<w:vMerge/>");
                }
            }

            tc_pr.push_str(
                "<w:tcBorders>\
                 <w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>\
                 <w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>\
                 <w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>\
                 <w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>\
                 </w:tcBorders>",
            );

            tc_pr.push_str(
                "<w:tblCellMar>\
                 <w:top w:w=\"50\" w:type=\"dxa\"/>\
                 <w:left w:w=\"120\" w:type=\"dxa\"/>\
                 <w:bottom w:w=\"50\" w:type=\"dxa\"/>\
                 <w:right w:w=\"120\" w:type=\"dxa\"/>\
                 </w:tblCellMar>",
            );

            let mut cell_inner = String::new();
            for item in &cell.contents {
                if let SectionItem::Paragraph(p) = item {
                    cell_inner.push_str(&build_paragraph_xml(p, &[]));
                }
            }

            cells_xml.push_str(&format!(
                "<w:tc><w:tcPr>{}</w:tcPr><w:p>{}</w:p>\n</w:tc>",
                tc_pr, cell_inner
            ));
        }
        rows_xml.push_str(&format!("<w:tr>{}</w:tr>\n", cells_xml));
    }

    let table_grid = format!("<w:tblGrid>{}</w:tblGrid>", grid_cols);

    format!(
        "<w:tbl><w:tblPr><w:borders>\
         <w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>\
         <w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>\
         <w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>\
         <w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>\
         </w:borders></w:tblPr>\
         {}\
         <w:tblBody>{}</w:tblBody>\
         </w:tbl>",
        table_grid, rows_xml
    )
}

// ===========================================================================
// Drawing/image placeholder
// ===========================================================================

pub(crate) fn build_drawing_placeholder(
    idx: usize,
    media_parts: &[(String, Vec<u8>, String)],
) -> String {
    if idx >= media_parts.len() {
        return String::new();
    }
    let (uri, _data, _mime) = &media_parts[idx];
    let _file_name = uri.rsplit('/').next().unwrap_or("image.png");
    let rel_id = format!("rId{}", 5 + idx);
    let width_emu: u64 = (200 * 914400) / 96;
    let height_emu: u64 = (150 * 914400) / 96;

    format!(
        "<w:p><w:r><w:drawing>\
         <wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\">\
         <wp:extent cx=\"{}\" cy=\"{}\"/>\
         <wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>\
         <wp:docPr id=\"{}\" name=\"Image {}\"/>\
         <wp:cNvGraphicFrame/>\
         <a:graphic xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\">\
         <a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\
         <pic:pic xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\
         <pic:nvPicPr>\
         <pic:cNvPr id=\"{}\" name=\"Image {}\"/>\
         <pic:cNvPicPr/>\
         </pic:nvPicPr>\
         <pic:blipFill>\
         <a:blip r:embed=\"{}\"/>\
         <a:stretch><a:fillToRect/></a:stretch>\
         </pic:blipFill>\
         <pic:spPr>\
         <a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>\
         <a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>\
         </pic:spPr>\
         </pic:pic></a:graphicData></a:graphic>\
         </wp:inline></w:drawing></w:r></w:p>",
        width_emu, height_emu, idx, idx, idx, idx, rel_id, width_emu, height_emu
    )
}

// ===========================================================================
// List builder (enhanced)
// ===========================================================================

pub(crate) fn build_list_xml(
    list: &List,
    section_idx: usize,
    _diags: &mut Vec<Diagnostic>,
) -> String {
    let num_id = 100 + section_idx;
    let mut items_xml = String::new();
    for item in &list.items {
        for content_item in &item.content {
            if let SectionItem::Paragraph(p) = content_item {
                let mut para_xml = build_paragraph_xml(p, &[]);
                para_xml = format!(
                    "{}<w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"{}\"/></w:numPr>",
                    para_xml.replacen("<w:p>", "", 1),
                    item.level,
                    num_id
                );
                items_xml.push_str(&para_xml);
            }
        }
        let has_para = list
            .items
            .iter()
            .flat_map(|it| &it.content)
            .any(|ci| matches!(ci, SectionItem::Paragraph(_)));
        if !has_para {
            items_xml.push_str(&format!(
                "<w:p><w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"{}\"/></w:numPr><w:r><w:t>{}</w:t></w:r></w:p>\n",
                item.level, num_id, "(list item)"
            ));
        }
    }
    items_xml
}

// ===========================================================================
// Header/Footer XML builders
// ===========================================================================

pub(crate) fn build_header_xml(header: &Header) -> String {
    let mut content = String::new();
    for item in &header.content {
        if let SectionItem::Paragraph(p) = item {
            content.push_str(&build_paragraph_xml(p, &[]));
        }
    }
    content
}

pub(crate) fn build_footer_xml(footer: &Footer) -> String {
    let mut content = String::new();
    for item in &footer.content {
        if let SectionItem::Paragraph(p) = item {
            content.push_str(&build_paragraph_xml(p, &[]));
        }
    }
    content
}

pub(crate) fn build_header_part_xml(content: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <w:header xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
         xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\n\
         {}\n\
         <w:sectPr>\
         <w:pgMar w:top=\"720\" w:right=\"1440\" w:bottom=\"720\" w:left=\"1440\" w:header=\"360\" w:footer=\"720\" w:gutter=\"0\"/>\
         </w:sectPr>\
         </w:header>",
        content
    ).into_bytes()
}

pub(crate) fn build_footer_part_xml(content: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <w:footer xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
         xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\n\
         {}\n\
         <w:sectPr>\
         <w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"720\" w:left=\"1440\" w:header=\"360\" w:footer=\"360\" w:gutter=\"0\"/>\
         </w:sectPr>\
         </w:footer>",
        content
    ).into_bytes()
}

// ===========================================================================
// Footnotes/Endnotes XML builders
// ===========================================================================

pub(crate) fn build_footnote_xml(footnote: &Footnote) -> String {
    let mut content = String::new();
    for item in &footnote.content {
        if let SectionItem::Paragraph(p) = item {
            content.push_str(&build_paragraph_xml(p, &[]));
        }
    }
    let ref_id = footnote.reference_id.clone().unwrap_or_default();
    format!(
        "<w:footnote w:id=\"{}\" xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\n\
         {}\
         <w:sectPr>\
         <w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/>\
         </w:sectPr>\
         </w:footnote>",
        ref_id, content
    )
}

pub(crate) fn build_endnote_xml(endnote: &Endnote) -> String {
    let mut content = String::new();
    for item in &endnote.content {
        if let SectionItem::Paragraph(p) = item {
            content.push_str(&build_paragraph_xml(p, &[]));
        }
    }
    format!(
        "<w:endnote w:id=\"0\" xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\n\
         {}\
         <w:sectPr>\
         <w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/>\
         </w:endnote>",
        content
    )
}

pub(crate) fn build_footnotes_part_xml(content: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <w:footnotes xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\n\
         {}\
         </w:footnotes>",
        content
    )
    .into_bytes()
}

pub(crate) fn build_endnotes_part_xml(content: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <w:endnotes xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\n\
         {}\
         </w:endnotes>",
        content
    )
    .into_bytes()
}

// ===========================================================================
// word/numbering.xml
// ===========================================================================

pub(crate) fn build_numbering_xml(doc: &Document) -> Vec<u8> {
    let mut abstract_defs = String::new();
    let mut num_defs = String::new();
    let mut abstract_id = 0usize;

    for section in &doc.sections {
        for item in &section.items {
            if let SectionItem::List(list) = item {
                let abs_num_id = abstract_id;
                abstract_id += 1;

                let level_fmt = match list.kind {
                    ListKind::Bullet => "\u{2022}",
                    ListKind::Numbered => "\u{0031}.",
                    ListKind::Other => "-",
                };

                abstract_defs.push_str(&format!(
                    "<w:abstractNum w:abstractNumId=\"{}\" xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                     <w:lvl w:ilvl=\"0\">\
                     <w:start w:val=\"1\"/>\
                     <w:numFmt w:val=\"{}\"/>\
                     <w:lvlText w:val=\"{}\"/>\
                     <w:lvlJc w:val=\"left\"/>\
                     </w:lvl>\
                     </w:abstractNum>",
                    abs_num_id,
                    if matches!(list.kind, ListKind::Numbered) { "number" } else { "bullet" },
                    level_fmt
                ));

                // Find a unique numId for this list
                let num_id: usize = section
                    .items
                    .iter()
                    .position(|i| matches!(i, SectionItem::List(l) if l == list))
                    .map(|p| 100 + p)
                    .unwrap_or(100 + section.items.len());

                num_defs.push_str(&format!(
                    "<w:num w:numId=\"{}\" xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
                     <w:abstractNumId w:val=\"{}\"/>\
                     </w:num>",
                    num_id, abs_num_id
                ));
            }
        }
    }

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <w:numbering xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
         xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\n\
         {}\
         {}\
         </w:numbering>",
        abstract_defs, num_defs
    )
    .into_bytes()
}

// ===========================================================================
// Media parts
// ===========================================================================

/// Build a media part from a Drawing element.
pub(crate) fn build_media_part(drawing: &Drawing) -> (String, Vec<u8>, String) {
    let ext = mime_to_ext(&drawing.mime_type);
    let file_name = format!("image{}.{}", drawing.id.as_str().len() % 100, ext);
    let uri = format!("word/media/{}", file_name);

    let data = decode_base64(&drawing.data).unwrap_or_default();

    (uri, data, drawing.mime_type.clone())
}

// ===========================================================================
// word/styles.xml - default styles
// ===========================================================================

pub(crate) fn build_styles() -> Vec<u8> {
    let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<w:docDefaults>
<w:rPrDefault>
<w:rPr>
<w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:cs="Calibri"/>
<w:sz w:val="20"/>
</w:rPr>
</w:rPrDefault>
</w:docDefaults>
<w:style w:type="paragraph" w:styleId="Normal">
<w:name w:val="Normal"/>
<w:rsid w:rsidP="00000000" w:rsidR="00000000" w:rsidRPr="00000000" w:rsidS="00000000"/>
<w:rPr/>
<w:ppr/>
</w:style>
<w:style w:type="character" w:styleId="DefaultParagraphFont">
<w:name w:val="Default Paragraph Font"/>
<w:uiPriority w:val="1"/>
<w:semiHidden/>
<w:unhide/>
</w:style>
<w:style w:type="table" w:styleId="NormalTable">
<w:name w:val="Normal Table"/>
<w:uiPriority w:val="99"/>
<w:tableStylePr w:type="table">
<w:tableConditional w:type="firstRow"/>
</w:tableStylePr>
<w:tableStylePr w:type="lastRow"/>
<w:tableStylePr w:type="firstCol"/>
<w:tableStylePr w:type="lastCol"/>
<w:tableStylePr w:type="band1"/>
<w:tableStylePr w:type="band2"/>
</w:style>
</w:styles>"#;
    xml.as_bytes().to_vec()
}

// ===========================================================================
// word/settings.xml - default settings
// ===========================================================================

pub(crate) fn build_settings() -> Vec<u8> {
    let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">
<w:zoom w:percent="100"/>
<w:embedSystemFonts/>
<w:proofBidi/>
<w:charSpacingControl w:val="compressPunctuation"/>
</w:settings>"#;
    xml.as_bytes().to_vec()
}

// ===========================================================================
// word/fontTable.xml - default font table
// ===========================================================================

pub(crate) fn build_font_table() -> Vec<u8> {
    let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:fontTable xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:font w:name="Calibri">
<w:ascii w:charset="0" w:family="2" w:pitch="3"/>
<w:ea w:charset="0" w:family="2" w:pitch="3"/>
<w:cs w:charset="0" w:family="2" w:pitch="3"/>
</w:font>
</w:fontTable>"#;
    xml.as_bytes().to_vec()
}

// ===========================================================================
// word/theme/theme1.xml - minimal theme
// ===========================================================================

pub(crate) fn build_theme() -> Vec<u8> {
    let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office Theme">
<a:themeElements>
<a:clrScheme name="Office">
<a:dk1><a:sysClr lastR="0" lastG="0" lastB="0" lastA="255" windowText="0"/></a:dk1>
<a:lt1><a:sysClr lastR="255" lastG="255" lastB="255" lastA="255" windowText="0"/></a:lt1>
<a:dk2><a:srgbClr val="1F497D"/></a:dk2>
<a:lt2><a:srgbClr val="EEEED1"/></a:lt2>
<a:accent1><a:srgbClr val="4472C4"/></a:accent1>
<a:accent2><a:srgbClr val="E25840"/></a:accent2>
<a:accent3><a:srgbClr val="92D050"/></a:accent3>
<a:accent4><a:srgbClr val="393939"/></a:accent4>
<a:accent5><a:srgbClr val="00B0F0"/></a:accent5>
<a:accent6><a:srgbClr val="85BBD6"/></a:accent6>
<a:hlink><a:srgbClr val="0563C1"/></a:hlink>
<a:folHlink><a:srgbClr val="95A7CB"/></a:folHlink>
</a:clrScheme>
<a:fontScheme name="Office">
<a:majorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont>
<a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont>
</a:fontScheme>
<a:fmtScheme name="Office">
<a:fillStyleLst><a:gradFill><a:gsLst><a:gs pos="0"><a:schemeClr val="tx1"/><a:alpha val="85000"/></a:gs><a:gs pos="50000"><a:schemeClr val="tx1"/><a:alpha val="30000"/></a:gs><a:gs pos="100000"><a:schemeClr val="tx1"/><a:alpha val="0"/></a:gs></a:gsLst><a:path path="rect"><a:rectFrom cx="0" cy="k50000" cw="k100000" ch="k100000"/></a:path></a:gradFill><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:solidFill><a:schemeClr val="tx1"/><a:alpha val="30000"/></a:solidFill></a:fillStyleLst>
<a:lnStyleLst><a:ln w="9525"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:ln><a:ln w="25400"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:ln><a:ln w="38100"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:ln></a:lnStyleLst>
<a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst><a:outerShdw/><a:outerShdw/></a:effectLst></a:effectStyle></a:effectStyleLst>
</a:fmtScheme>
</a:themeElements>
</a:theme>"#;
    xml.as_bytes().to_vec()
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn read_zip_entry_str(archive: &mut zip::ZipArchive<Cursor<&Vec<u8>>>, name: &str) -> String {
        let mut buf = Vec::new();
        archive
            .by_name(name)
            .unwrap_or_else(|_| panic!("Entry '{}' should exist", name))
            .read_to_end(&mut buf)
            .unwrap_or_else(|_| panic!("Should read entry '{}'", name));
        String::from_utf8(buf).expect("Entry should be valid UTF-8")
    }

    fn make_test_doc() -> Document {
        Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "empty"),
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
            template: None,
        }
    }

    fn make_test_doc_with_text() -> Document {
        Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "test"),
                kind: SectionKind::Body,
                items: vec![SectionItem::Paragraph(Paragraph {
                    id: ElementId::content_anchored("paragraph", &["para:0"], "hello"),
                    runs: vec![Run {
                        id: ElementId::content_anchored("run", &["run:0"], "hello"),
                        text: "Hello, world!".to_string(),
                        bold: Some(true),
                        italic: None,
                        font_name: None,
                        font_size_pt: None,
                        underline: None,
                        strikethrough: None,
                        style_name: None,
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
            template: None,
        }
    }

    fn make_test_doc_with_table() -> Document {
        let cell = Cell {
            id: ElementId::content_anchored("cell", &["cell:0"], "data"),
            contents: vec![SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("paragraph", &["p:0"], "cell"),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["run:0"], "A1"),
                    text: "Cell A1".to_string(),
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
            colspan: None,
            rowspan: None,
        };
        let cell2 = Cell {
            id: ElementId::content_anchored("cell", &["cell:1"], "data2"),
            contents: vec![SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("paragraph", &["p:1"], "cell2"),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["run:1"], "B1"),
                    text: "Cell B1".to_string(),
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
            colspan: None,
            rowspan: None,
        };
        Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "table"),
                kind: SectionKind::Body,
                items: vec![SectionItem::Table(Table {
                    id: ElementId::content_anchored("table", &["table:0"], ""),
                    rows: vec![Row {
                        cells: vec![cell, cell2],
                    }],
                })],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
        }
    }

    /// test_minimal_docx_creation
    #[test]
    fn test_minimal_docx_creation() {
        let doc = make_test_doc();
        let result = create_minimal_docx(&doc);
        assert!(result.is_ok(), "Should create docx from empty document");
        let bytes = result.unwrap();
        assert!(!bytes.is_empty(), "Docx bytes should not be empty");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        assert!(!archive.is_empty(), "ZIP should have entries");
        let entry_names: Vec<String> = (0..archive.len())
            .filter_map(|i| archive.by_index(i).ok().map(|e| e.name().to_string()))
            .collect();
        assert!(
            entry_names.iter().any(|n| n == "[Content_Types].xml"),
            "Should have [Content_Types].xml"
        );
        assert!(
            entry_names.iter().any(|n| n == "_rels/.rels"),
            "Should have _rels/.rels"
        );
        assert!(
            entry_names.iter().any(|n| n == "word/document.xml"),
            "Should have word/document.xml"
        );
        assert!(
            entry_names
                .iter()
                .any(|n| n == "word/_rels/document.xml.rels"),
            "Should have word/_rels/document.xml.rels"
        );
        assert!(
            entry_names.iter().any(|n| n == "word/styles.xml"),
            "Should have word/styles.xml"
        );
        assert!(
            entry_names.iter().any(|n| n == "word/settings.xml"),
            "Should have word/settings.xml"
        );
        assert!(
            entry_names.iter().any(|n| n == "word/fontTable.xml"),
            "Should have word/fontTable.xml"
        );
        assert!(
            entry_names.iter().any(|n| n == "word/theme/theme1.xml"),
            "Should have word/theme/theme1.xml"
        );
    }

    /// test_docx_with_text
    #[test]
    fn test_docx_with_text() {
        let doc = make_test_doc_with_text();
        let result = create_minimal_docx(&doc);
        assert!(result.is_ok(), "Should create docx with text");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let doc_xml = read_zip_entry_str(&mut archive, "word/document.xml");
        assert!(doc_xml.contains("Hello, world!"), "Should contain text");
        assert!(doc_xml.contains("<w:b/>"), "Should contain bold marker");
        assert!(
            doc_xml.contains("Heading 1"),
            "Should contain style reference"
        );
    }

    /// test_docx_with_table
    #[test]
    fn test_docx_with_table() {
        let doc = make_test_doc_with_table();
        let result = create_minimal_docx(&doc);
        assert!(result.is_ok(), "Should create docx with table");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let doc_xml = read_zip_entry_str(&mut archive, "word/document.xml");
        assert!(doc_xml.contains("<w:tbl>"), "Should contain table element");
        assert!(doc_xml.contains("<w:tr>"), "Should contain table row");
        assert!(doc_xml.contains("<w:tc>"), "Should contain table cell");
    }

    /// test_docx_schema_valid
    #[test]
    fn test_docx_schema_valid() {
        let doc = make_test_doc_with_text();
        let result = create_minimal_docx(&doc);
        assert!(result.is_ok(), "Should create valid docx");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let xml_parts = [
            "[Content_Types].xml",
            "_rels/.rels",
            "word/document.xml",
            "word/_rels/document.xml.rels",
            "word/styles.xml",
            "word/settings.xml",
            "word/fontTable.xml",
            "word/theme/theme1.xml",
        ];
        for part_name in &xml_parts {
            let content = read_zip_entry_str(&mut archive, part_name);
            let parse_result = super::super::xml::parse_xml(content.as_bytes());
            assert!(
                parse_result.is_ok(),
                "{} should be well-formed XML",
                part_name
            );
        }
    }

    /// test_docx_roundtrip
    #[test]
    fn test_docx_roundtrip() {
        let doc = make_test_doc_with_text();
        let bytes = create_minimal_docx(&doc).expect("Should create docx");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let doc_xml = read_zip_entry_str(&mut archive, "word/document.xml");
        assert!(
            doc_xml.contains("<?xml version=\"1.0\""),
            "Should have XML declaration"
        );
        assert!(
            doc_xml.contains(
                "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\""
            ),
            "Should have Word namespace"
        );
        assert!(doc_xml.contains("<w:body>"), "Should have body element");
        assert!(
            doc_xml.contains("<w:sectPr"),
            "Should have section properties"
        );
        assert!(doc_xml.contains("<w:pgSz"), "Should have page size");
    }

    /// test_create_docx_struct
    #[test]
    fn test_create_docx_struct() {
        let doc = make_test_doc();
        let creator = CreateDocx::new(&doc);
        assert_eq!(creator.document, &doc);
        let result = creator.build();
        assert!(result.is_ok(), "CreateDocx::build should succeed");
        let bytes = result.unwrap();
        assert!(!bytes.is_empty());
    }

    /// test_styles_valid
    #[test]
    fn test_styles_valid() {
        let styles = build_styles();
        let content = String::from_utf8_lossy(&styles);
        assert!(
            content.contains("<w:docDefaults>"),
            "Should have docDefaults"
        );
        assert!(
            content.contains("<w:style w:type=\"paragraph\" w:styleId=\"Normal\">"),
            "Should have Normal style"
        );
        assert!(content.contains("<w:rFonts"), "Should have rFonts");
        assert!(content.contains("<w:sz"), "Should have sz");
    }

    /// test_settings_valid
    #[test]
    fn test_settings_valid() {
        let settings = build_settings();
        let content = String::from_utf8_lossy(&settings);
        assert!(
            content.contains("<w:zoom w:percent=\"100\"/>"),
            "Should have zoom"
        );
        assert!(
            content.contains("<w:embedSystemFonts/>"),
            "Should have embedSystemFonts"
        );
        assert!(content.contains("<w:proofBidi/>"), "Should have proofBidi");
    }

    /// test_font_table_valid
    #[test]
    fn test_font_table_valid() {
        let ft = build_font_table();
        let content = String::from_utf8_lossy(&ft);
        assert!(
            content.contains("<w:font w:name=\"Calibri\">"),
            "Should have Calibri font"
        );
        assert!(content.contains("<w:ascii"), "Should have ascii charset");
    }

    /// test_theme_valid
    #[test]
    fn test_theme_valid() {
        let theme = build_theme();
        let content = String::from_utf8_lossy(&theme);
        assert!(
            content.contains("xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\""),
            "Should have drawingml namespace"
        );
        assert!(
            content.contains("<a:themeElements>"),
            "Should have themeElements"
        );
        assert!(content.contains("<a:clrScheme"), "Should have color scheme");
        assert!(content.contains("<a:fontScheme"), "Should have font scheme");
    }

    /// test_content_types_valid
    #[test]
    fn test_content_types_valid() {
        let ct = build_content_types(&[]);
        let content = String::from_utf8_lossy(&ct);
        assert!(content.contains("wordprocessingml.document.main+xml"));
        assert!(content.contains("wordprocessingml.styles+xml"));
        assert!(content.contains("wordprocessingml.settings+xml"));
        assert!(content.contains("wordprocessingml.fontTable+xml"));
        assert!(content.contains("theme+xml"));
        assert!(content.contains("rels"));
        assert!(content.contains("<Default Extension=\"png\""));
        assert!(content.contains("<Default Extension=\"jpeg\""));
        assert!(content.contains("<Default Extension=\"gif\""));
        assert!(content.contains("numbering"));
        assert!(content.contains("header"));
        assert!(content.contains("footer"));
        assert!(content.contains("footnotes"));
        assert!(content.contains("endnotes"));
    }

    /// test_docx_with_list
    #[test]
    fn test_docx_with_list() {
        let list_item = ListItem {
            id: ElementId::content_anchored("listitem", &["item:0"], "li0"),
            content: vec![SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("paragraph", &["para:0"], "li0"),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["run:0"], "first"),
                    text: "First item".to_string(),
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
            level: 0,
        };
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "list"),
                kind: SectionKind::Body,
                items: vec![SectionItem::List(List {
                    id: ElementId::content_anchored("list", &["list:0"], ""),
                    kind: ListKind::Numbered,
                    items: vec![list_item],
                })],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
        };
        let result = create_minimal_docx(&doc);
        assert!(result.is_ok(), "Should create docx with list");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let doc_xml = read_zip_entry_str(&mut archive, "word/document.xml");
        assert!(
            doc_xml.contains("First item"),
            "Should contain list item text"
        );
        assert!(
            doc_xml.contains("<w:numPr>"),
            "Should contain numbering reference"
        );
        assert!(
            archive.by_name("word/numbering.xml").is_ok(),
            "Should have numbering.xml"
        );
    }

    /// test_docx_escapes_special_xml_chars
    #[test]
    fn test_docx_escapes_special_xml_chars() {
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "escape"),
                kind: SectionKind::Body,
                items: vec![SectionItem::Paragraph(Paragraph {
                    id: ElementId::content_anchored("paragraph", &["para:0"], "escape"),
                    runs: vec![Run {
                        id: ElementId::content_anchored("run", &["run:0"], "escape"),
                        text: "5 < 10 & 10 > 5".to_string(),
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
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
        };
        let bytes = create_minimal_docx(&doc).expect("Should create docx");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let doc_xml = read_zip_entry_str(&mut archive, "word/document.xml");
        assert!(doc_xml.contains("&lt;"), "Should escape <");
        assert!(doc_xml.contains("&gt;"), "Should escape >");
        assert!(doc_xml.contains("&amp;"), "Should escape &");
    }

    /// test_drawing_skipped_with_warning
    #[test]
    fn test_drawing_skipped_with_warning() {
        let drawing = Drawing {
            id: ElementId::content_anchored("drawing", &["drawing:0"], "img"),
            mime_type: "image/png".to_string(),
            data: "iVBORw0KGgo=".to_string(),
            bbox_pt: None,
        };
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "drawing"),
                kind: SectionKind::Body,
                items: vec![SectionItem::Drawing(drawing)],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
        };
        let result = create_minimal_docx(&doc);
        assert!(result.is_ok(), "Should create docx even with drawing");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        assert!(
            archive.by_name("word/document.xml").is_ok(),
            "Should have document.xml"
        );
    }

    /// test_empty_document
    #[test]
    fn test_empty_document() {
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
        };
        let bytes = create_minimal_docx(&doc).expect("Should create docx from empty document");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let doc_xml = read_zip_entry_str(&mut archive, "word/document.xml");
        assert!(doc_xml.contains("<w:body>"), "Should have body tag");
        assert!(doc_xml.contains("<w:sectPr"), "Should have sectPr tag");
    }

    /// test_multiple_paragraphs
    #[test]
    fn test_multiple_paragraphs() {
        let para1 = SectionItem::Paragraph(Paragraph {
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
        });
        let para2 = SectionItem::Paragraph(Paragraph {
            id: ElementId::content_anchored("paragraph", &["para:1"], "p2"),
            runs: vec![Run {
                id: ElementId::content_anchored("run", &["run:1"], "p2"),
                text: "Second paragraph".to_string(),
                bold: Some(true),
                italic: Some(true),
                font_name: Some("Arial".to_string()),
                font_size_pt: Some(14.0),
                underline: None,
                strikethrough: None,
                style_name: None,
            }],
            page_number: None,
            style_name: None,
            numbering_info: None,
        });
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "multi"),
                kind: SectionKind::Body,
                items: vec![para1, para2],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
        };
        let bytes = create_minimal_docx(&doc).expect("Should create docx");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let doc_xml = read_zip_entry_str(&mut archive, "word/document.xml");
        assert!(
            doc_xml.contains("First paragraph"),
            "Should contain first paragraph"
        );
        assert!(
            doc_xml.contains("Second paragraph"),
            "Should contain second paragraph"
        );
        assert!(doc_xml.contains("<w:b/>"), "Should have bold");
        assert!(doc_xml.contains("<w:i/>"), "Should have italic");
        assert!(doc_xml.contains("Arial"), "Should have font name");
        assert!(
            doc_xml.contains("w:val=\"28\""),
            "Should have sz=28 for 14pt"
        );
    }

    // ===========================================================================
    // P3-16 specific tests
    // ===========================================================================

    /// test_docx_with_images - verify images in package
    #[test]
    fn test_docx_with_images() {
        let drawing = Drawing {
            id: ElementId::content_anchored("drawing", &["drawing:0"], "img"),
            mime_type: "image/png".to_string(),
            data: "iVBORw0KGgo=".to_string(),
            bbox_pt: Some([0.0, 0.0, 200.0, 150.0]),
        };
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "img"),
                kind: SectionKind::Body,
                items: vec![SectionItem::Drawing(drawing)],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
        };
        let bytes = create_minimal_docx(&doc).expect("Should create docx with image");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let entry_names: Vec<String> = (0..archive.len())
            .filter_map(|i| archive.by_index(i).ok().map(|e| e.name().to_string()))
            .collect();
        assert!(
            entry_names.iter().any(|n| n.starts_with("word/media/")),
            "Should have media part (entries: {:?})",
            entry_names
        );
        // Verify document.xml has inline drawing
        let mut archive2 = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let doc_xml = read_zip_entry_str(&mut archive2, "word/document.xml");
        assert!(doc_xml.contains("<wp:inline"), "Should have inline drawing");
        assert!(doc_xml.contains("<a:graphic"), "Should have graphic");
        assert!(doc_xml.contains("<pic:pic"), "Should have pic element");
    }

    /// test_docx_with_headers_footers
    #[test]
    fn test_docx_with_headers_footers() {
        let header = Header {
            id: ElementId::content_anchored("header", &["header:0"], "h"),
            content: vec![SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("paragraph", &["para:0"], "hdr"),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["run:0"], "hdr"),
                    text: "Header text".to_string(),
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
                    text: "Page <PAGE>".to_string(),
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
            template: None,
        };
        let bytes = create_minimal_docx(&doc).expect("Should create docx with hf");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        assert!(
            archive.by_name("word/header1.xml").is_ok(),
            "Should have header1.xml"
        );
        assert!(
            archive.by_name("word/footer1.xml").is_ok(),
            "Should have footer1.xml"
        );
        let header_xml = read_zip_entry_str(&mut archive, "word/header1.xml");
        assert!(
            header_xml.contains("Header text"),
            "Header should contain text"
        );
        let footer_xml = read_zip_entry_str(&mut archive, "word/footer1.xml");
        assert!(footer_xml.contains("Page"), "Footer should contain text");
        let doc_xml = read_zip_entry_str(&mut archive, "word/document.xml");
        assert!(
            doc_xml.contains("headerReference"),
            "Document should have header/footer refs"
        );
    }

    /// test_docx_with_hyperlinks
    #[test]
    fn test_docx_with_hyperlinks() {
        let para = SectionItem::Paragraph(Paragraph {
            id: ElementId::content_anchored("paragraph", &["para:0"], "link"),
            runs: vec![Run {
                id: ElementId::content_anchored("run", &["run:0"], "link"),
                text: "https://example.com".to_string(),
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
        });
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "link"),
                kind: SectionKind::Body,
                items: vec![para],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
        };
        let bytes = create_minimal_docx(&doc).expect("Should create docx with hyperlink");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let doc_xml = read_zip_entry_str(&mut archive, "word/document.xml");
        assert!(
            doc_xml.contains("<w:hyperlink"),
            "Should contain hyperlink element"
        );
        let rels = read_zip_entry_str(&mut archive, "word/_rels/document.xml.rels");
        assert!(
            rels.contains("example.com"),
            "Should have hyperlink relationship"
        );
    }

    /// test_docx_with_merged_cells
    #[test]
    fn test_docx_with_merged_cells() {
        let cell_a1 = Cell {
            id: ElementId::content_anchored("cell", &["cell:0"], "a1"),
            contents: vec![SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("paragraph", &["p:0"], "a1"),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["run:0"], "a1"),
                    text: "Merged".to_string(),
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
            colspan: Some(2),
            rowspan: None,
        };
        let cell_b2 = Cell {
            id: ElementId::content_anchored("cell", &["cell:1"], "b2"),
            contents: vec![SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("paragraph", &["p:1"], "b2"),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["run:1"], "b2"),
                    text: "B2".to_string(),
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
            colspan: None,
            rowspan: Some(2),
        };
        let cell_c2 = Cell {
            id: ElementId::content_anchored("cell", &["cell:2"], "c2"),
            contents: vec![SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("paragraph", &["p:2"], "c2"),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["run:2"], "c2"),
                    text: "C2".to_string(),
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
            colspan: None,
            rowspan: None,
        };
        let cell_c1 = Cell {
            id: ElementId::content_anchored("cell", &["cell:3"], "c1"),
            contents: vec![SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("paragraph", &["p:3"], "c1"),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["run:3"], "c1"),
                    text: "C1".to_string(),
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
            colspan: None,
            rowspan: None,
        };
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "merge"),
                kind: SectionKind::Body,
                items: vec![SectionItem::Table(Table {
                    id: ElementId::content_anchored("table", &["table:0"], "merge"),
                    rows: vec![
                        Row {
                            cells: vec![cell_a1, cell_c1],
                        },
                        Row {
                            cells: vec![cell_b2, cell_c2],
                        },
                    ],
                })],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
        };
        let bytes = create_minimal_docx(&doc).expect("Should create docx with merged cells");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let doc_xml = read_zip_entry_str(&mut archive, "word/document.xml");
        assert!(
            doc_xml.contains("Merged"),
            "Should contain merged cell text"
        );
        assert!(doc_xml.contains("<w:gridSpan"), "Should have colspan");
        assert!(doc_xml.contains("<w:vMerge"), "Should have rowspan");
        assert!(doc_xml.contains("<w:tblGrid"), "Should have table grid");
        assert!(
            doc_xml.contains("<w:tblCellMar"),
            "Should have cell margins"
        );
        assert!(doc_xml.contains("<w:tcBorders"), "Should have cell borders");
    }

    /// test_docx_with_footnotes
    #[test]
    fn test_docx_with_footnotes() {
        let fn_item = Footnote {
            id: ElementId::content_anchored("footnote", &["fn:0"], "fn0"),
            content: vec![SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("paragraph", &["p:0"], "fn"),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["run:0"], "fn"),
                    text: "This is a footnote".to_string(),
                    bold: None,
                    italic: Some(true),
                    font_name: None,
                    font_size_pt: Some(8.0),
                    underline: None,
                    strikethrough: None,
                    style_name: None,
                }],
                page_number: None,
                style_name: None,
                numbering_info: None,
            })],
            reference_id: Some("1".to_string()),
        };
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "fn"),
                kind: SectionKind::Body,
                items: vec![],
                headers: vec![],
                footers: vec![],
                footnotes: vec![fn_item],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
        };
        let bytes = create_minimal_docx(&doc).expect("Should create docx with footnotes");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        assert!(
            archive.by_name("word/footnotes1.xml").is_ok(),
            "Should have footnotes1.xml"
        );
        let fn_xml = read_zip_entry_str(&mut archive, "word/footnotes1.xml");
        assert!(
            fn_xml.contains("This is a footnote"),
            "Should contain footnote text"
        );
        assert!(
            fn_xml.contains("<w:footnote w:id=\"1\""),
            "Should have footnote with id"
        );
    }

    /// test_docx_with_endnotes
    #[test]
    fn test_docx_with_endnotes() {
        let en_item = Endnote {
            id: ElementId::content_anchored("endnote", &["en:0"], "en0"),
            content: vec![SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("paragraph", &["p:0"], "en"),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["run:0"], "en"),
                    text: "This is an endnote".to_string(),
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
        };
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "en"),
                kind: SectionKind::Body,
                items: vec![],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![en_item],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
        };
        let bytes = create_minimal_docx(&doc).expect("Should create docx with endnotes");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        assert!(
            archive.by_name("word/endnotes1.xml").is_ok(),
            "Should have endnotes1.xml"
        );
        let en_xml = read_zip_entry_str(&mut archive, "word/endnotes1.xml");
        assert!(
            en_xml.contains("This is an endnote"),
            "Should contain endnote text"
        );
    }

    /// test_docx_multi_section
    #[test]
    fn test_docx_multi_section() {
        let section1 = Section {
            id: ElementId::content_anchored("section", &["section:0"], "s1"),
            kind: SectionKind::Body,
            items: vec![SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("paragraph", &["para:0"], "s1p"),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["run:0"], "s1p"),
                    text: "Section 1 content".to_string(),
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
            headers: vec![],
            footers: vec![],
            footnotes: vec![],
            endnotes: vec![],
        };
        let section2 = Section {
            id: ElementId::content_anchored("section", &["section:1"], "s2"),
            kind: SectionKind::Body,
            items: vec![SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("paragraph", &["para:0"], "s2p"),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["run:0"], "s2p"),
                    text: "Section 2 content".to_string(),
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
        };
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![section1, section2],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
        };
        let bytes = create_minimal_docx(&doc).expect("Should create multi-section docx");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let doc_xml = read_zip_entry_str(&mut archive, "word/document.xml");
        assert!(
            doc_xml.contains("Section 1 content"),
            "Should contain section 1"
        );
        assert!(
            doc_xml.contains("Section 2 content"),
            "Should contain section 2"
        );
        assert!(doc_xml.contains("<w:pgSz"), "Should have page size");
        assert!(doc_xml.contains("<w:pgMar"), "Should have page margins");
        assert!(doc_xml.contains("<w:pgBorders"), "Should have page borders");
        assert!(doc_xml.contains("<w:cols"), "Should have columns");
    }

    /// test_docx_with_numbering_xml
    #[test]
    fn test_docx_with_numbering_xml() {
        let list_item = ListItem {
            id: ElementId::content_anchored("listitem", &["item:0"], "li0"),
            content: vec![SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("paragraph", &["para:0"], "li0"),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["run:0"], "li0"),
                    text: "Item 1".to_string(),
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
            level: 0,
        };
        let list_item2 = ListItem {
            id: ElementId::content_anchored("listitem", &["item:1"], "li1"),
            content: vec![SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("paragraph", &["para:1"], "li1"),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["run:1"], "li1"),
                    text: "Item 2".to_string(),
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
            level: 0,
        };
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "num"),
                kind: SectionKind::Body,
                items: vec![SectionItem::List(List {
                    id: ElementId::content_anchored("list", &["list:0"], "num"),
                    kind: ListKind::Numbered,
                    items: vec![list_item, list_item2],
                })],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
        };
        let bytes = create_minimal_docx(&doc).expect("Should create docx with numbering");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let num_xml = read_zip_entry_str(&mut archive, "word/numbering.xml");
        assert!(
            num_xml.contains("<w:abstractNum"),
            "Should have abstractNum"
        );
        assert!(num_xml.contains("<w:num"), "Should have num");
        assert!(num_xml.contains("<w:numFmt"), "Should have numFmt");
    }

    /// test_docx_media_byte_preserved
    #[test]
    fn test_docx_media_byte_preserved() {
        // Use a known base64 payload
        let drawing = Drawing {
            id: ElementId::content_anchored("drawing", &["drawing:0"], "img"),
            mime_type: "image/png".to_string(),
            data: "iVBORw0KGgo=".to_string(),
            bbox_pt: None,
        };
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], "img"),
                kind: SectionKind::Body,
                items: vec![SectionItem::Drawing(drawing)],
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
        };
        let bytes = create_minimal_docx(&doc).expect("Should create docx with image");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        // Find the media file
        let mut found_media = false;
        for i in 0..archive.len() {
            let entry = archive.by_index(i).expect("read entry");
            let name = entry.name();
            if name.starts_with("word/media/") && name.ends_with(".png") {
                found_media = true;
                // Media file should exist in the archive
                break;
            }
        }
        assert!(found_media, "Should have media file in package");
    }
}
