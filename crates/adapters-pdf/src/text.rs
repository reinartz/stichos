// adapters-pdf: text extraction with word-level bounding boxes.
#![allow(clippy::iter_cloned_collect)]
use gp_core::diagnostics::Diagnostic;
use gp_core::ir::{ElementId, TextWithBbox};

use crate::parser::{parse_pdf_data, PdfDocumentData, PdfObject};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Extract text with word-level bounding boxes from a PDF file.
///
/// Parses content streams, walks text operators (BT/ET, Tj, TJ, ', ", Tm, T*),
/// and computes bounding boxes using the text matrix and font metrics.
pub fn extract_text_with_bboxes(
    pdf_path: &std::path::Path,
) -> Result<Vec<TextWithBbox>, Vec<Diagnostic>> {
    let data = std::fs::read(pdf_path).map_err(|e| {
        vec![Diagnostic::new(
            "PDF-E-001",
            gp_core::diagnostics::Severity::Error,
            format!("Cannot read PDF: {e}"),
        )]
    })?;
    extract_text_bytes(&data)
}

/// Extract text from in-memory PDF bytes.
pub fn extract_text_bytes(pdf_bytes: &[u8]) -> Result<Vec<TextWithBbox>, Vec<Diagnostic>> {
    let doc_data = parse_pdf_data(pdf_bytes)?;
    Ok(extract_text_from_doc_data(&doc_data))
}

/// Generate a G5 evidence report comparing our word bboxes against poppler -bbox output.
pub fn generate_g5_report(pdf_path: &std::path::Path) -> Result<G5Report, Vec<Diagnostic>> {
    let our_text = extract_text_with_bboxes(pdf_path)?;
    let poppler_words = try_poppler_bbox(pdf_path);
    let total_words = our_text.len();
    let matched = poppler_words
        .iter()
        .filter(|pw| {
            our_text.iter().any(|ot| {
                let dx = (ot.bbox[2] - pw.bbox[2]).abs();
                let dy = (ot.bbox[1] - pw.bbox[1]).abs();
                let ddx = (ot.bbox[0] - pw.bbox[0]).abs();
                let ddy = (ot.bbox[3] - pw.bbox[3]).abs();
                (dx.min(ddx)) < 5.0 && (dy.min(ddy)) < 5.0
            })
        })
        .count();
    let text_match_rate = if total_words > 0 {
        matched as f64 / total_words as f64
    } else {
        0.0
    };
    Ok(G5Report {
        pdf_path: pdf_path.to_path_buf(),
        our_word_count: total_words,
        poppler_word_count: poppler_words.len(),
        matched_word_count: matched,
        text_match_rate,
        our_text: our_text.clone(),
        poppler_text: poppler_words,
    })
}

// ---------------------------------------------------------------------------
// Internal extraction logic
// ---------------------------------------------------------------------------

fn extract_text_from_doc_data(doc_data: &PdfDocumentData) -> Vec<TextWithBbox> {
    let mut results: Vec<TextWithBbox> = Vec::new();
    let mut seen_content: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let page_objnums: Vec<u32> = find_page_objnums(doc_data);

    for (idx, &page_objnum) in page_objnums.iter().enumerate() {
        let page_num = idx + 1;
        if let Some(obj) = doc_data.objects.get(&page_objnum) {
            let contents_ref = extract_contents_ref(obj);
            if let Some(content_objnum) = contents_ref {
                if !seen_content.insert(content_objnum) {
                    continue;
                }
                if let Some(PdfObject::Stream {
                    data: stream_data, ..
                }) = doc_data.objects.get(&content_objnum)
                {
                    let text_items = parse_content_stream(stream_data);
                    for item in text_items {
                        let preview = if item.text.len() > 8 {
                            &item.text[..8]
                        } else {
                            &item.text
                        };
                        results.push(TextWithBbox {
                            id: ElementId::content_anchored(
                                "text",
                                &["page", &page_num.to_string(), preview],
                                &format!("{}:{}:{}", page_num, item.x0, item.text),
                            ),
                            text: item.text.clone(),
                            bbox: item.bbox(),
                        });
                    }
                }
            }
        }
    }
    results
}

/// Extract text with bounding boxes, grouped by 1-indexed page number.
///
/// Used by `PdfReader::read()` so per-page text coordinates survive into the
/// tagged IR tree. The returned map is keyed by page number (as recorded in
/// each `PdfPage.number`) so pages can be matched to the document tree.
pub(crate) fn extract_text_grouped_by_page(
    doc_data: &PdfDocumentData,
) -> std::collections::HashMap<usize, Vec<TextWithBbox>> {
    let mut grouped: std::collections::HashMap<usize, Vec<TextWithBbox>> =
        std::collections::HashMap::new();
    let mut seen_content: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let page_objnums: Vec<u32> = find_page_objnums(doc_data);

    for (idx, &page_objnum) in page_objnums.iter().enumerate() {
        let page_num = idx + 1;
        if let Some(obj) = doc_data.objects.get(&page_objnum) {
            let contents_ref = extract_contents_ref(obj);
            if let Some(content_objnum) = contents_ref {
                if !seen_content.insert(content_objnum) {
                    continue;
                }
                if let Some(PdfObject::Stream {
                    data: stream_data, ..
                }) = doc_data.objects.get(&content_objnum)
                {
                    let text_items = parse_content_stream(stream_data);
                    for item in text_items {
                        let preview = if item.text.len() > 8 {
                            &item.text[..8]
                        } else {
                            &item.text
                        };
                        grouped.entry(page_num).or_default().push(TextWithBbox {
                            id: ElementId::content_anchored(
                                "text",
                                &["page", &page_num.to_string(), preview],
                                &format!("{}:{}:{}", page_num, item.x0, item.text),
                            ),
                            text: item.text.clone(),
                            bbox: item.bbox(),
                        });
                    }
                }
            }
        }
    }
    grouped
}

fn find_page_objnums(doc_data: &PdfDocumentData) -> Vec<u32> {
    let mut page_nums = Vec::new();
    for (objnum, obj) in &doc_data.objects {
        if let PdfObject::Stream { dict, .. } | PdfObject::Dict(dict) = obj {
            if let Some(PdfObject::Name(name)) = dict.get("Type") {
                if name == "Page" {
                    page_nums.push(*objnum);
                }
            }
        }
    }
    page_nums.sort();
    page_nums
}

fn extract_contents_ref(obj: &PdfObject) -> Option<u32> {
    let dict = match obj {
        PdfObject::Stream { dict, .. } | PdfObject::Dict(dict) => dict,
        _ => return None,
    };
    if let Some(contents) = dict.get("Contents") {
        match contents {
            PdfObject::Reference { object_number, .. } => Some(*object_number),
            PdfObject::Array(ref arr) => arr.first().and_then(|a| {
                if let PdfObject::Reference { object_number, .. } = a {
                    Some(*object_number)
                } else {
                    None
                }
            }),
            _ => None,
        }
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Content stream parser
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct TextItem {
    text: String,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl TextItem {
    fn bbox(&self) -> [f64; 4] {
        let left = self.x0.min(self.x1);
        let right = self.x0.max(self.x1);
        let top = self.y0.max(self.y1);
        let bottom = self.y0.min(self.y1);
        [left, top, right, bottom]
    }
}

fn parse_content_stream(data: &[u8]) -> Vec<TextItem> {
    let tokens = tokenize_stream(data);
    let mut items = Vec::new();
    let mut idx = 0;
    let mut tm_x: f64 = 0.0;
    let mut tm_y: f64 = 0.0;
    let mut font_size: f64 = 12.0;

    while idx < tokens.len() {
        let tok = tokens[idx].as_str();
        match tok {
            "BT" => {
                tm_x = 0.0;
                tm_y = 0.0;
                idx += 1;
            }
            "ET" => {
                idx += 1;
            }
            "Tf" => {
                // Tf font-size — the font name precedes Tf, size follows
                // Token stream: ... "F1" "12" "Tf" ...
                // So size is at idx - 1 (the token before Tf)
                if idx > 0 {
                    if let Ok(size) = tokens[idx - 1].parse::<f64>() {
                        font_size = size;
                        idx += 1;
                    } else {
                        idx += 1;
                    }
                } else {
                    idx += 1;
                }
            }
            "Tj" => {
                // Operand (string) precedes operator: (text) Tj
                if idx > 0 {
                    if let Some((text, _)) = expect_string(&tokens, idx - 1) {
                        let char_count = text.chars().count() as f64;
                        let text_width = char_count * font_size * 0.55;
                        items.push(TextItem {
                            text,
                            x0: tm_x,
                            y0: tm_y - font_size,
                            x1: tm_x + text_width,
                            y1: tm_y,
                        });
                        idx += 1;
                        continue;
                    }
                }
                idx += 1;
            }
            "TJ" => {
                // Operand array precedes operator: [...] TJ
                // Find the opening bracket by scanning backward for ] then [
                if idx > 0 {
                    let mut array_start: Option<usize> = None;
                    for i in (0..idx).rev() {
                        if tokens[i] == "]" {
                            // Found end of array, now look for start
                        } else if tokens[i] == "[" {
                            array_start = Some(i);
                            break;
                        }
                    }
                    if let Some(start) = array_start {
                        let parts: Vec<String> = tokens[start + 1..idx].iter().cloned().collect();
                        let final_y = tm_y;
                        let mut tj_x = tm_x;
                        let mut i = 0;
                        while i < parts.len() {
                            let part = parts[i].clone();
                            if part.starts_with('(') && part.ends_with(')') {
                                let text = part[1..part.len() - 1].to_string();
                                let char_count = text.chars().count() as f64;
                                let text_width = char_count * font_size * 0.55;
                                items.push(TextItem {
                                    text,
                                    x0: tj_x,
                                    y0: final_y - font_size,
                                    x1: tj_x + text_width,
                                    y1: final_y,
                                });
                                // Check for spacing after this string
                                if i + 1 < parts.len() {
                                    if let Ok(spacing) = parts[i + 1].parse::<f64>() {
                                        tj_x -= spacing * font_size / 1000.0;
                                        i += 2; // Skip string and spacing
                                    } else {
                                        i += 1;
                                    }
                                } else {
                                    i += 1;
                                }
                            } else {
                                // Number (spacing without preceding string), skip
                                i += 1;
                            }
                        }
                        idx += 1;
                        continue;
                    }
                }
                idx += 1;
            }
            "'" => {
                // Move to next line and show text: (text) '
                if idx > 0 {
                    tm_y -= font_size * 1.2;
                    if let Some((text, _)) = expect_string(&tokens, idx - 1) {
                        let char_count = text.chars().count() as f64;
                        let text_width = char_count * font_size * 0.55;
                        items.push(TextItem {
                            text,
                            x0: tm_x,
                            y0: tm_y - font_size,
                            x1: tm_x + text_width,
                            y1: tm_y,
                        });
                        idx -= 1;
                    }
                }
                idx += 1;
            }
            "\"" => {
                // Set word spacing, char spacing, move to next line, show text: ws cs (text) "
                if idx > 2 {
                    tm_y -= font_size * 1.2;
                    if let Some((_, _ws)) = expect_number(&tokens, idx - 3) {
                        if let Some((_, _cs)) = expect_number(&tokens, idx - 2) {
                            if let Some((text, _)) = expect_string(&tokens, idx - 1) {
                                let char_count = text.chars().count() as f64;
                                let text_width = char_count * font_size * 0.55;
                                items.push(TextItem {
                                    text,
                                    x0: tm_x,
                                    y0: tm_y - font_size,
                                    x1: tm_x + text_width,
                                    y1: tm_y,
                                });
                                idx -= 1;
                            }
                        }
                    }
                }
                idx += 1;
            }
            "Td" | "TD" => {
                let is_td = tok == "TD";
                // In PDF, operands precede operators: dx dy Td
                // Look backwards for the two numbers
                if idx >= 2 {
                    if let (Some((dy, _)), Some((dx, _))) = (
                        expect_number(&tokens, idx - 1),
                        expect_number(&tokens, idx - 2),
                    ) {
                        if is_td {
                            tm_y -= dy;
                        }
                        tm_x = dx;
                        tm_y = if is_td { tm_y } else { dy };
                        // Move past Td and the two numbers (which are before Td)
                        idx += 1;
                    } else {
                        idx += 1;
                    }
                } else {
                    idx += 1;
                }
            }
            "T*" => {
                tm_y -= font_size * 1.2;
                idx += 1;
            }
            "Tm" => {
                if idx + 5 < tokens.len() {
                    if let (
                        Some((_, _a)),
                        Some((_, _b)),
                        Some((_, _c)),
                        Some((_, _d)),
                        Some((e, _)),
                        Some((f, _)),
                    ) = (
                        expect_number(&tokens, idx),
                        expect_number(&tokens, idx + 1),
                        expect_number(&tokens, idx + 2),
                        expect_number(&tokens, idx + 3),
                        expect_number(&tokens, idx + 4),
                        expect_number(&tokens, idx + 5),
                    ) {
                        tm_x = e;
                        tm_y = f;
                        idx = f as usize + 1;
                    } else {
                        idx = (idx + 6).min(tokens.len());
                    }
                } else {
                    idx = (idx + 6).min(tokens.len());
                }
            }
            "Tc" | "Tw" | "Tz" | "Ts" => {
                idx += 1;
                if let Some((_, end)) = expect_number(&tokens, idx) {
                    idx = end + 1;
                }
            }
            _ => {
                idx += 1;
            }
        }
    }
    items
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

fn tokenize_stream(data: &[u8]) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut pos = 0;

    while pos < data.len() {
        if data[pos].is_ascii_whitespace() {
            pos += 1;
            continue;
        }
        if data[pos] == b'%' {
            pos += 1;
            while pos < data.len() && data[pos] != b'\n' && data[pos] != b'\r' {
                pos += 1;
            }
            continue;
        }
        if [0xE2, 0xE3, 0xCF, 0xD3].contains(&data[pos]) {
            pos += 1;
            continue;
        }
        if data[pos] == b'(' {
            let start = pos;
            pos += 1;
            let mut esc = false;
            let mut depth = 0;
            while pos < data.len() {
                if esc {
                    esc = false;
                    pos += 1;
                    continue;
                }
                match data[pos] {
                    b'\\' => esc = true,
                    b'(' => depth += 1,
                    b')' if depth == 0 => {
                        pos += 1;
                        break;
                    }
                    b')' => depth -= 1,
                    _ => {}
                }
                pos += 1;
            }
            let token = std::str::from_utf8(&data[start..pos])
                .unwrap_or("")
                .to_string();
            tokens.push(token);
            continue;
        }
        if data[pos] == b'<' {
            pos += 1;
            while pos < data.len() && data[pos] != b'>' {
                pos += 1;
            }
            if pos < data.len() {
                pos += 1;
            }
            continue;
        }
        if data[pos] == b'[' {
            tokens.push("[".to_string());
            pos += 1;
            continue;
        }
        if data[pos] == b']' {
            tokens.push("]".to_string());
            pos += 1;
            continue;
        }
        // Name token — include value (without /)
        if data[pos] == b'/' {
            let name_start = pos + 1;
            pos += 1;
            while pos < data.len() {
                match data[pos] {
                    b' ' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'\n'
                    | b'\r' => break,
                    b'#' => pos += 2,
                    _ => pos += 1,
                }
            }
            if pos > name_start {
                let name = std::str::from_utf8(&data[name_start..pos]).unwrap_or("");
                tokens.push(name.to_string());
            }
            continue;
        }
        // Number or keyword
        let start = pos;
        while pos < data.len()
            && (data[pos].is_ascii_alphanumeric()
                || data[pos] == b'+'
                || data[pos] == b'-'
                || data[pos] == b'.')
        {
            pos += 1;
        }
        if pos > start {
            let token = std::str::from_utf8(&data[start..pos])
                .unwrap_or("")
                .to_string();
            tokens.push(token);
        } else {
            // Unrecognized byte (e.g. undecoded binary stream data). Skip it so
            // tokenization always terminates instead of spinning on the byte.
            pos += 1;
        }
    }
    tokens
}

fn expect_string(tokens: &[String], idx: usize) -> Option<(String, usize)> {
    if idx >= tokens.len() {
        return None;
    }
    let tok = &tokens[idx];
    if tok.starts_with('(') && tok.ends_with(')') {
        let inner = &tok[1..tok.len() - 1];
        let text = unescape_pdf_string(inner);
        Some((text, idx))
    } else {
        None
    }
}

fn expect_number(tokens: &[String], idx: usize) -> Option<(f64, usize)> {
    if idx >= tokens.len() {
        return None;
    }
    let tok = &tokens[idx];
    if let Ok(n) = tok.parse::<f64>() {
        Some((n, idx))
    } else {
        None
    }
}

fn unescape_pdf_string(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => result.push('\\'),
                Some('(') => result.push('('),
                Some(')') => result.push(')'),
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some('b') => result.push('\x08'),
                Some('f') => result.push('\x0C'),
                Some(n) => {
                    let mut digits = String::from(n);
                    for _ in 0..2 {
                        if let Some(d) = chars.next() {
                            if d.is_ascii_digit() {
                                digits.push(d);
                            } else {
                                break;
                            }
                        }
                    }
                    if let Ok(byte) = u8::from_str_radix(&digits, 8) {
                        result.push(byte as char);
                    }
                }
                _ => {}
            }
        } else {
            result.push(c);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// G5 Report types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct G5Report {
    pub pdf_path: std::path::PathBuf,
    pub our_word_count: usize,
    pub poppler_word_count: usize,
    pub matched_word_count: usize,
    pub text_match_rate: f64,
    pub our_text: Vec<TextWithBbox>,
    pub poppler_text: Vec<PopplerBboxWord>,
}

#[derive(Debug, Clone)]
pub struct PopplerBboxWord {
    pub text: String,
    pub bbox: [f64; 4],
}

fn try_poppler_bbox(pdf_path: &std::path::Path) -> Vec<PopplerBboxWord> {
    let output = std::process::Command::new("pdftotext")
        .args(["-bbox", pdf_path.to_str().unwrap_or(""), "-"])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            parse_poppler_bbox_output(&String::from_utf8_lossy(&out.stdout))
        }
        _ => Vec::new(),
    }
}

fn parse_poppler_bbox_output(output: &str) -> Vec<PopplerBboxWord> {
    let mut words = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(5, ' ').collect();
        if parts.len() >= 5 {
            if let (Ok(x0), Ok(y0), Ok(x1), Ok(y1)) = (
                parts[0].parse::<f64>(),
                parts[1].parse::<f64>(),
                parts[2].parse::<f64>(),
                parts[3].parse::<f64>(),
            ) {
                words.push(PopplerBboxWord {
                    text: parts[4].to_string(),
                    bbox: [x0, y1, x1, y0],
                });
            }
        }
    }
    words
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_simple_pdf_with_text() -> Vec<u8> {
        let mut b = Vec::new();
        let o1 = b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n";
        let o2 = b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n";
        let o3 = b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n";
        let content = b"BT /F1 12 Tf 100 700 Td (Hello World) Tj ET";
        let content_len = content.len();
        let o4 = format!(
            "4 0 obj\n<< /Length {cl} >>\nstream\n{c}\nendstream\nendobj\n",
            cl = content_len,
            c = String::from_utf8_lossy(content)
        );
        let o5 = b"5 0 obj\n<< /Title (Test) >>\nendobj\n";
        b.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        b.extend_from_slice(o1);
        b.extend_from_slice(o2);
        b.extend_from_slice(o3);
        b.extend_from_slice(o4.as_bytes());
        b.extend_from_slice(o5);
        let xref_start = b.len();
        b.extend_from_slice(b"xref\n0 6\n");
        b.extend_from_slice(b"0000000000 65535 f \n");
        b.extend_from_slice(format!("{:010} 00000 n \n", 20).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", 20 + o1.len()).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", 20 + o1.len() + o2.len()).as_bytes());
        b.extend_from_slice(
            format!("{:010} 00000 n \n", 20 + o1.len() + o2.len() + o3.len()).as_bytes(),
        );
        b.extend_from_slice(
            format!(
                "{:010} 00000 n \n",
                20 + o1.len() + o2.len() + o3.len() + o4.len()
            )
            .as_bytes(),
        );
        b.extend_from_slice(b"trailer\n<< /Size 6 /Root 1 0 R >>\n");
        b.extend_from_slice(b"startxref\n");
        b.extend_from_slice(format!("{}", xref_start).as_bytes());
        b.extend_from_slice(b"\n%%EOF\n");
        b
    }

    fn make_pdf_with_tj_operator() -> Vec<u8> {
        let mut b = Vec::new();
        let o1 = b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n";
        let o2 = b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n";
        let o3 = b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n";
        let content = b"BT /F1 12 Tf 100 700 Td [(Hello) 10 (World) 20] TJ ET";
        let content_len = content.len();
        let o4 = format!(
            "4 0 obj\n<< /Length {cl} >>\nstream\n{c}\nendstream\nendobj\n",
            cl = content_len,
            c = String::from_utf8_lossy(content)
        );
        let o5 = b"5 0 obj\n<< /Title (TJ Test) >>\nendobj\n";
        b.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        b.extend_from_slice(o1);
        b.extend_from_slice(o2);
        b.extend_from_slice(o3);
        b.extend_from_slice(o4.as_bytes());
        b.extend_from_slice(o5);
        let xref_start = b.len();
        b.extend_from_slice(b"xref\n0 6\n");
        b.extend_from_slice(b"0000000000 65535 f \n");
        b.extend_from_slice(format!("{:010} 00000 n \n", 20).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", 20 + o1.len()).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", 20 + o1.len() + o2.len()).as_bytes());
        b.extend_from_slice(
            format!("{:010} 00000 n \n", 20 + o1.len() + o2.len() + o3.len()).as_bytes(),
        );
        b.extend_from_slice(
            format!(
                "{:010} 00000 n \n",
                20 + o1.len() + o2.len() + o3.len() + o4.len()
            )
            .as_bytes(),
        );
        b.extend_from_slice(b"trailer\n<< /Size 6 /Root 1 0 R >>\n");
        b.extend_from_slice(b"startxref\n");
        b.extend_from_slice(format!("{}", xref_start).as_bytes());
        b.extend_from_slice(b"\n%%EOF\n");
        b
    }

    fn make_pdf_with_multiple_words() -> Vec<u8> {
        let mut b = Vec::new();
        let o1 = b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n";
        let o2 = b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n";
        let o3 = b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n";
        let content = b"BT /F1 12 Tf 100 700 Td (Hello) Tj 30 0 Td (World) Tj ET";
        let content_len = content.len();
        let o4 = format!(
            "4 0 obj\n<< /Length {cl} >>\nstream\n{c}\nendstream\nendobj\n",
            cl = content_len,
            c = String::from_utf8_lossy(content)
        );
        let o5 = b"5 0 obj\n<< /Title (Multi) >>\nendobj\n";
        b.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        b.extend_from_slice(o1);
        b.extend_from_slice(o2);
        b.extend_from_slice(o3);
        b.extend_from_slice(o4.as_bytes());
        b.extend_from_slice(o5);
        let xref_start = b.len();
        b.extend_from_slice(b"xref\n0 6\n");
        b.extend_from_slice(b"0000000000 65535 f \n");
        b.extend_from_slice(format!("{:010} 00000 n \n", 20).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", 20 + o1.len()).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", 20 + o1.len() + o2.len()).as_bytes());
        b.extend_from_slice(
            format!("{:010} 00000 n \n", 20 + o1.len() + o2.len() + o3.len()).as_bytes(),
        );
        b.extend_from_slice(
            format!(
                "{:010} 00000 n \n",
                20 + o1.len() + o2.len() + o3.len() + o4.len()
            )
            .as_bytes(),
        );
        b.extend_from_slice(b"trailer\n<< /Size 6 /Root 1 0 R >>\n");
        b.extend_from_slice(b"startxref\n");
        b.extend_from_slice(format!("{}", xref_start).as_bytes());
        b.extend_from_slice(b"\n%%EOF\n");
        b
    }

    fn make_golden_pdf() -> Vec<u8> {
        let mut b = Vec::new();
        let o1 = b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n";
        let o2 = b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n";
        let o3 = b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n";
        let content = b"BT /F1 10 Tf 50 700 Tm (Page One Text) Tj 0 -20 Td (Second line) Tj ET";
        let content_len = content.len();
        let o4 = format!(
            "4 0 obj\n<< /Length {cl} >>\nstream\n{c}\nendstream\nendobj\n",
            cl = content_len,
            c = String::from_utf8_lossy(content)
        );
        let o5 = b"5 0 obj\n<< /Title (Golden) >>\nendobj\n";
        b.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        b.extend_from_slice(o1);
        b.extend_from_slice(o2);
        b.extend_from_slice(o3);
        b.extend_from_slice(o4.as_bytes());
        b.extend_from_slice(o5);
        let xref_start = b.len();
        b.extend_from_slice(b"xref\n0 6\n");
        b.extend_from_slice(b"0000000000 65535 f \n");
        b.extend_from_slice(format!("{:010} 00000 n \n", 20).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", 20 + o1.len()).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", 20 + o1.len() + o2.len()).as_bytes());
        b.extend_from_slice(
            format!("{:010} 00000 n \n", 20 + o1.len() + o2.len() + o3.len()).as_bytes(),
        );
        b.extend_from_slice(
            format!(
                "{:010} 00000 n \n",
                20 + o1.len() + o2.len() + o3.len() + o4.len()
            )
            .as_bytes(),
        );
        b.extend_from_slice(b"trailer\n<< /Size 6 /Root 1 0 R >>\n");
        b.extend_from_slice(b"startxref\n");
        b.extend_from_slice(format!("{}", xref_start).as_bytes());
        b.extend_from_slice(b"\n%%EOF\n");
        b
    }

    // ---- Extraction tests ----

    #[test]
    fn test_extract_text_simple() {
        let data = make_simple_pdf_with_text();
        let result = extract_text_bytes(&data);
        assert!(
            result.is_ok(),
            "simple text extraction failed: {:?}",
            result.err()
        );
        let items = result.unwrap();
        assert!(
            !items.is_empty(),
            "Expected at least 1 text item, got {}",
            items.len()
        );
    }

    #[test]
    fn test_extract_text_with_bboxes() {
        let data = make_simple_pdf_with_text();
        let result = extract_text_bytes(&data);
        assert!(result.is_ok(), "bbox extraction failed: {:?}", result.err());
        let items = result.unwrap();
        assert!(
            !items.is_empty(),
            "Expected at least 1 item, got {}",
            items.len()
        );
        for item in &items {
            assert_eq!(item.bbox.len(), 4, "bbox has 4 elements");
            assert!(item.bbox[2] >= item.bbox[0], "right >= left");
            assert!(item.bbox[1] >= item.bbox[3], "top >= bottom");
        }
    }

    #[test]
    fn test_extract_text_tj_operator() {
        let data = make_pdf_with_tj_operator();
        let result = extract_text_bytes(&data);
        assert!(result.is_ok(), "TJ extraction failed: {:?}", result.err());
        let items = result.unwrap();
        assert!(
            items.len() >= 2,
            "TJ should produce >= 2 items, got {}",
            items.len()
        );
    }

    #[test]
    fn test_extract_text_multiple_words() {
        let data = make_pdf_with_multiple_words();
        let result = extract_text_bytes(&data);
        assert!(
            result.is_ok(),
            "multi-word extraction failed: {:?}",
            result.err()
        );
        let items = result.unwrap();
        assert!(
            items.len() >= 2,
            "Expected >= 2 text items, got {}",
            items.len()
        );
    }

    #[test]
    fn test_extract_text_all_pdf_fixtures() {
        let test_fixtures: Vec<(String, Vec<u8>)> = vec![
            ("simple_with_text".to_string(), make_simple_pdf_with_text()),
            ("tj_operator".to_string(), make_pdf_with_tj_operator()),
            ("multiple_words".to_string(), make_pdf_with_multiple_words()),
            ("golden".to_string(), make_golden_pdf()),
        ];
        for (name, data) in test_fixtures {
            let result = extract_text_bytes(&data);
            assert!(
                result.is_ok(),
                "fixture '{}' failed: {:?}",
                name,
                result.err()
            );
            let _items = result.unwrap();
        }
    }

    #[test]
    fn test_text_matches_pdf_fixture_content() {
        let data = make_simple_pdf_with_text();
        let result = extract_text_bytes(&data);
        assert!(result.is_ok(), "extraction failed");
        let items = result.unwrap();
        assert!(!items.is_empty(), "Expected at least 1 text item");
        let text_concat: String = items.iter().map(|i| i.text.as_str()).collect();
        assert!(
            text_concat.contains("Hello"),
            "Expected 'Hello' in extracted text, got: {}",
            text_concat
        );
    }

    #[test]
    fn test_g5_report_generated() {
        let pdf_data = make_simple_pdf_with_text();
        let tmp_path = std::env::temp_dir().join("stichos_g5_test.pdf");
        std::fs::write(&tmp_path, &pdf_data).expect("write tmp pdf");
        let result = generate_g5_report(&tmp_path);
        assert!(
            result.is_ok(),
            "G5 report generation failed: {:?}",
            result.err()
        );
        let report = result.unwrap();
        assert!(report.pdf_path.ends_with("stichos_g5_test.pdf"));
        let _ = std::fs::remove_file(&tmp_path);
    }

    #[test]
    fn test_word_bbox_coordinates() {
        let data = make_simple_pdf_with_text();
        let result = extract_text_bytes(&data);
        assert!(result.is_ok(), "extraction failed");
        let items = result.unwrap();
        assert!(!items.is_empty(), "Expected at least 1 item");
        for item in &items {
            assert!(
                item.bbox[0] <= item.bbox[2],
                "left <= right for: {:?}",
                item.bbox
            );
            assert!(
                item.bbox[1] >= item.bbox[3],
                "top >= bottom for: {:?}",
                item.bbox
            );
            assert!(item.bbox[3] >= -100.0, "bottom >= -100");
            assert!(item.bbox[1] <= 1000.0, "top <= 1000");
        }
    }

    // ---- Tokenizer tests ----

    #[test]
    fn test_tokenize_basic() {
        let data = b"BT /F1 12 Tf 100 700 Td (Hello World) Tj ET";
        let tokens = tokenize_stream(data);
        assert!(tokens.contains(&"BT".to_string()));
        assert!(tokens.contains(&"Tj".to_string()));
        assert!(tokens.contains(&"ET".to_string()));
        assert!(tokens.contains(&"(Hello World)".to_string()));
        // Name tokens should be included
        assert!(tokens.contains(&"F1".to_string()));
        assert!(tokens.contains(&"12".to_string()));
        assert!(tokens.contains(&"Tf".to_string()));
    }

    #[test]
    fn test_tokenize_tj_array() {
        let data = b"BT [(Hello) 10 (World) 20] TJ ET";
        let tokens = tokenize_stream(data);
        assert!(tokens.contains(&"[".to_string()));
        assert!(tokens.contains(&"]".to_string()));
        assert!(tokens.contains(&"(Hello)".to_string()));
        assert!(tokens.contains(&"(World)".to_string()));
        assert!(tokens.contains(&"10".to_string()));
    }

    #[test]
    fn test_tokenize_comments() {
        let data = b"BT % comment\n100 700 Td (Hello) Tj ET";
        let tokens = tokenize_stream(data);
        assert!(tokens.contains(&"BT".to_string()));
        assert!(tokens.contains(&"Td".to_string()));
        assert!(!tokens.iter().any(|t| t.contains("comment")));
    }

    #[test]
    fn test_tokenize_binary_appended_after_extra_spaces() {
        let data = b"BT   [ (Hello) 10 ] TJ ET  \n\x00\x01\xff\xfe";
        let tokens = tokenize_stream(data);
        assert!(tokens.contains(&"(Hello)".to_string()));
        assert!(tokens.contains(&"TJ".to_string()));
    }

    #[test]
    fn test_unescape_pdf_string() {
        assert_eq!(unescape_pdf_string("Hello"), "Hello");
        assert_eq!(unescape_pdf_string("Hello\\nWorld"), "Hello\nWorld");
        assert_eq!(unescape_pdf_string("Tab\\tHere"), "Tab\tHere");
    }

    #[test]
    fn debug_tj() {
        let data = b"BT /F1 12 Tf 100 700 Td [(Hello) 10 (World) 20] TJ ET";
        let _tokens = tokenize_stream(data);
        let items = parse_content_stream(data);
        assert!(
            items.len() >= 2,
            "TJ should produce >= 2, got {}",
            items.len()
        );
    }
}
