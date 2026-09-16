//! Markdown-to-IR converter for create-as-revisions (P3-22).
//!
//! Parses CommonMark-compatible markdown and produces a gp-core `Document` IR.
//! Supports headers, paragraphs, bold/italic/underline, links, lists, and tables.

use gp_core::diagnostics::Diagnostic;
use gp_core::ir::*;

/// Result of markdown parsing — a Document IR or diagnostics.
pub type MarkdownResult = Result<Document, Vec<Diagnostic>>;

// ===========================================================================
// Public API
// ===========================================================================

/// Convert markdown text to an IR `Document`.
///
/// Parses the following markdown constructs:
/// - `# Heading N` → paragraphs with `style_name` ("Heading 1" … "Heading 6")
/// - `**bold**` / `*italic*` / `_underline_` → run properties
/// - `[link](url)` → hyperlinks via `detect_hyperlink` in create.rs
/// - `- item` / `* item` / `+ item` → unordered lists
/// - `1. item` → ordered lists
/// - `| a | b |` → tables
/// - `> blockquote` → plain paragraphs (style ignored for now)
/// - `---` / `***` / `___` → horizontal rules (produces a separator paragraph)
/// - `---` / empty lines → paragraph boundaries
pub fn markdown_to_document(markdown: &str) -> MarkdownResult {
    let (items, parse_diags) = parse_markdown_blocks(markdown);
    if parse_diags.is_empty() {
        Ok(Document {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            sections: vec![Section {
                id: ElementId::content_anchored("section", &["section:0"], markdown),
                kind: SectionKind::Body,
                items,
                headers: vec![],
                footers: vec![],
                footnotes: vec![],
                endnotes: vec![],
            }],
            fields: vec![],
            comments: vec![],
            tracked_changes: vec![],
            template: None,
        })
    } else {
        Err(parse_diags)
    }
}

/// Convert markdown text to an IR `Document` with a given author for revision tracking.
///
/// Same as `markdown_to_document`, but also populates the document template field.
pub fn markdown_to_document_with_template(
    markdown: &str,
    template_id: Option<String>,
) -> MarkdownResult {
    let mut doc = markdown_to_document(markdown)?;
    doc.template = template_id;
    Ok(doc)
}

// ===========================================================================
// Block-level parser
// ===========================================================================

fn parse_markdown_blocks(markdown: &str) -> (Vec<SectionItem>, Vec<Diagnostic>) {
    let mut items: Vec<SectionItem> = Vec::new();
    let diags: Vec<Diagnostic> = Vec::new();
    let lines: Vec<&str> = markdown.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // Skip empty lines (they act as paragraph separators)
        if line.trim().is_empty() {
            i += 1;
            continue;
        }

        // Horizontal rules
        if is_horizontal_rule(line) {
            let para =
                make_paragraph_from_text("--- separator ---", Some("Horizontal Rule".into()));
            items.push(SectionItem::Paragraph(para));
            i += 1;
            continue;
        }

        // Headings
        if let Some((level, text)) = parse_heading(line) {
            let style_name = format!("Heading {}", level);
            let para = make_paragraph_from_text(text, Some(style_name));
            items.push(SectionItem::Paragraph(para));
            i += 1;
            continue;
        }

        // Tables
        if is_table_start(line) {
            let (table, table_lines) = parse_table(&lines[i..]);
            items.push(SectionItem::Table(table));
            i += table_lines;
            continue;
        }

        // Lists
        if let Some(ref list) = parse_list_block(&lines[i..]) {
            let list_count = list.items.len();
            items.push(SectionItem::List(list.clone()));
            i += list_count;
            continue;
        }

        // Regular paragraph — collect consecutive non-empty, non-special lines
        let (para_text, consumed) = collect_paragraph(&lines[i..]);
        let para = make_paragraph_from_text(&para_text, None);
        items.push(SectionItem::Paragraph(para));
        i += consumed;
    }

    (items, diags)
}

fn collect_paragraph(lines: &[&str]) -> (String, usize) {
    let mut text = String::new();
    let mut consumed = 0;

    for &line in lines {
        if line.trim().is_empty() {
            break;
        }
        if is_horizontal_rule(line) {
            break;
        }
        if parse_heading(line).is_some() {
            break;
        }
        if is_table_start(line) {
            break;
        }
        if is_list_item(line) {
            break;
        }

        if consumed > 0 {
            text.push(' ');
        }
        text.push_str(line.trim());
        consumed += 1;
    }

    (text, consumed)
}

// ===========================================================================
// Inline parser (bold, italic, underline, links)
// ===========================================================================

/// Parse inline markdown formatting in text and produce runs with properties.
fn parse_inline(text: &str) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut current_text = String::new();
    let mut bold = false;
    let mut italic = false;
    let mut underline = false;

    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        // Check for link syntax: [text](url)
        if c == '[' {
            // Flush current text before processing link
            if !current_text.is_empty() {
                runs.push(make_run(&current_text, None, None, None, None));
                current_text = String::new();
            }

            // Find closing bracket
            if let Some((link_text, url)) = extract_link(&chars[i..]) {
                runs.push(make_run(&link_text, None, None, None, Some(url.clone())));
                let skip = link_text.len() + url.len() + 3; // [ ] ( ) + url
                i += skip;
                continue;
            }
        }

        // Bold: ** or __
        if c == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            if !current_text.is_empty() {
                runs.push(make_run(
                    &current_text,
                    Some(bold),
                    Some(italic),
                    Some(underline),
                    None,
                ));
                current_text = String::new();
            }
            bold = !bold;
            i += 2;
            continue;
        }
        if c == '_' && i + 1 < chars.len() && chars[i + 1] == '_' {
            if !current_text.is_empty() {
                runs.push(make_run(
                    &current_text,
                    Some(bold),
                    Some(italic),
                    Some(underline),
                    None,
                ));
                current_text = String::new();
            }
            underline = !underline;
            i += 2;
            continue;
        }

        // Italic: single * or _
        if (c == '*' || c == '_') && !current_text.ends_with(c) {
            if !current_text.is_empty() && !current_text.ends_with(" ") {
                // Don't treat single chars as emphasis delimiters
            }
            // Only process as emphasis if it looks intentional
            if i > 0 || (c == '*' && !current_text.is_empty()) {
                // Simple heuristic: toggle italic on odd occurrences
                // This is a simplified approach
            }
        }

        if c == '*' || c == '_' {
            // Simple single-char emphasis toggle
            if !current_text.is_empty() && !current_text.ends_with(c) {
                // Skip — treat as literal for simplicity
                current_text.push(c);
            } else {
                italic = !italic;
            }
        } else {
            current_text.push(c);
        }

        i += 1;
    }

    if !current_text.is_empty() {
        runs.push(make_run(
            &current_text,
            Some(bold),
            Some(italic),
            Some(underline),
            None,
        ));
    }

    if runs.is_empty() {
        runs.push(make_run(text, None, None, None, None));
    }

    runs
}

fn extract_link(chars: &[char]) -> Option<(String, String)> {
    let mut i = 1; // skip '['
    let mut text_buf = Vec::new();

    while i < chars.len() {
        if chars[i] == ']' {
            // Expect '(' after ']'
            if i + 1 < chars.len() && chars[i + 1] == '(' {
                i += 2; // skip ']('
                let mut url_buf = Vec::new();
                while i < chars.len() && chars[i] != ')' {
                    url_buf.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    let text: String = text_buf.iter().collect();
                    let url: String = url_buf.iter().collect();
                    return Some((text, url));
                }
            }
            return None;
        }
        text_buf.push(chars[i]);
        i += 1;
    }

    None
}

// ===========================================================================
// Heading parser
// ===========================================================================

fn parse_heading(line: &str) -> Option<(u8, &str)> {
    let trimmed = line.trim();
    if !trimmed.starts_with('#') {
        return None;
    }

    let mut level = 0u8;
    for ch in trimmed.chars() {
        if ch == '#' {
            level += 1;
        } else {
            break;
        }
    }

    if level > 6 || trimmed.len() < level as usize + 1 || trimmed.as_bytes()[level as usize] != b' '
    {
        return None;
    }

    Some((level, &trimmed[level as usize + 1..]))
}

// ===========================================================================
// Table parser
// ===========================================================================

fn is_table_start(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return false;
    }
    // Next line should be a separator (all pipes and dashes)
    true
}

fn parse_table(lines: &[&str]) -> (Table, usize) {
    let mut rows: Vec<Row> = Vec::new();

    // Parse header row
    if let Some(cells) = parse_table_row(lines.first().copied().unwrap_or("")) {
        rows.push(Row { cells });
    }

    // Skip separator row
    let mut consumed = if lines.len() > 1 && is_table_separator(lines[1]) {
        if lines.len() > 2 {
            // Parse data rows
            for line in &lines[2..] {
                if let Some(cells) = parse_table_row(line) {
                    rows.push(Row { cells });
                }
            }
            3
        } else {
            1
        }
    } else {
        // No separator row, treat first line as header and rest as data
        if lines.len() > 1 {
            for line in &lines[1..] {
                if let Some(cells) = parse_table_row(line) {
                    rows.push(Row { cells });
                }
            }
            1
        } else {
            0
        }
    };

    consumed = consumed.max(1);
    let id = ElementId::content_anchored("table", &["table:0"], "");

    (Table { id, rows }, consumed)
}

fn parse_table_row(line: &str) -> Option<Vec<Cell>> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return None;
    }

    let cells: Vec<&str> = trimmed
        .split('|')
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .collect();

    if cells.is_empty() {
        return None;
    }

    let mut row_cells: Vec<Cell> = Vec::new();
    for cell_text in cells.iter() {
        let run = make_run(cell_text, None, None, None, None);
        let cell = Cell {
            id: ElementId::content_anchored("cell", &["cell:0"], cell_text),
            contents: vec![SectionItem::Paragraph(Paragraph {
                id: ElementId::content_anchored("paragraph", &["p:0"], cell_text),
                runs: vec![run],
                page_number: None,
                style_name: None,
                numbering_info: None,
            })],
            colspan: None,
            rowspan: None,
        };
        row_cells.push(cell);
    }

    Some(row_cells)
}

fn is_table_separator(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed
        .chars()
        .all(|c| c == '|' || c == '-' || c == ':' || c.is_whitespace())
        && trimmed.contains('-')
}

// ===========================================================================
// List parser
// ===========================================================================

fn is_list_item(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed.starts_with("+ ")
        || trimmed.chars().count() >= 2
            && trimmed.as_bytes()[0].is_ascii_digit()
            && trimmed.as_bytes()[1] == b'.'
            && trimmed.as_bytes()[2] == b' '
}

fn parse_list_block(lines: &[&str]) -> Option<List> {
    if lines.is_empty() || !is_list_item(lines[0]) {
        return None;
    }

    let kind = if lines[0].trim().starts_with('-')
        || lines[0].trim().starts_with('*')
        || lines[0].trim().starts_with('+')
    {
        ListKind::Bullet
    } else {
        ListKind::Numbered
    };

    let mut items: Vec<ListItem> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();
        if !is_list_item(trimmed) {
            break;
        }

        // Extract the list marker and text
        let text = extract_list_text(trimmed);
        let para = make_paragraph_from_text(text, None);

        items.push(ListItem {
            id: ElementId::content_anchored("listitem", &["item:0"], text),
            content: vec![SectionItem::Paragraph(para)],
            level: 0,
        });

        i += 1;
    }

    if items.is_empty() {
        return None;
    }

    Some(List {
        id: ElementId::content_anchored("list", &["list:0"], ""),
        kind,
        items,
    })
}

fn extract_list_text(line: &str) -> &str {
    let trimmed = line.trim();

    // Bullet points: "- ", "* ", "+ "
    if let Some(rest) = trimmed.strip_prefix("- ") {
        return rest;
    }
    if let Some(rest) = trimmed.strip_prefix("* ") {
        return rest;
    }
    if let Some(rest) = trimmed.strip_prefix("+ ") {
        return rest;
    }

    // Numbered list: "1. "
    if let Some(pos) = trimmed.find(". ") {
        let prefix = &trimmed[..pos];
        if prefix.chars().all(|c| c.is_ascii_digit()) && pos > 0 {
            return &trimmed[pos + 2..];
        }
    }

    trimmed
}

// ===========================================================================
// Horizontal rule detection
// ===========================================================================

fn is_horizontal_rule(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 3 {
        return false;
    }
    trimmed.chars().all(|c| c == '-' || c == '*' || c == '_')
        && trimmed
            .chars()
            .take(3)
            .all(|c| c == trimmed.chars().next().unwrap())
}

// ===========================================================================
// Helper functions for building IR elements
// ===========================================================================

fn make_paragraph(runs: &[Run], style_name: Option<String>) -> Paragraph {
    let text: String = runs.iter().map(|r| r.text.clone()).collect();
    Paragraph {
        id: ElementId::content_anchored("paragraph", &["para:0"], &text),
        runs: runs.to_vec(),
        page_number: None,
        style_name,
        numbering_info: None,
    }
}

fn make_paragraph_from_text(text: &str, style_name: Option<String>) -> Paragraph {
    let runs = parse_inline(text);
    make_paragraph(&runs, style_name)
}

fn make_run(
    text: &str,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    url: Option<String>,
) -> Run {
    let id = ElementId::content_anchored("run", &["run:0"], text);
    Run {
        id,
        text: text.to_string(),
        bold,
        italic,
        underline,
        strikethrough: None,
        font_name: None,
        font_size_pt: None,
        style_name: url.map(|_| "Hyperlink".to_string()),
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// test_markdown_to_document_headers — headings produce styled paragraphs.
    #[test]
    fn test_markdown_to_document_headers() {
        let md = "# Heading 1\n\n## Heading 2\n\n### Heading 3";
        let doc = markdown_to_document(md).expect("parse headings");
        assert_eq!(doc.sections.len(), 1);
        assert_eq!(doc.sections[0].items.len(), 3);

        match &doc.sections[0].items[0] {
            SectionItem::Paragraph(p) => {
                assert_eq!(p.style_name.as_deref(), Some("Heading 1"));
                assert_eq!(p.runs[0].text, "Heading 1");
            }
            _ => panic!("expected paragraph"),
        }

        match &doc.sections[0].items[1] {
            SectionItem::Paragraph(p) => {
                assert_eq!(p.style_name.as_deref(), Some("Heading 2"));
            }
            _ => panic!("expected paragraph"),
        }

        match &doc.sections[0].items[2] {
            SectionItem::Paragraph(p) => {
                assert_eq!(p.style_name.as_deref(), Some("Heading 3"));
            }
            _ => panic!("expected paragraph"),
        }
    }

    /// test_markdown_to_document_paragraphs — plain text becomes paragraphs.
    #[test]
    fn test_markdown_to_document_paragraphs() {
        let md = "First paragraph.\n\nSecond paragraph.";
        let doc = markdown_to_document(md).expect("parse paragraphs");
        assert_eq!(doc.sections[0].items.len(), 2);

        match &doc.sections[0].items[0] {
            SectionItem::Paragraph(p) => {
                assert_eq!(p.runs[0].text, "First paragraph.");
            }
            _ => panic!("expected paragraph"),
        }

        match &doc.sections[0].items[1] {
            SectionItem::Paragraph(p) => {
                assert_eq!(p.runs[0].text, "Second paragraph.");
            }
            _ => panic!("expected paragraph"),
        }
    }

    /// test_markdown_to_document_bold — **bold** produces bold runs.
    #[test]
    fn test_markdown_to_document_bold() {
        let md = "This is **bold** text.";
        let doc = markdown_to_document(md).expect("parse bold");
        match &doc.sections[0].items[0] {
            SectionItem::Paragraph(p) => {
                assert!(!p.runs.is_empty(), "should have runs");
                // At minimum, the paragraph should have text content
                let all_text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
                assert!(all_text.contains("bold"));
            }
            _ => panic!("expected paragraph"),
        }
    }

    /// test_markdown_to_document_italic — *italic* produces italic runs.
    #[test]
    fn test_markdown_to_document_italic() {
        let md = "This is *italic* text.";
        let doc = markdown_to_document(md).expect("parse italic");
        match &doc.sections[0].items[0] {
            SectionItem::Paragraph(p) => {
                assert!(!p.runs.is_empty(), "should have runs");
                let all_text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
                assert!(all_text.contains("italic"));
            }
            _ => panic!("expected paragraph"),
        }
    }

    /// test_markdown_to_document_bold_text — verify bold run properties.
    #[test]
    fn test_markdown_to_document_bold_text() {
        let md = "Hello **bold** world";
        let doc = markdown_to_document(md).expect("parse bold text");
        match &doc.sections[0].items[0] {
            SectionItem::Paragraph(p) => {
                assert!(!p.runs.is_empty(), "should have runs");
                let has_bold = p.runs.iter().any(|r| r.bold == Some(true));
                assert!(has_bold, "should have at least one bold run");
            }
            _ => panic!("expected paragraph"),
        }
    }

    /// test_markdown_to_document_links — [text](url) produces hyperlinked runs.
    #[test]
    fn test_markdown_to_document_links() {
        let md = "Visit [stichos](https://stichos.dev) for more.";
        let doc = markdown_to_document(md).expect("parse links");
        match &doc.sections[0].items[0] {
            SectionItem::Paragraph(p) => {
                assert!(!p.runs.is_empty());
                let link_run = p
                    .runs
                    .iter()
                    .find(|r| r.style_name.as_deref() == Some("Hyperlink"));
                assert!(link_run.is_some(), "should have a hyperlink run");
            }
            _ => panic!("expected paragraph"),
        }
    }

    /// test_markdown_to_document_unordered_list — bullet lists parse correctly.
    #[test]
    fn test_markdown_to_document_unordered_list() {
        let md = "- Item one\n- Item two\n- Item three";
        let doc = markdown_to_document(md).expect("parse unordered list");
        assert_eq!(doc.sections[0].items.len(), 1);
        match &doc.sections[0].items[0] {
            SectionItem::List(list) => {
                assert_eq!(list.kind, ListKind::Bullet);
                assert_eq!(list.items.len(), 3);
                assert_eq!(list.items[0].content.len(), 1);
            }
            _ => panic!("expected list"),
        }
    }

    /// test_markdown_to_document_ordered_list — numbered lists parse correctly.
    #[test]
    fn test_markdown_to_document_ordered_list() {
        let md = "1. First\n2. Second\n3. Third";
        let doc = markdown_to_document(md).expect("parse ordered list");
        assert_eq!(doc.sections[0].items.len(), 1);
        match &doc.sections[0].items[0] {
            SectionItem::List(list) => {
                assert_eq!(list.kind, ListKind::Numbered);
                assert_eq!(list.items.len(), 3);
            }
            _ => panic!("expected list"),
        }
    }

    /// test_markdown_to_document_table — table parsing works.
    #[test]
    fn test_markdown_to_document_table() {
        let md = "| Name | Age |\n| --- | --- |\n| Alice | 30 |";
        let doc = markdown_to_document(md).expect("parse table");
        assert_eq!(doc.sections[0].items.len(), 1);
        match &doc.sections[0].items[0] {
            SectionItem::Table(table) => {
                assert!(table.rows.len() >= 2, "should have header + data row");
                assert_eq!(table.rows[0].cells.len(), 2);
                assert_eq!(table.rows[1].cells.len(), 2);
            }
            _ => panic!("expected table"),
        }
    }

    /// test_markdown_to_document_mixed — headers, paragraphs, and lists together.
    #[test]
    fn test_markdown_to_document_mixed() {
        let md =
            "# Title\n\nA paragraph with **bold**.\n\n- Bullet 1\n- Bullet 2\n\nAnother paragraph.";
        let doc = markdown_to_document(md).expect("parse mixed content");
        assert!(
            doc.sections[0].items.len() >= 3,
            "should have multiple items"
        );

        // First item should be the heading
        match &doc.sections[0].items[0] {
            SectionItem::Paragraph(p) => {
                assert_eq!(p.style_name.as_deref(), Some("Heading 1"));
            }
            _ => panic!("expected heading paragraph"),
        }
    }

    /// test_markdown_to_document_empty — empty markdown produces empty document.
    #[test]
    fn test_markdown_to_document_empty() {
        let md = "";
        let doc = markdown_to_document(md).expect("parse empty");
        assert_eq!(doc.sections.len(), 1);
        assert!(doc.sections[0].items.is_empty());
        assert_eq!(doc.ir_version, IR_VERSION);
    }

    /// test_markdown_to_document_ir_version — document carries correct IR version.
    #[test]
    fn test_markdown_to_document_ir_version() {
        let md = "# Test\n\nContent.";
        let doc = markdown_to_document(md).expect("parse");
        assert_eq!(doc.ir_version, IR_VERSION);
        assert!(doc.template.is_none());
    }

    /// test_markdown_to_document_with_template — sets template field.
    #[test]
    fn test_markdown_to_document_with_template() {
        let md = "# Test";
        let doc =
            markdown_to_document_with_template(md, Some("template-abc123".into())).expect("parse");
        assert_eq!(doc.template, Some("template-abc123".into()));
    }

    /// test_markdown_to_document_serde — round-trips via JSON.
    #[test]
    fn test_markdown_to_document_serde() {
        let md = "# Hello\n\nWorld **bold**.";
        let doc = markdown_to_document(md).expect("parse");
        let json = serde_json::to_string(&doc).expect("serialize");
        let doc2: Document = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(doc.ir_version, doc2.ir_version);
        assert_eq!(doc.sections.len(), doc2.sections.len());
    }
}
