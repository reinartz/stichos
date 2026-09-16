//! Create a valid xlsx from a `gp_core::ir::Spreadsheet` IR.
//!
//! Writes a valid SpreadsheetML workbook with:
//!   - `[Content_Types].xml` — content-type overrides
//!   - `_rels/.rels` — package-level relationships
//!   - `xl/workbook.xml` — workbook with sheets
//!   - `xl/_rels/workbook.xml.rels` — worksheet relationships
//!   - `xl/sharedStrings.xml` — shared string table
//!   - `xl/styles.xml` — cell styles and number formats
//!   - `xl/worksheets/sheetN.xml` — per-sheet worksheet data

use gp_core::diagnostics::Diagnostic;
use gp_core::ir::*;

use std::collections::BTreeMap;
use std::io::{Cursor, Write};

// ===========================================================================
// Public API
// ===========================================================================

/// Create a minimal xlsx from an IR Spreadsheet.
pub fn create_minimal_xlsx(spreadsheet: &Spreadsheet) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let diags: Vec<Diagnostic> = Vec::new();
    let parts = build_parts(spreadsheet, &mut diags.clone());
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
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Convert a column index (0-based) to Excel column letter(s).
/// 0 → "A", 25 → "Z", 26 → "AA"
fn col_to_letter(col: usize) -> String {
    let mut result = String::new();
    let mut n = col;
    loop {
        let remainder = n % 26;
        result.insert(0, (b'A' + remainder as u8) as char);
        n /= 26;
        if n == 0 {
            break;
        }
        n -= 1;
    }
    result
}

/// Convert row (1-based) and col (0-based) to Excel cell reference like "A1", "B2".
fn cell_ref(row: usize, col: usize) -> String {
    let row_1 = row + 1;
    format!("{}{}", col_to_letter(col), row_1)
}

/// Convert merged range to Excel ref string like "A1:B3".
fn merged_range_ref(start_row: usize, start_col: usize, end_row: usize, end_col: usize) -> String {
    format!(
        "{}{}:{}{}",
        col_to_letter(start_col),
        start_row + 1,
        col_to_letter(end_col),
        end_row + 1
    )
}

// ===========================================================================
// Shared strings collector
// ===========================================================================

/// Collect all unique string values across all sheets, deduplicate, and map to indices.
fn collect_shared_strings(spreadsheet: &Spreadsheet) -> (Vec<String>, BTreeMap<String, usize>) {
    let mut string_to_index: BTreeMap<String, usize> = BTreeMap::new();
    let mut index_map: BTreeMap<String, usize> = BTreeMap::new();
    let mut strings: Vec<String> = Vec::new();

    for sheet in &spreadsheet.sheets {
        for row in &sheet.rows {
            for cell in &row.cells {
                if let Some(ref value) = cell.value {
                    if let Some(idx) = string_to_index.get(value.as_str()) {
                        index_map.insert(cell.id.to_string(), *idx);
                    } else {
                        let idx = strings.len();
                        strings.push(value.clone());
                        string_to_index.insert(value.clone(), idx);
                        index_map.insert(cell.id.to_string(), idx);
                    }
                }
            }
        }
    }

    (strings, index_map)
}

// ===========================================================================
// Styles collector
// ===========================================================================

/// Collect style names used across all cells and assign style indices.
fn collect_style_indices(spreadsheet: &Spreadsheet) -> BTreeMap<String, usize> {
    let mut name_to_index: BTreeMap<String, usize> = BTreeMap::new();
    let mut next_idx: usize = 1;

    // Index 0 is always the default "Normal" style
    name_to_index.insert("__default__".to_string(), 0);

    for sheet in &spreadsheet.sheets {
        for row in &sheet.rows {
            for cell in &row.cells {
                if let Some(ref style_name) = cell.style_name {
                    if !name_to_index.contains_key(style_name) {
                        name_to_index.insert(style_name.clone(), next_idx);
                        next_idx += 1;
                    }
                }
            }
        }
    }

    name_to_index
}

// ===========================================================================
// Build all parts
// ===========================================================================

fn build_parts(spreadsheet: &Spreadsheet, _diags: &mut Vec<Diagnostic>) -> Vec<(String, Vec<u8>)> {
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();

    // Collect shared strings and styles
    let (shared_strings, _string_index_map) = collect_shared_strings(spreadsheet);
    let _style_name_to_index = collect_style_indices(spreadsheet);

    let num_sheets = spreadsheet.sheets.len();
    let num_shared_strings = shared_strings.len();

    parts.push((
        "[Content_Types].xml".to_string(),
        build_content_types(num_sheets, num_shared_strings > 0),
    ));
    parts.push(("_rels/.rels".to_string(), build_package_rels()));
    parts.push((
        "xl/workbook.xml".to_string(),
        build_workbook_xml(&spreadsheet.sheets),
    ));
    parts.push((
        "xl/_rels/workbook.xml.rels".to_string(),
        build_workbook_rels(num_sheets),
    ));
    if num_shared_strings > 0 {
        parts.push((
            "xl/sharedStrings.xml".to_string(),
            build_shared_strings_xml(&shared_strings),
        ));
    } else {
        parts.push((
            "xl/sharedStrings.xml".to_string(),
            build_shared_strings_xml(&[]),
        ));
    }
    parts.push(("xl/styles.xml".to_string(), build_styles_xml()));

    for (sheet_idx, sheet) in spreadsheet.sheets.iter().enumerate() {
        parts.push((
            format!("xl/worksheets/sheet{}.xml", sheet_idx + 1),
            build_worksheet_xml(sheet, &shared_strings, _diags),
        ));
    }

    parts.push(("docProps/core.xml".to_string(), build_core_properties()));
    parts.push(("docProps/app.xml".to_string(), build_app_properties()));

    parts
}

// ===========================================================================
// [Content_Types].xml
// ===========================================================================

fn build_content_types(num_sheets: usize, has_shared_strings: bool) -> Vec<u8> {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\
         <Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-package.application-properties+xml\"/>\
         <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>",
    );

    if has_shared_strings {
        xml.push_str("\n<Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml\"/>");
    }
    xml.push_str("\n<Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>");

    for i in 1..=num_sheets {
        xml.push_str(&format!(
            "\n<Override PartName=\"/xl/worksheets/sheet{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>",
            i
        ));
    }

    xml.push_str("\n</Types>");
    xml.into_bytes()
}

// ===========================================================================
// _rels/.rels
// ===========================================================================

fn build_package_rels() -> Vec<u8> {
    b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
       <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
       <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
       </Relationships>"
        .to_vec()
}

// ===========================================================================
// xl/workbook.xml
// ===========================================================================

fn build_workbook_xml(sheets: &[Sheet]) -> Vec<u8> {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
         xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
         <sheets>",
    );

    for (i, sheet) in sheets.iter().enumerate() {
        xml.push_str(&format!(
            "\n<sheet name=\"{}\" sheetId=\"{}\" r:id=\"rId{}\"/>",
            escape_xml(&sheet.name),
            i + 1,
            i + 1
        ));
    }

    xml.push_str("\n</sheets>\n</workbook>");
    xml.into_bytes()
}

// ===========================================================================
// xl/_rels/workbook.xml.rels
// ===========================================================================

fn build_workbook_rels(num_sheets: usize) -> Vec<u8> {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );

    for i in 1..=num_sheets {
        xml.push_str(&format!(
            "\n<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{}.xml\"/>",
            i, i
        ));
    }

    xml.push_str("\n</Relationships>");
    xml.into_bytes()
}

// ===========================================================================
// xl/sharedStrings.xml
// ===========================================================================

fn build_shared_strings_xml(strings: &[String]) -> Vec<u8> {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <sst xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" ",
    );
    xml.push_str(&format!(
        "count=\"{}\" uniqueCount=\"{}\"",
        strings.len(),
        strings.len()
    ));
    xml.push('>');

    for s in strings {
        xml.push_str("<si><t>");
        xml.push_str(&escape_xml(s));
        xml.push_str("</t></si>");
    }

    xml.push_str("</sst>");
    xml.into_bytes()
}

// ===========================================================================
// xl/styles.xml
// ===========================================================================

fn build_styles_xml() -> Vec<u8> {
    let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>
<fills count="1"><fill><patternFill patternType="none"/></fill></fills>
<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
<cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs>
</styleSheet>"#;
    xml.as_bytes().to_vec()
}

// ===========================================================================
// xl/worksheets/sheetN.xml
// ===========================================================================

fn build_worksheet_xml(
    sheet: &Sheet,
    shared_strings: &[String],
    _diags: &mut Vec<Diagnostic>,
) -> Vec<u8> {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
         <sheetData>",
    );

    for row in &sheet.rows {
        xml.push_str(&format!("\n<row r=\"{}\">", row.id.as_str()));

        for cell in &row.cells {
            let ref_str = cell_ref(cell.row, cell.col);
            xml.push_str(&build_cell_xml(cell, shared_strings));
            let _ = ref_str;
        }

        xml.push_str("</row>");
    }

    xml.push_str("</sheetData>");

    if !sheet.merged_ranges.is_empty() {
        xml.push_str("<mergeCells count=\"");
        xml.push_str(&sheet.merged_ranges.len().to_string());
        xml.push_str("\">");
        for m in &sheet.merged_ranges {
            let ref_str = merged_range_ref(m.start_row, m.start_col, m.end_row, m.end_col);
            xml.push_str(&format!("\n<mergeCell ref=\"{}\"/>", escape_xml(&ref_str)));
        }
        xml.push_str("</mergeCells>");
    }

    xml.push_str("</worksheet>");
    xml.into_bytes()
}

/// Build XML for a single cell.
fn build_cell_xml(cell: &SheetCell, shared_strings: &[String]) -> String {
    let ref_str = cell_ref(cell.row, cell.col);

    match (&cell.formula, &cell.value, &cell.cached_value) {
        // Formula cell
        (Some(formula), _, _) => {
            let mut xml = format!("<c r=\"{}\" t=\"str\">", ref_str);
            xml.push_str(&format!("<f>{}</f>", escape_xml(formula)));
            if let Some(ref cv) = cell.cached_value {
                xml.push_str(&format!("<v>{}</v>", escape_xml(cv)));
            } else if let Some(ref v) = cell.value {
                xml.push_str(&format!("<v>{}</v>", escape_xml(v)));
            }
            xml.push_str("</c>");
            xml
        }
        // Numeric cell
        (_, Some(val), _) if is_numeric(val) => {
            format!(
                "<c r=\"{}\" t=\"n\"><v>{}</v></c>",
                ref_str,
                escape_xml(val)
            )
        }
        // Boolean cell
        (_, Some(val), _)
            if val.eq_ignore_ascii_case("true") || val.eq_ignore_ascii_case("false") =>
        {
            format!(
                "<c r=\"{}\" t=\"b\"><v>{}</v></c>",
                ref_str,
                val.to_uppercase()
            )
        }
        // Error cell
        (_, Some(val), _) if val.starts_with('#') => {
            format!(
                "<c r=\"{}\" t=\"e\"><v>{}</v></c>",
                ref_str,
                escape_xml(val)
            )
        }
        // String cell (shared string reference)
        (_, Some(_), _) => {
            let mut xml = format!("<c r=\"{}\" t=\"s\">", ref_str);

            // Find the index in shared strings
            let search_val = cell.value.as_deref().unwrap_or("");
            if let Some(idx) = shared_strings.iter().position(|s| s == search_val) {
                xml.push_str(&format!("<v>{}</v>", idx));
            } else {
                xml.push_str("<v>0</v>");
            }

            xml.push_str("</c>");
            xml
        }
        // Empty cell
        _ => {
            format!("<c r=\"{}\"><v></v></c>", ref_str)
        }
    }
}

/// Check if a string looks numeric.
fn is_numeric(s: &str) -> bool {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return false;
    }
    // Allow: integers, decimals, negative numbers, scientific notation
    trimmed
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e' || c == 'E')
        && trimmed.parse::<f64>().is_ok()
}

// ===========================================================================
// docProps/core.xml
// ===========================================================================

fn build_core_properties() -> Vec<u8> {
    b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
       <cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
       xmlns:dc=\"http://purl.org/dc/elements/1.1/\" \
       xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" \
       xmlns:dcterms=\"http://purl.org/dc/terms/\" \
       xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
       <dcterms:created xsi:type=\"dcterms:W3CDTF\">2026-01-01T00:00:00Z</dcterms:created>\
       <dc:creator>stichos</dc:creator>\
       </cp:coreProperties>"
        .to_vec()
}

// ===========================================================================
// docProps/app.xml
// ===========================================================================

fn build_app_properties() -> Vec<u8> {
    b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
       <Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\" \
       xmlns:vt=\"http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes\">\
       <Application>stichos</Application>\
       </Properties>"
        .to_vec()
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn make_id(kind: &str, path: &[&str], content: &str) -> ElementId {
        let s = String::from(content);
        ElementId::derive(kind, path, &s)
    }

    fn read_zip_entry_str(archive: &mut zip::ZipArchive<Cursor<&Vec<u8>>>, name: &str) -> String {
        let mut buf = Vec::new();
        archive
            .by_name(name)
            .unwrap_or_else(|_| panic!("Entry '{}' should exist", name))
            .read_to_end(&mut buf)
            .unwrap_or_else(|_| panic!("Should read entry '{}'", name));
        String::from_utf8(buf).expect("Entry should be valid UTF-8")
    }

    fn make_empty_spreadsheet() -> Spreadsheet {
        Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets: vec![],
        }
    }

    fn make_spreadsheet_with_data() -> Spreadsheet {
        Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets: vec![Sheet {
                id: make_id("sheet", &["sheet:0"], "Data"),
                name: "Data".to_string(),
                rows: vec![
                    SheetRow {
                        id: make_id("row", &["row:0"], "row:0"),
                        cells: vec![
                            SheetCell {
                                id: make_id("cell", &["cell:A1"], "A1"),
                                row: 0,
                                col: 0,
                                value: Some("Name".to_string()),
                                formula: None,
                                cached_value: None,
                                style_name: None,
                            },
                            SheetCell {
                                id: make_id("cell", &["cell:B1"], "B1"),
                                row: 0,
                                col: 1,
                                value: Some("Value".to_string()),
                                formula: None,
                                cached_value: None,
                                style_name: None,
                            },
                        ],
                    },
                    SheetRow {
                        id: make_id("row", &["row:1"], "row:1"),
                        cells: vec![
                            SheetCell {
                                id: make_id("cell", &["cell:A2"], "A2"),
                                row: 1,
                                col: 0,
                                value: Some("Alice".to_string()),
                                formula: None,
                                cached_value: None,
                                style_name: None,
                            },
                            SheetCell {
                                id: make_id("cell", &["cell:B2"], "B2"),
                                row: 1,
                                col: 1,
                                value: Some("42".to_string()),
                                formula: None,
                                cached_value: Some("42".to_string()),
                                style_name: None,
                            },
                        ],
                    },
                    SheetRow {
                        id: make_id("row", &["row:2"], "row:2"),
                        cells: vec![
                            SheetCell {
                                id: make_id("cell", &["cell:A3"], "A3"),
                                row: 2,
                                col: 0,
                                value: Some("Bob".to_string()),
                                formula: None,
                                cached_value: None,
                                style_name: None,
                            },
                            SheetCell {
                                id: make_id("cell", &["cell:B3"], "B3"),
                                row: 2,
                                col: 1,
                                value: Some("100".to_string()),
                                formula: None,
                                cached_value: Some("100".to_string()),
                                style_name: None,
                            },
                        ],
                    },
                ],
                merged_ranges: vec![],
                header_row: Some(0),
                footer_row: None,
            }],
        }
    }

    fn make_spreadsheet_with_shared_strings() -> Spreadsheet {
        let shared = "SharedString".to_string();
        Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets: vec![
                Sheet {
                    id: make_id("sheet", &["sheet:0"], "Sheet1"),
                    name: "Sheet1".to_string(),
                    rows: vec![SheetRow {
                        id: make_id("row", &["row:0"], "row:0"),
                        cells: vec![SheetCell {
                            id: make_id("cell", &["cell:A1"], "A1"),
                            row: 0,
                            col: 0,
                            value: Some(shared.clone()),
                            formula: None,
                            cached_value: None,
                            style_name: None,
                        }],
                    }],
                    merged_ranges: vec![],
                    header_row: None,
                    footer_row: None,
                },
                Sheet {
                    id: make_id("sheet", &["sheet:1"], "Sheet2"),
                    name: "Sheet2".to_string(),
                    rows: vec![SheetRow {
                        id: make_id("row", &["row:0"], "row:0"),
                        cells: vec![SheetCell {
                            id: make_id("cell", &["cell:A1"], "A1"),
                            row: 0,
                            col: 0,
                            value: Some(shared.clone()),
                            formula: None,
                            cached_value: None,
                            style_name: None,
                        }],
                    }],
                    merged_ranges: vec![],
                    header_row: None,
                    footer_row: None,
                },
            ],
        }
    }

    fn make_spreadsheet_with_formulas() -> Spreadsheet {
        Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets: vec![Sheet {
                id: make_id("sheet", &["sheet:0"], "Data"),
                name: "Data".to_string(),
                rows: vec![
                    SheetRow {
                        id: make_id("row", &["row:0"], "row:0"),
                        cells: vec![
                            SheetCell {
                                id: make_id("cell", &["cell:A1"], "A1"),
                                row: 0,
                                col: 0,
                                value: Some("10".to_string()),
                                formula: None,
                                cached_value: None,
                                style_name: None,
                            },
                            SheetCell {
                                id: make_id("cell", &["cell:B1"], "B1"),
                                row: 0,
                                col: 1,
                                value: Some("20".to_string()),
                                formula: None,
                                cached_value: None,
                                style_name: None,
                            },
                        ],
                    },
                    SheetRow {
                        id: make_id("row", &["row:1"], "row:1"),
                        cells: vec![SheetCell {
                            id: make_id("cell", &["cell:A2"], "A2"),
                            row: 1,
                            col: 0,
                            value: None,
                            formula: Some("=SUM(A1:B1)".to_string()),
                            cached_value: Some("30".to_string()),
                            style_name: None,
                        }],
                    },
                ],
                merged_ranges: vec![],
                header_row: None,
                footer_row: None,
            }],
        }
    }

    fn make_spreadsheet_with_merged_ranges() -> Spreadsheet {
        Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets: vec![Sheet {
                id: make_id("sheet", &["sheet:0"], "Data"),
                name: "Data".to_string(),
                rows: vec![SheetRow {
                    id: make_id("row", &["row:0"], "row:0"),
                    cells: vec![SheetCell {
                        id: make_id("cell", &["cell:A1"], "A1"),
                        row: 0,
                        col: 0,
                        value: Some("Merged".to_string()),
                        formula: None,
                        cached_value: None,
                        style_name: None,
                    }],
                }],
                merged_ranges: vec![
                    MergedRange {
                        id: make_id("merged", &["merged:0"], "A1:B1"),
                        start_row: 0,
                        start_col: 0,
                        end_row: 0,
                        end_col: 1,
                    },
                    MergedRange {
                        id: make_id("merged", &["merged:1"], "A2:B3"),
                        start_row: 1,
                        start_col: 0,
                        end_row: 2,
                        end_col: 1,
                    },
                ],
                header_row: None,
                footer_row: None,
            }],
        }
    }

    fn make_spreadsheet_with_styles() -> Spreadsheet {
        Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets: vec![Sheet {
                id: make_id("sheet", &["sheet:0"], "Data"),
                name: "Data".to_string(),
                rows: vec![SheetRow {
                    id: make_id("row", &["row:0"], "row:0"),
                    cells: vec![
                        SheetCell {
                            id: make_id("cell", &["cell:A1"], "A1"),
                            row: 0,
                            col: 0,
                            value: Some("Bold Header".to_string()),
                            formula: None,
                            cached_value: None,
                            style_name: Some("bold".to_string()),
                        },
                        SheetCell {
                            id: make_id("cell", &["cell:B1"], "B1"),
                            row: 0,
                            col: 1,
                            value: Some("3.14".to_string()),
                            formula: None,
                            cached_value: None,
                            style_name: Some("currency".to_string()),
                        },
                    ],
                }],
                merged_ranges: vec![],
                header_row: None,
                footer_row: None,
            }],
        }
    }

    /// test_minimal_xlsx_creation
    #[test]
    fn test_minimal_xlsx_creation() {
        let spreadsheet = make_empty_spreadsheet();
        let result = create_minimal_xlsx(&spreadsheet);
        assert!(result.is_ok(), "Should create xlsx from empty spreadsheet");
        let bytes = result.unwrap();
        assert!(!bytes.is_empty(), "Xlsx bytes should not be empty");
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
            entry_names.iter().any(|n| n == "xl/workbook.xml"),
            "Should have xl/workbook.xml"
        );
        assert!(
            entry_names.iter().any(|n| n == "xl/styles.xml"),
            "Should have xl/styles.xml"
        );
        assert!(
            entry_names.iter().any(|n| n == "xl/sharedStrings.xml"),
            "Should have xl/sharedStrings.xml"
        );
        assert!(
            entry_names.iter().any(|n| n == "docProps/core.xml"),
            "Should have docProps/core.xml"
        );
        assert!(
            entry_names.iter().any(|n| n == "docProps/app.xml"),
            "Should have docProps/app.xml"
        );
    }

    /// test_xlsx_with_data
    #[test]
    fn test_xlsx_with_data() {
        let spreadsheet = make_spreadsheet_with_data();
        let result = create_minimal_xlsx(&spreadsheet);
        assert!(result.is_ok(), "Should create xlsx with data");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let sheet_xml = read_zip_entry_str(&mut archive, "xl/worksheets/sheet1.xml");
        assert!(
            sheet_xml.contains("<sheetData>"),
            "Should contain sheetData"
        );
        assert!(sheet_xml.contains("<row"), "Should contain row elements");
        // String values are in sharedStrings.xml
        let strings_xml = read_zip_entry_str(&mut archive, "xl/sharedStrings.xml");
        assert!(
            strings_xml.contains("Name"),
            "sharedStrings should contain 'Name'"
        );
        assert!(
            strings_xml.contains("Value"),
            "sharedStrings should contain 'Value'"
        );
        assert!(
            strings_xml.contains("Alice"),
            "sharedStrings should contain 'Alice'"
        );
        assert!(
            strings_xml.contains("Bob"),
            "sharedStrings should contain 'Bob'"
        );
        // Numeric values are inline in sheet XML
        assert!(sheet_xml.contains("42"), "Should contain '42'");
        assert!(sheet_xml.contains("100"), "Should contain '100'");
    }

    /// test_xlsx_shared_strings
    #[test]
    fn test_xlsx_shared_strings() {
        let spreadsheet = make_spreadsheet_with_shared_strings();
        let result = create_minimal_xlsx(&spreadsheet);
        assert!(result.is_ok(), "Should create xlsx with shared strings");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let strings_xml = read_zip_entry_str(&mut archive, "xl/sharedStrings.xml");
        assert!(
            strings_xml.contains("SharedString"),
            "sharedStrings.xml should contain the shared string"
        );
        // Count si elements — should have exactly 1 unique string
        let si_count = strings_xml.matches("<si>").count();
        assert_eq!(si_count, 1, "Should have exactly 1 shared string entry");
        let sheet1_xml = read_zip_entry_str(&mut archive, "xl/worksheets/sheet1.xml");
        assert!(
            sheet1_xml.contains("<v>0</v>"),
            "sheet1 should reference shared string index 0"
        );
        let sheet2_xml = read_zip_entry_str(&mut archive, "xl/worksheets/sheet2.xml");
        assert!(
            sheet2_xml.contains("<v>0</v>"),
            "sheet2 should reference shared string index 0"
        );
    }

    /// test_xlsx_with_formulas
    #[test]
    fn test_xlsx_with_formulas() {
        let spreadsheet = make_spreadsheet_with_formulas();
        let result = create_minimal_xlsx(&spreadsheet);
        assert!(result.is_ok(), "Should create xlsx with formulas");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let sheet_xml = read_zip_entry_str(&mut archive, "xl/worksheets/sheet1.xml");
        assert!(sheet_xml.contains("<f>"), "Should contain formula element");
        assert!(
            sheet_xml.contains("SUM(A1:B1)"),
            "Should contain SUM formula"
        );
        assert!(
            sheet_xml.contains("<v>30</v>"),
            "Should contain cached value 30"
        );
    }

    /// test_xlsx_merged_ranges
    #[test]
    fn test_xlsx_merged_ranges() {
        let spreadsheet = make_spreadsheet_with_merged_ranges();
        let result = create_minimal_xlsx(&spreadsheet);
        assert!(result.is_ok(), "Should create xlsx with merged ranges");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let sheet_xml = read_zip_entry_str(&mut archive, "xl/worksheets/sheet1.xml");
        assert!(
            sheet_xml.contains("<mergeCells"),
            "Should contain mergeCells"
        );
        assert!(
            sheet_xml.contains("A1:B1"),
            "Should contain merged range A1:B1"
        );
        assert!(
            sheet_xml.contains("A2:B3"),
            "Should contain merged range A2:B3"
        );
    }

    /// test_xlsx_roundtrip
    #[test]
    fn test_xlsx_roundtrip() {
        let spreadsheet = make_spreadsheet_with_data();
        let bytes = create_minimal_xlsx(&spreadsheet).expect("Should create xlsx");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");

        // Verify workbook.xml
        let workbook_xml = read_zip_entry_str(&mut archive, "xl/workbook.xml");
        assert!(
            workbook_xml.contains("<workbook"),
            "Should have workbook element"
        );
        assert!(workbook_xml.contains("Data"), "Should contain sheet name");
        assert!(
            workbook_xml.contains("r:id"),
            "Should contain relationship id"
        );

        // Verify workbook rels
        let rels_xml = read_zip_entry_str(&mut archive, "xl/_rels/workbook.xml.rels");
        assert!(
            rels_xml.contains("<Relationship"),
            "Should contain Relationship elements"
        );
        assert!(
            rels_xml.contains("worksheet"),
            "Should reference worksheet type"
        );

        // Verify styles
        let styles_xml = read_zip_entry_str(&mut archive, "xl/styles.xml");
        assert!(styles_xml.contains("<cellXfs"), "Should contain cellXfs");
        assert!(styles_xml.contains("<fonts"), "Should contain fonts");

        // Verify content types
        let ct_xml = read_zip_entry_str(&mut archive, "[Content_Types].xml");
        assert!(
            ct_xml.contains("spreadsheetml"),
            "Should have spreadsheet content type"
        );
        assert!(
            ct_xml.contains("sharedStrings"),
            "Should have sharedStrings content type"
        );
    }

    /// test_xlsx_with_styles
    #[test]
    fn test_xlsx_with_styles() {
        let spreadsheet = make_spreadsheet_with_styles();
        let result = create_minimal_xlsx(&spreadsheet);
        assert!(result.is_ok(), "Should create xlsx with styles");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let sheet_xml = read_zip_entry_str(&mut archive, "xl/worksheets/sheet1.xml");
        let strings_xml = read_zip_entry_str(&mut archive, "xl/sharedStrings.xml");
        assert!(
            strings_xml.contains("Bold Header"),
            "Should contain styled text"
        );
        // Numeric values are inline in sheet XML
        assert!(
            sheet_xml.contains("3.14"),
            "Should contain numeric styled cell"
        );
        assert!(
            sheet_xml.contains(" t=\"s\""),
            "Should contain string type reference"
        );
        assert!(
            sheet_xml.contains(" t=\"n\""),
            "Should contain numeric type reference"
        );
    }

    /// test_cell_ref
    #[test]
    fn test_cell_ref() {
        assert_eq!(cell_ref(0, 0), "A1");
        assert_eq!(cell_ref(0, 1), "B1");
        assert_eq!(cell_ref(1, 0), "A2");
        assert_eq!(cell_ref(9, 9), "J10");
        assert_eq!(cell_ref(0, 25), "Z1");
        assert_eq!(cell_ref(0, 26), "AA1");
    }

    /// test_col_to_letter
    #[test]
    fn test_col_to_letter() {
        assert_eq!(col_to_letter(0), "A");
        assert_eq!(col_to_letter(25), "Z");
        assert_eq!(col_to_letter(26), "AA");
        assert_eq!(col_to_letter(27), "AB");
        assert_eq!(col_to_letter(51), "AZ");
        assert_eq!(col_to_letter(52), "BA");
    }

    /// test_is_numeric
    #[test]
    fn test_is_numeric() {
        assert!(is_numeric("42"));
        assert!(is_numeric("3.14"));
        assert!(is_numeric("-100"));
        assert!(is_numeric("1e10"));
        assert!(!is_numeric("abc"));
        assert!(!is_numeric(""));
        assert!(!is_numeric("hello"));
        assert!(is_numeric("100"));
    }

    /// test_escape_xml
    #[test]
    fn test_escape_xml() {
        assert_eq!(escape_xml("a&b"), "a&amp;b");
        assert_eq!(escape_xml("a<b"), "a&lt;b");
        assert_eq!(escape_xml("a>b"), "a&gt;b");
        assert_eq!(escape_xml("a\"b"), "a&quot;b");
        assert_eq!(escape_xml("a'b"), "a&apos;b");
    }

    /// test_merged_range_ref
    #[test]
    fn test_merged_range_ref() {
        assert_eq!(merged_range_ref(0, 0, 0, 1), "A1:B1");
        assert_eq!(merged_range_ref(1, 0, 2, 1), "A2:B3");
        assert_eq!(merged_range_ref(0, 0, 4, 4), "A1:E5");
    }
}
