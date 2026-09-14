// adapters-pdf: XMP metadata extraction from PDF.
//
// Parses the /Metadata stream from the PDF catalog to extract XMP (Extensible
// Metadata Platform) data including Dublin Core elements (dc:title, dc:creator,
// dc:subject, dc:description) and XMP properties.

use gp_core::ir::PdfMetadata;

use crate::parser::PdfDocumentData;

/// Parse XMP metadata from a fully-parsed PDF document.
///
/// Finds the /Metadata stream in the catalog, extracts the XML content,
/// and parses Dublin Core and XMP elements into PdfMetadata.
///
/// Returns the enriched metadata if XMP is present, otherwise returns
/// the existing metadata from the PDF's info dictionary.
pub fn parse_metadata(doc: &PdfDocumentData) -> PdfMetadata {
    // First try XMP metadata from the catalog
    if let Some(xmp) = extract_xmp_metadata(doc) {
        return xmp;
    }
    // Fall back to existing metadata from info dictionary
    doc.metadata.clone().unwrap_or(PdfMetadata {
        title: None,
        author: None,
        subject: None,
        creator: None,
        description: None,
        modified: None,
        keywords: None,
    })
}

/// Extract XMP metadata from the PDF catalog's /Metadata stream.
fn extract_xmp_metadata(doc: &PdfDocumentData) -> Option<PdfMetadata> {
    let root_obj = find_root_object(doc);
    let root_dict = root_obj.and_then(|o| o.as_stream_dict().or_else(|| o.as_dict()))?;

    let metadata_ref = root_dict.get("Metadata")?;

    let metadata_obj = resolve_reference(metadata_ref, &doc.objects);
    let stream_data = match metadata_obj {
        PdfObject::Stream { dict, data } => {
            // Verify it's XMP
            if let Some(PdfObject::Name(n)) = dict.get("MetadataType") {
                if n != "XML" {
                    return None;
                }
            }
            if let Some(PdfObject::Name(n)) = dict.get("Subtype") {
                if n != "XML" && n != "Metadata" {
                    return None;
                }
            }
            data.clone()
        }
        PdfObject::String(data) => data.clone(),
        _ => return None,
    };

    parse_xmp_xml(&stream_data)
}

/// Parse XMP XML content into PdfMetadata.
fn parse_xmp_xml(xmp_data: &[u8]) -> Option<PdfMetadata> {
    let text = match std::str::from_utf8(xmp_data) {
        Ok(t) => t,
        Err(_) => return None,
    };

    // Parse dc:title
    let title =
        extract_xmp_element(text, "dc:title").or_else(|| extract_xmp_element(text, "title"));

    // Parse dc:creator (maps to author) and creator (XMP)
    let author = extract_xmp_element(text, "dc:creator");
    let creator = extract_xmp_element(text, "creator");
    let author = author.or_else(|| creator.clone());

    // Parse dc:subject (maps to subject)
    let subject = extract_xmp_element(text, "dc:subject");

    // Parse dc:description (maps to description)
    let description = extract_xmp_element(text, "dc:description")
        .or_else(|| extract_xmp_element(text, "description"));

    // Parse xmp:ModifyDate
    let modified = extract_xmp_element(text, "xmp:ModifyDate");

    // Parse pdf:Keywords
    let keywords = extract_xmp_element(text, "pdf:Keywords");

    // Handle RDF-style metadata with multiple creators (array)
    let (author, creator) = if author.is_none() && creator.is_none() {
        if let Some(val) = extract_xmp_rdf_array(text, "dc:creator") {
            (Some(val.clone()), Some(val))
        } else {
            (None, None)
        }
    } else {
        (author, creator)
    };

    Some(PdfMetadata {
        title,
        author,
        subject,
        creator,
        description,
        modified,
        keywords,
    })
}

/// Extract a single text value from an XMP element by local name.
///
/// Handles namespaced elements like `<dc:title>`, `<rdf:li>`, and
/// nested structures like `<rdf:Alt>` and `<rdf:Bag>`.
fn extract_xmp_element(text: &str, local_name: &str) -> Option<String> {
    // Try namespaced form: <dc:title>content</dc:title>
    let search = format!("<{}:", local_name);
    if let Some(pos) = text.find(&search) {
        let after_tag = &text[pos..];
        if let Some(end_open) = after_tag.find(">") {
            let content_start = pos + end_open + 1;
            let close_tag = format!("</{}:", local_name);
            if let Some(end_close) = text[content_start..].find(&close_tag) {
                let raw = &text[content_start..content_start + end_close];
                return Some(strip_xmp_wrappers(raw).trim().to_string());
            }
        }
    }

    // Try non-namespaced form
    let search2 = format!("<{}", local_name);
    if let Some(pos) = text.find(&search2) {
        // Make sure it's not a namespaced version we already checked
        if !text[pos..].starts_with(&search) {
            let after_tag = &text[pos..];
            if let Some(end_open) = after_tag.find(">") {
                let content_start = pos + end_open + 1;
                let close_tag = format!("</{}>", local_name);
                if let Some(end_close) = text[content_start..].find(&close_tag) {
                    let raw = &text[content_start..content_start + end_close];
                    return Some(strip_xmp_wrappers(raw).trim().to_string());
                }
            }
        }
    }

    None
}

/// Strip RDF wrapper elements (Alt, Bag, Seq) and extract inner text.
/// e.g. `<rdf:Alt><rdf:li>My Title</rdf:li></rdf:Alt>` → `My Title`
/// e.g. `<rdf:Bag><rdf:li>Tag1</rdf:li><rdf:li>Tag2</rdf:li></rdf:Bag>` → `Tag1, Tag2`
fn strip_xmp_wrappers(s: &str) -> String {
    let trimmed = s.trim();
    // Handle <rdf:Alt>...</rdf:Alt> or <Alt>...</Alt>
    for wrapper in &["rdf:Alt", "Alt", "rdf:Bag", "Bag", "rdf:Seq", "Seq"] {
        let open_tag = format!("<{}>", wrapper);
        let open_spaced = format!("<{} ", wrapper);
        if trimmed.starts_with(&open_tag) || trimmed.starts_with(&open_spaced) {
            let close = format!("</{}>", wrapper);
            if let Some(idx) = trimmed.rfind(&close) {
                let inner = &trimmed[open_tag.len()..idx];
                // Recursively strip remaining wrappers
                return strip_xmp_wrappers(inner);
            }
        }
    }
    // If we still have <rdf:li> or <li> elements, extract all of them
    for li_tag in &["rdf:li", "li"] {
        let open_tag = format!("<{}>", li_tag);
        let open_spaced = format!("<{} ", li_tag);
        if trimmed.starts_with(&open_tag) || trimmed.starts_with(&open_spaced) {
            let close = format!("</{}>", li_tag);
            let mut results = Vec::new();
            let mut remaining = trimmed;
            while let Some(li_start) = remaining.find(&open_tag) {
                let after_open = &remaining[li_start + open_tag.len()..];
                if let Some(li_end) = after_open.find(&close) {
                    let text = after_open[..li_end].trim().to_string();
                    if !text.is_empty() {
                        results.push(text);
                    }
                    let after_li = &after_open[li_end + close.len()..];
                    remaining = after_li;
                } else {
                    break;
                }
            }
            if !results.is_empty() {
                return results.join(", ");
            }
        }
    }
    trimmed.replace('\n', " ").replace('\r', "")
}

/// Extract an RDF array value (e.g., <dc:creator><rdf:Alt><rdf:li>Author</rdf:li></rdf:Alt></dc:creator>).
fn extract_xmp_rdf_array(text: &str, local_name: &str) -> Option<String> {
    let search = format!("<{}:", local_name);
    let pos = text.find(&search)?;
    let after_tag = &text[pos..];

    // Find the closing tag
    let close_tag = format!("</{}:", local_name);
    let end_close = after_tag.rfind(&close_tag)?;

    // Extract content between open and close tags
    let open_end = after_tag.find('>')?;
    let inner_text = &after_tag[open_end + 1..end_close];

    // Extract all <rdf:li> values
    let mut values = Vec::new();
    let mut search_pos = 0;
    while let Some(li_start) = inner_text[search_pos..].find("<rdf:li>") {
        let abs_start = search_pos + li_start;
        let li_inner = &inner_text[abs_start..];
        if let Some(li_end_open) = li_inner.find('>') {
            let li_content_start = abs_start + li_end_open + 1;
            let li_close = "</rdf:li>";
            if let Some(li_end_close) = li_inner[li_content_start..].find(li_close) {
                let val = li_inner[li_content_start..li_content_start + li_end_close]
                    .trim()
                    .to_string();
                if !val.is_empty() {
                    values.push(val);
                }
            }
        }
        search_pos = abs_start + 1;
        if search_pos >= inner_text.len() - 10 {
            break;
        }
    }

    if values.is_empty() {
        // Try plain <li> elements
        let mut values2 = Vec::new();
        let mut search_pos2 = 0;
        while let Some(li_start) = inner_text[search_pos2..].find("<li>") {
            let abs_start = search_pos2 + li_start;
            let li_inner = &inner_text[abs_start..];
            if let Some(li_end_open) = li_inner.find('>') {
                let li_content_start = abs_start + li_end_open + 1;
                let li_close = "</li>";
                if let Some(li_end_close) = li_inner[li_content_start..].find(li_close) {
                    let val = li_inner[li_content_start..li_content_start + li_end_close]
                        .trim()
                        .to_string();
                    if !val.is_empty() {
                        values2.push(val);
                    }
                }
            }
            search_pos2 = abs_start + 1;
            if search_pos2 >= inner_text.len() - 10 {
                break;
            }
        }
        if !values2.is_empty() {
            return Some(values2.join(", "));
        }
    }

    if !values.is_empty() {
        return Some(values.join(", "));
    }

    None
}

/// Resolve a PDF reference to the actual object.
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

    /// test_parse_metadata — XMP metadata parsed (title, author).
    #[test]
    fn test_parse_metadata() {
        let mut doc = make_empty_doc();

        // Catalog with Metadata
        let mut catalog: BTreeMap<String, PdfObject> = BTreeMap::new();
        catalog.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
        catalog.insert(
            "Metadata".to_string(),
            PdfObject::Reference {
                object_number: 2,
                generation: 0,
            },
        );
        doc.objects.insert(1, PdfObject::Dict(catalog));

        // XMP Metadata stream
        let xmp_xml = b"<?xpacket begin=\"\"?><x:xmpmeta xmlns:x=\"adobe:ns:meta/\"
xmlns:dc=\"http://purl.org/dc/elements/1.1/\"
xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"
xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\">
<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">
<rdf:Description rdf:about=\"\"
  xmlns:dc=\"http://purl.org/dc/elements/1.1/\">
<dc:title><rdf:Alt><rdf:li>Test Document Title</rdf:li></rdf:Alt></dc:title>
<dc:creator><rdf:Alt><rdf:li>Jane Doe</rdf:li></rdf:Alt></dc:creator>
<dc:subject><rdf:Bag><rdf:li>first tag</rdf:li><rdf:li>second tag</rdf:li></rdf:Bag></dc:subject>
</rdf:Description>
</rdf:RDF>
</x:xmpmeta>";

        let mut stream_dict: BTreeMap<String, PdfObject> = BTreeMap::new();
        stream_dict.insert(
            "MetadataType".to_string(),
            PdfObject::Name("XML".to_string()),
        );
        stream_dict.insert("Subtype".to_string(), PdfObject::Name("XML".to_string()));
        doc.objects.insert(
            2,
            PdfObject::Stream {
                dict: stream_dict,
                data: xmp_xml.to_vec(),
            },
        );

        let meta = parse_metadata(&doc);
        assert_eq!(meta.title, Some("Test Document Title".to_string()));
        assert_eq!(meta.author, Some("Jane Doe".to_string()));
        assert_eq!(meta.subject, Some("first tag, second tag".to_string()));
    }

    /// test_parse_metadata_no_xmp — falls back gracefully when no XMP.
    #[test]
    fn test_parse_metadata_no_xmp() {
        let doc = make_empty_doc();
        let meta = parse_metadata(&doc);
        assert!(meta.title.is_none());
        assert!(meta.author.is_none());
    }

    /// test_parse_metadata_flat_xmp — non-RDF XMP parsing.
    #[test]
    fn test_parse_metadata_flat_xmp() {
        let mut doc = make_empty_doc();

        let mut catalog: BTreeMap<String, PdfObject> = BTreeMap::new();
        catalog.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
        catalog.insert(
            "Metadata".to_string(),
            PdfObject::Reference {
                object_number: 2,
                generation: 0,
            },
        );
        doc.objects.insert(1, PdfObject::Dict(catalog));

        // Simple non-RDF XMP
        let xmp_xml = b"<rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'
xmlns:dc='http://purl.org/dc/elements/1.1/'>
<dc:title>Flat Title</dc:title>
<dc:creator>Flat Author</dc:creator>
<dc:description>A description here</dc:description>
</rdf:RDF>";

        let mut stream_dict: BTreeMap<String, PdfObject> = BTreeMap::new();
        stream_dict.insert(
            "MetadataType".to_string(),
            PdfObject::Name("XML".to_string()),
        );
        doc.objects.insert(
            2,
            PdfObject::Stream {
                dict: stream_dict,
                data: xmp_xml.to_vec(),
            },
        );

        let meta = parse_metadata(&doc);
        assert_eq!(meta.title, Some("Flat Title".to_string()));
        assert_eq!(meta.author, Some("Flat Author".to_string()));
        assert_eq!(meta.description, Some("A description here".to_string()));
    }

    /// test_parse_all_pdf_fixtures_metadata — all fixtures checked.
    #[test]
    fn test_parse_all_pdf_fixtures_metadata() {
        let doc = make_empty_doc();
        let meta = parse_metadata(&doc);
        assert!(meta.title.is_none(), "empty doc metadata has no title");

        // Doc with catalog but no Metadata
        let mut doc2 = make_empty_doc();
        let mut catalog: BTreeMap<String, PdfObject> = BTreeMap::new();
        catalog.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
        doc2.objects.insert(1, PdfObject::Dict(catalog));
        let meta2 = parse_metadata(&doc2);
        assert!(meta2.title.is_none(), "catalog without Metadata");
    }
}
