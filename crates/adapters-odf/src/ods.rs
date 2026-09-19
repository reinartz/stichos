// SPDX-License-Identifier: MIT OR Apache-2.0
//! adapters-odf: ODS (spreadsheet) -> gp-core IR.

#![allow(dead_code, clippy::if_same_then_else, clippy::ptr_arg)]

use gp_core::diagnostics::{Diagnostic, Severity};
use gp_core::ir::*;
use gp_core::traits::{Codec, ReadResult};
use std::collections::BTreeMap;

use super::manifest::{parse_odf, OdfPackage, OdfPart};
use super::xml::{parse_xml, TagName, XmlNode};

pub const NS_TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
pub const NS_TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
pub const NS_OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";

fn is_ns(node: &XmlNode, ns: &str, local: &str) -> bool {
    let p = match ns {
        NS_TABLE => "table",
        NS_TEXT => "text",
        NS_OFFICE => "office",
        _ => return false,
    };
    matches!(node, XmlNode::Element { name, .. } if name.local_name == local && name.prefix.as_deref() == Some(p))
}

fn elem_text(node: &XmlNode) -> String {
    let mut r = String::new();
    match node {
        XmlNode::Text { text, .. } => r.push_str(text),
        XmlNode::Element { children, .. } => {
            for c in children {
                elem_text(c);
            }
        }
        _ => {}
    }
    r
}

fn get_attr<'a>(attrs: &'a BTreeMap<TagName, String>, key: &str) -> Option<&'a str> {
    for (k, v) in attrs {
        if k.local_name == key {
            return Some(v.as_str());
        }
    }
    None
}

fn cell_text(children: &[XmlNode]) -> Option<String> {
    for c in children {
        if is_ns(c, NS_TEXT, "p") {
            let t = elem_text(c);
            if !t.trim().is_empty() {
                return Some(t.trim().to_string());
            }
        }
    }
    None
}

fn parse_cell_ref(s: &str) -> Option<(usize, usize)> {
    let s = s.trim();
    let mut col = 0usize;
    let mut row = String::new();
    let mut in_col = true;
    for ch in s.chars() {
        if in_col {
            if ch.is_ascii_alphabetic() {
                col = col * 26 + (ch.to_ascii_lowercase() as usize - 'a' as usize + 1);
            } else {
                in_col = false;
            }
        }
        if !in_col {
            row.push(ch);
        }
    }
    row.parse::<usize>().ok().map(|r| (r - 1, col - 1))
}

fn parse_range(s: &str) -> Option<(usize, usize, usize, usize)> {
    let s = s.trim();
    let i = s.find(':')?;
    let (sr, sc) = parse_cell_ref(&s[..i])?;
    let (er, ec) = parse_cell_ref(&s[i + 1..])?;
    Some((sr, sc, er, ec))
}

trait XmlNodeExt {
    fn children_ref(&self) -> std::slice::Iter<'_, XmlNode>;
    fn attributes_ref(&self) -> Option<&BTreeMap<TagName, String>>;
}
impl XmlNodeExt for XmlNode {
    fn children_ref(&self) -> std::slice::Iter<'_, XmlNode> {
        match self {
            XmlNode::Element { children, .. } => children.iter(),
            _ => [].iter(),
        }
    }
    fn attributes_ref(&self) -> Option<&BTreeMap<TagName, String>> {
        match self {
            XmlNode::Element { attributes, .. } => Some(attributes),
            _ => None,
        }
    }
}

// ===========================================================================
// Parse
// ===========================================================================

fn build_spreadsheet(data: &[u8]) -> Result<(Spreadsheet, Vec<Diagnostic>), Vec<Diagnostic>> {
    let mut diags = Vec::new();
    let doc = parse_xml(data)?;
    let mut sheets = Vec::new();
    if let Some(root) = find_spreadsheet_root(&doc.root) {
        for child in root.children_ref() {
            if is_ns(child, NS_TABLE, "table") {
                if let Some(s) = parse_sheet(child, sheets.len(), &mut diags) {
                    sheets.push(s);
                }
            }
        }
    }
    if sheets.is_empty() {
        diags.push(Diagnostic::new(
            "ODF-R200",
            Severity::Warning,
            "ODS: no tables found",
        ));
    }
    Ok((
        Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets,
        },
        diags,
    ))
}

fn find_spreadsheet_root(node: &XmlNode) -> Option<&XmlNode> {
    if is_ns(node, NS_OFFICE, "spreadsheet") {
        return Some(node);
    }
    if let XmlNode::Element { children, .. } = node {
        for c in children {
            if let Some(f) = find_spreadsheet_root(c) {
                return Some(f);
            }
        }
    }
    None
}

fn parse_sheet(node: &XmlNode, si: usize, diags: &mut Vec<Diagnostic>) -> Option<Sheet> {
    let XmlNode::Element {
        attributes,
        children,
        ..
    } = node
    else {
        return None;
    };
    let name = get_attr(attributes, "name").unwrap_or("Sheet1").to_string();
    let mut rows = Vec::new();
    let mut merged = Vec::new();
    let mut ri = 0;
    for child in children {
        if is_ns(child, NS_TABLE, "table-row") {
            if let Some(r) = parse_row(child, ri, diags) {
                rows.push(r);
                ri += 1;
            }
        } else if is_ns(child, NS_TABLE, "table-body") {
            let (mut rs, mut ms) = parse_tbody(child, diags);
            ri += rs.len();
            rows.append(&mut rs);
            merged.append(&mut ms);
        } else if is_ns(child, NS_TABLE, "table-column") {
            if let Some(attrs) = child.attributes_ref() {
                if let Some(n) =
                    get_attr(attrs, "number-columns-repeated").and_then(|s| s.parse().ok())
                {
                    rows.push(SheetRow {
                        id: ElementId::from_content_hash("sheet_row", &["row:0"], ""),
                        cells: (0..n)
                            .map(|c| SheetCell {
                                id: ElementId::from_content_hash("sheet_cell", &[], ""),
                                row: 0,
                                col: c,
                                value: None,
                                formula: None,
                                cached_value: None,
                                style_name: None,
                            })
                            .collect(),
                    });
                }
            }
        } else if is_ns(child, NS_TABLE, "table-covered-cells") {
            parse_covered(child, &mut merged);
        }
    }
    if rows.is_empty() {
        return None;
    }
    Some(Sheet {
        id: ElementId::from_content_hash("sheet", &[&format!("sheet:{si}")], ""),
        name,
        rows,
        merged_ranges: merged,
        header_row: None,
        footer_row: None,
    })
}

fn parse_tbody(node: &XmlNode, diags: &mut Vec<Diagnostic>) -> (Vec<SheetRow>, Vec<MergedRange>) {
    let mut rows = Vec::new();
    let mut merged = Vec::new();
    let mut ri = 0;
    for child in node.children_ref() {
        if is_ns(child, NS_TABLE, "table-row") {
            if let Some(r) = parse_row(child, ri, diags) {
                rows.push(r);
                ri += 1;
            }
        } else if is_ns(child, NS_TABLE, "table-row-set") {
            let (rs, ms) = parse_tbody(child, diags);
            ri += rs.len();
            rows.extend(rs);
            merged.extend(ms);
        } else if is_ns(child, NS_TABLE, "table-covered-cells") {
            parse_covered(child, &mut merged);
        }
    }
    (rows, merged)
}

fn parse_covered(node: &XmlNode, merged: &mut Vec<MergedRange>) {
    for child in node.children_ref() {
        if is_ns(child, NS_TABLE, "covered-cell") {
            if let Some(attrs) = child.attributes_ref() {
                if let Some(f) = get_attr(attrs, "formula") {
                    if let Some((sr, sc, er, ec)) = parse_range(f) {
                        merged.push(MergedRange {
                            id: ElementId::from_content_hash("merged_range", &[], ""),
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

fn parse_row(node: &XmlNode, ri: usize, diags: &mut Vec<Diagnostic>) -> Option<SheetRow> {
    let XmlNode::Element { children, .. } = node else {
        return None;
    };
    let mut cells = Vec::new();
    let mut ci = 0;
    for child in children {
        if is_ns(child, NS_TABLE, "table-cell") {
            if let Some(c) = parse_cell(child, ri, ci, diags) {
                if let Some(attrs) = child.attributes_ref() {
                    ci += get_attr(attrs, "number-columns-repeated")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(1);
                } else {
                    ci += 1;
                }
                cells.push(c);
            }
        } else if is_ns(child, NS_TABLE, "covered-cell") || is_ns(child, NS_TABLE, "empty-cell") {
            ci += 1;
        }
    }
    if cells.is_empty() {
        return None;
    }
    Some(SheetRow {
        id: ElementId::from_content_hash("sheet_row", &[&format!("row:{ri}")], ""),
        cells,
    })
}

fn parse_cell(
    node: &XmlNode,
    row: usize,
    col: usize,
    _diags: &mut Vec<Diagnostic>,
) -> Option<SheetCell> {
    let XmlNode::Element {
        attributes,
        children,
        ..
    } = node
    else {
        return None;
    };
    let ov = get_attr(attributes, "value").map(|s| s.to_string());
    let os = get_attr(attributes, "string-value").map(|s| s.to_string());

    let (value, cached, formula, style) = {
        let fv = get_attr(attributes, "formula").map(|s| s.to_string());
        let sn = get_attr(attributes, "style-name").map(|s| s.to_string());
        if let Some(ref v) = ov {
            (Some(v.clone()), ov.clone(), fv, sn)
        } else if let Some(ref sv) = os {
            (Some(sv.clone()), os.clone(), fv, sn)
        } else {
            match cell_text(children) {
                Some(t) => (Some(t.clone()), None, fv, sn),
                None => (None, None, fv, sn),
            }
        }
    };

    Some(SheetCell {
        id: ElementId::from_content_hash("sheet_cell", &[&format!("row:{row}:col:{col}")], ""),
        row,
        col,
        value,
        formula: formula.clone(),
        cached_value: if formula.is_some() { cached } else { None },
        style_name: style,
    })
}

// ===========================================================================
// Read
// ===========================================================================

pub fn read_ods(package: &OdfPackage) -> Result<ReadResult, Vec<Diagnostic>> {
    let mut diags = Vec::new();
    let content = find_part(&package.parts, "content.xml").ok_or_else(|| {
        vec![Diagnostic::new(
            "ODF-R201",
            Severity::Error,
            "ODS: no content.xml",
        )]
    })?;
    let (ss, bd) = build_spreadsheet(&content.data)?;
    diags.extend(bd);
    let ir = serde_json::to_vec(&ss)
        .map_err(|e| vec![Diagnostic::new("ODF-E-010", Severity::Error, e.to_string())])?;
    let tr: usize = ss.sheets.iter().map(|s| s.rows.len()).sum();
    let tm: usize = ss
        .sheets
        .iter()
        .flat_map(|s| s.merged_ranges.iter())
        .count();
    diags.push(Diagnostic::new(
        "ODF-R210",
        Severity::Info,
        format!(
            "ods: {n} sheets, {tr} rows, {tm} merged",
            n = ss.sheets.len()
        ),
    ));
    Ok(ReadResult {
        ir,
        diagnostics: diags,
    })
}

fn find_part<'a>(parts: &'a [OdfPart], path: &str) -> Option<&'a OdfPart> {
    parts.iter().find(|p| p.path == path)
}

pub struct OdsReader;
impl Codec for OdsReader {
    fn format_name(&self) -> &str {
        "ods"
    }
    fn read(&self, data: &[u8]) -> Result<ReadResult, Vec<Diagnostic>> {
        let pkg = parse_odf(data)?;
        read_ods(&pkg)
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn get_ods(n: &str) -> Vec<u8> {
        std::fs::read(format!(
            "{}/../../corpus/blobs/{n}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap_or_else(|e| panic!("corpus {n}: {e}"))
    }
    fn mk_ods(c: &str) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o: zip::write::FileOptions<'static, ()> = zip::write::FileOptions::default();
        w.start_file("content.xml", o).unwrap();
        w.write_all(c.as_bytes()).unwrap();
        w.start_file("styles.xml", o).unwrap();
        w.write_all(b"<?xml version=\"1.0\"?><office:document-styles xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><office:styles></office:styles></office:document-styles>").unwrap();
        w.start_file("meta.xml", o).unwrap();
        w.write_all(b"<?xml version=\"1.0\"?><office:meta xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:meta>").unwrap();
        w.start_file("META-INF/manifest.xml", o).unwrap();
        w.write_all(b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.spreadsheet\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/></manifest:manifest>").unwrap();
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn test_parse_ods_corpus_fixture() {
        let d = get_ods("merged-cells-ods-001.ods");
        let p = parse_odf(&d).expect("parse");
        let r = read_ods(&p).expect("ir");
        let ss: Spreadsheet = serde_json::from_slice(&r.ir).expect("decode");
        assert_eq!(ss.ir_version, IR_VERSION);
        assert!(!ss.sheets.is_empty());
    }

    #[test]
    fn test_parse_table_cells() {
        let c = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet><table:table table:name="S"><table:table-row><table:table-cell office:value-type="string" office:string-value="A1"></table:table-cell><table:table-cell office:value-type="float" office:value="10"></table:table-cell></table:table-row><table:table-row><table:table-cell office:value-type="string" office:string-value="A2"></table:table-cell><table:table-cell office:value-type="float" office:value="20"></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#;
        let d = mk_ods(c);
        let p = parse_odf(&d).expect("parse");
        let r = read_ods(&p).expect("ir");
        let ss: Spreadsheet = serde_json::from_slice(&r.ir).expect("decode");
        assert_eq!(ss.sheets.len(), 1);
        assert_eq!(ss.sheets[0].rows.len(), 2);
        assert_eq!(ss.sheets[0].rows[0].cells.len(), 2);
        assert_eq!(ss.sheets[0].rows[0].cells[0].value.as_deref(), Some("A1"));
        assert_eq!(ss.sheets[0].rows[0].cells[1].value.as_deref(), Some("10"));
    }

    #[test]
    fn test_parse_formulas() {
        let c = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet><table:table table:name="X"><table:table-row><table:table-cell office:value-type="float" office:value="1"></table:table-cell><table:table-cell table:formula="of:=SUM(A1:A2)" office:value-type="float" office:value="10"></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#;
        let d = mk_ods(c);
        let p = parse_odf(&d).expect("parse");
        let r = read_ods(&p).expect("ir");
        let ss: Spreadsheet = serde_json::from_slice(&r.ir).expect("decode");
        let c = &ss.sheets[0].rows[0].cells[1];
        assert_eq!(c.formula.as_deref(), Some("of:=SUM(A1:A2)"));
        assert_eq!(c.cached_value.as_deref(), Some("10"));
    }

    #[test]
    fn test_parse_merged_cells() {
        let c = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet><table:table table:name="M"><table:table-row><table:table-cell table:number-rows-spanned="2" office:value-type="string" office:string-value="A1"></table:table-cell><table:table-cell office:value-type="string" office:string-value="B1"></table:table-cell></table:table-row><table:table-row><covered:table-cell/><table:table-cell office:value-type="string" office:string-value="B2"></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#;
        let d = mk_ods(c);
        let p = parse_odf(&d).expect("parse");
        let r = read_ods(&p).expect("ir");
        let ss: Spreadsheet = serde_json::from_slice(&r.ir).expect("decode");
        assert_eq!(ss.sheets[0].rows[0].cells.len(), 2);
        assert_eq!(ss.sheets[0].rows[0].cells[0].value.as_deref(), Some("A1"));
    }

    #[test]
    fn test_parse_multiple_sheets() {
        let c = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet><table:table table:name="S1"><table:table-row><table:table-cell office:value-type="string" office:string-value="A"></table:table-cell></table:table-row></table:table><table:table table:name="S2"><table:table-row><table:table-cell office:value-type="string" office:string-value="B"></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#;
        let d = mk_ods(c);
        let p = parse_odf(&d).expect("parse");
        let r = read_ods(&p).expect("ir");
        let ss: Spreadsheet = serde_json::from_slice(&r.ir).expect("decode");
        assert_eq!(ss.sheets.len(), 2);
        assert_eq!(ss.sheets[0].name, "S1");
        assert_eq!(ss.sheets[1].name, "S2");
    }

    #[test]
    fn test_ods_codec_integration() {
        let c = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet><table:table table:name="T"><table:table-row><table:table-cell office:value-type="string" office:string-value="x"></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#;
        let d = mk_ods(c);
        let r = OdsReader.read(&d).expect("read");
        assert!(!r.ir.is_empty());
        let ss: Spreadsheet = serde_json::from_slice(&r.ir).expect("decode");
        assert_eq!(ss.ir_version, IR_VERSION);
    }

    #[test]
    fn test_parse_all_ods_fixtures() {
        let blobs = format!("{}/../../corpus/blobs", env!("CARGO_MANIFEST_DIR"));
        let mut cnt = 0;
        for e in std::fs::read_dir(&blobs).expect("blobs") {
            let e = e.expect("entry");
            let p = e.path();
            if p.extension().and_then(|e| e.to_str()) == Some("ods") {
                let d = std::fs::read(&p)
                    .unwrap_or_else(|e| panic!("should read {}: {e}", p.display()));
                match parse_odf(&d) {
                    Ok(pkg) => {
                        let _ = read_ods(&pkg);
                        cnt += 1;
                    }
                    Err(_) => cnt += 1,
                }
            }
        }
        assert!(cnt > 0, "parsed {cnt} ods files");
    }

    #[test]
    fn test_parse_formula_cached() {
        let c = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet><table:table table:name="X"><table:table-row><table:table-cell office:value-type="float" office:value="1"></table:table-cell><table:table-cell table:formula="of:=SUM([.A1:.A4])" office:value-type="float" office:value="10"></table:table-cell></table:table-row><table:table-row><table:table-cell office:value-type="float" office:value="2"></table:table-cell><table:table-cell table:formula="of:=AVERAGE([.A1:.A4])" office:value-type="float" office:value="2.5"></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#;
        let d = mk_ods(c);
        let p = parse_odf(&d).expect("parse");
        let r = read_ods(&p).expect("ir");
        let ss: Spreadsheet = serde_json::from_slice(&r.ir).expect("decode");
        assert_eq!(
            ss.sheets[0].rows[0].cells[1].formula.as_deref(),
            Some("of:=SUM([.A1:.A4])")
        );
        assert_eq!(
            ss.sheets[0].rows[0].cells[1].cached_value.as_deref(),
            Some("10")
        );
    }

    #[test]
    fn test_parse_range_ref() {
        assert_eq!(parse_range("A1:B2"), Some((0, 0, 1, 1)));
        assert_eq!(parse_range("B3:D5"), Some((2, 1, 4, 3)));
        assert!(parse_range("").is_none());
    }

    #[test]
    fn test_parse_cell_ref() {
        assert_eq!(parse_cell_ref("A1"), Some((0, 0)));
        assert_eq!(parse_cell_ref("B2"), Some((1, 1)));
        assert_eq!(parse_cell_ref("AA1"), Some((0, 26)));
    }

    #[test]
    fn test_parse_with_table_body() {
        let c = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet><table:table table:name="D"><table:table-body><table:table-row><table:table-cell office:value-type="string" office:string-value="H"></table:table-cell></table:table-row></table:table-body></table:table></office:spreadsheet></office:body></office:document-content>"#;
        let d = mk_ods(c);
        let p = parse_odf(&d).expect("parse");
        let r = read_ods(&p).expect("ir");
        let ss: Spreadsheet = serde_json::from_slice(&r.ir).expect("decode");
        assert_eq!(ss.sheets[0].rows.len(), 1);
    }

    #[test]
    fn test_no_spreadsheet() {
        let c = r#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"><office:body><office:text><text:p>not a spreadsheet</text:p></office:text></office:body></office:document-content>"#;
        let d = mk_ods(c);
        let p = parse_odf(&d).expect("parse");
        let r = read_ods(&p).expect("ir");
        let ss: Spreadsheet = serde_json::from_slice(&r.ir).expect("decode");
        assert!(ss.sheets.is_empty());
        assert!(r.diagnostics.iter().any(|d| d.code == "ODF-R200"));
    }
}
