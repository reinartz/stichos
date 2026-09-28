// SPDX-License-Identifier: MIT OR Apache-2.0
//! Pandoc AST → gp-core IR bridge.
//!
//! Converts a deserialised pandoc JSON document into a `gp_core::ir::Document`
//! suitable for downstream fidelity inspection and IR analysis.
//!
//! # Mapping
//!
//! | Pandoc block      | IR element        | Notes                          |
//! |-------------------|-------------------|--------------------------------|
//! | `Header`          | `Paragraph`       | level stored in `style_name`   |
//! | `Para`            | `SectionItem::Paragraph` | runs from inline       |
//! | `BulletList`      | `SectionItem::List` | `kind = Bullet`          |
//! | `OrderedList`     | `SectionItem::List` | `kind = Numbered`        |
//! | `CodeBlock`       | `SectionItem::Paragraph` | plain text run, style="Code" |
//! | `Table`           | `SectionItem::Table` | header row + body rows |
//! | `HorizontalRule`  | `SectionItem::Paragraph` | `---` run, style="HR" |
//! | `BlockQuote`      | `SectionItem::Paragraph` | nested, style="Quote" |
//! | `LineBlock`       | `SectionItem::Paragraph` | runs with `\n`       |
//!
//! # Inline mapping
//!
//! | Pandoc inline       | IR `Run` field          |
//! |---------------------|-------------------------|
//! | `Str`               | `text` (concatenated)   |
//! | `Emph`              | `italic = true`         |
//! | `Strong`            | `bold = true`           |
//! | `Link`              | text + `style_name`     |
//! | `Code`              | `style_name = "Code"`   |
//! | `Space` / `SoftBreak` | whitespace in text    |
//!
//! # Metadata
//!
//! The first `Header` at level 1 is extracted as the document title in
//! `Document.sections[0].headers`.
//!
//! # Error handling
//!
//! Conversion failures produce `Diagnostic` items (never panics). Unhandled
//! pandoc AST nodes produce warning diagnostics and are skipped.

use gp_core::diagnostics::{Diagnostic, Severity};
use gp_core::ir::*;

use crate::pandoc_ast::{self, PandocBlock, PandocDocument, PandocInline};

// ---------------------------------------------------------------------------
// Segment — intermediate representation for inline formatting
// ---------------------------------------------------------------------------

/// An intermediate text segment with formatting attributes.
///
/// Used by `collect_segments` to gather adjacent inline text with the same
/// formatting before merging into `Run` objects.
struct Segment {
    text: String,
    bold: bool,
    italic: bool,
    code: bool,
    url: Option<String>,
}

trait SegmentStyle {
    fn font_name(&self) -> Option<String>;
}

impl SegmentStyle for Segment {
    fn font_name(&self) -> Option<String> {
        if self.code {
            Some("Monospace".to_string())
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Convert a pandoc AST document into a gp-core IR document.
///
/// On error returns a non-empty list of diagnostics describing what failed.
pub fn pandoc_to_ir(doc: &PandocDocument) -> Result<IRDocument, Vec<Diagnostic>> {
    let mut warnings = Vec::new();
    let mut items = Vec::new();
    let mut headings = Vec::new();

    for block in &doc.blocks {
        match block {
            PandocBlock::BlankLine | PandocBlock::Div { .. } | PandocBlock::Figure { .. } => {
                warnings.push(Diagnostic::new(
                    "ENG-W-011",
                    Severity::Warning,
                    format!("skipped unhandled block: {}", block_tag(block)),
                ));
            }
            _ => {
                convert_block(block, &mut items, &mut headings, &mut warnings);
            }
        }
    }

    let document = Document {
        ir_version: IR_VERSION.to_string(),
        ir_version_migration: None,
        sections: vec![Section {
            id: ElementId::content_anchored("document", &["document"], IR_VERSION),
            kind: SectionKind::Body,
            items,
            headers: headings,
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

    if warnings.is_empty() {
        Ok(IRDocument {
            ir_version: IR_VERSION.to_string(),
            root: TaggedNode {
                id: ElementId::content_anchored("document", &["document"], IR_VERSION),
                tag: "document".to_string(),
                alt_text: None,
                bbox: None,
                children: vec![TaggedNode {
                    id: document.sections[0].id.clone(),
                    tag: "body".to_string(),
                    alt_text: None,
                    bbox: None,
                    children: document_to_nodes(&document, 0),
                }],
            },
        })
    } else {
        Err(warnings)
    }
}

// ---------------------------------------------------------------------------
// Block conversion
// ---------------------------------------------------------------------------

fn convert_block(
    block: &PandocBlock,
    items: &mut Vec<SectionItem>,
    headings: &mut Vec<Header>,
    _warnings: &mut Vec<Diagnostic>,
) {
    match block {
        PandocBlock::Para(inlines) => {
            items.push(SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored(
                    "paragraph",
                    &[&format!("para:{}", items.len())],
                    "",
                ),
                runs: convert_inlines(inlines),
                page_number: None,
                style_name: None,
                numbering_info: None,
            }));
        }
        PandocBlock::Plain(inlines) => {
            items.push(SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored(
                    "paragraph",
                    &[&format!("plain:{}", items.len())],
                    "",
                ),
                runs: convert_inlines(inlines),
                page_number: None,
                style_name: None,
                numbering_info: None,
            }));
        }
        PandocBlock::Header(level, _, content) => {
            if *level == 1 {
                headings.push(make_heading_header(level, content));
            } else {
                items.push(SectionItem::Paragraph(Paragraph {
                    id: ElementId::content_anchored(
                        "heading",
                        &[&format!("h{}:{}", level, items.len())],
                        "",
                    ),
                    runs: convert_inlines(content),
                    page_number: None,
                    style_name: Some(format!("Heading {}", level)),
                    numbering_info: None,
                }));
            }
        }
        PandocBlock::BulletList(items_p) => {
            items.push(SectionItem::List(List {
                id: ElementId::content_anchored("list", &[&format!("bullet:{}", items.len())], ""),
                kind: ListKind::Bullet,
                items: convert_list_items(items_p),
            }));
        }
        PandocBlock::OrderedList(_, _, items_p) => {
            items.push(SectionItem::List(List {
                id: ElementId::content_anchored("list", &[&format!("ordered:{}", items.len())], ""),
                kind: ListKind::Numbered,
                items: convert_list_items(items_p),
            }));
        }
        PandocBlock::CodeBlock(_, text, _) => {
            items.push(SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("code", &[&format!("code:{}", items.len())], ""),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["code-text"], ""),
                    text: text.clone(),
                    bold: None,
                    italic: None,
                    font_name: None,
                    font_size_pt: None,
                    underline: None,
                    strikethrough: None,
                    style_name: Some("Code".to_string()),
                }],
                page_number: None,
                style_name: Some("CodeBlock".to_string()),
                numbering_info: None,
            }));
        }
        PandocBlock::Table(_, content) => {
            let rows = convert_table_rows(content);
            items.push(SectionItem::Table(Table {
                id: ElementId::content_anchored("table", &[&format!("table:{}", items.len())], ""),
                rows,
            }));
        }
        PandocBlock::HorizontalRule => {
            items.push(SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("hr", &[&format!("hr:{}", items.len())], ""),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["hr-text"], ""),
                    text: "---".to_string(),
                    bold: None,
                    italic: None,
                    font_name: None,
                    font_size_pt: None,
                    underline: None,
                    strikethrough: None,
                    style_name: Some("HorizontalRule".to_string()),
                }],
                page_number: None,
                style_name: Some("HorizontalRule".to_string()),
                numbering_info: None,
            }));
        }
        PandocBlock::BlockQuote(inner) => {
            let mut all_inlines = Vec::new();
            for ib in inner {
                collect_inlines_from_block(ib, &mut all_inlines);
            }
            items.push(SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored(
                    "blockquote",
                    &[&format!("quote:{}", items.len())],
                    "",
                ),
                runs: convert_inlines(&all_inlines),
                page_number: None,
                style_name: Some("BlockQuote".to_string()),
                numbering_info: None,
            }));
        }
        PandocBlock::LineBlock(lines) => {
            let mut inlines = Vec::new();
            for (li, line) in lines.iter().enumerate() {
                if li > 0 {
                    inlines.push(PandocInline::SoftBreak);
                }
                inlines.extend(line.clone());
            }
            items.push(SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored(
                    "lineblock",
                    &[&format!("lineblock:{}", items.len())],
                    "",
                ),
                runs: convert_inlines(&inlines),
                page_number: None,
                style_name: Some("LineBlock".to_string()),
                numbering_info: None,
            }));
        }
        PandocBlock::RawBlock(_, text) => {
            items.push(SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("raw", &[&format!("raw:{}", items.len())], ""),
                runs: vec![Run {
                    id: ElementId::content_anchored("run", &["raw-text"], ""),
                    text: text.clone(),
                    bold: None,
                    italic: None,
                    font_name: None,
                    font_size_pt: None,
                    underline: None,
                    strikethrough: None,
                    style_name: Some("RawBlock".to_string()),
                }],
                page_number: None,
                style_name: Some("RawBlock".to_string()),
                numbering_info: None,
            }));
        }
        PandocBlock::Div(_, div_content) => {
            for ib in div_content {
                convert_block(ib, items, headings, _warnings);
            }
        }
        PandocBlock::Figure { .. } => {}
        PandocBlock::BlankLine => {}
    }
}

// ---------------------------------------------------------------------------
// Inline conversion
// ---------------------------------------------------------------------------

/// Convert a pandoc inline list to gp-core IR `Run` objects.
fn convert_inlines(inlines: &[PandocInline]) -> Vec<Run> {
    if inlines.is_empty() {
        return vec![];
    }

    let mut segments = Vec::new();
    collect_segments(inlines, &mut segments);

    let mut runs: Vec<Run> = Vec::new();
    for seg in segments {
        if seg.text.is_empty() {
            continue;
        }
        let is_link = seg.url.is_some();
        let style = if seg.code {
            Some("Code".to_string())
        } else if is_link {
            Some(format!(
                "Link:{}",
                seg.url.as_ref().map_or("", |u| u.as_str())
            ))
        } else {
            None
        };

        let last = runs.last_mut();
        match last {
            Some(last_run)
                if last_run.bold == Some(seg.bold)
                    && last_run.italic == Some(seg.italic)
                    && last_run.font_name == seg.font_name()
                    && last_run.style_name == style =>
            {
                last_run.text.push_str(&seg.text);
            }
            _ => {
                runs.push(Run {
                    id: ElementId::content_anchored(
                        "run",
                        &[&format!("run:{}", runs.len())],
                        &seg.text,
                    ),
                    text: seg.text.clone(),
                    bold: if seg.bold { Some(true) } else { None },
                    italic: if seg.italic { Some(true) } else { None },
                    font_name: seg.font_name(),
                    font_size_pt: None,
                    underline: None,
                    strikethrough: None,
                    style_name: style,
                });
            }
        }
    }

    runs
}

fn collect_segments(inlines: &[PandocInline], segments: &mut Vec<Segment>) {
    for inline in inlines {
        match inline {
            PandocInline::Str(c) => {
                if let Some(last) = segments.last_mut() {
                    last.text.push_str(c);
                } else {
                    segments.push(Segment {
                        text: c.clone(),
                        bold: false,
                        italic: false,
                        code: false,
                        url: None,
                    });
                }
            }
            PandocInline::Space => {
                if let Some(last) = segments.last_mut() {
                    last.text.push(' ');
                } else {
                    segments.push(Segment {
                        text: " ".to_string(),
                        bold: false,
                        italic: false,
                        code: false,
                        url: None,
                    });
                }
            }
            PandocInline::SoftBreak => {
                if let Some(last) = segments.last_mut() {
                    last.text.push('\n');
                } else {
                    segments.push(Segment {
                        text: "\n".to_string(),
                        bold: false,
                        italic: false,
                        code: false,
                        url: None,
                    });
                }
            }
            PandocInline::Emph(inner) => {
                collect_segments(inner, segments);
                if let Some(last) = segments.last_mut() {
                    last.italic = true;
                }
            }
            PandocInline::Strong(inner) => {
                collect_segments(inner, segments);
                if let Some(last) = segments.last_mut() {
                    last.bold = true;
                }
            }
            PandocInline::Strikethrough(inner) => {
                collect_segments(inner, segments);
            }
            PandocInline::Code(_, text) => {
                if let Some(last) = segments.last_mut() {
                    last.code = true;
                    last.text.push_str(text);
                } else {
                    segments.push(Segment {
                        text: text.clone(),
                        bold: false,
                        italic: false,
                        code: true,
                        url: None,
                    });
                }
            }
            PandocInline::Link(_, content, url) => {
                let text = pandoc_ast::inline_texts(content).join(" ");
                if let Some(last) = segments.last_mut() {
                    last.text.push_str(&text);
                    last.url = Some(url.clone());
                } else {
                    segments.push(Segment {
                        text,
                        bold: false,
                        italic: false,
                        code: false,
                        url: Some(url.clone()),
                    });
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// List conversion
// ---------------------------------------------------------------------------

fn convert_list_items(items_p: &[Vec<PandocBlock>]) -> Vec<ListItem> {
    items_p
        .iter()
        .map(|block_list| ListItem {
            id: ElementId::content_anchored("listitem", &[&format!("li:{}", block_list.len())], ""),
            content: convert_block_list_to_items(block_list),
            level: 0,
        })
        .collect()
}

fn convert_block_list_to_items(blocks: &[PandocBlock]) -> Vec<SectionItem> {
    let mut items = Vec::new();
    for block in blocks {
        match block {
            PandocBlock::Para(inlines) | PandocBlock::Plain(inlines) => {
                items.push(SectionItem::Paragraph(Paragraph {
                    id: ElementId::content_anchored("para", &["list-para"], ""),
                    runs: convert_inlines(inlines),
                    page_number: None,
                    style_name: None,
                    numbering_info: None,
                }));
            }
            PandocBlock::Header(level, _, content) => {
                items.push(SectionItem::Paragraph(Paragraph {
                    id: ElementId::content_anchored("para", &["list-heading"], ""),
                    runs: convert_inlines(content),
                    page_number: None,
                    style_name: Some(format!("Heading {}", level)),
                    numbering_info: None,
                }));
            }
            PandocBlock::BulletList(inner) => {
                items.push(SectionItem::List(List {
                    id: ElementId::content_anchored("list", &["list-list"], ""),
                    kind: ListKind::Bullet,
                    items: convert_list_items(inner),
                }));
            }
            PandocBlock::OrderedList(_, _, inner) => {
                items.push(SectionItem::List(List {
                    id: ElementId::content_anchored("list", &["ordered-list"], ""),
                    kind: ListKind::Numbered,
                    items: convert_list_items(inner),
                }));
            }
            PandocBlock::CodeBlock(_, text, _) => {
                items.push(SectionItem::Paragraph(Paragraph {
                    id: ElementId::content_anchored("code", &["list-code"], ""),
                    runs: vec![Run {
                        id: ElementId::content_anchored("run", &["list-code-text"], ""),
                        text: text.clone(),
                        bold: None,
                        italic: None,
                        font_name: None,
                        font_size_pt: None,
                        underline: None,
                        strikethrough: None,
                        style_name: Some("Code".to_string()),
                    }],
                    page_number: None,
                    style_name: Some("CodeBlock".to_string()),
                    numbering_info: None,
                }));
            }
            _ => {}
        }
    }
    items
}

// ---------------------------------------------------------------------------
// Table conversion
// ---------------------------------------------------------------------------

fn convert_table_rows(content: &crate::pandoc_ast::TableContent) -> Vec<Row> {
    let cell_blocks: Vec<Vec<PandocBlock>> = content.body_rows();
    // Flatten all cells into a single row.
    let cell_items: Vec<Cell> = cell_blocks
        .into_iter()
        .map(|block_list| {
            let contents = convert_block_list_to_items(&block_list);
            Cell {
                id: ElementId::content_anchored("cell", &["table-cell"], ""),
                contents,
                colspan: None,
                rowspan: None,
            }
        })
        .collect();
    vec![Row { cells: cell_items }]
}

// ---------------------------------------------------------------------------
// Inline collection helpers
// ---------------------------------------------------------------------------

fn collect_inlines_from_block(block: &PandocBlock, out: &mut Vec<PandocInline>) {
    match block {
        PandocBlock::Para(i) | PandocBlock::Plain(i) => {
            out.extend(i.clone());
        }
        PandocBlock::Header(_, _, content) => {
            out.extend(content.clone());
        }
        PandocBlock::BulletList(items) | PandocBlock::OrderedList(_, _, items) => {
            for item in items {
                collect_inlines_from_block_list(item, out);
            }
        }
        PandocBlock::CodeBlock(_, text, _) => {
            out.push(PandocInline::Str(text.clone()));
        }
        PandocBlock::LineBlock(lines) => {
            for line in lines {
                out.extend(line.clone());
            }
        }
        PandocBlock::BlockQuote(blocks) | PandocBlock::Div(_, blocks) => {
            for b in blocks {
                collect_inlines_from_block(b, out);
            }
        }
        _ => {}
    }
}

fn collect_inlines_from_block_list(blocks: &[PandocBlock], out: &mut Vec<PandocInline>) {
    for b in blocks {
        collect_inlines_from_block(b, out);
    }
}

// ---------------------------------------------------------------------------
// Heading header builder
// ---------------------------------------------------------------------------

fn make_heading_header(_level: &usize, content: &[PandocInline]) -> Header {
    Header {
        id: ElementId::content_anchored("heading", &["title"], ""),
        content: convert_block_to_section_items(content),
        is_even_page: false,
    }
}

fn convert_block_to_section_items(inlines: &[PandocInline]) -> Vec<SectionItem> {
    vec![SectionItem::Paragraph(Paragraph {
        id: ElementId::content_anchored("para", &["heading-text"], ""),
        runs: convert_inlines(inlines),
        page_number: None,
        style_name: Some(format!("Heading {}", 1)),
        numbering_info: None,
    })]
}

// ---------------------------------------------------------------------------
// IRDocument tree builder
// ---------------------------------------------------------------------------

fn document_to_nodes(doc: &Document, section_idx: usize) -> Vec<TaggedNode> {
    let section = &doc.sections[section_idx];
    section.items.iter().map(section_item_to_node).collect()
}

fn section_item_to_node(item: &SectionItem) -> TaggedNode {
    let tag = match item {
        SectionItem::Paragraph(_) => "paragraph",
        SectionItem::Table(_) => "table",
        SectionItem::Drawing(_) => "drawing",
        SectionItem::List(_) => "list",
    };
    TaggedNode {
        id: element_id_for_item(item),
        tag: tag.to_string(),
        alt_text: section_item_alt_text(item),
        bbox: None,
        children: section_item_children(item),
    }
}

fn element_id_for_item(item: &SectionItem) -> ElementId {
    match item {
        SectionItem::Paragraph(p) => p.id.clone(),
        SectionItem::Table(t) => t.id.clone(),
        SectionItem::Drawing(d) => d.id.clone(),
        SectionItem::List(l) => l.id.clone(),
    }
}

fn section_item_alt_text(item: &SectionItem) -> Option<String> {
    match item {
        SectionItem::Paragraph(p) => {
            let text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
            let prefix = match &p.style_name {
                Some(s) if s.starts_with("Heading") => format!("{}: ", s),
                _ => String::new(),
            };
            if text.is_empty() {
                None
            } else {
                Some(format!("{}{}", prefix, text))
            }
        }
        SectionItem::Table(t) => {
            let row_count = t.rows.len();
            let cell_count: usize = t.rows.iter().map(|r| r.cells.len()).sum();
            Some(format!("Table: {} rows, {} cells", row_count, cell_count))
        }
        SectionItem::Drawing(_) => Some("[drawing]".to_string()),
        SectionItem::List(l) => {
            let item_count = l.items.len();
            let kind = match l.kind {
                ListKind::Bullet => "bullet",
                ListKind::Numbered => "numbered",
                ListKind::Other => "other",
            };
            Some(format!("{} list: {} items", kind, item_count))
        }
    }
}

fn section_item_children(item: &SectionItem) -> Vec<TaggedNode> {
    match item {
        SectionItem::Paragraph(p) => p
            .runs
            .iter()
            .map(|r| TaggedNode {
                id: r.id.clone(),
                tag: "run".to_string(),
                alt_text: if r.text.is_empty() {
                    None
                } else {
                    Some(r.text.clone())
                },
                bbox: None,
                children: vec![],
            })
            .collect(),
        SectionItem::Table(t) => t
            .rows
            .iter()
            .map(|row| {
                let mut cell_children = Vec::new();
                for cell in &row.cells {
                    let mut inner_children = Vec::new();
                    for ci in &cell.contents {
                        inner_children.push(section_item_to_node(ci));
                    }
                    cell_children.push(TaggedNode {
                        id: cell.id.clone(),
                        tag: "cell".to_string(),
                        alt_text: cell_text(cell),
                        bbox: None,
                        children: inner_children,
                    });
                }
                TaggedNode {
                    id: ElementId::content_anchored("row", &["table-row"], ""),
                    tag: "row".to_string(),
                    alt_text: None,
                    bbox: None,
                    children: cell_children,
                }
            })
            .collect(),
        SectionItem::Drawing(_) => vec![],
        SectionItem::List(l) => l
            .items
            .iter()
            .map(|li| {
                let mut inner_children = Vec::new();
                for si in &li.content {
                    inner_children.push(section_item_to_node(si));
                }
                TaggedNode {
                    id: li.id.clone(),
                    tag: "list_item".to_string(),
                    alt_text: list_item_alt_text(li),
                    bbox: None,
                    children: inner_children,
                }
            })
            .collect(),
    }
}

fn cell_text(cell: &Cell) -> Option<String> {
    let mut text = String::new();
    for si in &cell.contents {
        if let SectionItem::Paragraph(p) = si {
            for r in &p.runs {
                text.push_str(&r.text);
            }
        }
    }
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn list_item_alt_text(item: &ListItem) -> Option<String> {
    let mut text = String::new();
    for si in &item.content {
        if let SectionItem::Paragraph(p) = si {
            for r in &p.runs {
                text.push_str(&r.text);
            }
        }
    }
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

// ---------------------------------------------------------------------------
// Utility
// ---------------------------------------------------------------------------

fn block_tag(block: &PandocBlock) -> &'static str {
    match block {
        PandocBlock::Plain(_) => "Plain",
        PandocBlock::Para(_) => "Para",
        PandocBlock::Header { .. } => "Header",
        PandocBlock::CodeBlock { .. } => "CodeBlock",
        PandocBlock::BulletList(_) => "BulletList",
        PandocBlock::OrderedList { .. } => "OrderedList",
        PandocBlock::LineBlock(_) => "LineBlock",
        PandocBlock::HorizontalRule => "HorizontalRule",
        PandocBlock::BlockQuote(_) => "BlockQuote",
        PandocBlock::Table { .. } => "Table",
        PandocBlock::RawBlock { .. } => "RawBlock",
        PandocBlock::Div { .. } => "Div",
        PandocBlock::Figure { .. } => "Figure",
        PandocBlock::BlankLine => "BlankLine",
    }
}

// ========================================================================
// Tests
// ========================================================================

// Tests are covered by integration tests via PandocEngine::to_json().
// See pandoc.rs tests: test_to_json_markdown, test_from_json_to_markdown, test_json_roundtrip.
