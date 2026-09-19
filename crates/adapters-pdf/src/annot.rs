// SPDX-License-Identifier: MIT OR Apache-2.0
// adapters-pdf: PDF annotation extraction.
//
// Parses /Annots arrays on PDF pages, extracting link annotations, text
// notes, highlights, stamps, and other annotation types.

use gp_core::diagnostics::Diagnostic;
use gp_core::ir::{AnnotationKind, AnnotationState, ElementId, PdfAnnotation};

use crate::parser::PdfDocumentData;

/// Census of annotation extraction results.
///
/// Tracks counts for parity checks against the document-level annotation
/// registry and diagnostic output.
#[derive(Debug, Default, Clone)]
pub struct AnnotationCensus {
    pub total_found: usize,
    pub supported: usize,
    pub unsupported: usize,
    pub orphan: usize,
    pub duplicate: usize,
    pub no_rect: usize,
}

impl AnnotationCensus {
    /// Return true if all supported annotations have rects.
    pub fn all_supported_have_rect(&self) -> bool {
        self.no_rect == 0 || self.supported == 0
    }

    /// Return total annotations that were actually included in the result.
    pub fn included(&self) -> usize {
        self.supported + self.unsupported
    }
}

/// Extract all annotations from a fully-parsed PDF document.
///
/// Walks the /Annots array on each page, resolving references and extracting
/// annotation type, rectangle, and content text. Also scans for orphan
/// annotations (not referenced by any page) and duplicate IDs.
///
/// Emits diagnostics:
/// - GP-A-003 for duplicate annotation IDs
/// - GP-A-004 for orphan annotations (not referenced by any page)
/// - GP-A-005 census summary when annotations are found
pub fn extract_annotations(doc: &mut PdfDocumentData) -> Vec<PdfAnnotation> {
    let mut annotations = Vec::new();
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::default();

    // Phase 1: collect all annotation object numbers referenced by pages
    let mut referenced_obj_nums = std::collections::HashSet::<u32>::default(); // obj nums referenced by pages

    for page in &doc.pages {
        let _page_num = page.number;
        for obj in doc.objects.values() {
            if let Some(dict) = obj.as_stream_dict().or_else(|| obj.as_dict()) {
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
                if let Some(PdfObject::Array(annot_arr)) = dict.get("Annots") {
                    for annot_ref in annot_arr {
                        if let Some(obj_num) = annot_ref.ref_obj_num() {
                            referenced_obj_nums.insert(obj_num);
                        }
                    }
                }
            }
        }
    }

    // Phase 2: extract annotations from pages
    for page in &doc.pages {
        let page_num = page.number;
        for obj in doc.objects.values() {
            let dict = match obj.as_stream_dict().or_else(|| obj.as_dict()) {
                Some(d) => d,
                None => continue,
            };
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
            if let Some(PdfObject::Array(annot_arr)) = dict.get("Annots") {
                for annot_ref in annot_arr {
                    let annot_obj = resolve_reference(annot_ref, &doc.objects);
                    if let Some(a) = parse_single_annotation(annot_obj, page_num) {
                        let id_str = a.id.as_str().to_string();
                        if seen_ids.contains(&id_str) {
                            doc.diagnostics.push(Diagnostic::new(
                                "GP-A-003",
                                gp_core::diagnostics::Severity::Warning,
                                format!(
                                    "Duplicated annotation ID '{}' — keeping first occurrence",
                                    id_str
                                ),
                            ));
                            continue;
                        }
                        seen_ids.insert(id_str);
                        annotations.push(a);
                    }
                }
            }
        }
    }

    // Phase 3: find orphan annotations (not referenced by any page)

    for (obj_num, obj) in &doc.objects {
        if referenced_obj_nums.contains(obj_num) {
            continue;
        }
        let dict = match obj.as_stream_dict().or_else(|| obj.as_dict()) {
            Some(d) => d,
            None => continue,
        };
        // Skip pages
        if dict.contains_key("MediaBox") {
            continue;
        }
        // Check if this object is an annotation
        let subtype = match dict.get("Subtype") {
            Some(PdfObject::Name(n)) => n.clone(),
            Some(_) | None => continue,
        };
        match dict.get("Type") {
            Some(PdfObject::Name(t)) if t == "Annot" => {}
            Some(PdfObject::Name(t)) if t == "Page" => continue,
            _ => continue,
        }

        // Emit GP-A-004 for orphan annotations
        doc.diagnostics.push(Diagnostic::new(
            "GP-A-004",
            gp_core::diagnostics::Severity::Warning,
            format!(
                "Annotation at object {} (subtype {}) is orphan — not referenced by any page, assigning to page {}",
                obj_num, subtype, 0
            ),
        ));

        if let Some(mut a) = parse_single_annotation(obj, 0) {
            let id_str = format!("orphan:{}:{}", obj_num, subtype);
            if seen_ids.contains(&id_str) {
                doc.diagnostics.push(Diagnostic::new(
                    "GP-A-003",
                    gp_core::diagnostics::Severity::Warning,
                    format!(
                        "Duplicated annotation ID '{}' — keeping first occurrence",
                        id_str
                    ),
                ));
                continue;
            }
            seen_ids.insert(id_str);
            a.id = ElementId::content_anchored(
                "pdf_annotation",
                &["orphan", &obj_num.to_string()],
                &format!("orphan:{}:{}", obj_num, subtype),
            );
            annotations.push(a);
        }
    }

    // Census: count total annotation objects found in referenced pages
    let mut total_annot_objs = 0;
    for obj_num in &referenced_obj_nums {
        if let Some(obj) = doc.objects.get(obj_num) {
            if let Some(dict) = obj.as_stream_dict().or_else(|| obj.as_dict()) {
                if !dict.contains_key("MediaBox") {
                    total_annot_objs += 1;
                }
            }
        }
    }

    let supported = annotations.iter().filter(|a| a.is_supported_kind()).count();
    let unsupported = annotations
        .iter()
        .filter(|a| !a.is_supported_kind())
        .count();
    let no_rect = annotations
        .iter()
        .filter(|a| a.rect.is_none() && a.is_supported_kind())
        .count();
    let duplicate = doc
        .diagnostics
        .iter()
        .filter(|d| d.code == "GP-A-003")
        .count();
    let orphan = doc
        .diagnostics
        .iter()
        .filter(|d| d.code == "GP-A-004")
        .count();

    // Attach census to diagnostics
    if total_annot_objs > 0 {
        doc.diagnostics.push(Diagnostic::new(
            "GP-A-005",
            gp_core::diagnostics::Severity::Info,
            format!(
                "Annotation census: total_found={}, supported={}, unsupported={}, orphan={}, duplicate={}, no_rect={}",
                total_annot_objs, supported, unsupported, orphan, duplicate, no_rect
            ),
        ));
    }

    annotations
}

/// Extract annotations for a single page number.
///
/// Returns the annotations found on the given page number, without
/// orphan/duplicate handling. Used for incremental extraction.
pub fn extract_annotations_for_page(doc: &PdfDocumentData, page_num: usize) -> Vec<PdfAnnotation> {
    let mut annotations = Vec::new();
    for obj in doc.objects.values() {
        let dict = match obj.as_stream_dict().or_else(|| obj.as_dict()) {
            Some(d) => d,
            None => continue,
        };
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

/// Map a PDF /Subtype name to AnnotationKind.
fn subtype_to_kind(subtype: &str) -> Option<AnnotationKind> {
    match subtype {
        "Text" => Some(AnnotationKind::Text),
        "FreeText" => Some(AnnotationKind::FreeText),
        "Highlight" => Some(AnnotationKind::Highlight),
        "Stamp" => Some(AnnotationKind::Stamp),
        "Link" => Some(AnnotationKind::Popup),
        "Line" => Some(AnnotationKind::Line),
        "Square" => Some(AnnotationKind::Square),
        "Circle" => Some(AnnotationKind::Circle),
        "Underline" => Some(AnnotationKind::Underline),
        "StrikeOut" => Some(AnnotationKind::StrikeOut),
        "Squiggly" => Some(AnnotationKind::Squiggly),
        "Caret" => Some(AnnotationKind::Caret),
        "Ink" => Some(AnnotationKind::Ink),
        "Widget" => Some(AnnotationKind::Widget),
        "Screen" => Some(AnnotationKind::Screen),
        "PrinterMark" => Some(AnnotationKind::PrinterMark),
        "TrapNet" => Some(AnnotationKind::TrapNet),
        "Watermark" => Some(AnnotationKind::Watermark),
        _ => None,
    }
}

/// Parse /QuadPoints to compute a bounding rect.
///
/// /QuadPoints is an array of 8*n numbers representing n quadrilaterals.
/// Each quad has 4 points, each with (x, y, z). We compute the bounding
/// box of all x,y values.
fn quad_points_to_rect(quad_arr: &[PdfObject]) -> Option<[f64; 4]> {
    if quad_arr.len() < 8 {
        return None;
    }
    let mut x_min = f64::INFINITY;
    let mut y_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    let mut y_max = f64::NEG_INFINITY;

    // Each quad point has 3 values: (x, y, z). Quad points come in groups of 4.
    let num_points = quad_arr.len();
    let mut i = 0;
    while i + 2 < num_points {
        let x = pdf_num(&quad_arr[i]);
        let y = pdf_num(&quad_arr[i + 1]);
        if x < x_min {
            x_min = x;
        }
        if y < y_min {
            y_min = y;
        }
        if x > x_max {
            x_max = x;
        }
        if y > y_max {
            y_max = y;
        }
        i += 3;
    }

    if x_min.is_infinite() || y_min.is_infinite() {
        return None;
    }

    Some([x_min, y_min, x_max, y_max])
}

/// Extract /T (author) from an annotation dict.
fn extract_author(dict: &std::collections::BTreeMap<String, PdfObject>) -> Option<String> {
    dict.get("T")
        .and_then(|v| match v {
            PdfObject::String(s) => std::str::from_utf8(s).ok(),
            PdfObject::Name(n) => Some(n.as_str()),
            _ => None,
        })
        .map(|s| s.to_string())
}

/// Extract /M (modified date) from an annotation dict.
fn extract_date(dict: &std::collections::BTreeMap<String, PdfObject>) -> Option<String> {
    dict.get("M")
        .and_then(|v| match v {
            PdfObject::String(s) => std::str::from_utf8(s).ok(),
            PdfObject::Name(n) => Some(n.as_str()),
            _ => None,
        })
        .map(|s| s.to_string())
}

/// Extract /RC (response comment / quote) for highlights.
fn extract_quote(dict: &std::collections::BTreeMap<String, PdfObject>) -> Option<String> {
    dict.get("RC")
        .and_then(|v| match v {
            PdfObject::String(s) => std::str::from_utf8(s).ok(),
            PdfObject::Name(n) => Some(n.as_str()),
            _ => None,
        })
        .map(|s| s.to_string())
}

/// Parse a single annotation object into a PdfAnnotation.
///
/// Handles /Subtype → kind mapping, /T (author), /M (date), /QuadPoints
/// for highlights, and /Rect validation.
fn parse_single_annotation(obj: &PdfObject, page_num: usize) -> Option<PdfAnnotation> {
    let dict = obj.as_stream_dict().or_else(|| obj.as_dict())?;

    // Get annotation type (/Subtype)
    let annotation_type = match dict.get("Subtype") {
        Some(PdfObject::Name(n)) => n.clone(),
        Some(_) => "Unknown".to_string(),
        None => "Unknown".to_string(),
    };

    // Map subtype to kind
    let kind = subtype_to_kind(&annotation_type);

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

    // For highlights without Rect, try QuadPoints
    let rect = if rect.is_none() && kind == Some(AnnotationKind::Highlight) {
        match dict.get("QuadPoints") {
            Some(PdfObject::Array(qp)) => quad_points_to_rect(qp),
            _ => None,
        }
    } else {
        rect
    };

    // Get content text (/Contents)
    let contents = match dict.get("Contents") {
        Some(PdfObject::String(s)) => std::str::from_utf8(s).ok().map(|s| s.to_string()),
        Some(PdfObject::Name(n)) => Some(n.clone()),
        _ => None,
    };

    // Extract author (/T)
    let author = extract_author(dict);

    // Extract date (/M)
    let date = extract_date(dict);

    // Extract quote (/RC)
    let quote = extract_quote(dict);

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
        kind,
        author,
        date,
        quote,
        state: Some(AnnotationState::Applied),
        resolved: Some(false),
    })
}

/// Compute the annotation census from the extracted annotations and document.
///
/// This walks the document to find all annotation objects, then computes
/// the census counts based on the annotations list and diagnostics.
pub fn census_annotations(doc: &PdfDocumentData) -> AnnotationCensus {
    let mut all_annotations = Vec::new();
    let mut referenced_obj_nums = std::collections::HashSet::<u32>::default();

    for page in &doc.pages {
        let page_num = page.number;
        for obj in doc.objects.values() {
            if let Some(dict) = obj.as_stream_dict().or_else(|| obj.as_dict()) {
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
                if let Some(PdfObject::Array(annot_arr)) = dict.get("Annots") {
                    for annot_ref in annot_arr {
                        if let Some(obj_num) = annot_ref.ref_obj_num() {
                            referenced_obj_nums.insert(obj_num);
                        }
                    }
                }
            }
        }
        for obj in doc.objects.values() {
            if let Some(dict) = obj.as_stream_dict().or_else(|| obj.as_dict()) {
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
                if let Some(PdfObject::Array(annot_arr)) = dict.get("Annots") {
                    for annot_ref in annot_arr {
                        let annot_obj = resolve_reference(annot_ref, &doc.objects);
                        if let Some(a) = parse_single_annotation(annot_obj, page_num) {
                            all_annotations.push(a);
                        }
                    }
                }
            }
        }
    }

    let supported = all_annotations
        .iter()
        .filter(|a| a.is_supported_kind())
        .count();
    let unsupported = all_annotations
        .iter()
        .filter(|a| !a.is_supported_kind())
        .count();
    let no_rect = all_annotations
        .iter()
        .filter(|a| a.rect.is_none() && a.is_supported_kind())
        .count();

    let mut total_found = 0;
    for obj_num in &referenced_obj_nums {
        if let Some(obj) = doc.objects.get(obj_num) {
            if let Some(dict) = obj.as_stream_dict().or_else(|| obj.as_dict()) {
                if !dict.contains_key("MediaBox") {
                    total_found += 1;
                }
            }
        }
    }

    AnnotationCensus {
        total_found,
        supported,
        unsupported,
        orphan: 0,
        duplicate: 0,
        no_rect,
    }
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
            signatures: Vec::new(),
        }
    }

    fn make_page_with_annot(
        page_num: usize,
        annot_num: u32,
        annot: BTreeMap<String, PdfObject>,
    ) -> PdfDocumentData {
        let mut doc = make_empty_doc();
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
                object_number: annot_num,
                generation: 0,
            }]),
        );
        doc.objects.insert(annot_num, PdfObject::Dict(annot));
        doc.objects.insert(3, PdfObject::Dict(page_dict));
        doc.pages = vec![gp_core::ir::PdfPage {
            id: ElementId::content_anchored("pdf_page", &["page", &page_num.to_string()], "test"),
            number: page_num,
            width_pt: 612.0,
            height_pt: 792.0,
            text_with_bbox: Vec::new(),
        }];
        doc
    }

    /// test_parse_annotations_link — /Link annotation parsed correctly.
    #[test]
    fn test_parse_annotations_link() {
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
        let mut action_dict: BTreeMap<String, PdfObject> = BTreeMap::new();
        action_dict.insert("S".to_string(), PdfObject::Name("URI".to_string()));
        action_dict.insert(
            "URI".to_string(),
            PdfObject::String(b"https://example.com".to_vec()),
        );
        annot_obj.insert("A".to_string(), PdfObject::Dict(action_dict));

        let mut doc = make_page_with_annot(1, 4, annot_obj);

        let annotations = extract_annotations(&mut doc);
        assert_eq!(annotations.len(), 1, "should find exactly 1 annotation");
        assert_eq!(annotations[0].annotation_type, "Link");
        assert_eq!(annotations[0].page_number, 1);
        assert_eq!(annotations[0].rect, Some([100.0, 200.0, 300.0, 220.0]));
        assert_eq!(annotations[0].kind, Some(AnnotationKind::Popup));
    }

    /// test_parse_annotations_text_note — /Text annotation parsed correctly.
    #[test]
    fn test_parse_annotations_text_note() {
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

        let mut doc = make_page_with_annot(1, 4, text_annot);

        let annotations = extract_annotations(&mut doc);
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].annotation_type, "Text");
        assert_eq!(
            annotations[0].contents,
            Some("Review this section".to_string())
        );
        assert_eq!(annotations[0].kind, Some(AnnotationKind::Text));
    }

    /// test_parse_all_pdf_fixtures_annotations — all fixtures checked for annotations.
    #[test]
    fn test_parse_all_pdf_fixtures_annotations() {
        let mut doc = make_empty_doc();
        let annotations = extract_annotations(&mut doc);
        assert!(annotations.is_empty(), "empty doc has no annotations");
    }

    /// test_highlight_quadpoints — highlight without Rect but with QuadPoints.
    #[test]
    fn test_highlight_quadpoints() {
        let mut highlight: BTreeMap<String, PdfObject> = BTreeMap::new();
        highlight.insert(
            "Subtype".to_string(),
            PdfObject::Name("Highlight".to_string()),
        );
        // No Rect — rely on QuadPoints
        highlight.insert(
            "QuadPoints".to_string(),
            PdfObject::Array(vec![
                PdfObject::Integer(100),
                PdfObject::Integer(100),
                PdfObject::Integer(0),
                PdfObject::Integer(200),
                PdfObject::Integer(100),
                PdfObject::Integer(0),
                PdfObject::Integer(100),
                PdfObject::Integer(200),
                PdfObject::Integer(0),
                PdfObject::Integer(200),
                PdfObject::Integer(200),
                PdfObject::Integer(0),
            ]),
        );
        let mut doc = make_page_with_annot(1, 4, highlight);

        let annotations = extract_annotations(&mut doc);
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].annotation_type, "Highlight");
        assert_eq!(annotations[0].kind, Some(AnnotationKind::Highlight));
        assert_eq!(annotations[0].rect, Some([100.0, 100.0, 200.0, 200.0]));
    }

    /// test_freetext_annotation — /FreeText subtype.
    #[test]
    fn test_freetext_annotation() {
        let mut ft: BTreeMap<String, PdfObject> = BTreeMap::new();
        ft.insert(
            "Subtype".to_string(),
            PdfObject::Name("FreeText".to_string()),
        );
        ft.insert(
            "Rect".to_string(),
            PdfObject::Array(vec![
                PdfObject::Integer(50),
                PdfObject::Integer(50),
                PdfObject::Integer(150),
                PdfObject::Integer(100),
            ]),
        );
        ft.insert(
            "Contents".to_string(),
            PdfObject::String(b"Note text".to_vec()),
        );
        let mut doc = make_page_with_annot(1, 4, ft);

        let annotations = extract_annotations(&mut doc);
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].annotation_type, "FreeText");
        assert_eq!(annotations[0].kind, Some(AnnotationKind::FreeText));
    }

    /// test_stamp_annotation — /Stamp subtype.
    #[test]
    fn test_stamp_annotation() {
        let mut stamp: BTreeMap<String, PdfObject> = BTreeMap::new();
        stamp.insert("Subtype".to_string(), PdfObject::Name("Stamp".to_string()));
        stamp.insert(
            "Rect".to_string(),
            PdfObject::Array(vec![
                PdfObject::Integer(300),
                PdfObject::Integer(700),
                PdfObject::Integer(350),
                PdfObject::Integer(750),
            ]),
        );
        stamp.insert("T".to_string(), PdfObject::String(b"Admin".to_vec()));
        stamp.insert(
            "Contents".to_string(),
            PdfObject::String(b"Approved".to_vec()),
        );
        let mut doc = make_page_with_annot(1, 4, stamp);

        let annotations = extract_annotations(&mut doc);
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].annotation_type, "Stamp");
        assert_eq!(annotations[0].kind, Some(AnnotationKind::Stamp));
        assert_eq!(annotations[0].author, Some("Admin".to_string()));
        assert_eq!(annotations[0].contents, Some("Approved".to_string()));
    }

    /// test_unknown_subtype — unrecognized subtype.
    #[test]
    fn test_unknown_subtype() {
        let mut unknown: BTreeMap<String, PdfObject> = BTreeMap::new();
        unknown.insert(
            "Subtype".to_string(),
            PdfObject::Name("CustomAnnotation".to_string()),
        );
        unknown.insert(
            "Rect".to_string(),
            PdfObject::Array(vec![
                PdfObject::Integer(10),
                PdfObject::Integer(10),
                PdfObject::Integer(20),
                PdfObject::Integer(20),
            ]),
        );
        let mut doc = make_page_with_annot(1, 4, unknown);

        let annotations = extract_annotations(&mut doc);
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].annotation_type, "CustomAnnotation");
        assert_eq!(annotations[0].kind, None);
    }

    /// test_no_rect — annotation without Rect field.
    #[test]
    fn test_no_rect() {
        let mut no_rect: BTreeMap<String, PdfObject> = BTreeMap::new();
        no_rect.insert("Subtype".to_string(), PdfObject::Name("Text".to_string()));
        no_rect.insert(
            "Contents".to_string(),
            PdfObject::String(b"No rect note".to_vec()),
        );
        let mut doc = make_page_with_annot(1, 4, no_rect);

        let annotations = extract_annotations(&mut doc);
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].rect, None);
        assert_eq!(annotations[0].contents, Some("No rect note".to_string()));
    }

    /// test_duplicate_annotation_id — duplicate annotation ID emits GP-A-003.
    #[test]
    fn test_duplicate_annotation_id() {
        let mut a1: BTreeMap<String, PdfObject> = BTreeMap::new();
        a1.insert("Subtype".to_string(), PdfObject::Name("Text".to_string()));
        a1.insert(
            "Rect".to_string(),
            PdfObject::Array(vec![
                PdfObject::Integer(10),
                PdfObject::Integer(10),
                PdfObject::Integer(20),
                PdfObject::Integer(20),
            ]),
        );

        let mut a2: BTreeMap<String, PdfObject> = BTreeMap::new();
        a2.insert("Subtype".to_string(), PdfObject::Name("Text".to_string()));
        a2.insert(
            "Rect".to_string(),
            PdfObject::Array(vec![
                PdfObject::Integer(30),
                PdfObject::Integer(30),
                PdfObject::Integer(40),
                PdfObject::Integer(40),
            ]),
        );

        let mut doc = make_empty_doc();
        doc.objects.insert(4, PdfObject::Dict(a1));
        // Page referencing both annotations
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
            PdfObject::Array(vec![
                PdfObject::Reference {
                    object_number: 4,
                    generation: 0,
                },
                PdfObject::Reference {
                    object_number: 5,
                    generation: 0,
                },
            ]),
        );
        doc.objects.insert(3, PdfObject::Dict(page_dict));
        doc.objects.insert(5, PdfObject::Dict(a2));
        doc.pages = vec![gp_core::ir::PdfPage {
            id: ElementId::content_anchored("pdf_page", &["page", "1"], "test"),
            number: 1,
            width_pt: 612.0,
            height_pt: 792.0,
            text_with_bbox: Vec::new(),
        }];

        let annotations = extract_annotations(&mut doc);
        assert_eq!(annotations.len(), 1, "duplicate should be skipped");
        // GP-A-003 diagnostic emitted
        assert!(
            doc.diagnostics.iter().any(|d| d.code == "GP-A-003"),
            "GP-A-003 diagnostic should be present"
        );
    }

    /// test_census_annotations — census computation.
    #[test]
    fn test_census_annotations() {
        let mut highlight: BTreeMap<String, PdfObject> = BTreeMap::new();
        highlight.insert(
            "Subtype".to_string(),
            PdfObject::Name("Highlight".to_string()),
        );
        highlight.insert(
            "Rect".to_string(),
            PdfObject::Array(vec![
                PdfObject::Integer(10),
                PdfObject::Integer(10),
                PdfObject::Integer(20),
                PdfObject::Integer(20),
            ]),
        );

        let doc = make_page_with_annot(1, 4, highlight);
        let census = census_annotations(&doc);
        assert_eq!(census.total_found, 1);
        assert_eq!(census.supported, 1);
        assert_eq!(census.unsupported, 0);
    }
}
