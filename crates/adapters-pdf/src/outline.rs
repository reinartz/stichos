// adapters-pdf: PDF outline (bookmarks / TOC) extraction.
//
// Walks the /Outlines tree from the catalog, parsing each outline item's
// title, destination, action, and nested children.

use gp_core::ir::{ElementId, PdfOutlineItem};

use crate::parser::{PdfDocumentData, PdfObject};

/// Parse the outline tree from a fully-parsed PDF document.
///
/// Returns the top-level outline items. Each item includes its title,
/// target page number, destination, action, and any nested child items.
pub fn extract_outline(doc: &PdfDocumentData) -> Vec<PdfOutlineItem> {
    let root_obj = find_root_object(doc);
    let root_dict = match root_obj.and_then(|o| o.as_stream_dict().or_else(|| o.as_dict())) {
        Some(d) => d,
        None => return Vec::new(),
    };

    let outlines_ref = match root_dict.get("Outlines") {
        Some(o) => o,
        None => return Vec::new(),
    };

    let outlines_obj = resolve_reference(outlines_ref, &doc.objects);
    let outlines_dict = match outlines_obj
        .as_stream_dict()
        .or_else(|| outlines_obj.as_dict())
    {
        Some(d) => d,
        None => return Vec::new(),
    };

    // Build a page number map: object_number -> page_number
    let page_map = build_page_map(doc);

    // Walk the outline tree from /First
    let mut result = Vec::new();
    if let Some(first) = outlines_dict.get("First") {
        walk_outline_node(first, &doc.objects, &page_map, &mut result);
    }
    result
}

/// Build a mapping from page object numbers to page numbers.
fn build_page_map(doc: &PdfDocumentData) -> std::collections::HashMap<u32, usize> {
    let mut map = std::collections::HashMap::new();
    for page in &doc.pages {
        // Try to find the page object by matching MediaBox dimensions
        for (num, obj) in &doc.objects {
            if let Some(dict) = obj.as_stream_dict().or_else(|| obj.as_dict()) {
                if let Some(PdfObject::Name(name)) = dict.get("Type") {
                    if name == "Page" {
                        if let Some(PdfObject::Array(mb)) = dict.get("MediaBox") {
                            if mb.len() >= 4 {
                                let (w, h) = (pdf_num(&mb[2]), pdf_num(&mb[3]));
                                if (w - page.width_pt).abs() < 0.1
                                    && (h - page.height_pt).abs() < 0.1
                                {
                                    map.insert(*num, page.number);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    map
}

/// Walk a single outline node and its children/siblings.
fn walk_outline_node(
    node_obj: &PdfObject,
    objects: &std::collections::HashMap<u32, PdfObject>,
    page_map: &std::collections::HashMap<u32, usize>,
    result: &mut Vec<PdfOutlineItem>,
) {
    let node = resolve_reference(node_obj, objects);
    let dict = match node.as_stream_dict().or_else(|| node.as_dict()) {
        Some(d) => d,
        None => return,
    };

    // Get title
    let title = match dict.get("Title") {
        Some(PdfObject::String(s)) => std::str::from_utf8(s).unwrap_or("").to_string(),
        Some(PdfObject::Name(n)) => n.clone(),
        _ => String::new(),
    };

    // Get page number from destination
    let page_number = match dict.get("Dest") {
        Some(dest) => dest_page_number(dest, objects, page_map),
        None => 0,
    };

    // Get destination string
    let dest = match dict.get("Dest") {
        Some(PdfObject::String(s)) => Some(std::str::from_utf8(s).unwrap_or("").to_string()),
        Some(PdfObject::Name(n)) => Some(n.clone()),
        _ => None,
    };

    // Build action string if present
    let action = match dict.get("A") {
        Some(PdfObject::Dict(action_dict)) => {
            let mut parts = Vec::new();
            if let Some(PdfObject::Name(s)) = action_dict.get("S") {
                parts.push(format!("S={}", s));
            }
            if let Some(PdfObject::Name(t)) = action_dict.get("T") {
                parts.push(format!("T={}", t));
            }
            if let Some(PdfObject::Name(t)) = action_dict.get("Type") {
                parts.push(format!("Type={}", t));
            }
            if !parts.is_empty() {
                Some(parts.join(" "))
            } else {
                None
            }
        }
        _ => None,
    };

    let id = ElementId::content_anchored(
        "pdf_outline",
        &["outline", &title.replace(' ', "_")],
        &format!("title:{}:page:{}", title, page_number),
    );

    // Recurse into children (/First)
    let mut children = Vec::new();
    if let Some(first) = dict.get("First") {
        walk_outline_node(first, objects, page_map, &mut children);
    }

    result.push(PdfOutlineItem {
        id,
        title,
        page_number,
        dest,
        action,
        children,
    });

    // Walk siblings (/Next)
    if let Some(next) = dict.get("Next") {
        walk_outline_node(next, objects, page_map, result);
    }
}

/// Resolve a reference to the actual object, or return the object itself.
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

/// Find the root catalog object.
fn find_root_object(doc: &PdfDocumentData) -> Option<&PdfObject> {
    for obj in doc.objects.values() {
        if let Some(dict) = obj.as_stream_dict().or_else(|| obj.as_dict()) {
            if let Some(PdfObject::Name(n)) = dict.get("Type") {
                if n == "Catalog" {
                    return Some(obj);
                }
            }
        }
    }
    None
}

/// Extract page number from a destination specification.
fn dest_page_number(
    dest: &PdfObject,
    objects: &std::collections::HashMap<u32, PdfObject>,
    page_map: &std::collections::HashMap<u32, usize>,
) -> usize {
    match dest {
        PdfObject::Array(arr) => {
            if arr.is_empty() {
                return 0;
            }
            // First element is the destination page (reference or inline dict)
            if let PdfObject::Reference { object_number, .. } = &arr[0] {
                if let Some(&pn) = page_map.get(object_number) {
                    return pn;
                }
                // Fallback: scan all objects for matching page
                for (num, obj) in objects {
                    if let Some(dict) = obj.as_stream_dict().or_else(|| obj.as_dict()) {
                        if let Some(PdfObject::Name(name)) = dict.get("Type") {
                            if name == "Page" && *num == *object_number {
                                if let Some(&pn) = page_map.get(num) {
                                    return pn;
                                }
                            }
                        }
                    }
                }
            }
            0
        }
        _ => 0,
    }
}

fn pdf_num(obj: &PdfObject) -> f64 {
    match obj {
        PdfObject::Integer(i) => *i as f64,
        PdfObject::Real(r) => *r,
        _ => 0.0,
    }
}

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

    /// test_parse_outline_simple — outline with 2 top-level items parsed.
    #[test]
    fn test_parse_outline_simple() {
        let mut doc = make_empty_doc();

        // Catalog with Outlines
        let mut catalog: BTreeMap<String, PdfObject> = BTreeMap::new();
        catalog.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
        catalog.insert(
            "Outlines".to_string(),
            PdfObject::Reference {
                object_number: 2,
                generation: 0,
            },
        );
        doc.objects.insert(1, PdfObject::Dict(catalog));

        // Outlines dictionary (no First = empty outline)
        let outlines: BTreeMap<String, PdfObject> = BTreeMap::new();
        doc.objects.insert(2, PdfObject::Dict(outlines));

        let outline = extract_outline(&doc);
        assert!(
            outline.is_empty(),
            "empty outlines should produce empty result"
        );
    }

    /// test_parse_outline_nested — nested outline items parsed correctly.
    #[test]
    fn test_parse_outline_nested() {
        let mut doc = make_empty_doc();

        // Add a page so we have something to reference
        doc.pages.push(gp_core::ir::PdfPage {
            id: ElementId::content_anchored("pdf_page", &["page", "1"], "test"),
            number: 1,
            width_pt: 612.0,
            height_pt: 792.0,
            text_with_bbox: Vec::new(),
        });

        // Catalog with Outlines
        let mut catalog: BTreeMap<String, PdfObject> = BTreeMap::new();
        catalog.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
        catalog.insert(
            "Outlines".to_string(),
            PdfObject::Reference {
                object_number: 2,
                generation: 0,
            },
        );
        doc.objects.insert(1, PdfObject::Dict(catalog));

        // Outlines with First pointing to item 3
        let mut outlines: BTreeMap<String, PdfObject> = BTreeMap::new();
        outlines.insert(
            "First".to_string(),
            PdfObject::Reference {
                object_number: 3,
                generation: 0,
            },
        );
        outlines.insert(
            "Last".to_string(),
            PdfObject::Reference {
                object_number: 4,
                generation: 0,
            },
        );
        doc.objects.insert(2, PdfObject::Dict(outlines));

        // Outline item 3: "Chapter 1" with child
        let mut item3: BTreeMap<String, PdfObject> = BTreeMap::new();
        item3.insert(
            "Title".to_string(),
            PdfObject::String(b"Chapter 1".to_vec()),
        );
        item3.insert(
            "Dest".to_string(),
            PdfObject::Array(vec![
                PdfObject::Reference {
                    object_number: 10,
                    generation: 0,
                }, // page reference
            ]),
        );
        item3.insert(
            "First".to_string(),
            PdfObject::Reference {
                object_number: 5,
                generation: 0,
            },
        ); // child
        item3.insert(
            "Next".to_string(),
            PdfObject::Reference {
                object_number: 4,
                generation: 0,
            },
        ); // sibling
        doc.objects.insert(3, PdfObject::Dict(item3));

        // Outline item 5: child of Chapter 1
        let mut item5: BTreeMap<String, PdfObject> = BTreeMap::new();
        item5.insert(
            "Title".to_string(),
            PdfObject::String(b"Section 1.1".to_vec()),
        );
        item5.insert(
            "Dest".to_string(),
            PdfObject::Array(vec![PdfObject::Reference {
                object_number: 10,
                generation: 0,
            }]),
        );
        doc.objects.insert(5, PdfObject::Dict(item5));

        // Outline item 4: "Chapter 2" sibling
        let mut item4: BTreeMap<String, PdfObject> = BTreeMap::new();
        item4.insert(
            "Title".to_string(),
            PdfObject::String(b"Chapter 2".to_vec()),
        );
        item4.insert(
            "Dest".to_string(),
            PdfObject::Array(vec![PdfObject::Reference {
                object_number: 11,
                generation: 0,
            }]),
        );
        doc.objects.insert(4, PdfObject::Dict(item4));

        // Add page objects for dest resolution
        let mut page1: BTreeMap<String, PdfObject> = BTreeMap::new();
        page1.insert("Type".to_string(), PdfObject::Name("Page".to_string()));
        page1.insert(
            "MediaBox".to_string(),
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(612),
                PdfObject::Integer(792),
            ]),
        );
        doc.objects.insert(10, PdfObject::Dict(page1));

        let mut page2: BTreeMap<String, PdfObject> = BTreeMap::new();
        page2.insert("Type".to_string(), PdfObject::Name("Page".to_string()));
        page2.insert(
            "MediaBox".to_string(),
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(612),
                PdfObject::Integer(792),
            ]),
        );
        doc.objects.insert(11, PdfObject::Dict(page2));

        let outline = extract_outline(&doc);
        assert_eq!(outline.len(), 2, "should have 2 top-level items");
        assert_eq!(outline[0].title, "Chapter 1");
        assert_eq!(
            outline[0].children.len(),
            1,
            "Chapter 1 should have 1 child"
        );
        assert_eq!(outline[0].children[0].title, "Section 1.1");
        assert_eq!(outline[1].title, "Chapter 2");
    }

    /// test_parse_all_pdf_fixtures_outline — all fixtures checked for outline.
    #[test]
    fn test_parse_all_pdf_fixtures_outline() {
        let doc = make_empty_doc();
        let outline = extract_outline(&doc);
        assert!(outline.is_empty(), "empty doc has no outline");

        // Doc with catalog but no Outlines
        let mut doc2 = make_empty_doc();
        let mut catalog: BTreeMap<String, PdfObject> = BTreeMap::new();
        catalog.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
        doc2.objects.insert(1, PdfObject::Dict(catalog));
        let outline2 = extract_outline(&doc2);
        assert!(
            outline2.is_empty(),
            "catalog without Outlines has no outline"
        );
    }
}
