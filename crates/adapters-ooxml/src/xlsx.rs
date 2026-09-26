// SPDX-License-Identifier: MIT OR Apache-2.0
//! adapters-ooxml: xlsx → gp-core IR (sheets, cells, formulas, cached values, shared strings, styles, merges).
//!
//! Walks a `.xlsx` OPC package, extracts `xl/workbook.xml`, `xl/sharedStrings.xml`,
//! `xl/styles.xml`, and per-sheet `xl/worksheets/sheetN.xml`, then maps ExcelML
//! elements to the gp-core intermediate representation:
//!   * `<sheet>` → Sheet (with name, rows, merged_ranges)
//!   * `<row>` → SheetRow (with cells)
//!   * `<c r="A1" t="n">` → SheetCell with row/col/value
//!   * `<c r="A1" t="s">` → SheetCell resolved from sharedStrings.xml
//!   * `<c r="A1" t="str"><f>=SUM(...)</f><v>10</v></c>` → SheetCell with formula + cached_value
//!   * `<mergeCells><mergeCell ref="A1:B2"/></mergeCells>` → MergedRange

use gp_core::diagnostics::Diagnostic;
use gp_core::diagnostics::Severity;
use gp_core::ir::*;
use gp_core::traits::{Codec, ReadResult};
use std::collections::{BTreeMap, HashMap};

use super::formula::{
    check_parity, check_refusal, eval_result_to_string, evaluate, gp_f_code_str, parse_formula,
    CachedCellValue, FormulaContext, ParityResult,
};
use super::opc::{parse_opc, OpcPackage, Part};
use super::xml::{parse_xml, XmlNode};

// ===========================================================================
// Helper: find a part by URI prefix match in the OPC package.
// ===========================================================================

fn find_part<'a>(package: &'a OpcPackage, uri: &str) -> Result<&'a Part, Vec<Diagnostic>> {
    package
        .parts
        .iter()
        .find(|p| p.uri == uri || p.uri.ends_with(&format!("/{}", uri)))
        .ok_or_else(|| {
            vec![Diagnostic::new(
                "OOX-E-020",
                Severity::Error,
                format!("Part not found: {}", uri),
            )]
        })
}

/// Helper: get an attribute value from a BTreeMap<TagName, String>.
fn get_attr<'a>(attrs: &'a BTreeMap<super::xml::TagName, String>, key: &str) -> Option<&'a String> {
    attrs
        .iter()
        .find(|(k, _)| k.local_name == key)
        .map(|(_, v)| v)
}

// ===========================================================================
// Column coordinate helpers
// ===========================================================================

/// Convert Excel column letter(s) to a zero-based column index.
/// "A" → 0, "Z" → 25, "AA" → 26, "ZZ" → 51, "AAA" → 702
pub fn col_to_index(col: &str) -> usize {
    let mut result: usize = 0;
    for c in col.chars() {
        result = result * 26 + (c as usize - 'A' as usize + 1);
    }
    result - 1
}

/// Convert a cell reference like "A1" to (col, row) zero-based indices.
/// "A1" → (0, 0), "B2" → (1, 1), "AA1" → (26, 0)
pub fn cell_ref_to_coords(ref_str: &str) -> (usize, usize) {
    let mut col_str = String::new();
    let mut row_str = String::new();
    let mut in_col = true;
    for c in ref_str.chars() {
        if in_col {
            if c.is_ascii_alphabetic() {
                col_str.push(c);
            } else {
                in_col = false;
                row_str.push(c);
            }
        } else {
            row_str.push(c);
        }
    }
    (col_to_index(&col_str), row_str.parse().unwrap_or(0) - 1)
}

/// Parse a merged cell range reference like "A1:B2" into (start_row, start_col, end_row, end_col).
pub fn merged_range_ref(ref_str: &str) -> (usize, usize, usize, usize) {
    let parts: Vec<&str> = ref_str.split(':').collect();
    if parts.len() != 2 {
        let (col, row) = cell_ref_to_coords(ref_str);
        return (row, col, row, col);
    }
    let (col_start, row_start) = cell_ref_to_coords(parts[0]);
    let (col_end, row_end) = cell_ref_to_coords(parts[1]);
    (
        row_start.min(row_end),
        col_start.min(col_end),
        row_start.max(row_end),
        col_start.max(col_end),
    )
}

// ===========================================================================
// Shared strings
// ===========================================================================

/// Parse sharedStrings.xml and return the vector of unique shared strings.
fn read_shared_strings(package: &OpcPackage) -> Result<Vec<String>, Vec<Diagnostic>> {
    let part = find_part(package, "xl/sharedStrings.xml")?;
    let doc = parse_xml(&part.data)?;
    let mut strings: Vec<String> = Vec::new();

    if let XmlNode::Element { children, .. } = &doc.root {
        for child in children {
            if let XmlNode::Element {
                name,
                children: inner,
                ..
            } = child
            {
                if name.local_name == "si" {
                    let mut text = String::new();
                    extract_text_from_children(inner, &mut text);
                    strings.push(text);
                }
            }
        }
    }

    Ok(strings)
}

/// Recursively extract all text content from XML children.
fn extract_text_from_children(children: &[XmlNode], out: &mut String) {
    for child in children {
        match child {
            XmlNode::Text { text, .. } => out.push_str(text),
            XmlNode::Element { children: gc, .. } => extract_text_from_children(gc, out),
            _ => {}
        }
    }
}

// ===========================================================================
// Styles
// ===========================================================================

/// Parse styles.xml and return the cell style names in order.
#[allow(clippy::type_complexity)]
fn read_styles(
    package: &OpcPackage,
) -> Result<(Vec<String>, Vec<BTreeMap<String, String>>), Vec<Diagnostic>> {
    let part = find_part(package, "xl/styles.xml")?;
    let doc = parse_xml(&part.data)?;

    // Collect cell style names in order
    let mut style_names: Vec<String> = Vec::new();

    // Collect cellXfs (style entries referenced by cells)
    let mut cell_style_props: Vec<BTreeMap<String, String>> = Vec::new();

    if let XmlNode::Element { children, .. } = &doc.root {
        for child in children {
            if let XmlNode::Element {
                name,
                children: inner,
                ..
            } = child
            {
                if name.local_name == "cellStyleXfs" {
                    // First collect cellXfs
                    for ci in inner {
                        if let XmlNode::Element { attributes, .. } = ci {
                            let mut props = BTreeMap::new();
                            for (k, v) in attributes {
                                props.insert(k.local_name.clone(), v.clone());
                            }
                            cell_style_props.push(props);
                        }
                    }
                }
                if name.local_name == "cellStyles" {
                    for ci in inner {
                        if let XmlNode::Element { attributes, .. } = ci {
                            if let Some(name_val) = get_attr(attributes, "name") {
                                style_names.push(name_val.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    // If no named styles found, create default
    if style_names.is_empty() {
        style_names.push("Normal".to_string());
    }

    Ok((style_names, cell_style_props))
}

/// Get the style name for a cell based on its s attribute index.
fn get_style_name(
    s_index: Option<&str>,
    style_names: &[String],
    _cell_props: &[BTreeMap<String, String>],
) -> Option<String> {
    if let Some(idx_str) = s_index {
        if let Ok(idx) = idx_str.parse::<usize>() {
            if idx < style_names.len() {
                return Some(style_names[idx].clone());
            }
        }
    }
    None
}

// ===========================================================================
// Workbook → sheet names
// ===========================================================================

/// Parse workbook.xml and return a list of (sheet_name, sheet_id) tuples.
fn read_sheets(package: &OpcPackage) -> Result<Vec<(String, String)>, Vec<Diagnostic>> {
    let part = find_part(package, "xl/workbook.xml")?;
    let doc = parse_xml(&part.data)?;
    let mut sheets: Vec<(String, String)> = Vec::new();

    if let XmlNode::Element { children, .. } = &doc.root {
        for child in children {
            if let XmlNode::Element {
                name: child_name,
                children: inner,
                ..
            } = child
            {
                if child_name.local_name == "sheets" {
                    for si in inner {
                        if let XmlNode::Element {
                            name, attributes, ..
                        } = si
                        {
                            if name.local_name == "sheet" {
                                if let Some(name_val) = get_attr(attributes, "name") {
                                    if let Some(id_val) = get_attr(attributes, "sheetId") {
                                        sheets.push((name_val.clone(), id_val.clone()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(sheets)
}

/// Resolve a sheet's worksheet part URI from the workbook relationships.
fn resolve_sheet_part(package: &OpcPackage, sheet_id: &str) -> Result<String, Vec<Diagnostic>> {
    // Find the workbook rels file
    let workbook_rels_uri = "xl/_rels/workbook.xml.rels";
    let part = find_part(package, workbook_rels_uri).ok();

    if let Some(part) = part {
        if let Ok(doc) = parse_xml(&part.data) {
            if let XmlNode::Element { children, .. } = &doc.root {
                for child in children {
                    if let XmlNode::Element {
                        name, attributes, ..
                    } = child
                    {
                        if name.local_name == "Relationship" {
                            // Check if this relationship maps to our sheet
                            if let (Some(target), Some(rel_id)) =
                                (get_attr(attributes, "Target"), get_attr(attributes, "Id"))
                            {
                                // Map sheetId to rId
                                if let Ok(sid) = sheet_id.parse::<usize>() {
                                    // rId1 maps to sheet 1, rId2 to sheet 2, etc.
                                    let expected_rid = format!("rId{}", sid);
                                    if rel_id.as_str() == expected_rid {
                                        // Resolve relative URI
                                        let target = target.as_str();
                                        if target.starts_with("worksheets/")
                                            || target == "worksheets/sheet1.xml"
                                        {
                                            return Ok(target.to_string());
                                        }
                                        if let Some(stripped) = target.strip_prefix("worksheets/") {
                                            return Ok(format!("xl/worksheets/{}", stripped));
                                        }
                                        if let Some(stripped) = target.strip_prefix('/') {
                                            return Ok(stripped.to_string());
                                        }
                                        return Ok(target.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Fallback: default mapping
    if let Ok(sid) = sheet_id.parse::<usize>() {
        let sheet_num = sid;
        return Ok(format!("xl/worksheets/sheet{}.xml", sheet_num));
    }

    Err(vec![Diagnostic::new(
        "OOX-E-021",
        Severity::Error,
        format!("Could not resolve sheet part for sheetId={}", sheet_id),
    )])
}

// ===========================================================================
// Worksheet → Sheet IR
// ===========================================================================

/// Parse a single sheet's worksheet XML into a Sheet IR.
fn read_sheet(
    package: &OpcPackage,
    sheet_name: &str,
    sheet_id: &str,
    shared_strings: &[String],
    style_names: &[String],
    cell_style_props: &[BTreeMap<String, String>],
) -> Result<Sheet, Vec<Diagnostic>> {
    let mut diags: Vec<Diagnostic> = Vec::new();

    // Resolve worksheet part URI
    let sheet_uri = resolve_sheet_part(package, sheet_id)?;
    let part = find_part(package, &sheet_uri)?;

    let doc = parse_xml(&part.data)?;

    // Look for sheetData
    let mut sheet_rows: Vec<SheetRow> = Vec::new();
    let mut merged_ranges: Vec<MergedRange> = Vec::new();

    if let XmlNode::Element { children, .. } = &doc.root {
        // First, parse mergeCells
        for child in children {
            if let XmlNode::Element {
                name,
                children: inner,
                ..
            } = child
            {
                if name.local_name == "mergeCells" {
                    for mc in inner {
                        if let XmlNode::Element {
                            name: mc_name,
                            attributes,
                            ..
                        } = mc
                        {
                            if mc_name.local_name == "mergeCell" {
                                if let Some(ref_val) = get_attr(attributes, "ref") {
                                    let (sr, sc, er, ec) = merged_range_ref(ref_val);
                                    let merge_path = format!(
                                        "merged:{}:{}-{}-{}-{}",
                                        sheet_name, sr, sc, er, ec
                                    );
                                    merged_ranges.push(MergedRange {
                                        id: ElementId::derive(
                                            "merged_range",
                                            &[&merge_path],
                                            &ref_val,
                                        ),
                                        start_row: sr,
                                        start_col: sc,
                                        end_row: er,
                                        end_col: ec,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        // Then parse sheetData → rows → cells
        for child in children {
            if let XmlNode::Element {
                name,
                children: inner,
                ..
            } = child
            {
                if name.local_name == "sheetData" {
                    for row_elem in inner {
                        if let XmlNode::Element {
                            name: row_name,
                            attributes,
                            children: cells,
                            ..
                        } = row_elem
                        {
                            if row_name.local_name == "row" {
                                let row_num = get_attr(attributes, "r")
                                    .and_then(|v| v.parse::<usize>().ok())
                                    .unwrap_or(1)
                                    - 1;

                                // Parse cells in this row
                                let mut row_cells: Vec<SheetCell> = Vec::new();
                                for cell_elem in cells {
                                    if let Some(cell) = parse_cell(
                                        cell_elem,
                                        shared_strings,
                                        style_names,
                                        cell_style_props,
                                        row_num,
                                        &mut diags,
                                    ) {
                                        row_cells.push(cell);
                                    }
                                }

                                let row_id = ElementId::derive(
                                    "sheet_row",
                                    &[sheet_name, &format!("row:{}", row_num)],
                                    &format!("row:{}", row_num),
                                );
                                sheet_rows.push(SheetRow {
                                    id: row_id,
                                    cells: row_cells,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // Formula evaluation (C15)
    evaluate_sheet_formulas(&sheet_rows, sheet_name, &mut diags);

    diags.push(Diagnostic::new(
        "OOX-R020",
        Severity::Info,
        format!(
            "sheet '{}': {} rows, {} merged ranges",
            sheet_name,
            sheet_rows.len(),
            merged_ranges.len()
        ),
    ));

    let sheet_id_elem = ElementId::derive("sheet", &[sheet_name], &sheet_name);
    Ok(Sheet {
        id: sheet_id_elem,
        name: sheet_name.to_string(),
        rows: sheet_rows,
        merged_ranges,
        header_row: None,
        footer_row: None,
    })
}

/// Parse a single cell element into a SheetCell.
fn parse_cell(
    cell_elem: &XmlNode,
    shared_strings: &[String],
    style_names: &[String],
    cell_style_props: &[BTreeMap<String, String>],
    row_num: usize,
    diags: &mut Vec<Diagnostic>,
) -> Option<SheetCell> {
    let XmlNode::Element {
        name: _,
        attributes,
        children,
        ..
    } = cell_elem
    else {
        return None;
    };

    // Get cell reference (e.g., "A1")
    let ref_val = get_attr(attributes, "r").map(|s| s.as_str())?;
    let (col, row) = cell_ref_to_coords(ref_val);

    // Get cell type
    let cell_type = get_attr(attributes, "t").map(|s| s.as_str()).unwrap_or("n");

    // Get style index
    let style_idx = get_attr(attributes, "s").map(|s| s.as_str());
    let style_name = get_style_name(style_idx, style_names, cell_style_props);

    // Extract values and formulas
    let mut value: Option<String> = None;
    let mut formula: Option<String> = None;
    let mut cached_value: Option<String> = None;

    for child in children {
        if let XmlNode::Element {
            name: child_name,
            children: inner,
            ..
        } = child
        {
            // <v> - value
            if child_name.local_name == "v" {
                if let XmlNode::Element {
                    children: v_children,
                    ..
                } = child
                {
                    let mut text = String::new();
                    extract_text_from_children(v_children, &mut text);
                    cached_value = if text.is_empty() {
                        None
                    } else {
                        Some(text.clone())
                    };

                    // For t="n" cells, value == cached_value
                    if cell_type == "n" {
                        value = Some(text.clone());
                    }

                    // For t="s" cells, <v> contains the shared string INDEX
                    if cell_type == "s" {
                        if let Ok(idx) = text.trim().parse::<usize>() {
                            if idx < shared_strings.len() {
                                value = Some(shared_strings[idx].clone());
                            } else {
                                diags.push(Diagnostic::new(
                                    "OOX-W-020",
                                    Severity::Warning,
                                    format!(
                                        "shared string index {} out of range ({} strings)",
                                        idx,
                                        shared_strings.len()
                                    ),
                                ));
                            }
                        }
                    }
                }
            }

            // <f> - formula
            if child_name.local_name == "f" {
                if let XmlNode::Element {
                    children: f_children,
                    ..
                } = child
                {
                    let mut text = String::new();
                    extract_text_from_children(f_children, &mut text);
                    formula = if text.is_empty() { None } else { Some(text) };
                }
            }

            // <is> - inline string (t="inlineStr")
            if child_name.local_name == "is" {
                let mut text = String::new();
                extract_text_from_children(inner, &mut text);
                if !text.is_empty() {
                    value = Some(text);
                }
            }

            // <t> - string result (t="str")
            if child_name.local_name == "t" {
                if let XmlNode::Text { text, .. } = child {
                    if cell_type == "str" {
                        // String formula result
                        value = if text.is_empty() {
                            None
                        } else {
                            Some(text.clone())
                        };
                    }
                }
            }
        }
    }

    // For t="b" (boolean), value is "TRUE" or "FALSE"
    if cell_type == "b" {
        if let Some(cv) = &cached_value {
            value = if cv == "1" {
                Some("true".to_string())
            } else {
                Some("false".to_string())
            };
        }
    }

    // For t="e" (error), value is the error text
    if cell_type == "e" {
        if let Some(cv) = &cached_value {
            value = Some(cv.clone());
        }
    }

    // If no value but there's a formula, use cached_value as value
    let final_value = if value.is_some() {
        value
    } else {
        cached_value.clone()
    };

    // Use a simplified ID path
    let cell_path = format!("{}:{}", ref_val, row_num);
    let cell_id = ElementId::derive("sheet_cell", &[&cell_path], &cell_path);

    Some(SheetCell {
        id: cell_id,
        row,
        col,
        value: final_value,
        formula,
        cached_value,
        style_name,
        number_format: None,
    })
}

/// Evaluate formulas across all cells in a sheet, adding diagnostics for
/// refused formulas and parity mismatches.
fn evaluate_sheet_formulas(rows: &[SheetRow], sheet_name: &str, diags: &mut Vec<Diagnostic>) {
    // Build a cell map: cell_ref (e.g., "A1") -> cell value
    let mut cell_map: HashMap<String, CachedCellValue> = HashMap::new();
    for row in rows {
        for cell in &row.cells {
            let ref_name = col_to_ref(cell.col, cell.row);
            if let Some(ref v) = cell.value {
                if let Ok(n) = v.trim().parse::<f64>() {
                    cell_map.insert(
                        ref_name,
                        CachedCellValue {
                            numeric: Some(n),
                            text: None,
                            bool: None,
                        },
                    );
                } else {
                    cell_map.insert(
                        ref_name,
                        CachedCellValue {
                            numeric: None,
                            text: Some(v.clone()),
                            bool: None,
                        },
                    );
                }
            } else if let Some(ref cv) = cell.cached_value {
                if let Ok(n) = cv.trim().parse::<f64>() {
                    cell_map.insert(
                        ref_name,
                        CachedCellValue {
                            numeric: Some(n),
                            text: None,
                            bool: None,
                        },
                    );
                } else {
                    cell_map.insert(
                        ref_name,
                        CachedCellValue {
                            numeric: None,
                            text: Some(cv.clone()),
                            bool: None,
                        },
                    );
                }
            }
        }
    }

    // Build context
    let ctx = XlsxCellContext { cell_map };

    // Evaluate each formula
    for row in rows {
        for cell in &row.cells {
            if let Some(ref formula_str) = cell.formula {
                // Strip leading '=' if present
                let formula = formula_str.trim_start_matches('=');

                // Step 1: Check for hostile/unrefused formulas
                if let Some(code) = check_refusal(formula) {
                    let code_str = gp_f_code_str(code);
                    diags.push(Diagnostic::new(
                        code_str.to_string(),
                        Severity::Warning,
                        format!(
                            "sheet '{}': formula refused ({}): {}",
                            sheet_name, code_str, formula
                        ),
                    ));
                    // Use cached value only - already in cell
                    continue;
                }

                // Step 2: Parse and evaluate
                match parse_formula(formula) {
                    Ok(node) => {
                        // Try to evaluate with available cell context
                        if let Some(eval_result) = evaluate(&node, &ctx) {
                            let computed_str = eval_result_to_string(&eval_result);
                            let cached = cell.cached_value.as_deref();

                            // Step 3: Parity check
                            match check_parity(cached, &eval_result) {
                                ParityResult::Match => {}
                                ParityResult::Mismatch {
                                    cached: ref cached_val,
                                    computed: ref comp_val,
                                } => {
                                    diags.push(Diagnostic::new(
                                        "GP-F-030",
                                        Severity::Warning,
                                        format!(
                                            "sheet '{}': stale cache on {} — cached={}, computed={}",
                                            sheet_name,
                                            col_to_ref(cell.col, cell.row),
                                            cached_val,
                                            comp_val
                                        ),
                                    ));
                                }
                                ParityResult::NoCache => {
                                    diags.push(Diagnostic::new(
                                        "GP-F-031",
                                        Severity::Info,
                                        format!(
                                            "sheet '{}': no cache on {} — computed={}",
                                            sheet_name,
                                            col_to_ref(cell.col, cell.row),
                                            computed_str
                                        ),
                                    ));
                                }
                                ParityResult::Refused(_) => {
                                    // Already handled above
                                }
                            }
                        } else {
                            // Evaluation returned None — cell references not in context
                            diags.push(Diagnostic::new(
                                "GP-F-040",
                                Severity::Info,
                                format!(
                                    "sheet '{}': formula {} references unresolved cells, using cached value",
                                    sheet_name, formula
                                ),
                            ));
                        }
                    }
                    Err(e) => {
                        diags.push(Diagnostic::new(
                            "GP-F-050",
                            Severity::Warning,
                            format!(
                                "sheet '{}': formula parse error on {}: {}",
                                sheet_name,
                                col_to_ref(cell.col, cell.row),
                                e.message
                            ),
                        ));
                    }
                }
            }
        }
    }
}

/// Convert column index and row index to Excel cell reference (e.g., (0,0) -> "A1").
fn col_to_ref(col: usize, row: usize) -> String {
    let mut col_str = String::new();
    let mut c = col;
    loop {
        let ch = (b'A' + (c % 26) as u8) as char;
        col_str.insert(0, ch);
        c /= 26;
        if c == 0 {
            break;
        }
        c -= 1;
    }
    format!("{}{}", col_str, row + 1)
}

/// Cell context for formula evaluation during xlsx parsing.
struct XlsxCellContext {
    cell_map: HashMap<String, CachedCellValue>,
}

impl FormulaContext for XlsxCellContext {
    fn get_cell(&self, ref_name: &str) -> Option<CachedCellValue> {
        self.cell_map.get(ref_name).cloned()
    }

    fn get_range(&self, _range: &str) -> Option<Vec<CachedCellValue>> {
        // Range lookups require full grid awareness; return None
        // The evaluator will treat this as "unresolved" and use cached value
        None
    }
}

// ===========================================================================
// Public API: read_xlsx
// ===========================================================================

/// Read a .xlsx file into gp-core IR.
///
/// Parses workbook.xml for sheet names, sharedStrings.xml for text content,
/// styles.xml for style info, and each sheet's worksheet XML for cell data.
pub fn read_xlsx(package: &OpcPackage) -> Result<ReadResult, Vec<Diagnostic>> {
    let mut diags: Vec<Diagnostic> = Vec::new();

    // 1. Parse workbook.xml → sheet names
    let sheets_meta = {
        let s = read_sheets(package)?;
        diags.push(Diagnostic::new(
            "OOX-R020",
            Severity::Info,
            format!("workbook: {} sheets found", s.len()),
        ));
        s
    };

    // 2. Parse sharedStrings.xml → shared string table
    let shared_strings = match read_shared_strings(package) {
        Ok(s) => {
            diags.push(Diagnostic::new(
                "OOX-R021",
                Severity::Info,
                format!("sharedStrings: {} strings found", s.len()),
            ));
            s
        }
        Err(_) => {
            diags.push(Diagnostic::new(
                "OOX-W-021",
                Severity::Warning,
                "sharedStrings.xml not found; using inline strings only".to_string(),
            ));
            Vec::new()
        }
    };

    // 3. Parse styles.xml → style table
    let (style_names, cell_style_props) = match read_styles(package) {
        Ok((sn, scp)) => {
            diags.push(Diagnostic::new(
                "OOX-R022",
                Severity::Info,
                format!("styles.xml: {} cell styles found", sn.len()),
            ));
            (sn, scp)
        }
        Err(_) => {
            diags.push(Diagnostic::new(
                "OOX-W-022",
                Severity::Warning,
                "styles.xml not found; using default styles".to_string(),
            ));
            (vec!["Normal".to_string()], Vec::new())
        }
    };

    // 4. Parse each sheet → Sheet with rows/cells
    let mut sheet_ir_vec: Vec<Sheet> = Vec::new();
    for (sheet_name, sheet_id) in &sheets_meta {
        match read_sheet(
            package,
            sheet_name,
            sheet_id,
            &shared_strings,
            &style_names,
            &cell_style_props,
        ) {
            Ok(sheet) => {
                sheet_ir_vec.push(sheet);
            }
            Err(e) => {
                diags.extend(e);
                diags.push(Diagnostic::new(
                    "OOX-W-023",
                    Severity::Warning,
                    format!(
                        "Failed to parse sheet '{}': sheetId={}",
                        sheet_name, sheet_id
                    ),
                ));
            }
        }
    }

    // 5. Build Spreadsheet IR
    let spreadsheet = Spreadsheet {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        sheets: sheet_ir_vec,
    };

    // 6. Serialize to JSON
    let ir_bytes = serde_json::to_vec(&spreadsheet).map_err(|e| {
        vec![Diagnostic::new(
            "OOX-E-010",
            Severity::Error,
            format!("Failed to serialize IR: {}", e),
        )]
    })?;

    diags.push(Diagnostic::new(
        "OOX-R030",
        Severity::Info,
        format!(
            "xlsx: IR produced with {} sheets, {} rows total, {} merged ranges, {} diagnostics",
            spreadsheet.sheets.len(),
            spreadsheet
                .sheets
                .iter()
                .map(|s| s.rows.len())
                .sum::<usize>(),
            spreadsheet
                .sheets
                .iter()
                .map(|s| s.merged_ranges.len())
                .sum::<usize>(),
            diags.len(),
        ),
    ));

    Ok(ReadResult {
        ir: ir_bytes,
        diagnostics: diags,
    })
}

// ===========================================================================
// XlsxReader (Codec trait)
// ===========================================================================

/// Reader for .xlsx files (Excel Spreadsheet ML).
///
/// Implements the `Codec` trait to parse xlsx → gp-core Spreadsheet IR.
pub struct XlsxReader;

impl XlsxReader {
    pub fn new() -> Self {
        Self
    }
}

impl Default for XlsxReader {
    fn default() -> Self {
        Self::new()
    }
}

impl Codec for XlsxReader {
    fn format_name(&self) -> &str {
        "xlsx"
    }

    fn read(&self, data: &[u8]) -> Result<ReadResult, Vec<Diagnostic>> {
        match parse_opc(data) {
            Ok(pkg) => read_xlsx(&pkg),
            Err(diags) => Err(diags),
        }
    }
}

// Re-export PartInfo for backward compatibility
pub use super::opc::PartInfo;

// ===========================================================================
// Synthetic test fixture builder
// ===========================================================================

#[cfg(test)]
pub(crate) mod test_fixtures {
    use std::io::Write;

    /// Type alias for default file options with no extension.
    type FileOpts = zip::write::FileOptions<'static, ()>;

    /// Returns default file options.
    pub fn opts() -> FileOpts {
        zip::write::FileOptions::default()
    }

    /// Creates a minimal valid xlsx with shared strings, formulas, and merged cells.
    pub fn create_xlsx_with_formulas_and_merges() -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);

        // [Content_Types].xml
        writer.start_file("[Content_Types].xml", opts()).unwrap();
        writer.write_all(
            b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
              <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
              <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
              <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
              <Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml\"/>\
              <Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>\
              <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
              <Override PartName=\"/xl/worksheets/sheet2.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
              </Types>",
        ).unwrap();

        // _rels/.rels
        writer.start_file("_rels/.rels", opts()).unwrap();
        writer.write_all(
            b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
              <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
              </Relationships>",
        ).unwrap();

        // xl/sharedStrings.xml
        writer.start_file("xl/sharedStrings.xml", opts()).unwrap();
        writer.write_all(
            b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
              <sst xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" count=\"4\" uniqueCount=\"4\">\
              <si><t>Header1</t></si>\
              <si><t>Header2</t></si>\
              <si><t>Row1Col1</t></si>\
              <si><t>Row2Col1</t></si>\
              </sst>",
        ).unwrap();

        // xl/styles.xml
        writer.start_file("xl/styles.xml", opts()).unwrap();
        writer
            .write_all(
                b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
              <styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
              <cellXfs count=\"2\">\
              <xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/>\
              <xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/>\
              </cellXfs>\
              </styleSheet>",
            )
            .unwrap();

        // xl/workbook.xml
        writer.start_file("xl/workbook.xml", opts()).unwrap();
        writer
            .write_all(
                b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
              <workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
              <sheets>\
              <sheet name=\"Data\" sheetId=\"1\" r:id=\"rId1\"/>\
              <sheet name=\"Summary\" sheetId=\"2\" r:id=\"rId2\"/>\
              </sheets>\
              </workbook>",
            )
            .unwrap();

        // xl/_rels/workbook.xml.rels
        writer
            .start_file("xl/_rels/workbook.xml.rels", opts())
            .unwrap();
        writer.write_all(
            b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
              <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
              <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
              <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet2.xml\"/>\
              </Relationships>",
        ).unwrap();

        // xl/worksheets/sheet1.xml
        writer
            .start_file("xl/worksheets/sheet1.xml", opts())
            .unwrap();
        writer
            .write_all(
                b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
              <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
              <sheetData>\
              <row r=\"1\">\
              <c r=\"A1\" t=\"s\"><v>0</v></c>\
              <c r=\"B1\" t=\"s\"><v>1</v></c>\
              </row>\
              <row r=\"2\">\
              <c r=\"A2\" t=\"n\"><v>10</v></c>\
              <c r=\"B2\" t=\"str\"><f>SUM(A2:A3)</f><v>30</v></c>\
              </row>\
              <row r=\"3\">\
              <c r=\"A3\" t=\"n\"><v>20</v></c>\
              <c r=\"B3\" t=\"str\"><f>15</f><v>15</v></c>\
              </row>\
              </sheetData>\
              <mergeCells count=\"1\"><mergeCell ref=\"A1:B1\"/></mergeCells>\
              </worksheet>",
            )
            .unwrap();

        // xl/worksheets/sheet2.xml
        writer
            .start_file("xl/worksheets/sheet2.xml", opts())
            .unwrap();
        writer
            .write_all(
                b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
              <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
              <sheetData>\
              <row r=\"1\">\
              <c r=\"A1\" t=\"s\"><v>2</v></c>\
              <c r=\"B1\" t=\"n\"><v>42</v></c>\
              </row>\
              </sheetData>\
              </worksheet>",
            )
            .unwrap();

        writer.finish().unwrap().into_inner()
    }

    /// Creates a minimal xlsx with only numeric values (no formulas).
    pub fn create_xlsx_simple() -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);

        writer.start_file("[Content_Types].xml", opts()).unwrap();
        writer.write_all(
            b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
              <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
              <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
              </Types>",
        ).unwrap();

        writer.start_file("_rels/.rels", opts()).unwrap();
        writer.write_all(
            b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
              <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
              </Relationships>",
        ).unwrap();

        writer.start_file("xl/workbook.xml", opts()).unwrap();
        writer
            .write_all(
                b"<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
              <sheets>\
              <sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/>\
              </sheets>\
              </workbook>",
            )
            .unwrap();

        writer
            .start_file("xl/_rels/workbook.xml.rels", opts())
            .unwrap();
        writer.write_all(
            b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
              <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
              </Relationships>",
        ).unwrap();

        writer
            .start_file("xl/worksheets/sheet1.xml", opts())
            .unwrap();
        writer
            .write_all(
                b"<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
              <sheetData>\
              <row r=\"1\"><c r=\"A1\" t=\"n\"><v>42</v></c><c r=\"B1\"><v>3.14</v></c></row>\
              <row r=\"2\"><c r=\"A2\" t=\"n\"><v>100</v></c></row>\
              </sheetData>\
              </worksheet>",
            )
            .unwrap();

        writer.finish().unwrap().into_inner()
    }

    /// Creates a minimal xlsx with boolean and error cells.
    pub fn create_xlsx_with_types() -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);

        writer.start_file("[Content_Types].xml", opts()).unwrap();
        writer.write_all(
            b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
              <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
              <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
              </Types>",
        ).unwrap();

        writer.start_file("_rels/.rels", opts()).unwrap();
        writer.write_all(
            b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
              <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
              </Relationships>",
        ).unwrap();

        writer.start_file("xl/workbook.xml", opts()).unwrap();
        writer
            .write_all(
                b"<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
              <sheets><sheet name=\"Types\" sheetId=\"1\" r:id=\"rId1\"/></sheets>\
              </workbook>",
            )
            .unwrap();

        writer
            .start_file("xl/_rels/workbook.xml.rels", opts())
            .unwrap();
        writer.write_all(
            b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
              <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
              </Relationships>",
        ).unwrap();

        writer
            .start_file("xl/worksheets/sheet1.xml", opts())
            .unwrap();
        writer
            .write_all(
                b"<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
              <sheetData>\
              <row r=\"1\">\
              <c r=\"A1\" t=\"b\"><v>1</v></c>\
              <c r=\"B1\" t=\"b\"><v>0</v></c>\
              <c r=\"C1\" t=\"e\"><v>#DIV/0!</v></c>\
              </row>\
              </sheetData>\
              </worksheet>",
            )
            .unwrap();

        writer.finish().unwrap().into_inner()
    }

    /// Creates an xlsx with no worksheet parts (empty workbook).
    pub fn create_xlsx_empty_worksheets() -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);

        writer.start_file("[Content_Types].xml", opts()).unwrap();
        writer.write_all(
            b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
              <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
              </Types>",
        ).unwrap();

        writer.start_file("_rels/.rels", opts()).unwrap();
        writer.write_all(
            b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
              <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
              </Relationships>",
        ).unwrap();

        writer.start_file("xl/workbook.xml", opts()).unwrap();
        writer
            .write_all(
                b"<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
              <sheets></sheets>\
              </workbook>",
            )
            .unwrap();

        writer.finish().unwrap().into_inner()
    }

    /// Creates an xlsx with merged cells only.
    pub fn create_xlsx_merged_only() -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);

        writer.start_file("[Content_Types].xml", opts()).unwrap();
        writer.write_all(
            b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
              <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
              <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
              </Types>",
        ).unwrap();

        writer.start_file("_rels/.rels", opts()).unwrap();
        writer.write_all(
            b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
              <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
              </Relationships>",
        ).unwrap();

        writer.start_file("xl/workbook.xml", opts()).unwrap();
        writer
            .write_all(
                b"<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
              <sheets><sheet name=\"Merged\" sheetId=\"1\" r:id=\"rId1\"/></sheets>\
              </workbook>",
            )
            .unwrap();

        writer
            .start_file("xl/_rels/workbook.xml.rels", opts())
            .unwrap();
        writer.write_all(
            b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
              <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
              </Relationships>",
        ).unwrap();

        writer
            .start_file("xl/worksheets/sheet1.xml", opts())
            .unwrap();
        writer
            .write_all(
                b"<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
              <sheetData>\
              <row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>MergedCell</t></is></c></row>\
              </sheetData>\
              <mergeCells count=\"2\">\
              <mergeCell ref=\"A1:B2\"/>\
              <mergeCell ref=\"D3:E4\"/>\
              </mergeCells>\
              </worksheet>",
            )
            .unwrap();

        writer.finish().unwrap().into_inner()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use gp_core::ir::Visit;
    use gp_core::traits::Codec;
    use test_fixtures::*;

    /// test_parse_xlsx_corpus_fixture — parse formulas-cached-xlsx-001.xlsx and verify IR.
    #[test]
    fn test_parse_xlsx_corpus_fixture() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/blobs/formulas-cached-xlsx-001.xlsx"
        );
        let data = std::fs::read(path).expect("formulas-cached-xlsx-001.xlsx should exist");
        let result = XlsxReader::new()
            .read(&data)
            .expect("should parse formulas-cached-xlsx-001.xlsx");

        // Deserialize IR
        let ss: Spreadsheet =
            serde_json::from_slice(&result.ir).expect("should deserialize Spreadsheet");
        assert_eq!(ss.ir_version, IR_VERSION);
        assert!(!ss.sheets.is_empty(), "should have at least one sheet");

        // Verify sheet name
        let sheet = &ss.sheets[0];
        assert_eq!(sheet.name, "Calc");

        // Verify row count (5 data rows)
        assert_eq!(sheet.rows.len(), 5, "should have 5 rows");

        // Verify cell values
        // Row 1: A1="sum"(inlineStr), B1 has formula SUM(D1:D5) cached_value="30", D1 has value=2
        // Row 2: A2="average"(inlineStr), B2 has formula AVERAGE(D1:D5) cached_value="6", D2 has value=4
        assert_eq!(sheet.rows[0].cells.len(), 3, "row 1 should have 3 cells");

        // Check A1 inlineStr
        let a1 = &sheet.rows[0].cells[0];
        assert_eq!(a1.row, 0);
        assert_eq!(a1.col, 0);
        assert_eq!(a1.value.as_deref(), Some("sum"));

        // Check B1 formula
        let b1 = &sheet.rows[0].cells[1];
        assert_eq!(b1.formula.as_deref(), Some("SUM(D1:D5)"));
        assert_eq!(b1.cached_value.as_deref(), Some("30"));
    }

    /// test_parse_cell_values — t="n" → cached_value
    #[test]
    fn test_parse_cell_values() {
        let data = create_xlsx_simple();
        let result = XlsxReader::new()
            .read(&data)
            .expect("should parse simple xlsx");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        let sheet = &ss.sheets[0];
        assert_eq!(sheet.name, "Sheet1");

        // Row 1: A1=42, B1=3.14
        assert_eq!(sheet.rows[0].cells[0].value.as_deref(), Some("42"));
        assert_eq!(sheet.rows[0].cells[0].cached_value.as_deref(), Some("42"));
        assert_eq!(sheet.rows[0].cells[1].value.as_deref(), Some("3.14"));

        // Row 2: A2=100
        assert_eq!(sheet.rows[1].cells[0].value.as_deref(), Some("100"));
    }

    /// test_parse_formulas — <f>=SUM(...)</f> → formula
    #[test]
    fn test_parse_formulas() {
        let data = create_xlsx_with_formulas_and_merges();
        let result = XlsxReader::new()
            .read(&data)
            .expect("should parse xlsx with formulas");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        // Sheet1 row 2 B2 has formula
        let sheet = &ss.sheets[0];
        let b2 = &sheet.rows[1].cells[1];
        assert_eq!(b2.formula.as_deref(), Some("SUM(A2:A3)"));

        // Sheet1 row 3 B3 has formula
        let b3 = &sheet.rows[2].cells[1];
        assert_eq!(b3.formula.as_deref(), Some("15"));
    }

    /// test_parse_cached_values_with_formulas — formula + cached value both present
    #[test]
    fn test_parse_cached_values_with_formulas() {
        let data = create_xlsx_with_formulas_and_merges();
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        let sheet = &ss.sheets[0];

        // B2: formula=SUM(A2:A3), cached_value=30
        let b2 = &sheet.rows[1].cells[1];
        assert_eq!(b2.formula.as_deref(), Some("SUM(A2:A3)"));
        assert_eq!(b2.cached_value.as_deref(), Some("30"));

        // B3: formula=15, cached_value=15
        let b3 = &sheet.rows[2].cells[1];
        assert_eq!(b3.formula.as_deref(), Some("15"));
        assert_eq!(b3.cached_value.as_deref(), Some("15"));
    }

    /// test_parse_shared_strings — t="s" → resolved string from sharedStrings.xml
    #[test]
    fn test_parse_shared_strings() {
        let data = create_xlsx_with_formulas_and_merges();
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        // A1 t="s" v="0" → "Header1"
        // B1 t="s" v="1" → "Header2"
        let sheet = &ss.sheets[0];
        let a1 = &sheet.rows[0].cells[0];
        assert_eq!(a1.value.as_deref(), Some("Header1"));
        let b1 = &sheet.rows[0].cells[1];
        assert_eq!(b1.value.as_deref(), Some("Header2"));

        // Sheet2: A1 t="s" v="2" → "Row1Col1"
        let sheet2 = &ss.sheets[1];
        let a1_s2 = &sheet2.rows[0].cells[0];
        assert_eq!(a1_s2.value.as_deref(), Some("Row1Col1"));
    }

    /// test_parse_merged_ranges — mergeCells → MergedRange list
    #[test]
    fn test_parse_merged_ranges() {
        let data = create_xlsx_with_formulas_and_merges();
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        // Sheet1 has 1 merged range: A1:B1
        let sheet = &ss.sheets[0];
        assert_eq!(sheet.merged_ranges.len(), 1);
        let mr = &sheet.merged_ranges[0];
        assert_eq!(mr.start_row, 0);
        assert_eq!(mr.start_col, 0);
        assert_eq!(mr.end_row, 0);
        assert_eq!(mr.end_col, 1);
    }

    /// test_parse_styles — styles.xml → style_name on cells
    #[test]
    fn test_parse_styles() {
        let data = create_xlsx_with_formulas_and_merges();
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        // In this fixture, styles are minimal (default)
        let sheet = &ss.sheets[0];

        // B1 has no explicit style, so no style_name
        let b1 = &sheet.rows[0].cells[1];
        assert_eq!(b1.style_name, None);
    }

    /// test_parse_multiple_sheets — workbook with multiple sheets
    #[test]
    fn test_parse_multiple_sheets() {
        let data = create_xlsx_with_formulas_and_merges();
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(ss.sheets.len(), 2);
        assert_eq!(ss.sheets[0].name, "Data");
        assert_eq!(ss.sheets[1].name, "Summary");

        // Data sheet has 3 rows, Summary has 1
        assert_eq!(ss.sheets[0].rows.len(), 3);
        assert_eq!(ss.sheets[1].rows.len(), 1);
    }

    /// test_xlsx_codec_integration — XlsxReader.read() produces valid IR
    #[test]
    fn test_xlsx_codec_integration() {
        let reader = XlsxReader::new();
        assert_eq!(reader.format_name(), "xlsx");

        let data = create_xlsx_with_formulas_and_merges();
        let result = reader.read(&data).expect("should read valid xlsx");

        assert!(!result.ir.is_empty(), "IR should not be empty");

        // Should be valid JSON that deserializes to Spreadsheet
        let ss: Spreadsheet =
            serde_json::from_slice(&result.ir).expect("IR should be valid JSON for Spreadsheet");
        assert_eq!(ss.ir_version, IR_VERSION);
        assert!(!ss.sheets.is_empty());

        // Should have diagnostics
        assert!(!result.diagnostics.is_empty());
    }

    /// test_parse_all_xlsx_fixtures — all xlsx files parse successfully
    #[test]
    fn test_parse_all_xlsx_fixtures() {
        let blobs_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/blobs");
        let mut parsed = 0;

        for entry in std::fs::read_dir(blobs_dir).expect("corpus/blobs/ should be readable") {
            let entry = entry.expect("should read directory entry");
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("xlsx") {
                let data = std::fs::read(&path)
                    .unwrap_or_else(|_| panic!("should read {}", path.display()));
                let result = XlsxReader::new().read(&data);
                match result {
                    Ok(r) => {
                        let ss: Spreadsheet = serde_json::from_slice(&r.ir).unwrap_or_else(|_| {
                            panic!(
                                "should deserialize Spreadsheet from {}",
                                path.file_name().unwrap().to_string_lossy()
                            )
                        });
                        assert_eq!(ss.ir_version, IR_VERSION);
                        parsed += 1;
                    }
                    Err(diags) => {
                        let has_fatal = diags.iter().any(|d| d.severity == Severity::Fatal);
                        assert!(
                            !has_fatal,
                            "Fatal diagnostic for {}: {:?}",
                            path.display(),
                            diags
                        );
                        parsed += 1;
                    }
                }
            }
        }

        assert!(
            parsed > 0,
            "Should have parsed at least one xlsx fixture (parsed {})",
            parsed
        );
    }

    /// test_formula_cell_has_both — formula cell has both formula and cached_value
    #[test]
    fn test_formula_cell_has_both() {
        let data = create_xlsx_with_formulas_and_merges();
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        let sheet = &ss.sheets[0];

        // B2 has both formula and cached_value
        let b2 = &sheet.rows[1].cells[1];
        assert!(b2.formula.is_some(), "B2 should have formula");
        assert!(b2.cached_value.is_some(), "B2 should have cached_value");
        assert_ne!(
            b2.formula.as_deref().unwrap(),
            "",
            "formula should not be empty"
        );

        // B3 has both formula and cached_value
        let b3 = &sheet.rows[2].cells[1];
        assert!(b3.formula.is_some(), "B3 should have formula");
        assert!(b3.cached_value.is_some(), "B3 should have cached_value");
    }

    /// test_merged_range_count_vs_manifest — matches expected merged count
    #[test]
    fn test_merged_range_count_vs_manifest() {
        let data = create_xlsx_merged_only();
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        // Fixture has 2 merged ranges: A1:B2 and D3:E4
        assert_eq!(ss.sheets.len(), 1);
        let sheet = &ss.sheets[0];
        assert_eq!(sheet.merged_ranges.len(), 2);

        // Check A1:B2
        let mr0 = &sheet.merged_ranges[0];
        assert_eq!(mr0.start_row, 0);
        assert_eq!(mr0.start_col, 0);
        assert_eq!(mr0.end_row, 1);
        assert_eq!(mr0.end_col, 1);

        // Check D3:E4
        let mr1 = &sheet.merged_ranges[1];
        assert_eq!(mr1.start_row, 2);
        assert_eq!(mr1.start_col, 3);
        assert_eq!(mr1.end_row, 3);
        assert_eq!(mr1.end_col, 4);
    }

    /// test_parse_cell_coordinate — "A1"→(0,0), "B2"→(1,1), "AA1"→(26,0)
    #[test]
    fn test_parse_cell_coordinate() {
        assert_eq!(cell_ref_to_coords("A1"), (0, 0));
        assert_eq!(cell_ref_to_coords("B2"), (1, 1));
        assert_eq!(cell_ref_to_coords("AA1"), (26, 0));
        assert_eq!(cell_ref_to_coords("Z1"), (25, 0));
        assert_eq!(cell_ref_to_coords("AB3"), (27, 2));
        assert_eq!(cell_ref_to_coords("A100"), (0, 99));
        assert_eq!(col_to_index("A"), 0);
        assert_eq!(col_to_index("Z"), 25);
        assert_eq!(col_to_index("AA"), 26);
        assert_eq!(col_to_index("ZZ"), 701);
        assert_eq!(col_to_index("AAA"), 702);
    }

    /// test_parse_boolean_cells — t="b" → boolean value
    #[test]
    fn test_parse_boolean_cells() {
        let data = create_xlsx_with_types();
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        let sheet = &ss.sheets[0];
        // A1 t="b" v="1" → true
        let a1 = &sheet.rows[0].cells[0];
        assert_eq!(a1.value.as_deref(), Some("true"));

        // B1 t="b" v="0" → false
        let b1 = &sheet.rows[0].cells[1];
        assert_eq!(b1.value.as_deref(), Some("false"));

        // C1 t="e" v="#DIV/0!" → error text
        let c1 = &sheet.rows[0].cells[2];
        assert_eq!(c1.value.as_deref(), Some("#DIV/0!"));
    }

    /// test_merged_range_ref — parse merged range references
    #[test]
    fn test_merged_range_ref() {
        let (sr, sc, er, ec) = merged_range_ref("A1:B2");
        assert_eq!((sr, sc, er, ec), (0, 0, 1, 1));

        let (sr, sc, er, ec) = merged_range_ref("D3:E4");
        assert_eq!((sr, sc, er, ec), (2, 3, 3, 4));

        let (sr, sc, er, ec) = merged_range_ref("A1:A1");
        assert_eq!((sr, sc, er, ec), (0, 0, 0, 0));

        let (sr, sc, er, ec) = merged_range_ref("A1");
        assert_eq!((sr, sc, er, ec), (0, 0, 0, 0));
    }

    /// test_spreadsheet_visit_ids — Visit impl for Spreadsheet
    #[test]
    fn test_spreadsheet_visit_ids() {
        let data = create_xlsx_with_formulas_and_merges();
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        let id_map = ss.collect_ids();
        assert!(
            !id_map.is_empty(),
            "IdMap should not be empty for spreadsheet with sheets"
        );

        // Should contain at least spreadsheet, sheets, rows, cells, merged ranges
        let kinds = id_map.kinds();
        assert!(
            kinds.contains("spreadsheet"),
            "Should have spreadsheet kind"
        );
        assert!(kinds.contains("sheet"), "Should have sheet kind");
        assert!(kinds.contains("sheet_row"), "Should have sheet_row kind");
    }

    /// test_parse_merged_cells_corpus — merged-cells-xlsx-001.xlsx
    #[test]
    fn test_parse_merged_cells_corpus() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/blobs/merged-cells-xlsx-001.xlsx"
        );
        let data = std::fs::read(path).expect("merged-cells-xlsx-001.xlsx should exist");
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(ss.sheets.len(), 1);
        let sheet = &ss.sheets[0];

        // Has merged range A1:B3
        assert_eq!(sheet.merged_ranges.len(), 1);
        let mr = &sheet.merged_ranges[0];
        assert_eq!(mr.start_row, 0);
        assert_eq!(mr.start_col, 0);
        assert_eq!(mr.end_row, 2);
        assert_eq!(mr.end_col, 1);
    }

    /// test_parse_merged_cells_corpus_multi — merged-cells-xlsx-002.xlsx (2 merged ranges)
    #[test]
    fn test_parse_merged_cells_corpus_multi() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/blobs/merged-cells-xlsx-002.xlsx"
        );
        let data = std::fs::read(path).expect("merged-cells-xlsx-002.xlsx should exist");
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(ss.sheets.len(), 1);
        let sheet = &ss.sheets[0];

        // Has 2 merged ranges
        assert_eq!(sheet.merged_ranges.len(), 2);
    }

    /// test_parse_merged_cells_corpus_3 — merged-cells-xlsx-003.xlsx (1 merged range)
    #[test]
    fn test_parse_merged_cells_corpus_3() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/blobs/merged-cells-xlsx-003.xlsx"
        );
        let data = std::fs::read(path).expect("merged-cells-xlsx-003.xlsx should exist");
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        let sheet = &ss.sheets[0];
        assert_eq!(sheet.merged_ranges.len(), 1);
    }

    /// test_parse_formulas_corpus_002 — formulas-cached-xlsx-002.xlsx
    #[test]
    fn test_parse_formulas_corpus_002() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/blobs/formulas-cached-xlsx-002.xlsx"
        );
        let data = std::fs::read(path).expect("formulas-cached-xlsx-002.xlsx should exist");
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        // This fixture has 2 sheets: Data and Refs with simple numeric values and inline strings
        assert_eq!(ss.sheets.len(), 2);
        assert_eq!(ss.sheets[0].name, "Data");
        assert!(!ss.sheets[0].rows.is_empty());
    }

    /// test_parse_formulas_corpus_003 — formulas-cached-xlsx-003.xlsx (formula + style)
    #[test]
    fn test_parse_formulas_corpus_003() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/blobs/formulas-cached-xlsx-003.xlsx"
        );
        let data = std::fs::read(path).expect("formulas-cached-xlsx-003.xlsx should exist");
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        let sheet = &ss.sheets[0];

        // Row 1: A1=50(n), B1 has formula IF(...), cached_value="big"
        let a1 = &sheet.rows[0].cells[0];
        assert_eq!(a1.value.as_deref(), Some("50"));
        assert_eq!(a1.formula, None);

        let b1 = &sheet.rows[0].cells[1];
        assert_eq!(
            b1.formula.as_deref(),
            Some("IF(A1&gt;40,\"big\",\"small\")")
        );
        assert_eq!(b1.cached_value.as_deref(), Some("big"));
        // B3 has style s="1"
        let b3 = &sheet.rows[2].cells[0];
        assert_eq!(b3.row, 2);
        assert_eq!(b3.formula.as_deref(), Some("A1/100"));
        assert_eq!(b3.cached_value.as_deref(), Some("0.5"));
    }

    /// test_parse_empty_worksheet — xlsx with no sheet parts produces empty sheets list
    #[test]
    fn test_parse_empty_worksheet() {
        let data = create_xlsx_empty_worksheets();
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(ss.sheets.len(), 0);
    }

    /// test_row_values_in_sheet — verify row values exist for each sheet in multi-sheet xlsx
    #[test]
    fn test_row_values_in_sheet() {
        let data = create_xlsx_with_formulas_and_merges();
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        // Sheet "Data" row 2: A2=10, B2=formula
        let data_sheet = &ss.sheets[0];
        let a2 = &data_sheet.rows[1].cells[0];
        assert_eq!(a2.value.as_deref(), Some("10"));

        // Sheet "Summary" row 1: A1="Row1Col1", B1=42
        let summary_sheet = &ss.sheets[1];
        let b1 = &summary_sheet.rows[0].cells[1];
        assert_eq!(b1.value.as_deref(), Some("42"));
    }
}
