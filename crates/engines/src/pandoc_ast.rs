// SPDX-License-Identifier: MIT OR Apache-2.0
//! Pandoc JSON AST types — minimal deserialiser for pandoc's JSON AST format.
//!
//! Pandoc can render any supported input format as JSON via `--to=json`.
//! This module defines the minimal types needed to deserialise that JSON
//! and extract text, headings, lists, tables, and inline formatting.
//!
//! The JSON AST has a uniform structure:
//! - Top-level object with `pandoc-api-version`, `meta`, and `blocks` array
//! - Each block/inline is an object with `"t"` (type tag) and `"c"` (content)
//! - `"c"` is a serde_json::Value for flexibility — we only deserialise
//!   the fields we need at this time.
//!
//! See: <https://pandoc.org/lua-filters.html#data-types>

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ========================================================================
// Top-level document
// ========================================================================

/// A pandoc document in JSON AST form.
///
/// This is the root type produced by `pandoc --to=json`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PandocDocument {
    /// The pandoc API version, e.g. `[1,23,1,1]`.
    #[serde(default)]
    pub pandoc_api_version: Vec<Value>,
    /// Metadata dictionary (parsed when needed).
    #[serde(default)]
    pub meta: Value,
    /// The document blocks.
    pub blocks: Vec<PandocBlock>,
}

// ========================================================================
// Blocks
// ========================================================================

/// A single block-level element in pandoc's AST.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "t", content = "c")]
#[allow(clippy::large_enum_variant)]
pub enum PandocBlock {
    #[serde(rename = "Plain")]
    Plain(Vec<PandocInline>),
    #[serde(rename = "Para")]
    Para(Vec<PandocInline>),
    #[serde(rename = "Header")]
    Header(usize, Value, Vec<PandocInline>),
    #[serde(rename = "CodeBlock")]
    CodeBlock(Value, String, Value),
    #[serde(rename = "BulletList")]
    BulletList(Vec<Vec<PandocBlock>>),
    #[serde(rename = "OrderedList")]
    OrderedList(usize, Value, Vec<Vec<PandocBlock>>),
    #[serde(rename = "LineBlock")]
    LineBlock(Vec<Vec<PandocInline>>),
    #[serde(rename = "HorizontalRule")]
    HorizontalRule,
    #[serde(rename = "BlockQuote")]
    BlockQuote(Vec<PandocBlock>),
    #[serde(rename = "Table")]
    Table(Value, TableContent),
    #[serde(rename = "RawBlock")]
    RawBlock(String, String),
    #[serde(rename = "Div")]
    Div(Value, Vec<PandocBlock>),
    #[serde(rename = "Figure")]
    Figure(Value, Value),
    #[serde(rename = "BlankLine")]
    BlankLine,
}

/// Parsed table content — minimally captures header and body rows.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TableContent {
    #[serde(default)]
    header: Value,
    /// Table body: stored as raw value, parsed lazily.
    #[serde(default)]
    body: Value,
}

impl TableContent {
    /// Return the header inlines as a flat Vec<PandocInline>.
    pub(crate) fn header_inlines(&self) -> Option<Vec<PandocInline>> {
        let h = self.header.get("c")?;
        if h.is_string() {
            return Some(vec![PandocInline::Str(h.as_str()?.to_string())]);
        }
        h.get("c")
            .and_then(|c| serde_json::from_value::<Vec<PandocInline>>(c.clone()).ok())
    }

    /// Return the table body as Vec<Vec<Vec<PandocBlock>>> — rows of cells of blocks.
    pub(crate) fn body_rows(&self) -> Vec<Vec<PandocBlock>> {
        // Pandoc table body is nested: row_groups → rows → cells → content
        // We return a flat Vec<PandocBlock> per cell.
        // Since we just need cells (not rows), we flatten everything.
        let cells: Vec<Value> = if let Some(rows_val) = self.body.get("rows") {
            rows_val.as_array().cloned().unwrap_or_default()
        } else {
            self.body.as_array().cloned().unwrap_or_default()
        };
        cells
            .into_iter()
            .filter_map(|cell| {
                cell.get("content")
                    .and_then(|c| serde_json::from_value::<Vec<PandocBlock>>(c.clone()).ok())
            })
            .collect()
    }
}

// ========================================================================
// Inlines
// ========================================================================

/// A single inline-level element in pandoc's AST.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "t", content = "c")]
pub enum PandocInline {
    #[serde(rename = "Str")]
    Str(String),
    #[serde(rename = "Emph")]
    Emph(Vec<PandocInline>),
    #[serde(rename = "Strong")]
    Strong(Vec<PandocInline>),
    #[serde(rename = "Strikethrough")]
    Strikethrough(Vec<PandocInline>),
    #[serde(rename = "Superscript")]
    Superscript(Vec<PandocInline>),
    #[serde(rename = "Subscript")]
    Subscript(Vec<PandocInline>),
    #[serde(rename = "Quoted")]
    Quoted(String, Vec<PandocInline>),
    #[serde(rename = "Space")]
    Space,
    #[serde(rename = "SoftBreak")]
    SoftBreak,
    #[serde(rename = "Code")]
    Code(Value, String),
    #[serde(rename = "Math")]
    Math(String, String),
    #[serde(rename = "Cite")]
    Cite(Value, Vec<PandocInline>),
    #[serde(rename = "Link")]
    Link(Value, Vec<PandocInline>, String),
    #[serde(rename = "Image")]
    Image(Value, Vec<PandocInline>, String),
    #[serde(rename = "LineBreak")]
    LineBreak,
    #[serde(rename = "Note")]
    Note(Vec<PandocInline>),
    #[serde(rename = "Span")]
    Span(Value, Vec<PandocInline>),
    #[serde(rename = "RawInline")]
    RawInline(String, String),
    #[serde(rename = "InlineCode")]
    InlineCode(Value, String),
    #[serde(rename = "InlineMath")]
    InlineMath(String),
    #[serde(rename = "HtmlInline")]
    HtmlInline(String),
    #[serde(rename = "Null")]
    Null,
}

// ========================================================================
// Helper: collect all inline text from a block list
// ========================================================================

/// Recursively collect all inline text from a slice of blocks.
pub fn collect_block_texts(blocks: &[PandocBlock]) -> Vec<String> {
    blocks.iter().flat_map(block_texts).collect()
}

fn block_texts(block: &PandocBlock) -> Vec<String> {
    match block {
        PandocBlock::Plain(inlines) | PandocBlock::Para(inlines) => inline_texts(inlines),
        PandocBlock::Header(_, _, content) => inline_texts(content),
        PandocBlock::LineBlock(lines) => lines.iter().flat_map(|l| inline_texts(l)).collect(),
        PandocBlock::BulletList(items) | PandocBlock::OrderedList(_, _, items) => items
            .iter()
            .flat_map(|item: &Vec<PandocBlock>| collect_block_texts(item))
            .collect(),
        PandocBlock::BlockQuote(blocks) | PandocBlock::Div(_, blocks) => {
            collect_block_texts(blocks)
        }
        PandocBlock::CodeBlock(_, text, _) => vec![text.clone()],
        PandocBlock::Table(_, content) => {
            let mut texts = Vec::new();
            if let Some(hdr) = content.header_inlines() {
                texts.extend(inline_texts(&hdr));
            }
            for cell_blocks in content.body_rows() {
                texts.extend(collect_block_texts(&cell_blocks));
            }
            texts
        }
        _ => vec![],
    }
}

/// Recursively collect all inline text from a list of inline elements.
pub fn inline_texts(inlines: &[PandocInline]) -> Vec<String> {
    inlines.iter().flat_map(inline_text).collect()
}

fn inline_text(inline: &PandocInline) -> Vec<String> {
    match inline {
        PandocInline::Str(c) => vec![c.clone()],
        PandocInline::Emph(inner)
        | PandocInline::Strong(inner)
        | PandocInline::Strikethrough(inner)
        | PandocInline::Superscript(inner)
        | PandocInline::Subscript(inner)
        | PandocInline::Note(inner) => inline_texts(inner),
        PandocInline::Quoted(_, inner) => inline_texts(inner),
        PandocInline::Space => vec![" ".to_string()],
        PandocInline::SoftBreak => vec!["\n".to_string()],
        PandocInline::Code(_, text) => vec![text.clone()],
        PandocInline::Math(_, text) | PandocInline::InlineMath(text) => {
            vec![text.clone()]
        }
        PandocInline::Cite(_, content) => inline_texts(content),
        PandocInline::Link(_, content, _)
        | PandocInline::Span(_, content)
        | PandocInline::Image(_, content, _) => inline_texts(content),
        PandocInline::LineBreak | PandocInline::HtmlInline(_) | PandocInline::Null => {
            vec![]
        }
        PandocInline::RawInline(_, text) => vec![text.clone()],
        PandocInline::InlineCode(_, text) => vec![text.clone()],
    }
}

// ========================================================================
// Tests
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_document() {
        let json = r#"{
            "pandoc-api-version": [1,23,1,1],
            "meta": {},
            "blocks": [
                {"t": "Para", "c": [{"t": "Str", "c": "Hello World"}]}
            ]
        }"#;
        let doc: PandocDocument = serde_json::from_str(json).unwrap();
        assert_eq!(doc.blocks.len(), 1);
        match &doc.blocks[0] {
            PandocBlock::Para(inlines) => {
                assert_eq!(inlines.len(), 1);
                match &inlines[0] {
                    PandocInline::Str(c) => assert_eq!(c, "Hello World"),
                    _ => panic!("expected Str"),
                }
            }
            _ => panic!("expected Para"),
        }
    }

    #[test]
    fn test_parse_header_block() {
        let json = r#"{
            "blocks": [
                {"t": "Header", "c": [2, ["heading",[],[]], [{"t":"Str","c":"Title"}]]}
            ]
        }"#;
        let doc: PandocDocument = serde_json::from_str(json).unwrap();
        match &doc.blocks[0] {
            PandocBlock::Header(level, _, content) => {
                assert_eq!(*level, 2);
                assert_eq!(content.len(), 1);
            }
            _ => panic!("expected Header"),
        }
    }

    #[test]
    fn test_parse_bullet_list() {
        let json = r#"{
            "blocks": [
                {"t": "BulletList", "c": [[{"t":"Para","c":[{"t":"Str","c":"item1"}]}], [{"t":"Para","c":[{"t":"Str","c":"item2"}]}]]}
            ]
        }"#;
        let doc: PandocDocument = serde_json::from_str(json).unwrap();
        match &doc.blocks[0] {
            PandocBlock::BulletList(items) => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].len(), 1);
                assert_eq!(items[1].len(), 1);
            }
            _ => panic!("expected BulletList"),
        }
    }

    #[test]
    fn test_inline_texts_str() {
        let inlines = vec![
            PandocInline::Str("Hello".to_string()),
            PandocInline::Space,
            PandocInline::Str("World".to_string()),
        ];
        let texts = inline_texts(&inlines);
        assert_eq!(texts, vec!["Hello", " ", "World"]);
    }

    #[test]
    fn test_inline_texts_emph_strong() {
        let inlines = vec![
            PandocInline::Str("Plain".to_string()),
            PandocInline::Emph(vec![PandocInline::Str("Emphasized".to_string())]),
            PandocInline::Strong(vec![PandocInline::Str("Bold".to_string())]),
        ];
        let texts = inline_texts(&inlines);
        assert_eq!(texts, vec!["Plain", "Emphasized", "Bold"]);
    }

    #[test]
    fn test_inline_texts_link() {
        let inlines = vec![PandocInline::Link(
            Value::Null,
            vec![PandocInline::Str("click".to_string())],
            "https://example.com".to_string(),
        )];
        let texts = inline_texts(&inlines);
        assert_eq!(texts, vec!["click"]);
    }

    #[test]
    fn test_inline_texts_code() {
        let inlines = vec![
            PandocInline::Str("Use ".to_string()),
            PandocInline::Code(Value::Null, "fn main()".to_string()),
            PandocInline::Str(" here".to_string()),
        ];
        let texts = inline_texts(&inlines);
        assert_eq!(texts, vec!["Use ", "fn main()", " here"]);
    }

    #[test]
    fn test_inline_texts_softbreak_linebreak() {
        let inlines = vec![
            PandocInline::Str("line1".to_string()),
            PandocInline::SoftBreak,
            PandocInline::Str("line2".to_string()),
            PandocInline::LineBreak,
            PandocInline::Str("line3".to_string()),
        ];
        let texts = inline_texts(&inlines);
        assert_eq!(texts, vec!["line1", "\n", "line2", "line3"]);
    }

    #[test]
    fn test_collect_block_texts_para() {
        let blocks = vec![PandocBlock::Para(vec![
            PandocInline::Str("Hello".to_string()),
            PandocInline::Space,
            PandocInline::Str("World".to_string()),
        ])];
        let texts = collect_block_texts(&blocks);
        assert_eq!(texts, vec!["Hello", " ", "World"]);
    }

    #[test]
    fn test_collect_block_texts_codeblock() {
        let blocks = vec![PandocBlock::CodeBlock(
            Value::Null,
            "fn main() {}".to_string(),
            Value::Null,
        )];
        let texts = collect_block_texts(&blocks);
        assert_eq!(texts, vec!["fn main() {}"]);
    }

    #[test]
    fn test_table_content_body_rows_empty() {
        let json = r#"{"header": {}, "body": {}}"#;
        let tc: TableContent = serde_json::from_str(json).unwrap();
        let rows = tc.body_rows();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_parse_ordered_list() {
        let json = r#"{
            "blocks": [
                {"t": "OrderedList", "c": [1, {}, [[{"t":"Para","c":[{"t":"Str","c":"first"}]}], [{"t":"Para","c":[{"t":"Str","c":"second"}]}]]]}
            ]
        }"#;
        let doc: PandocDocument = serde_json::from_str(json).unwrap();
        match &doc.blocks[0] {
            PandocBlock::OrderedList(number, _, items) => {
                assert_eq!(*number, 1);
                assert_eq!(items.len(), 2);
            }
            _ => panic!("expected OrderedList"),
        }
    }

    #[test]
    fn test_parse_horizontal_rule() {
        let json = r#"{"blocks": [{"t": "HorizontalRule"}]}"#;
        let doc: PandocDocument = serde_json::from_str(json).unwrap();
        match &doc.blocks[0] {
            PandocBlock::HorizontalRule => {}
            _ => panic!("expected HorizontalRule"),
        }
    }

    #[test]
    fn test_parse_block_quote() {
        let json = r#"{
            "blocks": [
                {"t": "BlockQuote", "c": [{"t": "Para", "c": [{"t": "Str", "c": "quoted text"}]}]}
            ]
        }"#;
        let doc: PandocDocument = serde_json::from_str(json).unwrap();
        match &doc.blocks[0] {
            PandocBlock::BlockQuote(inner) => {
                assert_eq!(inner.len(), 1);
            }
            _ => panic!("expected BlockQuote"),
        }
    }

    #[test]
    fn test_parse_line_block() {
        let json = r#"{
            "blocks": [
                {"t": "LineBlock", "c": [[{"t":"Str","c":"line1"}], [{"t":"Str","c":"line2"}]]}
            ]
        }"#;
        let doc: PandocDocument = serde_json::from_str(json).unwrap();
        match &doc.blocks[0] {
            PandocBlock::LineBlock(lines) => {
                assert_eq!(lines.len(), 2);
                assert_eq!(lines[0].len(), 1);
            }
            _ => panic!("expected LineBlock"),
        }
    }

    #[test]
    fn test_parse_div() {
        let json = r#"{
            "blocks": [
                {"t": "Div", "c": [[], [{"t": "Para", "c": [{"t": "Str", "c": "div content"}]}]]}
            ]
        }"#;
        let doc: PandocDocument = serde_json::from_str(json).unwrap();
        match &doc.blocks[0] {
            PandocBlock::Div(_, blocks) => {
                assert_eq!(blocks.len(), 1);
            }
            _ => panic!("expected Div"),
        }
    }

    #[test]
    fn test_inline_texts_quoted() {
        let inlines = vec![PandocInline::Quoted(
            "Double".to_string(),
            vec![PandocInline::Str("quoted".to_string())],
        )];
        let texts = inline_texts(&inlines);
        assert_eq!(texts, vec!["quoted"]);
    }

    #[test]
    fn test_inline_texts_cite() {
        let inlines = vec![PandocInline::Cite(
            Value::Null,
            vec![PandocInline::Str("author ".to_string())],
        )];
        let texts = inline_texts(&inlines);
        assert_eq!(texts, vec!["author "]);
    }

    #[test]
    fn test_inline_texts_math() {
        let inlines = vec![PandocInline::Math(
            "DisplayMath".to_string(),
            "E=mc^2".to_string(),
        )];
        let texts = inline_texts(&inlines);
        assert_eq!(texts, vec!["E=mc^2"]);
    }

    #[test]
    fn test_inline_texts_superscript() {
        let inlines = vec![PandocInline::Superscript(vec![PandocInline::Str(
            "x".to_string(),
        )])];
        let texts = inline_texts(&inlines);
        assert_eq!(texts, vec!["x"]);
    }

    #[test]
    fn test_inline_texts_subscript() {
        let inlines = vec![PandocInline::Subscript(vec![PandocInline::Str(
            "n".to_string(),
        )])];
        let texts = inline_texts(&inlines);
        assert_eq!(texts, vec!["n"]);
    }

    #[test]
    fn test_inline_texts_note() {
        let inlines = vec![PandocInline::Note(vec![PandocInline::Str(
            "note".to_string(),
        )])];
        let texts = inline_texts(&inlines);
        assert_eq!(texts, vec!["note"]);
    }

    #[test]
    fn test_inline_texts_span() {
        let inlines = vec![PandocInline::Span(
            Value::Null,
            vec![PandocInline::Str("span".to_string())],
        )];
        let texts = inline_texts(&inlines);
        assert_eq!(texts, vec!["span"]);
    }

    #[test]
    fn test_inline_texts_raw_inline() {
        let inlines = vec![PandocInline::RawInline(
            "html".to_string(),
            "<em>raw</em>".to_string(),
        )];
        let texts = inline_texts(&inlines);
        assert_eq!(texts, vec!["<em>raw</em>"]);
    }

    #[test]
    fn test_inline_texts_inline_code() {
        let inlines = vec![PandocInline::InlineCode(Value::Null, "foo()".to_string())];
        let texts = inline_texts(&inlines);
        assert_eq!(texts, vec!["foo()"]);
    }

    #[test]
    fn test_inline_texts_inline_math() {
        let inlines = vec![PandocInline::InlineMath("x+y".to_string())];
        let texts = inline_texts(&inlines);
        assert_eq!(texts, vec!["x+y"]);
    }

    #[test]
    fn test_inline_texts_html_inline() {
        let inlines: Vec<PandocInline> = vec![PandocInline::HtmlInline("<br/>".to_string())];
        let texts = inline_texts(&inlines);
        assert!(texts.is_empty());
    }

    #[test]
    fn test_inline_texts_null() {
        let inlines: Vec<PandocInline> = vec![PandocInline::Null];
        let texts = inline_texts(&inlines);
        assert!(texts.is_empty());
    }

    #[test]
    fn test_inline_texts_mixed_formatting() {
        let inlines = vec![
            PandocInline::Str("Hello ".to_string()),
            PandocInline::Emph(vec![PandocInline::Str("world".to_string())]),
            PandocInline::Space,
            PandocInline::Strong(vec![PandocInline::Str("bold".to_string())]),
        ];
        let texts = inline_texts(&inlines);
        assert_eq!(texts, vec!["Hello ", "world", " ", "bold"]);
    }

    #[test]
    fn test_collect_block_texts_nested_list() {
        let blocks = vec![PandocBlock::BulletList(vec![vec![PandocBlock::Para(
            vec![PandocInline::Str("item".to_string())],
        )]])];
        let texts = collect_block_texts(&blocks);
        assert_eq!(texts, vec!["item"]);
    }

    #[test]
    fn test_parse_blank_line() {
        let json = r#"{"blocks": [{"t": "BlankLine"}]}"#;
        let doc: PandocDocument = serde_json::from_str(json).unwrap();
        match &doc.blocks[0] {
            PandocBlock::BlankLine => {}
            _ => panic!("expected BlankLine"),
        }
    }

    #[test]
    fn test_parse_raw_block() {
        let json = r#"{
            "blocks": [
                {"t": "RawBlock", "c": ["html", "<p>raw</p>"]}
            ]
        }"#;
        let doc: PandocDocument = serde_json::from_str(json).unwrap();
        match &doc.blocks[0] {
            PandocBlock::RawBlock(format, text) => {
                assert_eq!(format, "html");
                assert_eq!(text, "<p>raw</p>");
            }
            _ => panic!("expected RawBlock"),
        }
    }
}
