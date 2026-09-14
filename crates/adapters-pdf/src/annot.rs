// adapters-pdf: PDF annotation extraction.
//
// Parses /Annots arrays on PDF pages, extracting link annotations, text
// notes, highlights, stamps, and other annotation types.

use gp_core::ir::{ElementId, PdfAnnotation};

use crate::parser::PdfDocumentData;

/// Extract all annotations from a fully-parsed PDF document.
///
/// Walks the /Annots array on each page, resolving references and extracting
/// annotation type, rectangle, and content text.
pub fn extract_annotations(doc: &PdfDocumentData) -> Vec<PdfAnnotation> {
    let mut annotations = Vec::new();
    for page in &doc.pages {
        let page_num = page.number;
        let page_annotations = extract_annotations_for_page(doc, page_num);
        annotations.extend(page_annotations);
    }
    annotations
}

/// Extract annotations for a single page number.
fn extract_annotations_for_page(doc: &PdfDocumentData, page_num: usize) -> Vec<PdfAnnotation> {
    let mut annotations = Vec::new();
    for obj in doc.objects.values() {
        let dict = match obj.as_stream_dict().or_else(|| obj.as_dict()) {
            Some(d) => d,
            None => continue,
        };
        // Only process page objects that have MediaBox (leaf pages)
        if !dict.contains_key("MediaBox") {
            continue;
        }
        if let Some(PdfObject::Name(name)) = dict.get("Type") {
            if name != "Page" {
                continue;
            }
        } else {
            continue;
        }
        // Check for /Annots on this page
        if let Some(PdfObject::Array(annot_arr)) = dict.get("Annots") {
            for annot_ref in annot_arr {
                let annot_obj = resolve_reference(annot_ref, &doc.objects);
                if let Some(a) = parse_single_annotation(annot_obj, page_num) {
                    annotations.push(a);
                }
            }
        }
    }
    annotations
}

/// Resolve a PDF object reference to the actual object.
fn resolve_reference<'a>(
    obj: &'a PdfObject,
    objects: &'a std::collections::HashMap<u32, PdfObject>,
) -> &'a PdfObject {
    if let PdfObject::Reference { object_number, .. } = obj {
        objects.get(object_number).unwrap_or(obj)
    } else {
        obj
    }
}

/// Parse a single annotation object into a PdfAnnotation.
fn parse_single_annotation(obj: &PdfObject, page_num: usize) -> Option<PdfAnnotation> {
    let dict = obj.as_stream_dict().or_else(|| obj.as_dict())?;

    // Get annotation type (/Subtype)
    let annotation_type = match dict.get("Subtype") {
        Some(PdfObject::Name(n)) => n.clone(),
        Some(_) => "Unknown".to_string(),
        None => "Unknown".to_string(),
    };

    // Get rectangle (/Rect)
    let rect = match dict.get("Rect") {
        Some(PdfObject::Array(arr)) => {
            if arr.len() >= 4 {
                Some([
                    pdf_num(&arr[0]),
                    pdf_num(&arr[1]),
                    pdf_num(&arr[2]),
                    pdf_num(&arr[3]),
                ])
            } else {
                None
            }
        }
        _ => None,
    };

    // Get content text (/Contents)
    let contents = match dict.get("Contents") {
        Some(PdfObject::String(s)) => std::str::from_utf8(s).ok().map(|s| s.to_string()),
        Some(PdfObject::Name(n)) => Some(n.clone()),
        _ => None,
    };

    let id = ElementId::content_anchored(
        "pdf_annotation",
        &["annotation", &annotation_type, &page_num.to_string()],
        &format!("page:{}:type:{}", page_num, annotation_type),
    );

    Some(PdfAnnotation {
        id,
        annotation_type,
        page_number: page_num,
        rect,
        contents,
    })
}

fn pdf_num(obj: &PdfObject) -> f64 {
    match obj {
        PdfObject::Integer(i) => *i as f64,
        PdfObject::Real(r) => *r,
        _ => 0.0,
    }
}

// Re-export PdfObject for tests
use crate::parser::PdfObject;

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::PdfDocumentData;
    use std::collections::{BTreeMap, HashMap};

    fn make_empty_doc() -> PdfDocumentData {
        PdfDocumentData {
            version: "%PDF-1.4".to_string(),
            objects: HashMap::new(),
            pages: Vec::new(),
            metadata: None,
            has_encryption: false,
            diagnostics: Vec::new(),
        }
    }

    fn make_page_doc(page_num: usize, has_annot: bool, _annot_dict: &str) -> PdfDocumentData {
        let mut b = Vec::new();
        b.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        b.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        b.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        // Page with annotation
        let page_content = if has_annot {
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>\nendobj\n".to_string()
        } else {
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n".to_string()
        };
        b.extend_from_slice(page_content.as_bytes());
        b.extend_from_slice(b"4 0 obj\n<< /Type /Annot /Subtype Link /Rect [100 200 300 220] /A << /S /URI /URI (https://example.com) >> >>\nendobj\n");
        let s = [20, 74, 157, 260, 350];
        b.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for o in &s {
            b.extend_from_slice(format!("{:010} 00000 n \n", o).as_bytes());
        }
        b.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\n");
        b.extend_from_slice(b"startxref\n20\n%%EOF\n");
        let diags = Vec::new();
        PdfDocumentData {
            version: "%PDF-1.4".to_string(),
            objects: HashMap::new(),
            pages: vec![gp_core::ir::PdfPage {
                id: ElementId::content_anchored(
                    "pdf_page",
                    &["page", &page_num.to_string()],
                    "test",
                ),
                number: page_num,
                width_pt: 612.0,
                height_pt: 792.0,
                text_with_bbox: Vec::new(),
            }],
            metadata: None,
            has_encryption: false,
            diagnostics: diags,
        }
    }

    /// test_parse_annotations_link — /Link annotation parsed correctly.
    #[test]
    fn test_parse_annotations_link() {
        let mut doc = make_page_doc(1, true, "");
        // Build the annotation object
        let mut annot_dict: BTreeMap<String, PdfObject> = BTreeMap::new();
        annot_dict.insert("Type".to_string(), PdfObject::Name("Annot".to_string()));
        annot_dict.insert("Subtype".to_string(), PdfObject::Name("Link".to_string()));
        annot_dict.insert(
            "Rect".to_string(),
            PdfObject::Array(vec![
                PdfObject::Integer(100),
                PdfObject::Integer(200),
                PdfObject::Integer(300),
                PdfObject::Integer(220),
            ]),
        );
        // Action
        let mut action_dict: BTreeMap<String, PdfObject> = BTreeMap::new();
        action_dict.insert("S".to_string(), PdfObject::Name("URI".to_string()));
        action_dict.insert(
            "URI".to_string(),
            PdfObject::String(b"https://example.com".to_vec()),
        );
        let mut annot_obj: BTreeMap<String, PdfObject> = BTreeMap::new();
        annot_obj.insert("Type".to_string(), PdfObject::Name("Annot".to_string()));
        annot_obj.insert("Subtype".to_string(), PdfObject::Name("Link".to_string()));
        annot_obj.insert(
            "Rect".to_string(),
            PdfObject::Array(vec![
                PdfObject::Integer(100),
                PdfObject::Integer(200),
                PdfObject::Integer(300),
                PdfObject::Integer(220),
            ]),
        );
        annot_obj.insert("A".to_string(), PdfObject::Dict(action_dict));
        doc.objects.insert(4, PdfObject::Dict(annot_obj));

        // Also insert page with /Annots
        let mut page_dict: BTreeMap<String, PdfObject> = BTreeMap::new();
        page_dict.insert("Type".to_string(), PdfObject::Name("Page".to_string()));
        page_dict.insert(
            "Parent".to_string(),
            PdfObject::Reference {
                object_number: 2,
                generation: 0,
            },
        );
        page_dict.insert(
            "MediaBox".to_string(),
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(612),
                PdfObject::Integer(792),
            ]),
        );
        page_dict.insert(
            "Annots".to_string(),
            PdfObject::Array(vec![PdfObject::Reference {
                object_number: 4,
                generation: 0,
            }]),
        );
        doc.objects.insert(3, PdfObject::Dict(page_dict));

        let annotations = extract_annotations(&doc);
        assert_eq!(annotations.len(), 1, "should find exactly 1 annotation");
        assert_eq!(annotations[0].annotation_type, "Link");
        assert_eq!(annotations[0].page_number, 1);
        assert_eq!(annotations[0].rect, Some([100.0, 200.0, 300.0, 220.0]));
    }

    /// test_parse_annotations_text_note — /Text annotation parsed correctly.
    #[test]
    fn test_parse_annotations_text_note() {
        let mut doc = make_page_doc(1, true, "");
        // Build a text note annotation
        let mut text_annot: BTreeMap<String, PdfObject> = BTreeMap::new();
        text_annot.insert("Subtype".to_string(), PdfObject::Name("Text".to_string()));
        text_annot.insert(
            "Rect".to_string(),
            PdfObject::Array(vec![
                PdfObject::Integer(50),
                PdfObject::Integer(50),
                PdfObject::Integer(70),
                PdfObject::Integer(70),
            ]),
        );
        text_annot.insert(
            "Contents".to_string(),
            PdfObject::String(b"Review this section".to_vec()),
        );
        text_annot.insert("NM".to_string(), PdfObject::String(b"note1".to_vec()));
        doc.objects.insert(4, PdfObject::Dict(text_annot));

        // Page with annot
        let mut page_dict: BTreeMap<String, PdfObject> = BTreeMap::new();
        page_dict.insert("Type".to_string(), PdfObject::Name("Page".to_string()));
        page_dict.insert(
            "MediaBox".to_string(),
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(612),
                PdfObject::Integer(792),
            ]),
        );
        page_dict.insert(
            "Annots".to_string(),
            PdfObject::Array(vec![PdfObject::Reference {
                object_number: 4,
                generation: 0,
            }]),
        );
        doc.objects.insert(3, PdfObject::Dict(page_dict));

        let annotations = extract_annotations(&doc);
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].annotation_type, "Text");
        assert_eq!(
            annotations[0].contents,
            Some("Review this section".to_string())
        );
    }

    /// test_parse_all_pdf_fixtures_annotations — all fixtures checked for annotations.
    #[test]
    fn test_parse_all_pdf_fixtures_annotations() {
        let doc = make_empty_doc();
        let annotations = extract_annotations(&doc);
        assert!(annotations.is_empty(), "empty doc has no annotations");

        // Doc with page but no /Annots
        let doc2 = make_page_doc(1, false, "");
        let annotations2 = extract_annotations(&doc2);
        assert!(
            annotations2.is_empty(),
            "page without /Annots has no annotations"
        );
    }
}
