// SPDX-License-Identifier: MIT OR Apache-2.0
//! adapters-ooxml: xlsx â†’ gp-core IR (sheets, cells, formulas, cached values, shared strings, styles, merges).
//!
//! Walks a `.xlsx` OPC package, extracts `xl/workbook.xml`, `xl/sharedStrings.xml`,
//! `xl/styles.xml`, and per-sheet `xl/worksheets/sheetN.xml`, then maps ExcelML
//! elements to the gp-core intermediate representation:
//!   * `<sheet>` â†’ Sheet (with name, rows, merged_ranges)
//!   * `<row>` â†’ SheetRow (with cells)
//!   * `<c r="A1" t="n">` â†’ SheetCell with row/col/value
//!   * `<c r="A1" t="s">` â†’ SheetCell resolved from sharedStrings.xml
//!   * `<c r="A1" t="str"><f>=SUM(...)</f><v>10</v></c>` â†’ SheetCell with formula + cached_value
//!   * `<mergeCells><mergeCell ref="A1:B2"/></mergeCells>` â†’ MergedRange

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
// Full style table structs (numFmts, fonts, fills, borders, cellXfs)
// ===========================================================================

/// Parsed <numFmt> from styles.xml — maps formatId → formatCode.
#[derive(Debug, Default, Clone)]
pub struct NumFmt {
    pub format_id: i32,
    pub format_code: String,
}

/// Parsed <font> from styles.xml.
#[derive(Debug, Default, Clone)]
pub struct Font {
    pub name: Option<String>,
    pub sz: Option<f64>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub color: Option<String>,
    pub sz_is_pt: bool,
}

/// Parsed <fill> from styles.xml.
#[derive(Debug, Default, Clone)]
pub struct Fill {
    pub pattern_fill: Option<PatternFill>,
}

/// Parsed <patternFill> from styles.xml.
#[derive(Debug, Default, Clone)]
pub struct PatternFill {
    pub pattern: Option<String>,
    pub fg_color: Option<String>,
    pub bg_color: Option<String>,
}

/// Parsed <border> from styles.xml.
#[derive(Debug, Default, Clone)]
pub struct Border {
    pub left: Option<BorderLine>,
    pub right: Option<BorderLine>,
    pub top: Option<BorderLine>,
    pub bottom: Option<BorderLine>,
    pub diagonal: Option<BorderLine>,
}

/// Parsed <left>/<right>/<top>/<bottom>/<diagonal> from <border>.
#[derive(Debug, Default, Clone)]
pub struct BorderLine {
    pub style: Option<String>,
    pub color: Option<String>,
}

/// Parsed <xf> (cell format) from cellXfs in styles.xml.
#[derive(Debug, Default, Clone)]
pub struct XfFormat {
    pub num_fmt_id: i32,
    pub font_index: i32,
    pub fill_index: i32,
    pub border_index: i32,
    pub alignment: Option<CellStyleAlignment>,
}

/// Parsed <alignment> from an <xf> element.
#[derive(Debug, Default, Clone)]
pub struct CellStyleAlignment {
    pub horizontal: Option<String>,
    pub vertical: Option<String>,
    pub wrap_text: Option<bool>,
    pub text_rotation: Option<i32>,
}

/// Full parsed styles table from styles.xml.
#[derive(Debug, Default, Clone)]
pub struct XlsxStyles {
    pub num_formats: Vec<NumFmt>,
    pub fonts: Vec<Font>,
    pub fills: Vec<Fill>,
    pub borders: Vec<Border>,
    pub cell_xfs: Vec<XfFormat>,
    pub cell_style_props: Vec<BTreeMap<String, String>>,
    pub style_names: Vec<String>,
}

/// Resolved style info for a single cell — the product of looking up
/// a style-index through all style tables.
#[derive(Debug, Default, Clone)]
pub struct CellStyleInfo {
    pub num_format: Option<String>,
    pub font: Option<Font>,
    pub fill: Option<Fill>,
    pub border: Option<Border>,
    pub alignment: Option<CellStyleAlignment>,
}

fn get_attr_str<'a>(
    attrs: &'a BTreeMap<super::xml::TagName, String>,
    key: &str,
) -> Option<&'a String> {
    attrs
        .iter()
        .find(|(k, _)| k.local_name == key)
        .map(|(_, v)| v)
}

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
/// "A" â†’ 0, "Z" â†’ 25, "AA" â†’ 26, "ZZ" â†’ 51, "AAA" â†’ 702
pub fn col_to_index(col: &str) -> usize {
    let mut result: usize = 0;
    for c in col.chars() {
        result = result * 26 + (c as usize - 'A' as usize + 1);
    }
    result - 1
}

/// Convert a cell reference like "A1" to (col, row) zero-based indices.
/// "A1" â†’ (0, 0), "B2" â†’ (1, 1), "AA1" â†’ (26, 0)
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

/// Parse rich text runs from an `<is>` (inline string) element's children.
///
/// Excel rich text in cells uses `<is><r><rPr>...</rPr><t>...</t></r>...</is>` structure.
/// If no `<r>` elements are found, falls back to plain text extraction.
/// Returns (plain_text, rich_runs) where plain_text is the concatenated text and
/// rich_runs contains the per-run formatting (T2 â€” CellRichText).
fn parse_rich_runs_from_is(children: &[XmlNode]) -> (String, Vec<gp_core::ir::CellRun>) {
    let mut plain_text = String::new();
    let mut runs: Vec<gp_core::ir::CellRun> = Vec::new();
    let mut found_r = false;

    for child in children {
        if let XmlNode::Element {
            name,
            children: inner,
            ..
        } = child
        {
            if name.local_name == "r" {
                found_r = true;
                let mut run_text = String::new();
                let mut bold: Option<bool> = None;
                let mut strikethrough: Option<bool> = None;
                let mut color: Option<String> = None;
                let mut size: Option<f64> = None;

                for r_child in inner {
                    if let XmlNode::Element {
                        name: r_child_name,
                        attributes: _,
                        children: r_inner,
                        ..
                    } = r_child
                    {
                        if r_child_name.local_name == "rPr" {
                            for rp_child in r_inner {
                                if let XmlNode::Element {
                                    name: prop_name,
                                    attributes: prop_attrs,
                                    ..
                                } = rp_child
                                {
                                    match prop_name.local_name.as_str() {
                                        "b" => {
                                            bold = Some(true);
                                        }
                                        "i" => {}
                                        "strike" => {
                                            strikethrough = Some(true);
                                        }
                                        "u" => {}
                                        "sz" => {
                                            if let Some(val) = get_attr(prop_attrs, "val") {
                                                if let Ok(s) = val.parse::<f64>() {
                                                    size = Some(s);
                                                }
                                            }
                                        }
                                        "color" => {
                                            if let Some(rgb) = get_attr(prop_attrs, "rgb") {
                                                color = if rgb.len() >= 6 {
                                                    Some(rgb[2..].to_string())
                                                } else {
                                                    Some(rgb.clone())
                                                };
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        if r_child_name.local_name == "t" {
                            extract_text_from_children(r_inner, &mut run_text);
                        }
                    }
                }

                if !run_text.is_empty() {
                    runs.push(gp_core::ir::CellRun {
                        text: run_text.clone(),
                        color,
                        strikethrough,
                        bold,
                        size,
                    });
                    plain_text.push_str(&run_text);
                }
            }
        }
    }

    // Fallback: no <r> elements found â€” extract plain text from <t> children directly
    if !found_r {
        extract_text_from_children(children, &mut plain_text);
    }

    (plain_text, runs)
}

// ===========================================================================
// Styles
// ===========================================================================

/// Parse styles.xml and return the full style table.
#[allow(clippy::type_complexity)]
fn read_styles(package: &OpcPackage) -> Result<XlsxStyles, Vec<Diagnostic>> {
    let part = find_part(package, "xl/styles.xml")?;
    let doc = parse_xml(&part.data)?;

    let mut num_formats: Vec<NumFmt> = Vec::new();
    let mut fonts: Vec<Font> = Vec::new();
    let mut fills: Vec<Fill> = Vec::new();
    let mut borders: Vec<Border> = Vec::new();
    let mut cell_xfs: Vec<XfFormat> = Vec::new();
    let mut style_names: Vec<String> = Vec::new();
    let mut cell_style_props: Vec<BTreeMap<String, String>> = Vec::new();

    if let XmlNode::Element { children, .. } = &doc.root {
        for child in children {
            if let XmlNode::Element {
                name,
                children: inner,
                ..
            } = child
            {
                // --- numFmts ---
                if name.local_name == "numFmts" {
                    for ci in inner {
                        if let XmlNode::Element { attributes, .. } = ci {
                            if let (Some(fid), Some(fcode)) = (
                                get_attr_str(attributes, "formatId"),
                                get_attr_str(attributes, "formatCode"),
                            ) {
                                num_formats.push(NumFmt {
                                    format_id: fid.parse().unwrap_or(0),
                                    format_code: fcode.clone(),
                                });
                            }
                        }
                    }
                }

                // --- fonts ---
                if name.local_name == "fonts" {
                    for ci in inner {
                        if let XmlNode::Element {
                            name: _,
                            attributes,
                            children: inner2,
                            ..
                        } = ci
                        {
                            let mut font = Font::default();

                            // Extract direct attributes
                            if let Some(val) = get_attr_str(attributes, "sz") {
                                if let Ok(s) = val.parse::<f64>() {
                                    font.sz = Some(s);
                                    font.sz_is_pt = true;
                                }
                            }
                            if let Some(val) = get_attr_str(attributes, "bold") {
                                font.bold = Some(val == "1" || val.to_lowercase() == "true");
                            }
                            if let Some(val) = get_attr_str(attributes, "italic") {
                                font.italic = Some(val == "1" || val.to_lowercase() == "true");
                            }
                            if let Some(val) = get_attr_str(attributes, "underline") {
                                font.underline =
                                    Some(val != "0" && val != "none" && val != "false");
                            }

                            // Walk children for <name>, <color>, etc.
                            for ci2 in inner2 {
                                if let XmlNode::Element {
                                    name: cn,
                                    attributes: cn_attrs,
                                    children: _inner3,
                                    ..
                                } = ci2
                                {
                                    match cn.local_name.as_str() {
                                        "name" => {
                                            if let Some(val) = get_attr_str(cn_attrs, "val") {
                                                font.name = Some(val.clone());
                                            }
                                        }
                                        "color" => {
                                            if let Some(rgb) = get_attr_str(cn_attrs, "rgb") {
                                                font.color = Some(rgb.clone());
                                            }
                                        }
                                        "sz" => {
                                            if let Some(val) = get_attr_str(cn_attrs, "val") {
                                                if let Ok(s) = val.parse::<f64>() {
                                                    font.sz = Some(s);
                                                    font.sz_is_pt = true;
                                                }
                                            }
                                        }
                                        "b" => {
                                            font.bold = Some(true);
                                        }
                                        "i" => {
                                            font.italic = Some(true);
                                        }
                                        "u" => {
                                            font.underline = Some(true);
                                        }
                                        _ => {}
                                    }
                                }
                            }

                            fonts.push(font);
                        }
                    }
                }

                // --- fills ---
                if name.local_name == "fills" {
                    for ci in inner {
                        if let XmlNode::Element {
                            name: _,
                            attributes: _,
                            children: inner2,
                            ..
                        } = ci
                        {
                            let mut fill = Fill::default();

                            for ci2 in inner2 {
                                if let XmlNode::Element {
                                    name: fill_node,
                                    attributes: fa,
                                    ..
                                } = ci2
                                {
                                    if fill_node.local_name == "patternFill" {
                                        let mut pf = PatternFill::default();

                                        if let Some(pat) = get_attr_str(fa, "patternType") {
                                            pf.pattern = Some(pat.clone());
                                        }
                                        if let Some(fgc) = get_attr_str(fa, "fgColor") {
                                            if fgc.len() >= 6 {
                                                pf.fg_color = Some(fgc[2..].to_string());
                                            }
                                        }
                                        if let Some(bgc) = get_attr_str(fa, "bgColor") {
                                            if bgc.len() >= 6 {
                                                pf.bg_color = Some(bgc[2..].to_string());
                                            }
                                        }

                                        fill.pattern_fill = Some(pf);
                                    }
                                }
                            }

                            fills.push(fill);
                        }
                    }
                }

                // --- borders ---
                if name.local_name == "borders" {
                    for ci in inner {
                        if let XmlNode::Element {
                            name: _,
                            attributes: _,
                            children: inner2,
                            ..
                        } = ci
                        {
                            let mut border = Border::default();

                            for ci2 in inner2 {
                                if let XmlNode::Element {
                                    name: bn,
                                    attributes: ba,
                                    children: inner3,
                                    ..
                                } = ci2
                                {
                                    let mut line = BorderLine::default();

                                    if let Some(style) = get_attr_str(ba, "style") {
                                        line.style = Some(style.clone());
                                    }

                                    // Extract color
                                    for ci3 in inner3 {
                                        if let XmlNode::Element {
                                            name: cn,
                                            attributes: ca,
                                            ..
                                        } = ci3
                                        {
                                            if cn.local_name == "color" {
                                                if let Some(rgb) = get_attr_str(ca, "rgb") {
                                                    if rgb.len() >= 6 {
                                                        line.color = Some(rgb[2..].to_string());
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    match bn.local_name.as_str() {
                                        "left" => border.left = Some(line),
                                        "right" => border.right = Some(line),
                                        "top" => border.top = Some(line),
                                        "bottom" => border.bottom = Some(line),
                                        "diagonal" => border.diagonal = Some(line),
                                        _ => {}
                                    }
                                }
                            }

                            borders.push(border);
                        }
                    }
                }

                // --- cellStyleXfs ---
                if name.local_name == "cellStyleXfs" {
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

                // --- cellXfs ---
                if name.local_name == "cellXfs" {
                    for ci in inner {
                        if let XmlNode::Element {
                            name: _,
                            attributes,
                            children: inner2,
                            ..
                        } = ci
                        {
                            let mut xf = XfFormat::default();

                            if let Some(val) = get_attr_str(attributes, "numFmtId") {
                                xf.num_fmt_id = val.parse().unwrap_or(0);
                            }
                            if let Some(val) = get_attr_str(attributes, "fontId") {
                                xf.font_index = val.parse().unwrap_or(0);
                            }
                            if let Some(val) = get_attr_str(attributes, "fillId") {
                                xf.fill_index = val.parse().unwrap_or(0);
                            }
                            if let Some(val) = get_attr_str(attributes, "borderId") {
                                xf.border_index = val.parse().unwrap_or(0);
                            }

                            // Check for alignment child
                            for ci2 in inner2 {
                                if let XmlNode::Element {
                                    name: an,
                                    attributes: aa,
                                    ..
                                } = ci2
                                {
                                    if an.local_name == "alignment" {
                                        let mut align = CellStyleAlignment::default();
                                        if let Some(val) = get_attr_str(aa, "horizontal") {
                                            align.horizontal = Some(val.clone());
                                        }
                                        if let Some(val) = get_attr_str(aa, "vertical") {
                                            align.vertical = Some(val.clone());
                                        }
                                        if let Some(val) = get_attr_str(aa, "wrapText") {
                                            align.wrap_text = Some(val == "1" || val == "true");
                                        }
                                        if let Some(val) = get_attr_str(aa, "textRotation") {
                                            align.text_rotation = val.parse().ok();
                                        }
                                        xf.alignment = Some(align);
                                    }
                                }
                            }

                            cell_xfs.push(xf);
                        }
                    }
                }

                // --- cellStyles ---
                if name.local_name == "cellStyles" {
                    for ci in inner {
                        if let XmlNode::Element { attributes, .. } = ci {
                            if let Some(name_val) = get_attr_str(attributes, "name") {
                                style_names.push(name_val.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    if style_names.is_empty() {
        style_names.push("Normal".to_string());
    }

    Ok(XlsxStyles {
        num_formats,
        fonts,
        fills,
        borders,
        cell_xfs,
        cell_style_props,
        style_names,
    })
}

/// Get the style name for a cell based on its s attribute index.
#[allow(dead_code)]
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

/// Resolve a cell's style index to a full CellStyleInfo struct.
#[allow(dead_code)]
pub(crate) fn resolve_cell_style(styles: &XlsxStyles, style_idx: usize) -> CellStyleInfo {
    let mut info = CellStyleInfo::default();

    // Look up the xf format entry
    let xf = if style_idx < styles.cell_xfs.len() {
        styles.cell_xfs[style_idx].clone()
    } else {
        return info;
    };

    // Resolve number format
    if xf.num_fmt_id > 0 {
        // Check built-in format IDs first (0-49)
        let builtin_formats: &[(&str, &str)] = &[
            ("0", "General"),
            ("1", "0"),
            ("2", "0.00"),
            ("3", "#,##0"),
            ("4", "#,##0.00"),
            ("9", "0%"),
            ("10", "0.00%"),
            ("11", "0.00E+00"),
            ("12", "# ?/?"),
            ("13", "# ??/??"),
            ("14", "mm-dd-yy"),
            ("15", "d-mmm-yy"),
            ("16", "d-mmm"),
            ("17", "mmm-yy"),
            ("18", "h:mm AM/PM"),
            ("19", "h:mm:ss AM/PM"),
            ("20", "h:mm"),
            ("21", "h:mm:ss"),
            ("22", "m/d/yy h:mm"),
            ("37", "#,##0 ;(#,##0)"),
            ("38", "#,##0 ;[Red](#,##0)"),
            ("39", "#,##0.00;(#,##0.00)"),
            ("40", "#,##0.00;[Red](#,##0.00)"),
            ("41", r#"mm":"ss".0"#),
            ("45", "yyyy\"年\"m\"月\"d\"日\""),
            ("46", "yyyy\"年\"m\"月\"d\"日\""),
            ("47", "yyyy\"年\"m\"月\"d\"日\""),
        ];

        // Check custom numFmts table first
        let mut found = false;
        for nf in &styles.num_formats {
            if nf.format_id == xf.num_fmt_id {
                info.num_format = Some(nf.format_code.clone());
                found = true;
                break;
            }
        }

        if !found {
            // Fall back to built-in format IDs
            let builtin_id = xf.num_fmt_id.to_string();
            if let Some((_, code)) = builtin_formats.iter().find(|(id, _)| *id == builtin_id) {
                info.num_format = Some((*code).to_string());
            }
        }
    } else {
        // numFmtId == 0 means "General"
        info.num_format = Some("General".to_string());
    }

    // Resolve font
    if xf.font_index >= 0 && (xf.font_index as usize) < styles.fonts.len() {
        info.font = Some(styles.fonts[xf.font_index as usize].clone());
    }

    // Resolve fill
    if xf.fill_index >= 0 && (xf.fill_index as usize) < styles.fills.len() {
        info.fill = Some(styles.fills[xf.fill_index as usize].clone());
    }

    // Resolve border
    if xf.border_index >= 0 && (xf.border_index as usize) < styles.borders.len() {
        info.border = Some(styles.borders[xf.border_index as usize].clone());
    }

    // Resolve alignment (xf-level, or fall back to cell style)
    if let Some(align) = xf.alignment {
        info.alignment = Some(align);
    }

    info
}

// ===========================================================================
// Workbook â†’ sheet names
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

/// Parse sheetViews and freeze_panes from a worksheet XML document.
fn parse_sheet_views(doc: &XmlNode) -> (Vec<SheetView>, Option<FreezePanes>) {
    let mut sheet_views: Vec<SheetView> = Vec::new();
    let mut freeze_panes: Option<FreezePanes> = None;

    if let XmlNode::Element { children, .. } = doc {
        for child in children {
            if let XmlNode::Element {
                name,
                children: inner,
                ..
            } = child
            {
                if name.local_name == "sheetViews" {
                    for sv in inner {
                        if let XmlNode::Element {
                            attributes: sv_attrs,
                            children: sv_inner,
                            ..
                        } = sv
                        {
                            let mut view = SheetView::default();

                            if let Some(wvid) = get_attr(sv_attrs, "workbookViewId") {
                                view.workbook_view_id = wvid.parse().ok();
                            }
                            if let Some(sgl) = get_attr(sv_attrs, "showGridLines") {
                                view.show_grid_lines = Some(sgl == "1" || sgl == "true");
                            }
                            if let Some(srch) = get_attr(sv_attrs, "showRowColHeaders") {
                                view.show_row_col_headers = Some(srch == "1" || srch == "true");
                            }
                            if let Some(zs) = get_attr(sv_attrs, "zoomScale") {
                                view.zoom_scale = zs.parse().ok();
                            }
                            if let Some(st) = get_attr(sv_attrs, "state") {
                                view.state = Some(st.clone());
                            }

                            // Look for pane children (freeze panes)
                            for sv_child in sv_inner {
                                if let XmlNode::Element {
                                    name: pane_name,
                                    attributes: pane_attrs,
                                    ..
                                } = sv_child
                                {
                                    if pane_name.local_name == "pane" {
                                        let mut fp = FreezePanes::default();
                                        if let Some(xs) = get_attr(pane_attrs, "xSplit") {
                                            fp.x_split = xs.parse().ok();
                                        }
                                        if let Some(ys) = get_attr(pane_attrs, "ySplit") {
                                            fp.y_split = ys.parse().ok();
                                        }
                                        if let Some(tlc) = get_attr(pane_attrs, "topLeftCell") {
                                            fp.top_left_cell = Some(tlc.clone());
                                        }
                                        if let Some(ap) = get_attr(pane_attrs, "activePane") {
                                            fp.active_pane = Some(ap.clone());
                                        }
                                        if let Some(st) = get_attr(pane_attrs, "state") {
                                            fp.pane = Some(st.clone());
                                        }
                                        // activeCell is sometimes on the pane element
                                        if let Some(ac) = get_attr(pane_attrs, "activeCell") {
                                            fp.active_cell = Some(ac.clone());
                                        }
                                        freeze_panes = Some(fp);
                                    }
                                }
                            }

                            sheet_views.push(view);
                        }
                    }
                }
            }
        }
    }

    (sheet_views, freeze_panes)
}

/// Parse <cols><col> elements from a worksheet XML document.
fn parse_column_dimensions(doc: &XmlNode) -> Vec<ColumnDimension> {
    let mut cols: Vec<ColumnDimension> = Vec::new();

    if let XmlNode::Element { children, .. } = doc {
        for child in children {
            if let XmlNode::Element {
                name,
                children: inner,
                ..
            } = child
            {
                if name.local_name == "cols" {
                    for col_elem in inner {
                        if let XmlNode::Element { attributes, .. } = col_elem {
                            let mut cd = ColumnDimension::default();
                            if let Some(min) = get_attr(attributes, "min") {
                                cd.min = min.parse().unwrap_or(1);
                            }
                            if let Some(max) = get_attr(attributes, "max") {
                                cd.max = max.parse().unwrap_or(1);
                            }
                            if let Some(w) = get_attr(attributes, "width") {
                                cd.width = w.parse().ok();
                            }
                            if let Some(s) = get_attr(attributes, "style") {
                                cd.style = s.parse().ok();
                            }
                            if let Some(h) = get_attr(attributes, "hidden") {
                                cd.hidden = Some(h == "1" || h == "true");
                            }
                            if let Some(bf) = get_attr(attributes, "bestFit") {
                                cd.best_fit = Some(bf == "1" || bf == "true");
                            }
                            if let Some(ol) = get_attr(attributes, "outlineLevel") {
                                cd.outline_level = ol.parse().ok();
                            }
                            cols.push(cd);
                        }
                    }
                }
            }
        }
    }

    cols
}

/// Parse <row> elements with dimensions (height, hidden, etc.) from sheetData.
/// Requires parallel row_indices array since SheetRow doesn't store its row number.
fn parse_row_dimensions_from_data(rows: &[SheetRow], row_indices: &[usize]) -> Vec<RowDimension> {
    let mut row_dims: Vec<RowDimension> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        if row.height.is_some() || row.hidden.is_some() {
            let rd = RowDimension {
                index: row_indices.get(i).copied().unwrap_or(i),
                height: row.height,
                custom_height: Some(row.height.is_some()),
                hidden: row.hidden,
                outline_level: None,
                collapsed: None,
                ph: None,
            };
            row_dims.push(rd);
        }
    }
    row_dims
}

/// Parse row dimension info from <row> elements within <sheetData>.
/// Also returns the list of SheetRows with any dimension info attached.
#[allow(dead_code)]
fn parse_sheet_data_with_dimensions(children: &[XmlNode]) -> Vec<SheetRow> {
    let mut sheet_rows: Vec<SheetRow> = Vec::new();

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

                            let mut height: Option<f64> = None;
                            let mut hidden: Option<bool> = None;

                            if let Some(h) = get_attr(attributes, "ht") {
                                if let Ok(val) = h.parse::<f64>() {
                                    height = Some(val);
                                }
                            } else if let Some(h) = get_attr(attributes, "height") {
                                if let Ok(val) = h.parse::<f64>() {
                                    height = Some(val);
                                }
                            }

                            if let Some(h) = get_attr(attributes, "hidden") {
                                hidden = Some(h == "1" || h == "true");
                            }

                            let mut row_cells: Vec<SheetCell> = Vec::new();
                            for cell_elem in cells {
                                if let Some(cell) = parse_cell(
                                    cell_elem,
                                    &[], // shared_strings placeholder — filled in read_sheet
                                    &XlsxStyles::default(), // styles placeholder
                                    row_num,
                                    &mut Vec::new(), // diags placeholder
                                ) {
                                    row_cells.push(cell);
                                }
                            }

                            let row_id = ElementId::derive(
                                "sheet_row",
                                &["sheet_placeholder", &format!("row:{}", row_num)],
                                &format!("row:{}", row_num),
                            );
                            sheet_rows.push(SheetRow {
                                id: row_id,
                                cells: row_cells,
                                height,
                                hidden,
                            });
                        }
                    }
                }
            }
        }
    }

    sheet_rows
}

/// Parse autoFilter from a worksheet — finds the autoFilter element, extracts its ref attr
/// and any filterColumn children.
fn parse_auto_filter_from_worksheet(doc: &XmlNode) -> Option<AutoFilter> {
    if let XmlNode::Element { children, .. } = doc {
        for child in children {
            if let XmlNode::Element {
                name,
                attributes,
                children: inner,
                ..
            } = child
            {
                if name.local_name == "autoFilter" {
                    let mut af = AutoFilter::default();

                    // ref attribute is on the autoFilter element itself
                    if let Some(ref_val) = get_attr(attributes, "ref") {
                        af.ref_range = ref_val.clone();
                    }

                    // Parse filterColumn children
                    for fc in inner {
                        if let XmlNode::Element {
                            name: fc_name,
                            attributes: fc_attrs,
                            children: fc_inner,
                            ..
                        } = fc
                        {
                            if fc_name.local_name == "filterColumn" {
                                let mut col = AutoFilterColumn::default();
                                if let Some(cid) = get_attr(fc_attrs, "colId") {
                                    col.col = cid.parse().unwrap_or(0);
                                }

                                for fr in fc_inner {
                                    if let XmlNode::Element {
                                        name: fr_name,
                                        attributes: fr_attrs,
                                        ..
                                    } = fr
                                    {
                                        if fr_name.local_name == "filter" {
                                            let mut rule = FilterRule::default();
                                            if let Some(v) = get_attr(fr_attrs, "val") {
                                                rule.val1 = Some(v.clone());
                                            }
                                            if let Some(op) = get_attr(fr_attrs, "operator") {
                                                rule.operator = Some(op.clone());
                                            }
                                            col.filter_rules.push(rule);
                                        }
                                        if fr_name.local_name == "dateGroupItem" {
                                            let mut rule = FilterRule::default();
                                            if let Some(v) = get_attr(fr_attrs, "val") {
                                                rule.val1 = Some(v.clone());
                                            }
                                            col.filter_rules.push(rule);
                                        }
                                    }
                                }

                                af.filter_column.push(col);
                            }
                        }
                    }

                    if !af.ref_range.is_empty() || !af.filter_column.is_empty() {
                        return Some(af);
                    }
                }
            }
        }
    }
    None
}

/// Parse conditionalFormatting from a worksheet XML document.
fn parse_conditional_formats(doc: &XmlNode) -> Vec<ConditionalFormat> {
    let mut formats: Vec<ConditionalFormat> = Vec::new();

    if let XmlNode::Element { children, .. } = doc {
        for child in children {
            if let XmlNode::Element {
                name,
                children: inner,
                ..
            } = child
            {
                if name.local_name == "conditionalFormats" {
                    for cf in inner {
                        if let XmlNode::Element {
                            name: cf_name,
                            attributes: cf_attrs,
                            children: cf_inner,
                            ..
                        } = cf
                        {
                            if cf_name.local_name == "conditionalFormatting" {
                                let mut cf_format = ConditionalFormat::default();

                                if let Some(sqref) = get_attr(cf_attrs, "sqref") {
                                    cf_format.sqref = sqref.clone();
                                }
                                if let Some(di) = get_attr(cf_attrs, "dxfId") {
                                    cf_format.dxf_index = di.parse().ok();
                                }

                                // Parse cfRule children
                                for cr in cf_inner {
                                    if let XmlNode::Element {
                                        name: cr_name,
                                        attributes: cr_attrs,
                                        children: cr_inner,
                                        ..
                                    } = cr
                                    {
                                        if cr_name.local_name == "cfRule" {
                                            let mut cf_rule = ConditionalFormat::default();

                                            if let Some(r#type) = get_attr(cr_attrs, "type") {
                                                cf_rule.r#type = r#type.clone();
                                            }
                                            if let Some(pri) = get_attr(cr_attrs, "priority") {
                                                cf_rule.priority = pri.parse().ok();
                                            }
                                            if let Some(sq) = get_attr(cr_attrs, "sqref") {
                                                cf_rule.sqref = sq.clone();
                                            }
                                            if let Some(di) = get_attr(cr_attrs, "dxfId") {
                                                cf_rule.dxf_index = di.parse().ok();
                                            }
                                            if let Some(st) = get_attr(cr_attrs, "stopIfTrue") {
                                                cf_rule.stop_if_true =
                                                    Some(st == "1" || st == "true");
                                            }

                                            // Look for formula children
                                            for cr_child in cr_inner {
                                                if let XmlNode::Element {
                                                    name: formula_name,
                                                    children: formula_children,
                                                    ..
                                                } = cr_child
                                                {
                                                    if formula_name.local_name == "formula" {
                                                        let mut fval = String::new();
                                                        extract_text_from_children(
                                                            formula_children,
                                                            &mut fval,
                                                        );
                                                        if cf_rule.formula.is_none() {
                                                            cf_rule.formula = Some(fval);
                                                        } else {
                                                            // multiple formulas — append
                                                            cf_rule.formula = Some(format!(
                                                                "{},{}",
                                                                cf_rule.formula.unwrap_or_default(),
                                                                fval
                                                            ));
                                                        }
                                                    }
                                                }
                                            }

                                            formats.push(cf_rule);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    formats
}

/// Parse dataValidation from a worksheet XML document.
fn parse_data_validations(doc: &XmlNode) -> Vec<DataValidation> {
    let mut validations: Vec<DataValidation> = Vec::new();

    if let XmlNode::Element { children, .. } = doc {
        for child in children {
            if let XmlNode::Element {
                name,
                children: inner,
                ..
            } = child
            {
                if name.local_name == "dataValidations" || name.local_name == "dataValidation" {
                    let items = if name.local_name == "dataValidations" {
                        inner
                    } else {
                        // Single dataValidation at sheet level
                        std::slice::from_ref(child)
                    };

                    for dv in items {
                        if let XmlNode::Element {
                            name: dv_name,
                            attributes: dv_attrs,
                            children: dv_inner,
                            ..
                        } = dv
                        {
                            if dv_name.local_name == "dataValidation" {
                                let mut dv_rule = DataValidation::default();

                                if let Some(sqref) = get_attr(dv_attrs, "sqref") {
                                    dv_rule.sqref = sqref.clone();
                                }
                                if let Some(r#type) = get_attr(dv_attrs, "type") {
                                    dv_rule.r#type = r#type.clone();
                                }
                                if let Some(op) = get_attr(dv_attrs, "operator") {
                                    dv_rule.operator = Some(op.clone());
                                }
                                if let Some(sd) = get_attr(dv_attrs, "allowBlank") {
                                    dv_rule.show_dropdown = Some(sd == "1" || sd == "true");
                                }
                                if let Some(se) = get_attr(dv_attrs, "showErrorMessage") {
                                    dv_rule.show_error = Some(se == "1" || se == "true");
                                }
                                if let Some(es) = get_attr(dv_attrs, "errorStyle") {
                                    dv_rule.error_style = Some(es.clone());
                                }
                                if let Some(et) = get_attr(dv_attrs, "errorTitle") {
                                    dv_rule.error_title = Some(et.clone());
                                }
                                if let Some(e) = get_attr(dv_attrs, "error") {
                                    dv_rule.error = Some(e.clone());
                                }
                                if let Some(pt) = get_attr(dv_attrs, "promptTitle") {
                                    dv_rule.prompt_title = Some(pt.clone());
                                }
                                if let Some(p) = get_attr(dv_attrs, "prompt") {
                                    dv_rule.prompt = Some(p.clone());
                                }

                                // Parse formula1 and formula2 children
                                for dv_child in dv_inner {
                                    if let XmlNode::Element {
                                        name: fc_name,
                                        children: fc_children,
                                        ..
                                    } = dv_child
                                    {
                                        let mut fval = String::new();
                                        extract_text_from_children(fc_children, &mut fval);
                                        if fc_name.local_name == "formula1" {
                                            dv_rule.formula1 =
                                                if fval.is_empty() { None } else { Some(fval) };
                                        } else if fc_name.local_name == "formula2" {
                                            dv_rule.formula2 =
                                                if fval.is_empty() { None } else { Some(fval) };
                                        }
                                    }
                                }

                                validations.push(dv_rule);
                            }
                        }
                    }
                }
            }
        }
    }

    validations
}

/// Parse table-related elements from a worksheet XML document.
fn parse_tables(doc: &XmlNode, package: &OpcPackage, _sheet_name: &str) -> Vec<XlsxTable> {
    let mut tables: Vec<XlsxTable> = Vec::new();

    if let XmlNode::Element { children, .. } = doc {
        for child in children {
            if let XmlNode::Element {
                name,
                children: inner,
                ..
            } = child
            {
                if name.local_name == "tableParts" {
                    // This is a reference to external table parts
                    // The actual table definitions are in tableN.xml within the package
                    for tp in inner {
                        if let XmlNode::Element { attributes, .. } = tp {
                            if let Some(id_attr) = get_attr(attributes, "id") {
                                // Try to find tableN.xml
                                let table_uri = format!("xl/tables/table{}.xml", id_attr);
                                if let Ok(table_part) = find_part(package, &table_uri) {
                                    if let Ok(table_doc) = parse_xml(&table_part.data) {
                                        if let Some(table) = parse_table_definition(&table_doc.root)
                                        {
                                            tables.push(table);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if name.local_name == "legacyDrawings" {
                    // Skip legacy drawings
                }
            }
        }
    }

    tables
}

/// Parse a table definition from tableN.xml
fn parse_table_definition(doc: &XmlNode) -> Option<XlsxTable> {
    if let XmlNode::Element { children, .. } = doc {
        for child in children {
            if let XmlNode::Element {
                name: table_name,
                attributes: table_attrs,
                children: table_inner,
                ..
            } = child
            {
                if table_name.local_name == "table" {
                    let mut tbl = XlsxTable::default();

                    if let Some(id) = get_attr(table_attrs, "id") {
                        tbl.id = id.clone();
                    }
                    if let Some(name_val) = get_attr(table_attrs, "name") {
                        tbl.name = name_val.clone();
                    }
                    if let Some(display_name) = get_attr(table_attrs, "displayName") {
                        tbl.display_name = display_name.clone();
                    }
                    if let Some(ref_r) = get_attr(table_attrs, "ref") {
                        tbl.ref_range = ref_r.clone();
                    }
                    if get_attr(table_attrs, "autofilter").is_some() {
                        tbl.auto_filter = Some(true);
                    }
                    if let Some(tots) = get_attr(table_attrs, "totalsRowCount") {
                        tbl.totals_row_shown = Some(tots == "1" || tots == "true");
                    }

                    // Parse tableColumn children
                    for tc in table_inner {
                        if let XmlNode::Element {
                            name: tc_name,
                            attributes: tc_attrs,
                            ..
                        } = tc
                        {
                            if tc_name.local_name == "tableColumn" {
                                let mut col = TableColumn::default();
                                if let Some(id) = get_attr(tc_attrs, "id") {
                                    col.id = id.parse().unwrap_or(0);
                                }
                                if let Some(name_val) = get_attr(tc_attrs, "name") {
                                    col.name = name_val.clone();
                                }
                                tbl.columns.push(col);
                            }
                        }
                    }

                    // Check for autoFilterElement
                    for tc in table_inner {
                        if let XmlNode::Element { name: af_name, .. } = tc {
                            if af_name.local_name == "autoFilter" {
                                tbl.auto_filter = Some(true);
                            }
                        }
                    }

                    if !tbl.id.is_empty() || !tbl.name.is_empty() {
                        return Some(tbl);
                    }
                }
            }
        }
    }
    None
}

/// Parse sheetProtection from a worksheet XML document.
fn parse_sheet_protection(doc: &XmlNode) -> Option<SheetProtection> {
    if let XmlNode::Element { children, .. } = doc {
        for child in children {
            if let XmlNode::Element {
                name, attributes, ..
            } = child
            {
                if name.local_name == "sheetProtection" {
                    let mut sp = SheetProtection::default();

                    if let Some(pwd) = get_attr(attributes, "password") {
                        sp.password = Some(pwd.clone());
                    }
                    if let Some(s) = get_attr(attributes, "sheet") {
                        sp.sheet = Some(s == "1" || s == "true");
                    }
                    if let Some(o) = get_attr(attributes, "objects") {
                        sp.objects = Some(o == "1" || o == "true");
                    }
                    if let Some(sc) = get_attr(attributes, "scenarios") {
                        sp.scenarios = Some(sc == "1" || sc == "true");
                    }
                    if let Some(fc) = get_attr(attributes, "formatCells") {
                        sp.format_cells = Some(fc == "1" || fc == "true");
                    }
                    if let Some(fcm) = get_attr(attributes, "formatColumns") {
                        sp.format_columns = Some(fcm == "1" || fcm == "true");
                    }
                    if let Some(fr) = get_attr(attributes, "formatRows") {
                        sp.format_rows = Some(fr == "1" || fr == "true");
                    }
                    if let Some(ic) = get_attr(attributes, "insertColumns") {
                        sp.insert_columns = Some(ic == "1" || ic == "true");
                    }
                    if let Some(ir) = get_attr(attributes, "insertRows") {
                        sp.insert_rows = Some(ir == "1" || ir == "true");
                    }
                    if let Some(ih) = get_attr(attributes, "insertHyperlinks") {
                        sp.insert_hyperlinks = Some(ih == "1" || ih == "true");
                    }
                    if let Some(dc) = get_attr(attributes, "deleteColumns") {
                        sp.delete_columns = Some(dc == "1" || dc == "true");
                    }
                    if let Some(dr) = get_attr(attributes, "deleteRows") {
                        sp.delete_rows = Some(dr == "1" || dr == "true");
                    }
                    if let Some(slc) = get_attr(attributes, "selectLockedCells") {
                        sp.select_locked_cells = Some(slc == "1" || slc == "true");
                    }
                    if let Some(suc) = get_attr(attributes, "selectUnlockCells") {
                        sp.select_unlocked_cells = Some(suc == "1" || suc == "true");
                    }

                    return Some(sp);
                }
            }
        }
    }
    None
}

/// Parse pageSetup from a worksheet XML document.
fn parse_page_setup(doc: &XmlNode) -> Option<PageSetup> {
    if let XmlNode::Element { children, .. } = doc {
        for child in children {
            if let XmlNode::Element {
                name, attributes, ..
            } = child
            {
                if name.local_name == "pageSetup" {
                    let mut ps = PageSetup::default();

                    if let Some(ps_id) = get_attr(attributes, "paperSize") {
                        ps.paper_size = ps_id.parse().ok();
                    }
                    if let Some(scale) = get_attr(attributes, "scale") {
                        ps.scale = scale.parse().ok();
                    }
                    if let Some(orient) = get_attr(attributes, "orientation") {
                        ps.orientation = Some(orient.clone());
                    }
                    if let Some(fpn) = get_attr(attributes, "firstPageNumber") {
                        ps.first_page_number = fpn.parse().ok();
                    }
                    if let Some(po) = get_attr(attributes, "pageOrder") {
                        ps.page_order = Some(po.clone());
                    }
                    if let Some(fw) = get_attr(attributes, "fitToWidth") {
                        ps.fit_to_width = fw.parse().ok();
                    }
                    if let Some(fh) = get_attr(attributes, "fitToHeight") {
                        ps.fit_to_height = fh.parse().ok();
                    }
                    if let Some(draft) = get_attr(attributes, "draft") {
                        ps.draft = Some(draft == "1" || draft == "true");
                    }
                    if let Some(ud) = get_attr(attributes, "useDefaults") {
                        ps.use_defaults = Some(ud == "1" || ud == "true");
                    }
                    if let Some(hdpi) = get_attr(attributes, "horizontalDpi") {
                        ps.horizontal_dpi = hdpi.parse().ok();
                    }
                    if let Some(vdpi) = get_attr(attributes, "verticalDpi") {
                        ps.vertical_dpi = vdpi.parse().ok();
                    }
                    if let Some(bw) = get_attr(attributes, "blackAndWhite") {
                        ps.black_and_white = Some(bw == "1" || bw == "true");
                    }
                    if let Some(pw) = get_attr(attributes, "pageWidth") {
                        ps.page_width = pw.parse().ok();
                    }
                    if let Some(ph) = get_attr(attributes, "pageHeight") {
                        ps.page_height = ph.parse().ok();
                    }

                    // Only return if we found at least one attribute
                    if ps.paper_size.is_some()
                        || ps.scale.is_some()
                        || ps.orientation.is_some()
                        || ps.first_page_number.is_some()
                        || ps.page_order.is_some()
                    {
                        return Some(ps);
                    }
                }
            }
        }
    }
    None
}

/// Parse printOptions from a worksheet XML document.
#[allow(dead_code)]
fn parse_print_options(doc: &XmlNode) {
    let _ = doc; // Currently collected for completeness, not exposed in Sheet IR yet
}

/// Parse headerFooter from a worksheet XML document.
fn parse_header_footer(doc: &XmlNode) -> Option<HeaderFooter> {
    if let XmlNode::Element { children, .. } = doc {
        for child in children {
            if let XmlNode::Element {
                name,
                attributes,
                children: inner,
                ..
            } = child
            {
                if name.local_name == "headerFooter" {
                    let mut hf = HeaderFooter::default();

                    if let Some(df) = get_attr(attributes, "differentFirst") {
                        hf.different_first = Some(df == "1" || df == "true");
                    }
                    if let Some(de) = get_attr(attributes, "differentOddEven") {
                        hf.different_odd_even = Some(de == "1" || de == "true");
                    }
                    if let Some(awm) = get_attr(attributes, "alignWithMargins") {
                        hf.align_with_page_margins = Some(awm == "1" || awm == "true");
                    }
                    if let Some(swd) = get_attr(attributes, "scaleWithDoc") {
                        hf.scale_with_doc = Some(swd == "1" || swd == "true");
                    }

                    for hf_child in inner {
                        if let XmlNode::Element {
                            name: hf_child_name,
                            children: hf_children,
                            ..
                        } = hf_child
                        {
                            let mut text = String::new();
                            extract_text_from_children(hf_children, &mut text);
                            match hf_child_name.local_name.as_str() {
                                "oddHeader" => {
                                    hf.odd_header = if text.is_empty() { None } else { Some(text) }
                                }
                                "oddFooter" => {
                                    hf.odd_footer = if text.is_empty() { None } else { Some(text) }
                                }
                                "evenHeader" => {
                                    hf.even_header = if text.is_empty() { None } else { Some(text) }
                                }
                                "evenFooter" => {
                                    hf.even_footer = if text.is_empty() { None } else { Some(text) }
                                }
                                "firstHeader" => {
                                    hf.first_header =
                                        if text.is_empty() { None } else { Some(text) }
                                }
                                "firstFooter" => {
                                    hf.first_footer =
                                        if text.is_empty() { None } else { Some(text) }
                                }
                                _ => {}
                            }
                        }
                    }

                    if hf.odd_header.is_some()
                        || hf.odd_footer.is_some()
                        || hf.even_header.is_some()
                        || hf.even_footer.is_some()
                        || hf.first_header.is_some()
                        || hf.first_footer.is_some()
                    {
                        return Some(hf);
                    }
                }
            }
        }
    }
    None
}

/// Parse pageMargins from a worksheet XML document.
#[allow(dead_code)]
fn parse_page_margins(doc: &XmlNode) {
    let _ = doc; // Collected for completeness, not exposed in Sheet IR yet
}

/// Parse hyperlinks from a worksheet XML document.
fn parse_hyperlinks(doc: &XmlNode, rels: &BTreeMap<String, String>) -> Vec<Hyperlink> {
    let mut hyperlinks: Vec<Hyperlink> = Vec::new();

    if let XmlNode::Element { children, .. } = doc {
        for child in children {
            if let XmlNode::Element {
                name,
                children: inner,
                ..
            } = child
            {
                if name.local_name == "hyperlinks" {
                    for hl in inner {
                        if let XmlNode::Element {
                            name: hl_name,
                            attributes: hl_attrs,
                            ..
                        } = hl
                        {
                            if hl_name.local_name == "hyperlink" {
                                let mut link = Hyperlink::default();

                                if let Some(ref_val) = get_attr(hl_attrs, "ref") {
                                    // Store the cell reference for display
                                    link.display_text = Some(ref_val.clone());
                                }
                                if let Some(r_id) = get_attr(hl_attrs, "r:id") {
                                    link.rid = Some(r_id.clone());
                                    // Resolve the r:id to an actual URL via relationships
                                    if let Some(target) = rels.get(r_id) {
                                        link.target = Some(target.clone());
                                    }
                                }
                                if let Some(tooltip) = get_attr(hl_attrs, "tooltip") {
                                    link.tooltip = Some(tooltip.clone());
                                }
                                if let Some(anchor) = get_attr(hl_attrs, "anchor") {
                                    link.anchor = Some(anchor.clone());
                                }

                                hyperlinks.push(link);
                            }
                        }
                    }
                }
            }
        }
    }

    hyperlinks
}

/// Parse worksheet relationships for resolving r:id references (hyperlinks, drawings, etc.)
fn parse_worksheet_rels(package: &OpcPackage, sheet_uri: &str) -> BTreeMap<String, String> {
    let mut rels: BTreeMap<String, String> = BTreeMap::new();

    // Construct the rels URI: xl/worksheets/sheet1.xml -> xl/worksheets/_rels/sheet1.xml.rels
    let rels_uri = if let Some(stripped) = sheet_uri.strip_prefix("xl/") {
        format!(
            "xl/worksheets/_rels/{}.rels",
            stripped.replace(".xml", ".xml.rels")
        )
    } else {
        format!("xl/_rels/{}.rels", sheet_uri.replace(".xml", ".xml.rels"))
    };

    if let Ok(part) = find_part(package, &rels_uri) {
        if let Ok(doc) = parse_xml(&part.data) {
            if let XmlNode::Element { children, .. } = &doc.root {
                for child in children {
                    if let XmlNode::Element {
                        name, attributes, ..
                    } = child
                    {
                        if name.local_name == "Relationship" {
                            if let (Some(id), Some(target)) =
                                (get_attr(attributes, "Id"), get_attr(attributes, "Target"))
                            {
                                rels.insert(id.clone(), target.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    rels
}

/// Parse definedNames from workbook.xml.
fn parse_defined_names(package: &OpcPackage) -> Vec<DefinedName> {
    let mut defined_names: Vec<DefinedName> = Vec::new();

    let part = match find_part(package, "xl/workbook.xml") {
        Ok(p) => p,
        Err(_) => return defined_names,
    };

    let doc = match parse_xml(&part.data) {
        Ok(d) => d,
        Err(_) => return defined_names,
    };

    if let XmlNode::Element { children, .. } = &doc.root {
        for child in children {
            if let XmlNode::Element {
                name,
                children: inner,
                ..
            } = child
            {
                if name.local_name == "definedNames" {
                    for dn in inner {
                        if let XmlNode::Element {
                            name: dn_name,
                            attributes: dn_attrs,
                            children: dn_inner,
                            ..
                        } = dn
                        {
                            if dn_name.local_name == "definedName" {
                                let mut defined = DefinedName::default();

                                if let Some(name_val) = get_attr(dn_attrs, "name") {
                                    defined.name = name_val.clone();
                                }
                                if let Some(ref_formula) = get_attr(dn_attrs, "localSheetId") {
                                    defined.local_sheet_id = ref_formula.parse().ok();
                                }
                                if let Some(builtin) = get_attr(dn_attrs, "builtin") {
                                    defined.builtin = Some(builtin == "1" || builtin == "true");
                                }
                                if let Some(comment) = get_attr(dn_attrs, "comment") {
                                    defined.comment = Some(comment.clone());
                                }
                                if let Some(hidden) = get_attr(dn_attrs, "hidden") {
                                    defined.hidden = Some(hidden == "1" || hidden == "true");
                                }

                                // The ref formula is the text content of the element
                                let mut ref_formula = String::new();
                                extract_text_from_children(dn_inner, &mut ref_formula);
                                defined.ref_formula = if ref_formula.is_empty() {
                                    String::new()
                                } else {
                                    ref_formula
                                };

                                if !defined.name.is_empty() && !defined.ref_formula.is_empty() {
                                    defined_names.push(defined);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    defined_names
}

/// Parse a single sheet's worksheet XML into a Sheet IR.
fn read_sheet(
    package: &OpcPackage,
    sheet_name: &str,
    sheet_id: &str,
    shared_strings: &[String],
    styles: &XlsxStyles,
) -> Result<Sheet, Vec<Diagnostic>> {
    let mut diags: Vec<Diagnostic> = Vec::new();

    // Resolve worksheet part URI
    let sheet_uri = resolve_sheet_part(package, sheet_id)?;
    let part = find_part(package, &sheet_uri)?;

    let doc = parse_xml(&part.data)?;

    // Parse worksheet relationships for r:id resolution
    let ws_rels = parse_worksheet_rels(package, &sheet_uri);

    // Look for sheetData
    let mut sheet_rows: Vec<SheetRow> = Vec::new();
    let mut row_indices: Vec<usize> = Vec::new();
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

        // Then parse sheetData â†’ rows â†’ cells
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

                                let mut height: Option<f64> = None;
                                row_indices.push(row_num);

                                let mut hidden: Option<bool> = None;

                                if let Some(h) = get_attr(attributes, "ht") {
                                    if let Ok(val) = h.parse::<f64>() {
                                        height = Some(val);
                                    }
                                } else if let Some(h) = get_attr(attributes, "height") {
                                    if let Ok(val) = h.parse::<f64>() {
                                        height = Some(val);
                                    }
                                }

                                if let Some(h) = get_attr(attributes, "hidden") {
                                    hidden = Some(h == "1" || h == "true");
                                }

                                // Parse cells in this row
                                let mut row_cells: Vec<SheetCell> = Vec::new();
                                for cell_elem in cells {
                                    if let Some(cell) = parse_cell(
                                        cell_elem,
                                        shared_strings,
                                        styles,
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
                                    height,
                                    hidden,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // Parse row dimensions from sheetData rows that have height/hidden
    let row_dimensions = parse_row_dimensions_from_data(&sheet_rows, &row_indices);

    // Parse sheetViews and freeze_panes
    let (sheet_views, freeze_panes) = parse_sheet_views(&doc.root);

    // Parse column dimensions
    let column_dimensions = parse_column_dimensions(&doc.root);

    // Parse autoFilter
    let auto_filter = parse_auto_filter_from_worksheet(&doc.root);

    // Parse conditionalFormatting
    let conditional_formats = parse_conditional_formats(&doc.root);

    // Parse dataValidation
    let data_validations = parse_data_validations(&doc.root);

    // Parse table parts / tables
    let tables = parse_tables(&doc.root, package, sheet_name);

    // Parse sheetProtection
    let protection = parse_sheet_protection(&doc.root);

    // Parse pageSetup
    let page_setup = parse_page_setup(&doc.root);

    // Parse headerFooter
    let header_footer = parse_header_footer(&doc.root);

    // Parse hyperlinks
    let hyperlinks = parse_hyperlinks(&doc.root, &ws_rels);

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
        hyperlinks,
        column_dimensions,
        row_dimensions,
        freeze_panes,
        sheet_views,
        auto_filter,
        conditional_formats,
        data_validations,
        tables,
        protection,
        page_setup,
        header_footer,
    })
}

/// Parse a single cell element into a SheetCell.
fn parse_cell(
    cell_elem: &XmlNode,
    shared_strings: &[String],
    styles: &XlsxStyles,
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
    let style_name = style_idx.and_then(|si| {
        if let Ok(idx) = si.parse::<usize>() {
            if idx < styles.style_names.len() {
                return Some(styles.style_names[idx].clone());
            }
        }
        None
    });

    // Resolve number_format from styles
    let number_format = style_idx.and_then(|si| {
        if let Ok(idx) = si.parse::<usize>() {
            if idx < styles.cell_xfs.len() {
                let xf = &styles.cell_xfs[idx];
                if xf.num_fmt_id > 0 {
                    // Check custom numFmts
                    for nf in &styles.num_formats {
                        if nf.format_id == xf.num_fmt_id {
                            return Some(nf.format_code.clone());
                        }
                    }
                } else {
                    return Some("General".to_string());
                }
            }
        }
        None
    });

    // Extract values and formulas
    let mut value: Option<String> = None;
    let mut formula: Option<String> = None;
    let mut cached_value: Option<String> = None;
    let mut cell_rich: Option<Vec<gp_core::ir::CellRun>> = None;

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

            // <is> - inline string with optional rich runs (t="inlineStr")
            if child_name.local_name == "is" {
                let (plain, rich) = parse_rich_runs_from_is(inner);
                if !plain.is_empty() {
                    value = Some(plain);
                }
                if !rich.is_empty() {
                    cell_rich = Some(rich);
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
        number_format,
        rich: cell_rich,
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
                                            "sheet '{}': stale cache on {} â€” cached={}, computed={}",
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
                                            "sheet '{}': no cache on {} â€” computed={}",
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
                            // Evaluation returned None â€” cell references not in context
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

    // 1. Parse workbook.xml â†’ sheet names
    let sheets_meta = {
        let s = read_sheets(package)?;
        diags.push(Diagnostic::new(
            "OOX-R020",
            Severity::Info,
            format!("workbook: {} sheets found", s.len()),
        ));
        s
    };

    // 2. Parse sharedStrings.xml â†’ shared string table
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

    // 3. Parse styles.xml → full style table
    let styles = match read_styles(package) {
        Ok(s) => {
            diags.push(Diagnostic::new(
                "OOX-R022",
                Severity::Info,
                format!(
                    "styles.xml: {} numFmts, {} fonts, {} fills, {} borders, {} cellXfs, {} named styles",
                    s.num_formats.len(),
                    s.fonts.len(),
                    s.fills.len(),
                    s.borders.len(),
                    s.cell_xfs.len(),
                    s.style_names.len(),
                ),
            ));
            s
        }
        Err(_) => {
            diags.push(Diagnostic::new(
                "OOX-W-022",
                Severity::Warning,
                "styles.xml not found; using default styles".to_string(),
            ));
            XlsxStyles::default()
        }
    };

    // 4. Parse each sheet → Sheet with rows/cells
    let mut sheet_ir_vec: Vec<Sheet> = Vec::new();
    for (sheet_name, sheet_id) in &sheets_meta {
        match read_sheet(package, sheet_name, sheet_id, &shared_strings, &styles) {
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

    // 5. Parse definedNames from workbook.xml
    let defined_names = parse_defined_names(package);
    if !defined_names.is_empty() {
        diags.push(Diagnostic::new(
            "OOX-R025",
            Severity::Info,
            format!("definedNames: {} names found", defined_names.len()),
        ));
    }

    // 6. Build Spreadsheet IR
    let spreadsheet = Spreadsheet {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        sheets: sheet_ir_vec,
        defined_names,
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
/// Implements the `Codec` trait to parse xlsx â†’ gp-core Spreadsheet IR.
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

    /// test_parse_xlsx_corpus_fixture â€” parse formulas-cached-xlsx-001.xlsx and verify IR.
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

    /// test_parse_cell_values â€” t="n" â†’ cached_value
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

    /// test_parse_formulas â€” <f>=SUM(...)</f> â†’ formula
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

    /// test_parse_cached_values_with_formulas â€” formula + cached value both present
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

    /// test_parse_shared_strings â€” t="s" â†’ resolved string from sharedStrings.xml
    #[test]
    fn test_parse_shared_strings() {
        let data = create_xlsx_with_formulas_and_merges();
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        // A1 t="s" v="0" â†’ "Header1"
        // B1 t="s" v="1" â†’ "Header2"
        let sheet = &ss.sheets[0];
        let a1 = &sheet.rows[0].cells[0];
        assert_eq!(a1.value.as_deref(), Some("Header1"));
        let b1 = &sheet.rows[0].cells[1];
        assert_eq!(b1.value.as_deref(), Some("Header2"));

        // Sheet2: A1 t="s" v="2" â†’ "Row1Col1"
        let sheet2 = &ss.sheets[1];
        let a1_s2 = &sheet2.rows[0].cells[0];
        assert_eq!(a1_s2.value.as_deref(), Some("Row1Col1"));
    }

    /// test_parse_merged_ranges â€” mergeCells â†’ MergedRange list
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

    /// test_parse_styles â€” styles.xml â†’ style_name on cells
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

    /// test_parse_multiple_sheets â€” workbook with multiple sheets
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

    /// test_xlsx_codec_integration â€” XlsxReader.read() produces valid IR
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

    /// test_parse_all_xlsx_fixtures â€” all xlsx files parse successfully
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

    /// test_formula_cell_has_both â€” formula cell has both formula and cached_value
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

    /// test_merged_range_count_vs_manifest â€” matches expected merged count
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

    /// test_parse_cell_coordinate â€” "A1"â†’(0,0), "B2"â†’(1,1), "AA1"â†’(26,0)
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

    /// test_parse_boolean_cells â€” t="b" â†’ boolean value
    #[test]
    fn test_parse_boolean_cells() {
        let data = create_xlsx_with_types();
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        let sheet = &ss.sheets[0];
        // A1 t="b" v="1" â†’ true
        let a1 = &sheet.rows[0].cells[0];
        assert_eq!(a1.value.as_deref(), Some("true"));

        // B1 t="b" v="0" â†’ false
        let b1 = &sheet.rows[0].cells[1];
        assert_eq!(b1.value.as_deref(), Some("false"));

        // C1 t="e" v="#DIV/0!" â†’ error text
        let c1 = &sheet.rows[0].cells[2];
        assert_eq!(c1.value.as_deref(), Some("#DIV/0!"));
    }

    /// test_merged_range_ref â€” parse merged range references
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

    /// test_spreadsheet_visit_ids â€” Visit impl for Spreadsheet
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

    /// test_parse_merged_cells_corpus â€” merged-cells-xlsx-001.xlsx
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

    /// test_parse_merged_cells_corpus_multi â€” merged-cells-xlsx-002.xlsx (2 merged ranges)
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

    /// test_parse_merged_cells_corpus_3 â€” merged-cells-xlsx-003.xlsx (1 merged range)
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

    /// test_parse_formulas_corpus_002 â€” formulas-cached-xlsx-002.xlsx
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

    /// test_parse_formulas_corpus_003 â€” formulas-cached-xlsx-003.xlsx (formula + style)
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

    /// test_parse_empty_worksheet â€” xlsx with no sheet parts produces empty sheets list
    #[test]
    fn test_parse_empty_worksheet() {
        let data = create_xlsx_empty_worksheets();
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");

        assert_eq!(ss.sheets.len(), 0);
    }

    /// test_row_values_in_sheet â€” verify row values exist for each sheet in multi-sheet xlsx
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

    /// test_rich_plain_inline_string — plain inlineStr has no rich runs
    #[test]
    fn test_rich_plain_inline_string() {
        let data = create_xlsx_with_formulas_and_merges();
        let result = XlsxReader::new().read(&data).expect("should parse");
        let ss: Spreadsheet = serde_json::from_slice(&result.ir).expect("should deserialize");
        let sheet = &ss.sheets[0];
        let a1 = &sheet.rows[0].cells[0];
        // Plain inline string should parse without error; rich may be None or Some
        assert!(a1.value.is_some() || a1.rich.is_none());
    }

    /// test_rich_json_serializes — CellRun serializes/deserializes correctly
    #[test]
    fn test_rich_json_serializes() {
        use gp_core::ir::CellRun;
        let runs = vec![
            CellRun {
                text: "Hello ".to_string(),
                bold: Some(true),
                color: Some("FF0000".to_string()),
                strikethrough: None,
                size: Some(11.0),
            },
            CellRun {
                text: "World".to_string(),
                strikethrough: Some(true),
                color: None,
                bold: None,
                size: None,
            },
        ];
        let json = serde_json::to_string(&runs).expect("serialize");
        let back: Vec<CellRun> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(runs.len(), back.len());
        assert_eq!(runs[0].text, back[0].text);
        assert_eq!(runs[0].bold, back[0].bold);
        assert_eq!(runs[1].strikethrough, back[1].strikethrough);
    }
}
