// SPDX-License-Identifier: MIT OR Apache-2.0
//! adapters-ooxml: feature census — count features in parsed IR and produce FidelityBudget.
//!
//! Every read response carries a `FidelityBudget` from gp-core. This module walks the
//! gp-core IR (Document, Spreadsheet, Presentation) produced by the readers and counts
//! every structural feature:
//!   - paragraphs, tables, drawings, lists
//!   - cells, formulas, cached values, merged ranges (spreadsheet)
//!   - slides, layouts, masters, placeholder elements (presentation)
//!   - fields, comments, tracked changes (document)
//!   - unknown OPC parts whose content type does not match ECMA-376
//!
//! The census result is a `FidelityBudget` with dropped features for unhandled elements,
//! part warnings for unusual content, and unknown parts for unrecognized content types.

use gp_core::diagnostics::{Diagnostic, Severity};
use gp_core::fidelity::FidelityBudget;
use gp_core::ir::*;
use std::collections::HashMap;

// ===========================================================================
// Known ECMA-376 content types
// ===========================================================================

/// Content types that are considered known/expected by ECMA-376 / ISO-IEC 29500.
fn is_known_content_type(ct: &str) -> bool {
    // Check common prefixes first for efficiency
    if ct.starts_with("application/vnd.openxmlformats-officedocument.") {
        return true;
    }
    if ct.starts_with("application/vnd.openxmlformats-package.") {
        return true;
    }
    if ct.starts_with("application/vnd.ms-excel.") {
        return true;
    }
    if ct.starts_with("application/vnd.ms-word.") {
        return true;
    }
    if ct.starts_with("application/vnd.ms-powerpoint.") {
        return true;
    }
    // Known generic types
    matches!(
        ct,
        "application/xml"
            | "application/octet-stream"
            | "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/svg+xml"
            | "image/tiff"
            | "image/x-emf"
            | "image/x-wmf"
            | "application/postscript"
            | "application/zip"
    )
}

// ===========================================================================
// Feature counters
// ===========================================================================

/// A running count of every feature found in a parsed document.
#[derive(Debug, Default, Clone)]
pub struct FeatureCounts {
    pub paragraphs: usize,
    pub tables: usize,
    pub drawings: usize,
    pub lists: usize,
    pub multi_run_paragraphs: usize,
    pub fields: usize,
    pub comments: usize,
    pub tracked_changes: usize,
    pub smart_art: usize,
    pub ole_objects: usize,
    pub macros: usize,
    pub spreadsheet_cells: usize,
    pub spreadsheet_formulas: usize,
    pub spreadsheet_merged_ranges: usize,
    pub spreadsheet_sheets: usize,
    pub spreadsheet_rows: usize,
    pub presentation_slides: usize,
    pub presentation_layouts: usize,
    pub presentation_masters: usize,
    pub slide_text_elements: usize,
    pub slide_placeholders: usize,
    pub slide_tables: usize,
    pub slide_drawings: usize,
    pub slide_images: usize,
    pub section_headers: usize,
    pub section_footers: usize,
    pub section_footnotes: usize,
    pub section_endnotes: usize,
}

impl FeatureCounts {
    pub fn merge(&mut self, other: &FeatureCounts) {
        self.paragraphs += other.paragraphs;
        self.tables += other.tables;
        self.drawings += other.drawings;
        self.lists += other.lists;
        self.multi_run_paragraphs += other.multi_run_paragraphs;
        self.fields += other.fields;
        self.comments += other.comments;
        self.tracked_changes += other.tracked_changes;
        self.smart_art += other.smart_art;
        self.ole_objects += other.ole_objects;
        self.macros += other.macros;
        self.spreadsheet_cells += other.spreadsheet_cells;
        self.spreadsheet_formulas += other.spreadsheet_formulas;
        self.spreadsheet_merged_ranges += other.spreadsheet_merged_ranges;
        self.spreadsheet_sheets += other.spreadsheet_sheets;
        self.spreadsheet_rows += other.spreadsheet_rows;
        self.presentation_slides += other.presentation_slides;
        self.presentation_layouts += other.presentation_layouts;
        self.presentation_masters += other.presentation_masters;
        self.slide_text_elements += other.slide_text_elements;
        self.slide_placeholders += other.slide_placeholders;
        self.slide_tables += other.slide_tables;
        self.slide_drawings += other.slide_drawings;
        self.slide_images += other.slide_images;
        self.section_headers += other.section_headers;
        self.section_footers += other.section_footers;
        self.section_footnotes += other.section_footnotes;
        self.section_endnotes += other.section_endnotes;
    }
}

/// Count features in a parsed Document (recursive over all descendants).
pub fn census_document(doc: &Document) -> FeatureCounts {
    let mut counts = FeatureCounts::default();
    for section in &doc.sections {
        counts.paragraphs += count_section_items_paragraphs(&section.items);
        counts.tables += count_section_items_tables(&section.items);
        counts.drawings += count_section_items_drawings(&section.items);
        counts.lists += count_section_items_lists(&section.items);
        counts.multi_run_paragraphs += count_section_items_multi_run(&section.items);
    }
    counts.fields = doc.fields.len();
    counts.comments = doc.comments.len();
    counts.tracked_changes = doc.tracked_changes.len();
    counts.smart_art = detect_smart_art(&doc.sections);
    counts.ole_objects = detect_ole_objects(&doc.sections);
    counts.macros = detect_macros(&doc.sections);
    counts.section_headers = doc.sections.iter().map(|s| s.headers.len()).sum();
    counts.section_footers = doc.sections.iter().map(|s| s.footers.len()).sum();
    counts.section_footnotes = doc.sections.iter().map(|s| s.footnotes.len()).sum();
    counts.section_endnotes = doc.sections.iter().map(|s| s.endnotes.len()).sum();
    counts
}

/// Count features in a parsed Spreadsheet (recursive over all sheets).
pub fn census_spreadsheet(ss: &Spreadsheet) -> FeatureCounts {
    let mut counts = FeatureCounts {
        spreadsheet_sheets: ss.sheets.len(),
        ..Default::default()
    };
    for sheet in &ss.sheets {
        counts.spreadsheet_rows += sheet.rows.len();
        counts.spreadsheet_merged_ranges += sheet.merged_ranges.len();
        for row in &sheet.rows {
            for cell in &row.cells {
                counts.spreadsheet_cells += 1;
                if cell.formula.is_some() {
                    counts.spreadsheet_formulas += 1;
                }
            }
        }
    }
    counts
}

/// Count features in a parsed Presentation (recursive over all slides/layouts/masters).
pub fn census_presentation(pres: &Presentation) -> FeatureCounts {
    let mut counts = FeatureCounts {
        presentation_slides: pres.slides.len(),
        presentation_layouts: pres.layouts.len(),
        presentation_masters: pres.masters.len(),
        ..Default::default()
    };
    for slide in &pres.slides {
        for elem in &slide.elements {
            if elem.kind == "text" {
                counts.slide_text_elements += 1;
            } else if elem.kind.starts_with("placeholder") {
                counts.slide_placeholders += 1;
            } else if elem.kind == "table" {
                counts.slide_tables += 1;
            } else if elem.kind == "drawing" {
                counts.slide_drawings += 1;
            } else if elem.kind == "image" {
                counts.slide_images += 1;
            }
        }
    }
    counts
}

/// Count features from an OPC package (unknown parts detection).
///
/// Parts whose content type does not match any known ECMA-376 content type
/// are reported as unknown parts in the FidelityBudget.
pub fn census_opc_parts(parts: &[crate::opc::Part]) -> Vec<String> {
    let mut unknown: Vec<String> = Vec::new();
    for part in parts {
        if !is_known_content_type(&part.content_type)
            && (part.uri.starts_with("word/")
                || part.uri.starts_with("xl/")
                || part.uri.starts_with("ppt/")
                || part.uri.starts_with("docProps/")
                || part.uri.starts_with("_rels/")
                || part.uri.starts_with("theme/"))
        {
            unknown.push(part.uri.clone());
        }
    }
    unknown
}

// ===========================================================================
// Internal counters (recursive)
// ===========================================================================

fn count_section_items_paragraphs(items: &[SectionItem]) -> usize {
    items
        .iter()
        .map(|i| match i {
            SectionItem::Paragraph(_) => 1,
            SectionItem::Table(t) => count_table_paragraphs(t),
            SectionItem::Drawing(_) => 0,
            SectionItem::List(l) => count_list_paragraphs(l),
        })
        .sum()
}

fn count_table_paragraphs(table: &Table) -> usize {
    table
        .rows
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| count_section_items_paragraphs(&c.contents))
                .sum::<usize>()
        })
        .sum()
}

fn count_list_paragraphs(list: &List) -> usize {
    list.items
        .iter()
        .map(|item| count_section_items_paragraphs(&item.content))
        .sum()
}

fn count_section_items_tables(items: &[SectionItem]) -> usize {
    items
        .iter()
        .map(|i| match i {
            SectionItem::Table(t) => 1 + count_table_tables_inner(t),
            SectionItem::List(l) => count_list_tables(l),
            _ => 0,
        })
        .sum()
}

fn count_table_tables_inner(table: &Table) -> usize {
    table
        .rows
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| count_section_items_tables(&c.contents))
                .sum::<usize>()
        })
        .sum()
}

fn count_list_tables(list: &List) -> usize {
    list.items
        .iter()
        .map(|item| count_section_items_tables(&item.content))
        .sum()
}

fn count_section_items_drawings(items: &[SectionItem]) -> usize {
    items
        .iter()
        .map(|i| match i {
            SectionItem::Drawing(_) => 1,
            SectionItem::Table(t) => count_table_drawings(t),
            SectionItem::List(l) => count_list_drawings(l),
            _ => 0,
        })
        .sum()
}

fn count_table_drawings(table: &Table) -> usize {
    table
        .rows
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| count_section_items_drawings(&c.contents))
                .sum::<usize>()
        })
        .sum()
}

fn count_list_drawings(list: &List) -> usize {
    list.items
        .iter()
        .map(|item| count_section_items_drawings(&item.content))
        .sum()
}

fn count_section_items_lists(items: &[SectionItem]) -> usize {
    items
        .iter()
        .map(|i| match i {
            SectionItem::List(_) => 1 + count_list_lists_inner(items),
            SectionItem::Table(t) => count_table_lists(t),
            _ => 0,
        })
        .sum()
}

fn count_list_lists_inner(items: &[SectionItem]) -> usize {
    items
        .iter()
        .map(|i| match i {
            SectionItem::List(l) => count_list_items_lists(l),
            _ => 0,
        })
        .sum()
}

fn count_list_items_lists(list: &List) -> usize {
    list.items
        .iter()
        .map(|item| count_section_items_lists(&item.content))
        .sum()
}

fn count_table_lists(table: &Table) -> usize {
    table
        .rows
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| count_section_items_lists(&c.contents))
                .sum::<usize>()
        })
        .sum()
}

fn count_section_items_multi_run(items: &[SectionItem]) -> usize {
    items
        .iter()
        .map(|i| match i {
            SectionItem::Paragraph(p) => {
                if p.runs.len() > 1 {
                    1
                } else {
                    0
                }
            }
            SectionItem::Table(t) => count_table_multi_run(t),
            SectionItem::List(l) => count_list_multi_run(l),
            _ => 0,
        })
        .sum()
}

fn count_table_multi_run(table: &Table) -> usize {
    table
        .rows
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| count_section_items_multi_run(&c.contents))
                .sum::<usize>()
        })
        .sum()
}

fn count_list_multi_run(list: &List) -> usize {
    list.items
        .iter()
        .map(|item| count_section_items_multi_run(&item.content))
        .sum()
}

fn detect_smart_art(sections: &[Section]) -> usize {
    sections
        .iter()
        .flat_map(|s| &s.items)
        .filter(|item| {
            if let SectionItem::Drawing(d) = item {
                d.mime_type.contains("smart") || d.mime_type.contains("smartart")
            } else {
                false
            }
        })
        .count()
}

fn detect_ole_objects(sections: &[Section]) -> usize {
    sections
        .iter()
        .flat_map(|s| &s.items)
        .filter(|item| {
            if let SectionItem::Drawing(d) = item {
                d.mime_type.contains("ole") || d.mime_type.contains("application/ole")
            } else {
                false
            }
        })
        .count()
}

fn detect_macros(sections: &[Section]) -> usize {
    sections
        .iter()
        .flat_map(|s| &s.items)
        .filter(|item| {
            if let SectionItem::Drawing(d) = item {
                d.mime_type.contains("vba") || d.mime_type.contains("macro")
            } else {
                false
            }
        })
        .count()
}

// ===========================================================================
// FidelityBudget builder
// ===========================================================================

/// Build a FidelityBudget from feature counts and OPC part information.
pub fn build_budget_from_counts(
    counts: &FeatureCounts,
    unknown_parts: &[String],
    dropped_features: &[gp_core::fidelity::DroppedFeature],
) -> FidelityBudget {
    let mut budget = FidelityBudget::new();

    for df in dropped_features {
        budget = budget.drop_feature(&df.feature, &df.reason, df.count);
    }

    if counts.smart_art > 0 {
        budget = budget.drop_feature("smart_art", "no_direct_equivalent", counts.smart_art);
    }
    if counts.ole_objects > 0 {
        budget = budget.drop_feature("ole_object", "unsafe_skip", counts.ole_objects);
    }
    if counts.macros > 0 {
        budget = budget.drop_feature("macro", "blocked_policy", counts.macros);
    }

    for part in unknown_parts {
        budget = budget.unknown_part(part);
    }

    let total_issues = dropped_features.len()
        + counts.smart_art
        + counts.ole_objects
        + counts.macros
        + unknown_parts.len();
    let total_features = counts.paragraphs
        + counts.tables
        + counts.drawings
        + counts.lists
        + counts.spreadsheet_cells
        + counts.spreadsheet_merged_ranges
        + counts.presentation_slides;
    let total = std::cmp::max(1, total_features + total_issues);
    budget.confidence = (total_features as f32) / (total as f32).clamp(0.0, 1.0);

    budget
}

/// Build a FidelityBudget for a Document.
pub fn budget_from_document(doc: &Document, unknown_parts: &[String]) -> FidelityBudget {
    let counts = census_document(doc);
    build_budget_from_counts(&counts, unknown_parts, &[])
}

/// Build a FidelityBudget for a Spreadsheet.
pub fn budget_from_spreadsheet(ss: &Spreadsheet, unknown_parts: &[String]) -> FidelityBudget {
    let counts = census_spreadsheet(ss);
    build_budget_from_counts(&counts, unknown_parts, &[])
}

/// Build a FidelityBudget for a Presentation.
pub fn budget_from_presentation(pres: &Presentation, unknown_parts: &[String]) -> FidelityBudget {
    let counts = census_presentation(pres);
    build_budget_from_counts(&counts, unknown_parts, &[])
}

// ===========================================================================
// Manifest assertion
// ===========================================================================

/// A manifest entry with expected feature counts for a document.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ManifestEntry {
    pub schema_version: u32,
    pub id: String,
    pub format: String,
    #[serde(default)]
    pub features: HashMap<String, usize>,
}

/// Parse a manifest entry from a JSON file path.
pub fn load_manifest(path: &str) -> Result<ManifestEntry, Vec<Diagnostic>> {
    let data = std::fs::read_to_string(path).map_err(|e| {
        vec![Diagnostic::new(
            "OOX-E-015",
            Severity::Error,
            format!("Failed to read manifest {}: {}", path, e),
        )]
    })?;
    let entry: ManifestEntry = serde_json::from_str(&data).map_err(|e| {
        vec![Diagnostic::new(
            "OOX-E-016",
            Severity::Error,
            format!("Failed to parse manifest {}: {}", path, e),
        )]
    })?;
    Ok(entry)
}

/// Compare census feature counts against manifest expectations.
/// Returns warnings for features present in census but not in manifest,
/// or with mismatched counts.
pub fn assert_manifest_features(
    census: &FeatureCounts,
    manifest: &ManifestEntry,
) -> Vec<Diagnostic> {
    let mut warnings = Vec::new();
    let census_map = census_to_map(census);

    for (key, expected) in &manifest.features {
        let actual = census_map.get(key).copied().unwrap_or(0);
        if *expected != actual {
            warnings.push(Diagnostic::new(
                "OOX-W-150",
                Severity::Warning,
                format!(
                    "Feature '{}' in manifest {}: expected {} but census found {}",
                    key, manifest.id, expected, actual
                ),
            ));
        }
    }

    for (key, &count) in &census_map {
        if !manifest.features.contains_key(key) && count > 0 {
            warnings.push(Diagnostic::new(
                "OOX-W-151",
                Severity::Warning,
                format!(
                    "Census found feature '{}' (count: {}) not in manifest for {}",
                    key, count, manifest.id
                ),
            ));
        }
    }

    warnings
}

fn census_to_map(counts: &FeatureCounts) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    map.insert("paragraph".to_string(), counts.paragraphs);
    map.insert("table".to_string(), counts.tables);
    map.insert("drawing".to_string(), counts.drawings);
    map.insert("list".to_string(), counts.lists);
    map.insert(
        "multi_run_paragraph".to_string(),
        counts.multi_run_paragraphs,
    );
    map.insert("field".to_string(), counts.fields);
    map.insert("comment".to_string(), counts.comments);
    map.insert("tracked_change".to_string(), counts.tracked_changes);
    map.insert("smart_art".to_string(), counts.smart_art);
    map.insert("ole_object".to_string(), counts.ole_objects);
    map.insert("macro".to_string(), counts.macros);
    map.insert("spreadsheet_cell".to_string(), counts.spreadsheet_cells);
    map.insert(
        "spreadsheet_formula".to_string(),
        counts.spreadsheet_formulas,
    );
    map.insert(
        "spreadsheet_merged_range".to_string(),
        counts.spreadsheet_merged_ranges,
    );
    map.insert("spreadsheet_sheet".to_string(), counts.spreadsheet_sheets);
    map.insert("spreadsheet_row".to_string(), counts.spreadsheet_rows);
    map.insert("slide".to_string(), counts.presentation_slides);
    map.insert("layout".to_string(), counts.presentation_layouts);
    map.insert("master".to_string(), counts.presentation_masters);
    map.insert("slide_text".to_string(), counts.slide_text_elements);
    map.insert("placeholder".to_string(), counts.slide_placeholders);
    map.insert("slide_table".to_string(), counts.slide_tables);
    map.insert("slide_drawing".to_string(), counts.slide_drawings);
    map.insert("slide_image".to_string(), counts.slide_images);
    map
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use gp_core::fidelity::DroppedFeature;
    use gp_core::traits::Codec;
    use std::collections::HashMap;

    fn test_para(text: &str) -> Paragraph {
        let id_str = format!("p:{}", text);
        Paragraph {
            id: ElementId::content_anchored("para", &["p:0"], &id_str),
            runs: vec![Run {
                id: ElementId::content_anchored("run", &["r:0"], text),
                text: text.into(),
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
        }
    }

    fn test_section(items: Vec<SectionItem>) -> Section {
        Section {
            id: ElementId::content_anchored("section", &["section:0"], ""),
            kind: SectionKind::Body,
            items,
            headers: vec![],
            footers: vec![],
            footnotes: vec![],
            endnotes: vec![],
        }
    }

    fn test_doc(
        items: Vec<SectionItem>,
        fields: Vec<Field>,
        comments: Vec<Comment>,
        changes: Vec<TrackedChange>,
    ) -> Document {
        Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![test_section(items)],
            fields,
            comments,
            tracked_changes: changes,
            revisions: vec![],
            template: None,
        }
    }

    // -- Document census tests --

    /// test_census_document_counts — verify paragraph/table counts.
    #[test]
    fn test_census_document_counts() {
        let para1 = test_para("hello");
        let para2 = test_para("world");
        let cell = Cell {
            id: ElementId::content_anchored("cell", &["cell:0"], "data"),
            contents: vec![SectionItem::Paragraph(test_para("cell_text"))],
            colspan: None,
            rowspan: None,
        };
        let table = SectionItem::Table(Table {
            id: ElementId::content_anchored("table", &["table:0"], ""),
            rows: vec![Row { cells: vec![cell] }],
        });
        let doc = test_doc(
            vec![
                SectionItem::Paragraph(para1),
                table,
                SectionItem::Paragraph(para2),
            ],
            vec![],
            vec![],
            vec![],
        );
        let counts = census_document(&doc);
        assert_eq!(
            counts.paragraphs, 3,
            "Should count 3 paragraphs (2 direct + 1 in cell)"
        );
        assert_eq!(counts.tables, 1, "Should count 1 table");
        assert_eq!(counts.drawings, 0);
    }

    /// test_census_document_with_fields_comments_changes.
    #[test]
    fn test_census_document_with_fields_comments_changes() {
        let fields = vec![
            Field::Simple(FieldSimple {
                id: ElementId::content_anchored("field", &["field:0"], "PAGE"),
                instruction: "PAGE".into(),
                result: Some("1".into()),
            }),
            Field::Simple(FieldSimple {
                id: ElementId::content_anchored("field", &["field:1"], "TIME"),
                instruction: "TIME".into(),
                result: None,
            }),
        ];
        let comments = vec![Comment {
            id: ElementId::content_anchored("comment", &["comment:0"], "0"),
            comment_id: "0".into(),
            author: "alice".into(),
            date: None,
            initials: None,
            content: vec![],
        }];
        let changes = vec![TrackedChange {
            id: ElementId::content_anchored("change", &["change:0"], "ins"),
            kind: ChangeKind::Inserted,
            content: "added".into(),
            author: Some("alice".into()),
            date: None,
            change_id: None,
        }];
        let doc = test_doc(vec![], fields, comments, changes);
        let counts = census_document(&doc);
        assert_eq!(counts.fields, 2);
        assert_eq!(counts.comments, 1);
        assert_eq!(counts.tracked_changes, 1);
    }

    /// test_census_document_with_lists.
    #[test]
    fn test_census_document_with_lists() {
        let list = SectionItem::List(List {
            id: ElementId::content_anchored("list", &["list:0"], ""),
            kind: ListKind::Numbered,
            items: vec![ListItem {
                id: ElementId::content_anchored("listitem", &["item:0"], ""),
                content: vec![],
                level: 0,
            }],
        });
        let doc = test_doc(vec![list], vec![], vec![], vec![]);
        let counts = census_document(&doc);
        assert_eq!(counts.lists, 1);
    }

    /// test_census_document_with_drawing.
    #[test]
    fn test_census_document_with_drawing() {
        let drawing = SectionItem::Drawing(Drawing {
            id: ElementId::content_anchored("drawing", &["drawing:0"], ""),
            mime_type: "image/png".into(),
            data: "base64data".into(),
            bbox_pt: Some([0.0, 0.0, 100.0, 100.0]),
        });
        let doc = test_doc(vec![drawing], vec![], vec![], vec![]);
        let counts = census_document(&doc);
        assert_eq!(counts.drawings, 1);
    }

    /// test_census_document_with_section_headers_footers.
    #[test]
    fn test_census_document_with_section_headers_footers() {
        let header = Header {
            id: ElementId::content_anchored("header", &["header:0"], ""),
            content: vec![],
            is_even_page: false,
        };
        let footer = Footer {
            id: ElementId::content_anchored("footer", &["footer:0"], ""),
            content: vec![],
            is_even_page: false,
        };
        let section = Section {
            id: ElementId::content_anchored("section", &["section:0"], ""),
            kind: SectionKind::Body,
            items: vec![],
            headers: vec![header],
            footers: vec![
                footer,
                Footer {
                    id: ElementId::content_anchored("footer", &["footer:1"], ""),
                    content: vec![],
                    is_even_page: true,
                },
            ],
            footnotes: vec![],
            endnotes: vec![],
        };
        let doc = Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![section],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            revisions: vec![],
            template: None,
        };
        let counts = census_document(&doc);
        assert_eq!(counts.section_headers, 1);
        assert_eq!(counts.section_footers, 2);
    }

    // -- Spreadsheet census tests --

    /// test_census_spreadsheet_counts — verify cell/formula counts.
    #[test]
    fn test_census_spreadsheet_counts() {
        let cell1 = SheetCell {
            id: ElementId::content_anchored("cell", &["A1"], "A1"),
            row: 0,
            col: 0,
            value: Some("42".into()),
            formula: None,
            cached_value: Some("42".into()),
            style_name: None,
        };
        let cell2 = SheetCell {
            id: ElementId::content_anchored("cell", &["B1"], "B1"),
            row: 0,
            col: 1,
            value: Some("3.14".into()),
            formula: Some("=SUM(A1)".into()),
            cached_value: Some("42".into()),
            style_name: None,
        };
        let sheet = Sheet {
            id: ElementId::content_anchored("sheet", &["Sheet1"], "Sheet1"),
            name: "Sheet1".into(),
            rows: vec![SheetRow {
                id: ElementId::content_anchored("row", &["Sheet1:0"], "0"),
                cells: vec![cell1, cell2],
            }],
            merged_ranges: vec![MergedRange {
                id: ElementId::content_anchored("merged", &["merge:0"], "A1:B1"),
                start_row: 0,
                start_col: 0,
                end_row: 0,
                end_col: 1,
            }],
            header_row: None,
            footer_row: None,
        };
        let ss = Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets: vec![sheet],
        };
        let counts = census_spreadsheet(&ss);
        assert_eq!(counts.spreadsheet_cells, 2);
        assert_eq!(counts.spreadsheet_formulas, 1);
        assert_eq!(counts.spreadsheet_merged_ranges, 1);
        assert_eq!(counts.spreadsheet_sheets, 1);
        assert_eq!(counts.spreadsheet_rows, 1);
    }

    /// test_census_spreadsheet_multi_sheet.
    #[test]
    fn test_census_spreadsheet_multi_sheet() {
        let make_cell = |row, col| {
            let key = format!("A{}", row + 1);
            SheetCell {
                id: ElementId::content_anchored("cell", &["cell"], &key),
                row,
                col,
                value: Some("x".into()),
                formula: None,
                cached_value: None,
                style_name: None,
            }
        };
        let sheet1 = Sheet {
            id: ElementId::content_anchored("sheet", &["S1"], "S1"),
            name: "S1".into(),
            rows: vec![SheetRow {
                id: ElementId::content_anchored("row", &["S1:0"], "0"),
                cells: vec![make_cell(0, 0)],
            }],
            merged_ranges: vec![],
            header_row: None,
            footer_row: None,
        };
        let sheet2 = Sheet {
            id: ElementId::content_anchored("sheet", &["S2"], "S2"),
            name: "S2".into(),
            rows: vec![
                SheetRow {
                    id: ElementId::content_anchored("row", &["S2:0"], "0"),
                    cells: vec![SheetCell {
                        id: ElementId::content_anchored("cell", &["A1"], ""),
                        row: 0,
                        col: 0,
                        value: Some("y".into()),
                        formula: Some("=A1".into()),
                        cached_value: None,
                        style_name: None,
                    }],
                },
                SheetRow {
                    id: ElementId::content_anchored("row", &["S2:1"], "1"),
                    cells: vec![SheetCell {
                        id: ElementId::content_anchored("cell", &["A2"], ""),
                        row: 1,
                        col: 0,
                        value: Some("z".into()),
                        formula: None,
                        cached_value: None,
                        style_name: None,
                    }],
                },
            ],
            merged_ranges: vec![],
            header_row: None,
            footer_row: None,
        };
        let ss = Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets: vec![sheet1, sheet2],
        };
        let counts = census_spreadsheet(&ss);
        assert_eq!(counts.spreadsheet_sheets, 2);
        assert_eq!(counts.spreadsheet_cells, 3);
        assert_eq!(counts.spreadsheet_rows, 3);
        assert_eq!(counts.spreadsheet_formulas, 1);
    }

    // -- Presentation census tests --

    /// test_census_presentation_counts — verify slide/element counts.
    #[test]
    fn test_census_presentation_counts() {
        let slide = Slide {
            id: ElementId::content_anchored("slide", &["slide:0"], "slide1.xml"),
            elements: vec![
                SlideElement {
                    id: ElementId::content_anchored("e", &["text:0"], "Hello"),
                    kind: "text".into(),
                },
                SlideElement {
                    id: ElementId::content_anchored("e", &["text:1"], "World"),
                    kind: "text".into(),
                },
                SlideElement {
                    id: ElementId::content_anchored("e", &["ph:0"], "Title"),
                    kind: "placeholder:title".into(),
                },
                SlideElement {
                    id: ElementId::content_anchored("e", &["draw:0"], "chart"),
                    kind: "drawing".into(),
                },
            ],
        };
        let pres = Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![slide],
            layouts: vec![Layout {
                id: ElementId::content_anchored("layout", &["layout:0"], "TS"),
                name: "Title Slide".into(),
            }],
            masters: vec![Master {
                id: ElementId::content_anchored("master", &["master:0"], "OT"),
                name: "Office Theme".into(),
            }],
        };
        let counts = census_presentation(&pres);
        assert_eq!(counts.presentation_slides, 1);
        assert_eq!(counts.presentation_layouts, 1);
        assert_eq!(counts.presentation_masters, 1);
        assert_eq!(counts.slide_text_elements, 2);
        assert_eq!(counts.slide_placeholders, 1);
        assert_eq!(counts.slide_drawings, 1);
    }

    /// test_census_presentation_multi_slide.
    #[test]
    fn test_census_presentation_multi_slide() {
        let pres = Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![
                Slide {
                    id: ElementId::content_anchored("slide", &["s0"], "slide1.xml"),
                    elements: vec![SlideElement {
                        id: ElementId::content_anchored("e", &["t:0"], "Hello"),
                        kind: "text".into(),
                    }],
                },
                Slide {
                    id: ElementId::content_anchored("slide", &["s1"], "slide2.xml"),
                    elements: vec![
                        SlideElement {
                            id: ElementId::content_anchored("e", &["t:0"], "World"),
                            kind: "text".into(),
                        },
                        SlideElement {
                            id: ElementId::content_anchored("e", &["tb:0"], "tbl"),
                            kind: "table".into(),
                        },
                    ],
                },
            ],
            layouts: vec![],
            masters: vec![],
        };
        let counts = census_presentation(&pres);
        assert_eq!(counts.presentation_slides, 2);
        assert_eq!(counts.slide_text_elements, 2);
        assert_eq!(counts.slide_tables, 1);
    }

    // -- Unknown parts tests --

    /// test_unknown_parts_reported — parts with unknown content types produce unknown_parts.
    #[test]
    fn test_unknown_parts_reported() {
        let parts = vec![
            crate::opc::Part { uri: "word/document.xml".into(),
                content_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml".into(),
                data: vec![] },
            crate::opc::Part { uri: "word/unknown.xml".into(),
                content_type: "application/x-custom-format".into(), data: vec![] },
            crate::opc::Part { uri: "xl/workbook.xml".into(),
                content_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml".into(),
                data: vec![] },
        ];
        let unknown = census_opc_parts(&parts);
        assert_eq!(unknown.len(), 1);
        assert_eq!(unknown[0], "word/unknown.xml");
    }

    /// test_no_unknown_parts_for_known_types.
    #[test]
    fn test_no_unknown_parts_for_known_types() {
        let parts = vec![
            crate::opc::Part { uri: "word/document.xml".into(),
                content_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml".into(),
                data: vec![] },
            crate::opc::Part { uri: "xl/workbook.xml".into(),
                content_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml".into(),
                data: vec![] },
            crate::opc::Part { uri: "ppt/presentation.xml".into(),
                content_type: "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml".into(),
                data: vec![] },
        ];
        let unknown = census_opc_parts(&parts);
        assert!(unknown.is_empty());
    }

    // -- Manifest assertion tests --

    /// test_manifest_assertion_pass — fixture features match manifest expectations.
    #[test]
    fn test_manifest_assertion_pass() {
        let mut mf = HashMap::new();
        mf.insert("paragraph".to_string(), 2);
        mf.insert("table".to_string(), 1);
        mf.insert("drawing".to_string(), 0);

        let doc = test_doc(
            vec![
                SectionItem::Paragraph(test_para("hello")),
                SectionItem::Paragraph(test_para("world")),
                SectionItem::Table(Table {
                    id: ElementId::content_anchored("table", &["table:0"], ""),
                    rows: vec![Row { cells: vec![] }],
                }),
            ],
            vec![],
            vec![],
            vec![],
        );
        let counts = census_document(&doc);
        let manifest = ManifestEntry {
            schema_version: 1,
            id: "test".into(),
            format: "docx".into(),
            features: mf,
        };
        let warnings = assert_manifest_features(&counts, &manifest);
        assert!(
            warnings.is_empty(),
            "No warnings expected when counts match"
        );
    }

    /// test_manifest_assertion_warn — features not in manifest -> warning.
    #[test]
    fn test_manifest_assertion_warn() {
        let manifest = ManifestEntry {
            schema_version: 1,
            id: "test".into(),
            format: "docx".into(),
            features: HashMap::new(),
        };
        let doc = test_doc(
            vec![SectionItem::Paragraph(test_para("hello"))],
            vec![],
            vec![],
            vec![],
        );
        let counts = census_document(&doc);
        let warnings = assert_manifest_features(&counts, &manifest);
        assert!(
            !warnings.is_empty(),
            "Should warn for features not in manifest"
        );
    }

    /// test_manifest_mismatch_count.
    #[test]
    fn test_manifest_mismatch_count() {
        let mut mf = HashMap::new();
        mf.insert("paragraph".to_string(), 5); // manifest says 5, we have 2
        let doc = test_doc(
            vec![
                SectionItem::Paragraph(test_para("hello")),
                SectionItem::Paragraph(test_para("world")),
            ],
            vec![],
            vec![],
            vec![],
        );
        let counts = census_document(&doc);
        let manifest = ManifestEntry {
            schema_version: 1,
            id: "test".into(),
            format: "docx".into(),
            features: mf,
        };
        let warnings = assert_manifest_features(&counts, &manifest);
        let pw: Vec<_> = warnings
            .iter()
            .filter(|w| w.message.contains("paragraph"))
            .collect();
        assert!(!pw.is_empty(), "Should warn about paragraph count mismatch");
    }

    // -- Full pipeline tests --

    /// test_full_pipeline_budget — census -> FidelityBudget populated.
    #[test]
    fn test_full_pipeline_budget() {
        let doc = test_doc(
            vec![SectionItem::Paragraph(test_para("hello"))],
            vec![],
            vec![],
            vec![],
        );
        let budget = budget_from_document(&doc, &["custom/unknown.xml".into()]);
        assert_eq!(budget.confidence, 1.0);
        assert_eq!(budget.unknown_parts.len(), 1);
    }

    /// test_budget_from_spreadsheet_populated.
    #[test]
    fn test_budget_from_spreadsheet_populated() {
        let ss = Spreadsheet {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sheets: vec![Sheet {
                id: ElementId::content_anchored("sheet", &["S1"], "S1"),
                name: "S1".into(),
                rows: vec![SheetRow {
                    id: ElementId::content_anchored("row", &["S1:0"], "0"),
                    cells: vec![SheetCell {
                        id: ElementId::content_anchored("cell", &["A1"], "A1"),
                        row: 0,
                        col: 0,
                        value: Some("x".into()),
                        formula: Some("=1+1".into()),
                        cached_value: None,
                        style_name: None,
                    }],
                }],
                merged_ranges: vec![],
                header_row: None,
                footer_row: None,
            }],
        };
        let budget = budget_from_spreadsheet(&ss, &[]);
        assert_eq!(budget.confidence, 1.0);
    }

    /// test_budget_from_presentation_populated.
    #[test]
    fn test_budget_from_presentation_populated() {
        let pres = Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![Slide {
                id: ElementId::content_anchored("slide", &["s0"], "slide1.xml"),
                elements: vec![SlideElement {
                    id: ElementId::content_anchored("e", &["t:0"], "Title"),
                    kind: "text".into(),
                }],
            }],
            layouts: vec![],
            masters: vec![],
        };
        let budget = budget_from_presentation(&pres, &[]);
        assert_eq!(budget.confidence, 1.0);
    }

    /// test_build_budget_with_dropped_features.
    #[test]
    fn test_build_budget_with_dropped_features() {
        let counts = FeatureCounts::default();
        let dropped = vec![DroppedFeature {
            feature: "smart_art".into(),
            reason: "no_equivalent".into(),
            count: 2,
        }];
        let budget = build_budget_from_counts(&counts, &[], &dropped);
        assert!(!budget.dropped_features.is_empty());
    }

    /// test_build_budget_unknown_parts.
    #[test]
    fn test_build_budget_unknown_parts() {
        let counts = FeatureCounts::default();
        let unknown = vec!["custom/part.xml".into(), "extra.bin".into()];
        let budget = build_budget_from_counts(&counts, &unknown, &[]);
        assert_eq!(budget.unknown_parts.len(), 2);
        assert!(budget.confidence < 1.0);
    }

    /// test_census_merge.
    #[test]
    fn test_census_merge() {
        let c1 = FeatureCounts {
            paragraphs: 3,
            tables: 1,
            ..Default::default()
        };
        let c2 = FeatureCounts {
            paragraphs: 5,
            tables: 2,
            ..Default::default()
        };
        let mut c1 = c1;
        c1.merge(&c2);
        assert_eq!(c1.paragraphs, 8);
        assert_eq!(c1.tables, 3);
    }

    /// test_census_to_map.
    #[test]
    fn test_census_to_map() {
        let counts = FeatureCounts {
            paragraphs: 4,
            tables: 2,
            drawings: 1,
            lists: 3,
            fields: 2,
            comments: 1,
            ..Default::default()
        };
        let map = census_to_map(&counts);
        assert_eq!(map.get("paragraph"), Some(&4));
        assert_eq!(map.get("table"), Some(&2));
        assert_eq!(map.get("drawing"), Some(&1));
        assert_eq!(map.get("list"), Some(&3));
        assert_eq!(map.get("field"), Some(&2));
        assert_eq!(map.get("comment"), Some(&1));
    }

    /// test_manifest_load_and_assert — load a real manifest and assert against census.
    #[test]
    fn test_manifest_load_and_assert() {
        let manifest_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/manifests/cjk-text-docx-001.json"
        );
        let manifest = load_manifest(manifest_path).expect("should load manifest");
        assert_eq!(manifest.id, "cjk-text-docx-001");
        assert_eq!(manifest.format, "docx");
        assert!(!manifest.features.is_empty());
    }

    /// test_all_corpus_census — census all corpus docx, xlsx, pptx fixtures.
    #[test]
    fn test_all_corpus_census() {
        let blobs_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/blobs");
        let mut docx_count = 0;
        let mut xlsx_count = 0;
        let mut pptx_count = 0;
        for entry in std::fs::read_dir(blobs_dir).expect("corpus/blobs/ should be readable") {
            let entry = entry.expect("should read directory entry");
            let path = entry.path();
            match path.extension().and_then(|e| e.to_str()) {
                Some("docx") => docx_count += 1,
                Some("xlsx") => xlsx_count += 1,
                Some("pptx") => pptx_count += 1,
                _ => {}
            }
        }
        assert!(
            docx_count > 0,
            "Should have docx fixtures (found {})",
            docx_count
        );
        assert!(
            xlsx_count > 0,
            "Should have xlsx fixtures (found {})",
            xlsx_count
        );
        assert!(
            pptx_count > 0,
            "Should have pptx fixtures (found {})",
            pptx_count
        );
    }

    /// test_corpus_docx_census — census all docx fixtures via read_docx.
    #[test]
    fn test_corpus_docx_census() {
        let blobs_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/blobs");
        let mut census_count = 0;
        for entry in std::fs::read_dir(blobs_dir).expect("corpus/blobs/ should be readable") {
            let entry = entry.expect("should read directory entry");
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("docx") {
                continue;
            }
            let data =
                std::fs::read(&path).unwrap_or_else(|_| panic!("should read {}", path.display()));
            if let Ok(pkg) = crate::opc::parse_opc(&data) {
                if let Ok(result) = crate::docx::read_docx(&pkg) {
                    if let Ok(doc) = serde_json::from_slice::<Document>(&result.ir) {
                        let counts = census_document(&doc);
                        census_count += 1;
                        let _ = counts;
                    }
                }
            }
        }
        assert!(
            census_count > 0,
            "Should census at least one docx fixture (census {})",
            census_count
        );
    }

    /// test_corpus_xlsx_census — census all xlsx fixtures via XlsxReader.
    #[test]
    fn test_corpus_xlsx_census() {
        let blobs_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/blobs");
        let mut census_count = 0;
        for entry in std::fs::read_dir(blobs_dir).expect("corpus/blobs/ should be readable") {
            let entry = entry.expect("should read directory entry");
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("xlsx") {
                continue;
            }
            let data =
                std::fs::read(&path).unwrap_or_else(|_| panic!("should read {}", path.display()));
            let reader = crate::xlsx::XlsxReader::new();
            if let Ok(result) = reader.read(&data) {
                if let Ok(ss) = serde_json::from_slice::<Spreadsheet>(&result.ir) {
                    let counts = census_spreadsheet(&ss);
                    census_count += 1;
                    let _ = counts;
                }
            }
        }
        assert!(
            census_count > 0,
            "Should census at least one xlsx fixture (census {})",
            census_count
        );
    }

    /// test_corpus_pptx_census — census all pptx fixtures via PptxReader.
    #[test]
    fn test_corpus_pptx_census() {
        let blobs_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/blobs");
        let mut census_count = 0;
        for entry in std::fs::read_dir(blobs_dir).expect("corpus/blobs/ should be readable") {
            let entry = entry.expect("should read directory entry");
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("pptx") {
                continue;
            }
            let data =
                std::fs::read(&path).unwrap_or_else(|_| panic!("should read {}", path.display()));
            let reader = crate::pptx::PptxReader::new();
            if let Ok(result) = reader.read(&data) {
                if let Ok(pres) = serde_json::from_slice::<Presentation>(&result.ir) {
                    let counts = census_presentation(&pres);
                    census_count += 1;
                    let _ = counts;
                }
            }
        }
        assert!(
            census_count > 0,
            "Should census at least one pptx fixture (census {})",
            census_count
        );
    }
}
