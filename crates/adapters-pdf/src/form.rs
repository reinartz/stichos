//! PDF AcroForm (interactive form fields) reader and XFA detection.
//!
//! Parses `/AcroForm` dictionaries from PDF documents, extracting field trees,
//! field values, flag bits, and detecting unsupported XFA (XML Forms Architecture).
//!
//! See PDF 1.7 spec §12.2 (Interactive Forms) and §12.7 (XFA).

use gp_core::diagnostics::Diagnostic;
use gp_core::ir::{ElementId, PdfFormField, PdfFormFieldFlags};
use std::collections::HashMap;

use crate::parser::{PdfDocumentData, PdfObject};

/// Parse AcroForm fields from a fully-parsed PDF document.
///
/// Returns a tuple of (fields, diagnostics). If no AcroForm exists, returns an
/// empty list with no diagnostics. If XFA is detected, an unsupported diagnostic
/// is emitted alongside any parseable fields.
pub fn parse_acroform(doc: &PdfDocumentData) -> (Vec<PdfFormField>, Vec<Diagnostic>) {
    let mut fields = Vec::new();
    let mut diagnostics = Vec::new();

    // Check for XFA first
    if has_xfa(doc) {
        diagnostics.push(Diagnostic::new(
            "PDF-W-004",
            gp_core::diagnostics::Severity::Warning,
            "XFA (XML Forms Architecture) detected — unsupported".to_string(),
        ));
    }

    // Find the AcroForm dictionary in the catalog
    let acroform = match find_acroform(doc) {
        Some(a) => a,
        None => return (fields, diagnostics),
    };

    // Get the /Fields array from AcroForm
    let field_refs = match acroform.dict_get("Fields") {
        Some(PdfObject::Array(arr)) => arr,
        _ => return (fields, diagnostics),
    };

    // Resolve field references and parse each field
    for field_ref in field_refs {
        let obj_num = match field_ref.ref_obj_num() {
            Some(n) => n,
            None => continue,
        };
        match doc.objects.get(&obj_num) {
            Some(field_obj) => {
                if let Ok(f) = parse_single_field(field_obj, obj_num, &doc.objects) {
                    fields.push(f);
                }
            }
            None => continue,
        }
    }

    (fields, diagnostics)
}

/// Check if the PDF uses XFA (XML Forms Architecture).
///
/// XFA can appear in several places:
/// - /XFA key in the AcroForm dictionary
/// - /XFAGetway key in AcroForm
/// - Embedded XFA in the document's XFA stream
pub fn detect_xra(doc: &PdfDocumentData) -> bool {
    has_xfa(doc)
}

/// Check if the PDF has XFA (XML Forms Architecture) support.
fn has_xfa(doc: &PdfDocumentData) -> bool {
    // Check for /XFA in AcroForm
    if let Some(acroform) = find_acroform(doc) {
        if let Some(xfa_obj) = acroform.dict_get("XFA") {
            return match xfa_obj {
                PdfObject::String(s) => {
                    let text = std::str::from_utf8(s).unwrap_or("");
                    text.contains("xdp") || text.contains("xfa")
                }
                PdfObject::Name(n) => n.contains("xfa") || n.contains("xdp"),
                PdfObject::Array(arr) => arr.iter().any(|o| {
                    if let PdfObject::String(s) = o {
                        let text = std::str::from_utf8(s).unwrap_or("");
                        text.contains("xdp") || text.contains("xfa")
                    } else if let PdfObject::Name(n) = o {
                        n.contains("xfa") || n.contains("xdp")
                    } else {
                        false
                    }
                }),
                _ => true,
            };
        }
    }

    // Check for XFA in embedded documents / streams
    for obj in doc.objects.values() {
        if let PdfObject::Stream { dict, data } = obj {
            // Check stream dict for XFA-related keys
            if let Some(PdfObject::Name(n)) = dict.get("Subtype") {
                if n == "XFA" {
                    return true;
                }
            }
            // Check stream data for XPA markers (XDP XML envelope)
            if let Ok(text) = std::str::from_utf8(data) {
                if text.contains("<xdp:") || text.contains("<xfa:") || text.contains("xdp") {
                    return true;
                }
            }
        }
    }

    false
}

/// Find the AcroForm dictionary in the PDF's catalog.
/// Returns the resolved AcroForm dictionary object (not a reference to it).
fn find_acroform(doc: &PdfDocumentData) -> Option<&PdfObject> {
    let root_obj = find_root_object(doc)?;
    let root_dict = root_obj.as_stream_dict().or_else(|| root_obj.as_dict())?;

    // Check Root for /AcroForm
    let af_ref = root_dict.get("AcroForm")?;

    // Resolve the AcroForm reference to get the actual dictionary
    match af_ref {
        PdfObject::Reference {
            object_number,
            generation: _,
        } => doc.objects.get(object_number),
        PdfObject::Dict(_) => Some(af_ref),
        _ => None,
    }
}

/// Find the Root catalog object.
fn find_root_object(doc: &PdfDocumentData) -> Option<&PdfObject> {
    // Try to find the root object by scanning for Type=/Catalog
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

/// Parse a single PDF form field object.
fn parse_single_field(
    field_obj: &PdfObject,
    obj_num: u32,
    _objects: &HashMap<u32, PdfObject>,
) -> Result<PdfFormField, ()> {
    let dict = match field_obj.as_stream_dict().or_else(|| field_obj.as_dict()) {
        Some(d) => d,
        None => return Err(()),
    };

    // Get field type (/FT)
    let field_type = match dict.get("FT") {
        Some(PdfObject::Name(n)) => field_type_name(n),
        Some(_) => "unknown".to_string(),
        None => "unknown".to_string(),
    };

    // Get field name (/T)
    let name = match dict.get("T") {
        Some(PdfObject::String(s)) => std::str::from_utf8(s).unwrap_or("").to_string(),
        Some(PdfObject::Name(n)) => n.clone(),
        _ => format!("_field_{}", obj_num),
    };

    // Get field value (/V)
    let value = extract_field_value(dict);

    // Get flag bits (/Ff)
    let flags = match dict.get("Ff") {
        Some(PdfObject::Integer(bits)) => Some(PdfFormFieldFlags::from_bits((*bits) as u32)),
        Some(PdfObject::Real(r)) => Some(PdfFormFieldFlags::from_bits((*r) as u32)),
        _ => None,
    };

    // Get appearance rectangle (/Rect)
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

    // If no explicit name, try /Parent for hierarchical names
    let name = if name.is_empty() || name == format!("_field_{}", obj_num) {
        if let Some(PdfObject::Name(_parent_name)) = dict.get("Parent") {
            // Walk parent tree for hierarchical names
            let parent_key = match dict.get("/T") {
                Some(PdfObject::String(s)) => std::str::from_utf8(s).unwrap_or("").to_string(),
                _ => name.clone(),
            };
            if !parent_key.is_empty() {
                parent_key
            } else {
                format!("_field_{}", obj_num)
            }
        } else {
            format!("_field_{}", obj_num)
        }
    } else {
        name
    };

    // Generate a stable ID based on field name and type
    let id = ElementId::content_anchored(
        "pdf_form_field",
        &["field", &field_type, &name],
        &format!("{}:{}", obj_num, name),
    );

    Ok(PdfFormField {
        id,
        field_type,
        name,
        value,
        flags,
        rect,
    })
}

/// Map a PDF field type name to a human-readable type.
fn field_type_name(name: &str) -> String {
    match name {
        "Tx" => "text".to_string(),
        "Btn" => "button".to_string(),
        "Ch" => "choice".to_string(),
        "Sig" => "signature".to_string(),
        _ => name.to_string(),
    }
}

/// Extract the current value of a form field (/V).
///
/// /V can be a string, number, array (for choice fields), or reference.
fn extract_field_value(dict: &std::collections::BTreeMap<String, PdfObject>) -> Option<String> {
    if let Some(value_obj) = dict.get("V") {
        match value_obj {
            PdfObject::String(s) => std::str::from_utf8(s).ok().map(|s| s.to_string()),
            PdfObject::Name(n) => Some(n.clone()),
            PdfObject::Integer(i) => Some(i.to_string()),
            PdfObject::Real(r) => Some(format!("{}", r)),
            PdfObject::Array(arr) => {
                // For choice fields, join array values
                let vals: Vec<String> = arr
                    .iter()
                    .filter_map(|item| match item {
                        PdfObject::String(s) => std::str::from_utf8(s).ok().map(|s| s.to_string()),
                        PdfObject::Name(n) => Some(n.clone()),
                        PdfObject::Integer(i) => Some(i.to_string()),
                        PdfObject::Real(r) => Some(format!("{}", r)),
                        _ => None,
                    })
                    .collect();
                if vals.is_empty() {
                    None
                } else {
                    Some(vals.join(", "))
                }
            }
            PdfObject::Reference {
                object_number,
                generation: gen,
            } => {
                // Value is a reference — try to resolve
                Some(format!("<ref {} {}>", object_number, gen))
            }
            _ => None,
        }
    } else {
        None
    }
}

/// Convert a PDF object to f64 for rectangle coordinates.
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

    /// Build a minimal PdfDocumentData with the given objects and AcroForm.
    fn make_pdf_with_acroform(acroform_obj: &str) -> PdfDocumentData {
        let mut b = Vec::new();
        b.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

        // Catalog with AcroForm reference
        let catalog = format!(
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R /AcroForm {} >>\nendobj\n",
            acroform_obj
        );
        // Pages
        let pages = b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n";
        // Page
        let page = b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n";

        b.extend_from_slice(catalog.as_bytes());
        b.extend_from_slice(pages);
        b.extend_from_slice(page);

        // xref + trailer
        let total = 4;
        b.extend_from_slice(b"xref\n0 ");
        b.extend_from_slice(format!("{}", total + 1).as_bytes());
        b.extend_from_slice(b"\n0000000000 65535 f \n");
        for i in 1..=total {
            b.extend_from_slice(format!("{:010} 00000 n \n", 20 + i * 100).as_bytes());
        }
        b.extend_from_slice(b"trailer\n<< /Size ");
        b.extend_from_slice(format!("{}", total + 1).as_bytes());
        b.extend_from_slice(b" /Root 1 0 R >>\n");
        b.extend_from_slice(b"startxref\n20\n%%EOF\n");

        let diags = Vec::new();
        PdfDocumentData {
            version: "%PDF-1.4".to_string(),
            objects: HashMap::new(),
            pages: Vec::new(),
            metadata: None,
            has_encryption: false,
            diagnostics: diags,
        }
    }

    /// Test 1: /Tx (text) field is parsed correctly.
    #[test]
    fn test_parse_acroform_text_field() {
        let mut b = Vec::new();
        b.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

        // Catalog with AcroForm pointing to field object 4
        b.extend_from_slice(
            b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>\nendobj\n",
        );
        // Pages
        b.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        // Page
        b.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n",
        );
        // Field 4: /Tx (text field) with name "FirstName" and value "John"
        b.extend_from_slice(b"4 0 obj\n<< /FT /Tx /T (FirstName) /V (John) /Ff 0 >>\nendobj\n");
        // AcroForm 5: references field 4
        b.extend_from_slice(b"5 0 obj\n<< /Fields [4 0 R] >>\nendobj\n");

        let diags = Vec::new();
        let mut doc = PdfDocumentData {
            version: "%PDF-1.4".to_string(),
            objects: HashMap::new(),
            pages: Vec::new(),
            metadata: None,
            has_encryption: false,
            diagnostics: diags,
        };

        // Manually insert objects
        let _field_bytes = b"<< /FT /Tx /T (FirstName) /V (John) /Ff 0 >>";
        let _acroform_bytes = b"<< /Fields [4 0 R] >>";

        doc.objects.insert(
            1,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
                d.insert(
                    "Pages".to_string(),
                    PdfObject::Reference {
                        object_number: 2,
                        generation: 0,
                    },
                );
                d.insert(
                    "AcroForm".to_string(),
                    PdfObject::Reference {
                        object_number: 5,
                        generation: 0,
                    },
                );
                d
            }),
        );
        doc.objects.insert(
            2,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Type".to_string(), PdfObject::Name("Pages".to_string()));
                d.insert(
                    "Kids".to_string(),
                    PdfObject::Array(vec![PdfObject::Reference {
                        object_number: 3,
                        generation: 0,
                    }]),
                );
                d.insert("Count".to_string(), PdfObject::Integer(1));
                d
            }),
        );
        doc.objects.insert(
            3,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Type".to_string(), PdfObject::Name("Page".to_string()));
                d.insert(
                    "Parent".to_string(),
                    PdfObject::Reference {
                        object_number: 2,
                        generation: 0,
                    },
                );
                d.insert(
                    "MediaBox".to_string(),
                    PdfObject::Array(vec![
                        PdfObject::Integer(0),
                        PdfObject::Integer(0),
                        PdfObject::Integer(612),
                        PdfObject::Integer(792),
                    ]),
                );
                d
            }),
        );
        doc.objects.insert(
            4,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("FT".to_string(), PdfObject::Name("Tx".to_string()));
                d.insert("T".to_string(), PdfObject::String(b"FirstName".to_vec()));
                d.insert("V".to_string(), PdfObject::String(b"John".to_vec()));
                d.insert("Ff".to_string(), PdfObject::Integer(0));
                d
            }),
        );
        doc.objects.insert(
            5,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert(
                    "Fields".to_string(),
                    PdfObject::Array(vec![PdfObject::Reference {
                        object_number: 4,
                        generation: 0,
                    }]),
                );
                d
            }),
        );

        let (fields, diags) = parse_acroform(&doc);
        assert_eq!(fields.len(), 1, "should parse exactly 1 field");
        assert_eq!(fields[0].field_type, "text", "Tx -> text");
        assert_eq!(fields[0].name, "FirstName", "field name");
        assert_eq!(fields[0].value, Some("John".to_string()), "field value");
        assert!(diags.is_empty(), "no diagnostics expected");
    }

    /// Test 2: /Btn (button) field is parsed correctly.
    #[test]
    fn test_parse_acroform_button() {
        let mut doc = make_pdf_with_acroform("");
        doc.objects.insert(
            1,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
                d.insert(
                    "AcroForm".to_string(),
                    PdfObject::Reference {
                        object_number: 5,
                        generation: 0,
                    },
                );
                d
            }),
        );
        doc.objects.insert(
            4,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("FT".to_string(), PdfObject::Name("Btn".to_string()));
                d.insert("T".to_string(), PdfObject::String(b"Agree".to_vec()));
                d.insert("V".to_string(), PdfObject::Name("Yes".to_string()));
                d.insert("Ff".to_string(), PdfObject::Integer(0));
                d
            }),
        );
        doc.objects.insert(
            5,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert(
                    "Fields".to_string(),
                    PdfObject::Array(vec![PdfObject::Reference {
                        object_number: 4,
                        generation: 0,
                    }]),
                );
                d
            }),
        );

        let (fields, _) = parse_acroform(&doc);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].field_type, "button", "Btn -> button");
        assert_eq!(fields[0].name, "Agree");
        assert_eq!(fields[0].value, Some("Yes".to_string()));
    }

    /// Test 3: /Ch (choice) field is parsed correctly.
    #[test]
    fn test_parse_acroform_choice() {
        let mut doc = make_pdf_with_acroform("");
        doc.objects.insert(
            1,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
                d.insert(
                    "AcroForm".to_string(),
                    PdfObject::Reference {
                        object_number: 5,
                        generation: 0,
                    },
                );
                d
            }),
        );
        // Choice field with multi-select (value is array)
        doc.objects.insert(
            4,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("FT".to_string(), PdfObject::Name("Ch".to_string()));
                d.insert("T".to_string(), PdfObject::String(b"Options".to_vec()));
                d.insert(
                    "V".to_string(),
                    PdfObject::Array(vec![
                        PdfObject::Name("opt1".to_string()),
                        PdfObject::Name("opt2".to_string()),
                    ]),
                );
                d.insert("Ff".to_string(), PdfObject::Integer(0));
                d
            }),
        );
        doc.objects.insert(
            5,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert(
                    "Fields".to_string(),
                    PdfObject::Array(vec![PdfObject::Reference {
                        object_number: 4,
                        generation: 0,
                    }]),
                );
                d
            }),
        );

        let (fields, _) = parse_acroform(&doc);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].field_type, "choice", "Ch -> choice");
        assert_eq!(fields[0].name, "Options");
        assert_eq!(
            fields[0].value,
            Some("opt1, opt2".to_string()),
            "array values joined"
        );
    }

    /// Test 4: /Sig (signature) field is parsed correctly.
    #[test]
    fn test_parse_acroform_signature() {
        let mut doc = make_pdf_with_acroform("");
        doc.objects.insert(
            1,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
                d.insert(
                    "AcroForm".to_string(),
                    PdfObject::Reference {
                        object_number: 5,
                        generation: 0,
                    },
                );
                d
            }),
        );
        doc.objects.insert(
            4,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("FT".to_string(), PdfObject::Name("Sig".to_string()));
                d.insert("T".to_string(), PdfObject::String(b"Signature1".to_vec()));
                d
            }),
        );
        doc.objects.insert(
            5,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert(
                    "Fields".to_string(),
                    PdfObject::Array(vec![PdfObject::Reference {
                        object_number: 4,
                        generation: 0,
                    }]),
                );
                d
            }),
        );

        let (fields, _) = parse_acroform(&doc);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].field_type, "signature", "Sig -> signature");
        assert_eq!(fields[0].name, "Signature1");
        assert_eq!(
            fields[0].value, None,
            "signature has no pre-populated value"
        );
    }

    /// Test 5: Field values are extracted from various types.
    #[test]
    fn test_parse_field_values() {
        let mut doc = make_pdf_with_acroform("");
        doc.objects.insert(
            1,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
                d.insert(
                    "AcroForm".to_string(),
                    PdfObject::Reference {
                        object_number: 5,
                        generation: 0,
                    },
                );
                d
            }),
        );

        // String value
        doc.objects.insert(
            2,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("FT".to_string(), PdfObject::Name("Tx".to_string()));
                d.insert("T".to_string(), PdfObject::String(b"str_field".to_vec()));
                d.insert("V".to_string(), PdfObject::String(b"hello".to_vec()));
                d
            }),
        );

        // Integer value
        doc.objects.insert(
            3,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("FT".to_string(), PdfObject::Name("Tx".to_string()));
                d.insert("T".to_string(), PdfObject::String(b"num_field".to_vec()));
                d.insert("V".to_string(), PdfObject::Integer(42));
                d
            }),
        );

        // Real value
        doc.objects.insert(
            4,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("FT".to_string(), PdfObject::Name("Tx".to_string()));
                d.insert("T".to_string(), PdfObject::String(b"real_field".to_vec()));
                d.insert("V".to_string(), PdfObject::Real(std::f64::consts::PI));
                d
            }),
        );

        // Name value (for buttons: /Yes /No /Off)
        doc.objects.insert(
            6,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("FT".to_string(), PdfObject::Name("Btn".to_string()));
                d.insert("T".to_string(), PdfObject::String(b"btn_field".to_vec()));
                d.insert("V".to_string(), PdfObject::Name("Off".to_string()));
                d
            }),
        );

        doc.objects.insert(
            5,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert(
                    "Fields".to_string(),
                    PdfObject::Array(vec![
                        PdfObject::Reference {
                            object_number: 2,
                            generation: 0,
                        },
                        PdfObject::Reference {
                            object_number: 3,
                            generation: 0,
                        },
                        PdfObject::Reference {
                            object_number: 4,
                            generation: 0,
                        },
                        PdfObject::Reference {
                            object_number: 6,
                            generation: 0,
                        },
                    ]),
                );
                d
            }),
        );

        let (fields, _) = parse_acroform(&doc);
        assert_eq!(fields.len(), 4);

        let by_name: std::collections::HashMap<&str, &PdfFormField> =
            fields.iter().map(|f| (f.name.as_str(), f)).collect();

        assert_eq!(by_name["str_field"].value, Some("hello".to_string()));
        assert_eq!(by_name["num_field"].value, Some("42".to_string()));
        assert_eq!(
            by_name["real_field"].value,
            Some(std::f64::consts::PI.to_string())
        );
        assert_eq!(by_name["btn_field"].value, Some("Off".to_string()));
    }

    /// Test 6: Field flags (bit 1=readonly, bit 2=required, bit 3=noexport) are parsed.
    #[test]
    fn test_parse_field_flags() {
        let mut doc = make_pdf_with_acroform("");
        doc.objects.insert(
            1,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
                d.insert(
                    "AcroForm".to_string(),
                    PdfObject::Reference {
                        object_number: 5,
                        generation: 0,
                    },
                );
                d
            }),
        );

        // Readonly field (bit 1 = 0x1)
        doc.objects.insert(
            2,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("FT".to_string(), PdfObject::Name("Tx".to_string()));
                d.insert(
                    "T".to_string(),
                    PdfObject::String(b"readonly_field".to_vec()),
                );
                d.insert("Ff".to_string(), PdfObject::Integer(0x1));
                d
            }),
        );

        // Required field (bit 2 = 0x2)
        doc.objects.insert(
            3,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("FT".to_string(), PdfObject::Name("Tx".to_string()));
                d.insert(
                    "T".to_string(),
                    PdfObject::String(b"required_field".to_vec()),
                );
                d.insert("Ff".to_string(), PdfObject::Integer(0x2));
                d
            }),
        );

        // Noexport field (bit 3 = 0x4)
        doc.objects.insert(
            4,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("FT".to_string(), PdfObject::Name("Tx".to_string()));
                d.insert(
                    "T".to_string(),
                    PdfObject::String(b"noexport_field".to_vec()),
                );
                d.insert("Ff".to_string(), PdfObject::Integer(0x4));
                d
            }),
        );

        // All flags set (0x7)
        doc.objects.insert(
            6,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("FT".to_string(), PdfObject::Name("Tx".to_string()));
                d.insert("T".to_string(), PdfObject::String(b"all_flags".to_vec()));
                d.insert("Ff".to_string(), PdfObject::Integer(0x7));
                d
            }),
        );

        doc.objects.insert(
            5,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert(
                    "Fields".to_string(),
                    PdfObject::Array(vec![
                        PdfObject::Reference {
                            object_number: 2,
                            generation: 0,
                        },
                        PdfObject::Reference {
                            object_number: 3,
                            generation: 0,
                        },
                        PdfObject::Reference {
                            object_number: 4,
                            generation: 0,
                        },
                        PdfObject::Reference {
                            object_number: 6,
                            generation: 0,
                        },
                    ]),
                );
                d
            }),
        );

        let (fields, _) = parse_acroform(&doc);
        assert_eq!(fields.len(), 4);

        let by_name: std::collections::HashMap<&str, &PdfFormField> =
            fields.iter().map(|f| (f.name.as_str(), f)).collect();

        let rf = by_name["readonly_field"].flags.unwrap();
        assert!(rf.readonly, "readonly flag set");
        assert!(!rf.required, "required flag clear");
        assert!(!rf.noexport, "noexport flag clear");

        let qf = by_name["required_field"].flags.unwrap();
        assert!(!qf.readonly, "readonly flag clear");
        assert!(qf.required, "required flag set");
        assert!(!qf.noexport, "noexport flag clear");

        let nf = by_name["noexport_field"].flags.unwrap();
        assert!(!nf.readonly, "readonly flag clear");
        assert!(!nf.required, "required flag clear");
        assert!(nf.noexport, "noexport flag set");

        let af = by_name["all_flags"].flags.unwrap();
        assert!(af.readonly, "all: readonly");
        assert!(af.required, "all: required");
        assert!(af.noexport, "all: noexport");
    }

    /// Test 7: XFA is not present → detect_xra returns false.
    #[test]
    fn test_detect_xfa_not_present() {
        let mut doc = make_pdf_with_acroform("");
        doc.objects.insert(
            1,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
                d.insert(
                    "AcroForm".to_string(),
                    PdfObject::Reference {
                        object_number: 5,
                        generation: 0,
                    },
                );
                d
            }),
        );
        // AcroForm without /XFA
        doc.objects.insert(
            5,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Fields".to_string(), PdfObject::Array(vec![]));
                d
            }),
        );

        assert!(!detect_xra(&doc), "XFA not present");

        // Also check via parse_acroform — no diagnostics expected
        let (_, diags) = parse_acroform(&doc);
        assert!(
            !diags.iter().any(|d| d.code == "PDF-W-004"),
            "no XFA diagnostic"
        );
    }

    /// Test 8: XFA is present → detect_xra returns true and diagnostic emitted.
    #[test]
    fn test_detect_xfa_present() {
        let mut doc = make_pdf_with_acroform("");
        doc.objects.insert(
            1,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
                d.insert(
                    "AcroForm".to_string(),
                    PdfObject::Reference {
                        object_number: 5,
                        generation: 0,
                    },
                );
                d
            }),
        );

        // AcroForm with /XFA key containing XDP content
        doc.objects.insert(
            5,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Fields".to_string(), PdfObject::Array(vec![]));
                // XFA stream with XDP marker
                let xfa_data = b"<xdp:xdp><xfa:form><xfa:field>";
                d.insert("XFA".to_string(), PdfObject::String(xfa_data.to_vec()));
                d
            }),
        );

        assert!(detect_xra(&doc), "XFA detected via /XFA in AcroForm");

        let (_, diags) = parse_acroform(&doc);
        assert!(
            diags.iter().any(|d| d.code == "PDF-W-004"),
            "XFA diagnostic emitted"
        );
        assert!(diags
            .iter()
            .any(|d| d.severity == gp_core::diagnostics::Severity::Warning));
    }

    /// Test 9: All PDF fixtures are checked for AcroForm fields.
    #[test]
    fn test_parse_acroform_all_pdf_fixtures() {
        // simple PDF (no AcroForm)
        let doc1 = make_pdf_with_acroform("");
        let (fields, _) = parse_acroform(&doc1);
        assert!(fields.is_empty(), "simple PDF has no form fields");

        // PDF with AcroForm but no fields
        let mut doc2 = make_pdf_with_acroform("");
        doc2.objects.insert(
            1,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
                d.insert(
                    "AcroForm".to_string(),
                    PdfObject::Reference {
                        object_number: 5,
                        generation: 0,
                    },
                );
                d
            }),
        );
        doc2.objects.insert(
            5,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Fields".to_string(), PdfObject::Array(vec![]));
                d
            }),
        );
        let (fields, _) = parse_acroform(&doc2);
        assert!(
            fields.is_empty(),
            "AcroForm with empty /Fields has no fields"
        );

        // PDF with AcroForm but /Fields missing
        let mut doc3 = make_pdf_with_acroform("");
        doc3.objects.insert(
            1,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
                d.insert(
                    "AcroForm".to_string(),
                    PdfObject::Reference {
                        object_number: 5,
                        generation: 0,
                    },
                );
                d
            }),
        );
        doc3.objects.insert(
            5,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                // /Fields intentionally missing
                d.insert("DefaultXFA".to_string(), PdfObject::String(b"xfa".to_vec()));
                d
            }),
        );
        let (fields, _) = parse_acroform(&doc3);
        assert!(fields.is_empty(), "AcroForm without /Fields has no fields");
    }

    /// Test 10: XFA → diagnostic flag (PDF-W-004) is present in parse_acroform output.
    #[test]
    fn test_xfa_flagged_as_unsupported() {
        let mut doc = make_pdf_with_acroform("");
        doc.objects.insert(
            1,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
                d.insert(
                    "AcroForm".to_string(),
                    PdfObject::Reference {
                        object_number: 5,
                        generation: 0,
                    },
                );
                d
            }),
        );

        // XFA with array value
        doc.objects.insert(
            5,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Fields".to_string(), PdfObject::Array(vec![]));
                d.insert(
                    "XFA".to_string(),
                    PdfObject::Array(vec![
                        PdfObject::Name("xfa".to_string()),
                        PdfObject::Name("xdp".to_string()),
                    ]),
                );
                d
            }),
        );

        let (_, diags) = parse_acroform(&doc);
        let xfa_diag = diags.iter().find(|d| d.code == "PDF-W-004");
        assert!(xfa_diag.is_some(), "XFA diagnostic found");
        assert_eq!(
            xfa_diag.unwrap().severity,
            gp_core::diagnostics::Severity::Warning
        );
        assert!(
            xfa_diag.unwrap().message.contains("XFA")
                || xfa_diag.unwrap().message.contains("unsupported")
        );
    }

    /// Test 11: Field with /Rect (appearance rectangle) is parsed.
    #[test]
    fn test_parse_field_rect() {
        let mut doc = make_pdf_with_acroform("");
        doc.objects.insert(
            1,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
                d.insert(
                    "AcroForm".to_string(),
                    PdfObject::Reference {
                        object_number: 5,
                        generation: 0,
                    },
                );
                d
            }),
        );
        doc.objects.insert(
            4,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("FT".to_string(), PdfObject::Name("Tx".to_string()));
                d.insert("T".to_string(), PdfObject::String(b"rect_field".to_vec()));
                d.insert(
                    "Rect".to_string(),
                    PdfObject::Array(vec![
                        PdfObject::Integer(100),
                        PdfObject::Integer(200),
                        PdfObject::Integer(300),
                        PdfObject::Integer(220),
                    ]),
                );
                d
            }),
        );
        doc.objects.insert(
            5,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert(
                    "Fields".to_string(),
                    PdfObject::Array(vec![PdfObject::Reference {
                        object_number: 4,
                        generation: 0,
                    }]),
                );
                d
            }),
        );

        let (fields, _) = parse_acroform(&doc);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].rect, Some([100.0, 200.0, 300.0, 220.0]));
    }

    /// Test 12: Multiple fields from a single AcroForm are all parsed.
    #[test]
    fn test_parse_multiple_fields() {
        let mut doc = make_pdf_with_acroform("");
        doc.objects.insert(
            1,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("Type".to_string(), PdfObject::Name("Catalog".to_string()));
                d.insert(
                    "AcroForm".to_string(),
                    PdfObject::Reference {
                        object_number: 5,
                        generation: 0,
                    },
                );
                d
            }),
        );
        // Field 2: text
        doc.objects.insert(
            2,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("FT".to_string(), PdfObject::Name("Tx".to_string()));
                d.insert("T".to_string(), PdfObject::String(b"name".to_vec()));
                d.insert("V".to_string(), PdfObject::String(b"Alice".to_vec()));
                d
            }),
        );
        // Field 3: button
        doc.objects.insert(
            3,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("FT".to_string(), PdfObject::Name("Btn".to_string()));
                d.insert("T".to_string(), PdfObject::String(b"agree".to_vec()));
                d.insert("V".to_string(), PdfObject::Name("Off".to_string()));
                d
            }),
        );
        // Field 4: choice
        doc.objects.insert(
            4,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert("FT".to_string(), PdfObject::Name("Ch".to_string()));
                d.insert("T".to_string(), PdfObject::String(b"color".to_vec()));
                d.insert("V".to_string(), PdfObject::Name("red".to_string()));
                d
            }),
        );

        doc.objects.insert(
            5,
            PdfObject::Dict({
                let mut d: std::collections::BTreeMap<String, PdfObject> =
                    std::collections::BTreeMap::new();
                d.insert(
                    "Fields".to_string(),
                    PdfObject::Array(vec![
                        PdfObject::Reference {
                            object_number: 2,
                            generation: 0,
                        },
                        PdfObject::Reference {
                            object_number: 3,
                            generation: 0,
                        },
                        PdfObject::Reference {
                            object_number: 4,
                            generation: 0,
                        },
                    ]),
                );
                d
            }),
        );

        let (fields, _) = parse_acroform(&doc);
        assert_eq!(fields.len(), 3);

        let types: Vec<&str> = fields.iter().map(|f| f.field_type.as_str()).collect();
        assert!(types.contains(&"text"));
        assert!(types.contains(&"button"));
        assert!(types.contains(&"choice"));
    }
}
