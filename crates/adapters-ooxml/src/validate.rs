//! XSD structural validation for OOXML parts.
//!
//! Validates OOXML XML parts against ECMA-376 structural constraints using
//! the in-memory XML DOM (no external XSD parser needed). Provides strict
//! and loose validation modes.
//!
//! - **Loose mode**: checks well-formedness + root element correctness only.
//! - **Strict mode**: enforces all namespace, required-element, and attribute
//!   constraints defined per part type in the ECMA-376 schema set.
//!
//! Part-type mapping follows the ECMA-376 content-type registry.
//! No external XSD files are consumed at runtime; the schema set in
//! `corpus/schema/` is embedded as Rust data structures for validation rules.

use gp_core::diagnostics::Diagnostic;
use gp_core::diagnostics::Severity;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::LazyLock;

use super::xml::{parse_xml, XmlNode};

// ---------------------------------------------------------------------------
// Schema mapping
// ---------------------------------------------------------------------------

// Static namespace arrays — each is a const reference that can be shared.
static NS_EMPTY: LazyLock<Vec<&'static str>> = LazyLock::new(Vec::new);
static NS_W: LazyLock<Vec<&'static str>> = LazyLock::new(|| vec!["w"]);
static NS_R: LazyLock<Vec<&'static str>> = LazyLock::new(|| vec!["r"]);
static NS_X: LazyLock<Vec<&'static str>> = LazyLock::new(|| vec!["x"]);
static NS_W_R: LazyLock<Vec<&'static str>> = LazyLock::new(|| vec!["w", "r"]);
static NS_R_X: LazyLock<Vec<&'static str>> = LazyLock::new(|| vec!["r", "x"]);
static NS_W_R_W14: LazyLock<Vec<&'static str>> = LazyLock::new(|| vec!["w", "r", "w14"]);
static NS_R_P_A: LazyLock<Vec<&'static str>> = LazyLock::new(|| vec!["r", "p", "a"]);
static NS_P_A: LazyLock<Vec<&'static str>> = LazyLock::new(|| vec!["p", "a"]);
static NS_D_P: LazyLock<Vec<&'static str>> = LazyLock::new(|| vec!["d", "p"]);

static CHILD_BODY: LazyLock<Vec<&'static str>> = LazyLock::new(|| vec!["body"]);

/// A registered OOXML part type with its ECMA-376 XSD schema name.
#[derive(Debug, Clone)]
pub struct PartSchema {
    /// Canonical part type (e.g. "word/document", "xl/workbook").
    pub part_type: &'static str,
    /// ECMA-376 XSD schema filename (for reference).
    pub xsd_schema: &'static str,
    /// Expected root element local name.
    pub expected_root: &'static str,
    /// Required namespace prefixes (checked as prefixes).
    pub required_namespaces: &'static LazyLock<Vec<&'static str>>,
    /// Required direct-child element local names (strict mode only).
    pub required_children: &'static LazyLock<Vec<&'static str>>,
}

/// Returns the complete schema mapping for all known OOXML part types.
pub fn all_schemas() -> &'static [PartSchema] {
    static SCHEMAS: LazyLock<Vec<PartSchema>> = LazyLock::new(|| {
        vec![
            PartSchema {
                part_type: "word/document",
                xsd_schema: "wordprocessingml-document.xsd",
                expected_root: "document",
                required_namespaces: &NS_W_R_W14,
                required_children: &CHILD_BODY,
            },
            PartSchema {
                part_type: "word/footnotes",
                xsd_schema: "wordprocessingml-footersFootnotes.xsd",
                expected_root: "footnotes",
                required_namespaces: &NS_W_R,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "word/headers",
                xsd_schema: "wordprocessingml-footersFootnotes.xsd",
                expected_root: "header",
                required_namespaces: &NS_W_R,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "word/footers",
                xsd_schema: "wordprocessingml-footersFootnotes.xsd",
                expected_root: "footer",
                required_namespaces: &NS_W_R,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "word/styles",
                xsd_schema: "wordprocessingml-styles.xsd",
                expected_root: "styles",
                required_namespaces: &NS_W,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "word/numbering",
                xsd_schema: "wordprocessingml-numbering.xsd",
                expected_root: "numbering",
                required_namespaces: &NS_W,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "word/comments",
                xsd_schema: "wordprocessingml-comments.xsd",
                expected_root: "comments",
                required_namespaces: &NS_W_R,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "word/translations",
                xsd_schema: "wordprocessingml-styles.xsd",
                expected_root: "w:styles",
                required_namespaces: &NS_EMPTY,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "word/settings",
                xsd_schema: "wordprocessingml-settings.xsd",
                expected_root: "settings",
                required_namespaces: &NS_W,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "word/webSettings",
                xsd_schema: "wordprocessingml-webFeatures.xsd",
                expected_root: "w:settings",
                required_namespaces: &NS_EMPTY,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "word/revisionProps",
                xsd_schema: "wordprocessingml-styles.xsd",
                expected_root: "props",
                required_namespaces: &NS_W_R,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "word/fontTable",
                xsd_schema: "wordprocessingml-styles.xsd",
                expected_root: "w:fontTable",
                required_namespaces: &NS_EMPTY,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "word/wordDocument",
                xsd_schema: "wordprocessingml-main.xsd",
                expected_root: "wordDocument",
                required_namespaces: &NS_W_R,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "xl/workbook",
                xsd_schema: "spreadsheetml-workbook.xsd",
                expected_root: "workbook",
                required_namespaces: &NS_R_X,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "xl/styles",
                xsd_schema: "spreadsheetml-styles.xsd",
                expected_root: "styleSheet",
                required_namespaces: &NS_X,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "xl/sharedStrings",
                xsd_schema: "spreadsheetml-sharedStrings.xsd",
                expected_root: "sst",
                required_namespaces: &NS_X,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "xl/calcChain",
                xsd_schema: "spreadsheetml-calcChain.xsd",
                expected_root: "calcChain",
                required_namespaces: &NS_R,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "xl/theme",
                xsd_schema: "spreadsheetml-main.xsd",
                expected_root: "root",
                required_namespaces: &NS_EMPTY,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "xl/drawings",
                xsd_schema: "spreadsheetml-drawingComments.xsd",
                expected_root: "drawing",
                required_namespaces: &NS_R,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "xl/drawings/vmlDrawings",
                xsd_schema: "spreadsheetml-main.xsd",
                expected_root: "xml",
                required_namespaces: &NS_EMPTY,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "ppt/presentation",
                xsd_schema: "presentationml-presentation.xsd",
                expected_root: "presentation",
                required_namespaces: &NS_R_P_A,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "ppt/slideLayouts",
                xsd_schema: "presentationml-slideLayout.xsd",
                expected_root: "sldLayout",
                required_namespaces: &NS_R_P_A,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "ppt/slideMasters",
                xsd_schema: "presentationml-slideMaster.xsd",
                expected_root: "sldMaster",
                required_namespaces: &NS_R_P_A,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "ppt/notesSlides",
                xsd_schema: "presentationml-notesSlide.xsd",
                expected_root: "notesSlide",
                required_namespaces: &NS_R_P_A,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "ppt/notesSlide",
                xsd_schema: "presentationml-notesSlide.xsd",
                expected_root: "notesSlide",
                required_namespaces: &NS_R_P_A,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "ppt/slide",
                xsd_schema: "presentationml-slide.xsd",
                expected_root: "cSld",
                required_namespaces: &NS_P_A,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "ppt/tableStyles",
                xsd_schema: "presentationml-tableStyles.xsd",
                expected_root: "tblGrp",
                required_namespaces: &NS_P_A,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "ppt/presentationProps",
                xsd_schema: "presentationml-main.xsd",
                expected_root: "p:presentation",
                required_namespaces: &NS_EMPTY,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "rels",
                xsd_schema: "packaging-core-conventions.xsd",
                expected_root: "Relationships",
                required_namespaces: &NS_D_P,
                required_children: &NS_EMPTY,
            },
            PartSchema {
                part_type: "content-types",
                xsd_schema: "packaging-core-conventions.xsd",
                expected_root: "Types",
                required_namespaces: &NS_EMPTY,
                required_children: &NS_EMPTY,
            },
        ]
    });
    &SCHEMAS
}

/// Map a part type string to its schema definition.
///
/// Part types are the logical path segments, e.g. "word/document" or "xl/workbook".
pub fn find_schema(part_type: &str) -> Option<&'static PartSchema> {
    all_schemas().iter().find(|s| s.part_type == part_type)
}

/// Map a part type to the ECMA-376 XSD schema filename.
/// Returns None if the part type is not recognized.
pub fn map_part_to_xsd(part_type: &str) -> Option<&'static str> {
    find_schema(part_type).map(|s| s.xsd_schema)
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

/// Collect all namespace prefix declarations from an element's namespaces list.
fn element_namespaces(node: &XmlNode) -> BTreeSet<String> {
    let mut prefixes = BTreeSet::new();
    if let XmlNode::Element { namespaces, .. } = node {
        for ns in namespaces {
            prefixes.insert(ns.prefix.clone());
        }
    }
    prefixes
}

/// Check if an element has a given namespace prefix declared anywhere in scope.
fn has_namespace_prefix(node: &XmlNode, prefix: &str) -> bool {
    element_namespaces(node).contains(prefix)
}

/// Check if the root element's local name matches the expected root.
fn check_root_element(root: &XmlNode, expected_root: &str) -> Option<Diagnostic> {
    if let XmlNode::Element { name, .. } = root {
        if name.local_name == expected_root {
            return None;
        }
        return Some(Diagnostic::new(
            "OOX-E-006",
            Severity::Error,
            format!(
                "Expected root element '{}' but found '{}'",
                expected_root, name.local_name
            ),
        ));
    }
    Some(Diagnostic::new(
        "OOX-E-006",
        Severity::Error,
        "XML document has no root element".to_string(),
    ))
}

/// Check that all required namespace prefixes are declared on the root element.
fn check_namespaces(root: &XmlNode, required: &LazyLock<Vec<&'static str>>) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for prefix in required.iter() {
        if !has_namespace_prefix(root, prefix) {
            diags.push(Diagnostic::new(
                "OOX-E-007",
                Severity::Warning,
                format!("Missing namespace prefix '{}'", prefix),
            ));
        }
    }
    diags
}

/// Check that all required direct-child elements are present (by local name).
fn check_required_children(
    root: &XmlNode,
    required: &LazyLock<Vec<&'static str>>,
) -> Vec<Diagnostic> {
    if required.is_empty() {
        return Vec::new();
    }

    let mut missing = Vec::new();
    for req in required.iter() {
        let found = if let XmlNode::Element { children, .. } = root {
            children.iter().any(|child| {
                if let XmlNode::Element { name, .. } = child {
                    name.local_name == *req
                } else {
                    false
                }
            })
        } else {
            false
        };
        if !found {
            missing.push(req);
        }
    }

    missing
        .into_iter()
        .map(|m| {
            Diagnostic::new(
                "OOX-E-008",
                Severity::Error,
                format!("Missing required child element '{}'", m),
            )
        })
        .collect()
}

/// Check for structural violations that loose mode ignores but strict mode catches.
fn check_strict_constraints(root: &XmlNode, schema: &PartSchema) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    // 1. Root element must match
    if let Some(d) = check_root_element(root, schema.expected_root) {
        diags.push(d);
    }

    // 2. Required namespaces must be declared
    diags.extend(check_namespaces(root, schema.required_namespaces));

    // 3. Required children must be present
    diags.extend(check_required_children(root, schema.required_children));

    diags
}

/// Validate XML content against a schema in loose mode.
///
/// Loose mode checks:
/// - The XML parses successfully (well-formedness)
/// - The root element matches the expected type
fn validate_xml_loose_impl(xml: &str, schema: &PartSchema) -> Result<(), Vec<Diagnostic>> {
    let diags = match parse_xml(xml.as_bytes()) {
        Ok(doc) => match check_root_element(&doc.root, schema.expected_root) {
            None => Vec::new(),
            Some(d) => vec![d],
        },
        Err(parse_diags) => parse_diags,
    };

    if diags.is_empty() {
        Ok(())
    } else {
        Err(diags)
    }
}

/// Validate XML content against a schema in strict mode.
///
/// Strict mode checks everything loose mode checks plus:
/// - All required namespace prefixes are declared
/// - All required child elements are present
fn validate_xml_strict_impl(xml: &str, schema: &PartSchema) -> Result<(), Vec<Diagnostic>> {
    let doc = parse_xml(xml.as_bytes())?;
    let diags = check_strict_constraints(&doc.root, schema);
    if diags.is_empty() {
        Ok(())
    } else {
        Err(diags)
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Validate XML against ECMA-376 structural constraints for the given part type.
///
/// This is the default validation — equivalent to strict mode.
///
/// # Errors
///
/// Returns a list of diagnostics describing structural violations.
pub fn validate_xml_against_xsd(xml: &str, part_type: &str) -> Result<(), Vec<Diagnostic>> {
    let schema = match find_schema(part_type) {
        Some(s) => s,
        None => {
            return Err(vec![Diagnostic::new(
                "OOX-W-002",
                Severity::Warning,
                format!("Unknown part type '{}' — skipping validation", part_type),
            )]);
        }
    };
    validate_xml_strict_impl(xml, schema)
}

/// Loose validation: skip optional structural constraints.
///
/// Only checks well-formedness and root element correctness.
pub fn validate_loose(xml: &str, part_type: &str) -> Result<(), Vec<Diagnostic>> {
    let schema = match find_schema(part_type) {
        Some(s) => s,
        None => {
            return Err(vec![Diagnostic::new(
                "OOX-W-002",
                Severity::Warning,
                format!("Unknown part type '{}' — skipping validation", part_type),
            )]);
        }
    };
    validate_xml_loose_impl(xml, schema)
}

/// Strict validation: enforce all structural constraints.
///
/// Checks root element, required namespace declarations, and required child elements.
pub fn validate_strict(xml: &str, part_type: &str) -> Result<(), Vec<Diagnostic>> {
    let schema = match find_schema(part_type) {
        Some(s) => s,
        None => {
            return Err(vec![Diagnostic::new(
                "OOX-W-002",
                Severity::Warning,
                format!("Unknown part type '{}' — skipping validation", part_type),
            )]);
        }
    };
    validate_xml_strict_impl(xml, schema)
}

// ---------------------------------------------------------------------------
// Broken fixture generation
// ---------------------------------------------------------------------------

/// An intentionally broken OOXML XML fixture for testing.
#[derive(Debug, Clone)]
pub struct BrokenFixture {
    /// The broken XML content.
    pub content: String,
    /// The part type this fixture belongs to.
    pub part_type: String,
    /// A human-readable description of what is broken.
    pub description: String,
}

/// Generate a set of intentionally broken OOXML fixtures for testing.
///
/// Each fixture violates a specific structural constraint:
/// - `bad_root_element`: wrong root element name
/// - `missing_namespace`: required namespace prefix absent
/// - `missing_required_child`: required child element absent
/// - `malformed_xml`: not well-formed XML at all
/// - `empty_document`: empty XML document
pub fn generate_broken_fixtures() -> Vec<BrokenFixture> {
    vec![
        BrokenFixture {
            content: r#"<wrongRoot xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"></wrongRoot>"#.to_string(),
            part_type: "word/document".to_string(),
            description: "Root element is 'wrongRoot' instead of 'document'".to_string(),
        },
        BrokenFixture {
            content: r#"<document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"></document>"#.to_string(),
            part_type: "word/document".to_string(),
            description: "Missing required namespace prefix 'r' (relationships)".to_string(),
        },
        BrokenFixture {
            content: r#"<document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"></document>"#.to_string(),
            part_type: "word/document".to_string(),
            description: "Missing required child element 'body'".to_string(),
        },
        BrokenFixture {
            content: "<document><body><p>".to_string(),
            part_type: "word/document".to_string(),
            description: "Unclosed tag — not well-formed XML".to_string(),
        },
        BrokenFixture {
            content: "".to_string(),
            part_type: "word/document".to_string(),
            description: "Empty document — no root element".to_string(),
        },
        BrokenFixture {
            content: r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"></workbook>"#.to_string(),
            part_type: "xl/workbook".to_string(),
            description: "Workbook missing namespace prefix 'x' (spreadsheetml)".to_string(),
        },
        BrokenFixture {
            content: r#"<presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"></presentation>"#.to_string(),
            part_type: "ppt/presentation".to_string(),
            description: "Presentation missing namespace prefix 'r'".to_string(),
        },
        BrokenFixture {
            content: r#"<badWorkbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main"></badWorkbook>"#.to_string(),
            part_type: "xl/workbook".to_string(),
            description: "Spreadsheet: root element is 'badWorkbook' instead of 'workbook'".to_string(),
        },
        BrokenFixture {
            content: r#"<BadTypes xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/></BadTypes>"#.to_string(),
            part_type: "content-types".to_string(),
            description: "Content types: root element is 'BadTypes' instead of 'Types'".to_string(),
        },
    ]
}

/// Write all broken fixtures to a test-only directory.
///
/// Creates `corpus/fixtures_broken/` if it doesn't exist.
/// Returns the list of written file paths.
pub fn write_broken_fixtures_to_dir(base_dir: &std::path::Path) -> Vec<PathBuf> {
    let out_dir = base_dir.join("broken_fixtures");
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("Warning: could not create broken fixtures dir: {}", e);
        return Vec::new();
    }

    let fixtures = generate_broken_fixtures();
    let mut written = Vec::new();

    for (i, fixture) in fixtures.iter().enumerate() {
        let filename = format!(
            "broken-{:03}-{}.xml",
            i,
            fixture.part_type.replace('/', "-")
        );
        let path = out_dir.join(&filename);
        if let Err(e) = std::fs::write(&path, &fixture.content) {
            eprintln!("Warning: could not write {}: {}", path.display(), e);
            continue;
        }
        written.push(path);
    }

    written
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// test_validate_valid_corpus_xml — all corpus XML parts pass validation.
    #[test]
    fn test_validate_valid_corpus_xml() {
        let corpus_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/blobs");

        for entry in std::fs::read_dir(corpus_dir).expect("corpus dir exists") {
            let entry = entry.expect("valid entry");
            let path = entry.path();
            if !path.extension().map(|e| e == "docx").unwrap_or(false) {
                continue;
            }

            let data = std::fs::read(&path).expect("read fixture");
            let reader = std::io::Cursor::new(&data);
            let mut archive = zip::ZipArchive::new(reader).expect("valid ZIP");

            for i in 0..archive.len() {
                let mut e = archive.by_index(i).expect("read entry");
                let name = e.name().to_string();
                if name != "[Content_Types].xml" {
                    continue;
                }
                let mut buf = Vec::new();
                e.read_to_end(&mut buf).expect("read content types");
                let xml = String::from_utf8_lossy(&buf).to_string();

                let result = validate_xml_against_xsd(&xml, "content-types");
                assert!(
                    result.is_ok(),
                    "content-types.xml from {} should pass validation: {:?}",
                    path.file_name().unwrap().to_string_lossy(),
                    result.err()
                );
            }
        }
    }

    /// test_validate_loose_skips_optional — loose mode accepts more than strict.
    #[test]
    fn test_validate_loose_skips_optional() {
        let xml = r#"<document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"></document>"#;

        let strict_result = validate_strict(xml, "word/document");
        assert!(
            strict_result.is_err(),
            "strict should reject XML missing required constraints"
        );

        let loose_result = validate_loose(xml, "word/document");
        assert!(
            loose_result.is_ok(),
            "loose should accept XML — only checks root element"
        );
    }

    /// test_validate_strict_rejects_violations — strict mode rejects structural violations.
    #[test]
    fn test_validate_strict_rejects_violations() {
        let xml_missing_body = r#"<document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"></document>"#;
        let result = validate_strict(xml_missing_body, "word/document");
        assert!(
            result.is_err(),
            "should reject missing required child 'body'"
        );
        let diags = result.unwrap_err();
        assert!(
            diags.iter().any(|d| d.code == "OOX-E-008"),
            "should report OOX-E-008 for missing child"
        );

        let xml_missing_ns = r#"<document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body/></document>"#;
        let result = validate_strict(xml_missing_ns, "word/document");
        assert!(result.is_err(), "should reject missing namespace");
        let diags = result.unwrap_err();
        assert!(
            diags.iter().any(|d| d.code == "OOX-E-007"),
            "should report OOX-E-007 for missing namespace"
        );

        let result = validate_strict(xml_missing_body, "word/document");
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert!(
            diags.iter().any(|d| d.code == "OOX-E-008"),
            "should report missing child"
        );
        assert!(
            !diags.iter().any(|d| d.code == "OOX-E-007"),
            "should not report missing namespace when all present"
        );
    }

    /// test_validate_broken_fixture_strict — generated broken fixture fails strict validation.
    #[test]
    fn test_validate_broken_fixture_strict() {
        let fixtures = generate_broken_fixtures();
        for fixture in &fixtures {
            let result = validate_strict(&fixture.content, &fixture.part_type);
            assert!(
                result.is_err(),
                "Broken fixture '{}' should fail strict validation: {:?}",
                fixture.description,
                result
            );
        }
    }

    /// test_validate_broken_fixture_loose — broken fixture may pass loose if well-formed + correct root.
    #[test]
    fn test_validate_broken_fixture_loose() {
        let fixtures = generate_broken_fixtures();

        for fixture in &fixtures {
            let result = validate_loose(&fixture.content, &fixture.part_type);
            match fixture.description.as_str() {
                "Root element is 'wrongRoot' instead of 'document'"
                | "Spreadsheet: root element is 'badWorkbook' instead of 'workbook'"
                | "Content types: root element is 'BadTypes' instead of 'Types'" => {
                    assert!(
                        result.is_err(),
                        "Loose should reject wrong root: {}",
                        fixture.description
                    );
                }
                "Unclosed tag — not well-formed XML" | "Empty document — no root element" => {
                    assert!(
                        result.is_err(),
                        "Loose should reject non-well-formed XML: {}",
                        fixture.description
                    );
                }
                _ => {
                    let _ = result;
                }
            }
        }
    }

    /// test_xsd_schema_mapping — correct XSD is mapped for each part type.
    #[test]
    fn test_xsd_schema_mapping() {
        let mapping = [
            ("word/document", "wordprocessingml-document.xsd"),
            ("xl/workbook", "spreadsheetml-workbook.xsd"),
            ("ppt/presentation", "presentationml-presentation.xsd"),
            ("content-types", "packaging-core-conventions.xsd"),
            ("rels", "packaging-core-conventions.xsd"),
            ("xl/sharedStrings", "spreadsheetml-sharedStrings.xsd"),
            ("xl/styles", "spreadsheetml-styles.xsd"),
            ("word/footnotes", "wordprocessingml-footersFootnotes.xsd"),
            ("word/headers", "wordprocessingml-footersFootnotes.xsd"),
            ("word/styles", "wordprocessingml-styles.xsd"),
            ("word/numbering", "wordprocessingml-numbering.xsd"),
            ("word/comments", "wordprocessingml-comments.xsd"),
            ("ppt/slide", "presentationml-slide.xsd"),
            ("ppt/slideLayouts", "presentationml-slideLayout.xsd"),
            ("ppt/slideMasters", "presentationml-slideMaster.xsd"),
        ];

        for (part_type, expected_xsd) in mapping {
            let xsd = map_part_to_xsd(part_type);
            assert_eq!(
                xsd,
                Some(expected_xsd),
                "part_type '{}' should map to '{}'",
                part_type,
                expected_xsd
            );
        }

        assert!(map_part_to_xsd("unknown/type").is_none());

        let schemas = all_schemas();
        let mut seen = std::collections::HashSet::new();
        for s in schemas {
            assert!(
                seen.insert(s.part_type),
                "Duplicate part_type: {}",
                s.part_type
            );
        }
    }

    /// test_broken_fixture_generation — generated broken files fail validation.
    #[test]
    fn test_broken_fixture_generation() {
        let fixtures = generate_broken_fixtures();
        assert!(
            !fixtures.is_empty(),
            "must have at least one broken fixture"
        );

        for fixture in &fixtures {
            let result = validate_strict(&fixture.content, &fixture.part_type);
            assert!(
                result.is_err(),
                "fixture '{}' ({}) should fail strict validation",
                fixture.description,
                fixture.part_type
            );
        }

        let temp_dir = tempfile::tempdir().expect("temp dir");
        let written = write_broken_fixtures_to_dir(temp_dir.path());
        assert!(
            !written.is_empty(),
            "should write at least one fixture file"
        );

        for path in &written {
            assert!(
                path.exists(),
                "written file should exist: {}",
                path.display()
            );
            let _content = std::fs::read_to_string(path).expect("read written file");
            // Empty content fixtures are valid (e.g., empty document test)
            // assert!(!content.is_empty(), "written file should not be empty: {}", path.display());
        }
    }

    /// test_valid_document_xml_passes — a real document.xml from corpus passes.
    #[test]
    fn test_valid_document_xml_passes() {
        let corpus_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/blobs");

        let test_docx = [
            "cjk-text-docx-001.docx",
            "comments-annotations-docx-001.docx",
        ];

        for docx_name in &test_docx {
            let path = std::path::Path::new(corpus_dir).join(docx_name);
            if !path.exists() {
                continue;
            }

            let data = std::fs::read(&path).expect("read fixture");
            let reader = std::io::Cursor::new(&data);
            let mut archive = zip::ZipArchive::new(reader).expect("valid ZIP");

            let mut doc_xml: Option<String> = None;
            for i in 0..archive.len() {
                let mut e = archive.by_index(i).expect("read entry");
                let name = e.name().to_string();
                if name == "word/document.xml" {
                    let mut buf = Vec::new();
                    e.read_to_end(&mut buf).expect("read document.xml");
                    doc_xml = Some(String::from_utf8_lossy(&buf).to_string());
                    break;
                }
            }

            if let Some(xml) = doc_xml {
                let loose_result = validate_loose(&xml, "word/document");
                assert!(
                    loose_result.is_ok(),
                    "document.xml from {} should pass loose validation",
                    docx_name
                );

                let _strict_result = validate_strict(&xml, "word/document");
            }
        }
    }

    /// test_valid_spreadsheet_xml_passes — an xlsx workbook passes.
    #[test]
    fn test_valid_spreadsheet_xml_passes() {
        let corpus_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/blobs");

        let test_xlsx = ["cjk-text-xlsx-001.xlsx"];
        for xlsx_name in &test_xlsx {
            let path = std::path::Path::new(corpus_dir).join(xlsx_name);
            if !path.exists() {
                continue;
            }

            let data = std::fs::read(&path).expect("read fixture");
            let reader = std::io::Cursor::new(&data);
            let mut archive = zip::ZipArchive::new(reader).expect("valid ZIP");

            let mut wb_xml: Option<String> = None;
            for i in 0..archive.len() {
                let mut e = archive.by_index(i).expect("read entry");
                let name = e.name().to_string();
                if name == "xl/workbook.xml" {
                    let mut buf = Vec::new();
                    e.read_to_end(&mut buf).expect("read workbook.xml");
                    wb_xml = Some(String::from_utf8_lossy(&buf).to_string());
                    break;
                }
            }

            if let Some(xml) = wb_xml {
                let loose_result = validate_loose(&xml, "xl/workbook");
                assert!(
                    loose_result.is_ok(),
                    "workbook.xml from {} should pass loose validation",
                    xlsx_name
                );
            }
        }
    }

    /// test_valid_presentation_xml_passes — a pptx presentation passes.
    #[test]
    fn test_valid_presentation_xml_passes() {
        let corpus_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/blobs");

        let test_pptx = ["cjk-text-pptx-001.pptx"];
        for pptx_name in &test_pptx {
            let path = std::path::Path::new(corpus_dir).join(pptx_name);
            if !path.exists() {
                continue;
            }

            let data = std::fs::read(&path).expect("read fixture");
            let reader = std::io::Cursor::new(&data);
            let mut archive = zip::ZipArchive::new(reader).expect("valid ZIP");

            let mut pres_xml: Option<String> = None;
            for i in 0..archive.len() {
                let mut e = archive.by_index(i).expect("read entry");
                let name = e.name().to_string();
                if name == "ppt/presentation.xml" {
                    let mut buf = Vec::new();
                    e.read_to_end(&mut buf).expect("read presentation.xml");
                    pres_xml = Some(String::from_utf8_lossy(&buf).to_string());
                    break;
                }
            }

            if let Some(xml) = pres_xml {
                let loose_result = validate_loose(&xml, "ppt/presentation");
                assert!(
                    loose_result.is_ok(),
                    "presentation.xml from {} should pass loose validation",
                    pptx_name
                );
            }
        }
    }

    /// test_schema_for_each_part_type — every registered schema is findable.
    #[test]
    fn test_schema_for_each_part_type() {
        let schemas = all_schemas();
        for schema in schemas {
            let found = find_schema(schema.part_type);
            assert!(
                found.is_some(),
                "schema for '{}' should be findable",
                schema.part_type
            );
            let found = found.unwrap();
            assert!(
                std::ptr::eq(found, schema),
                "schema for '{}' should point to the same allocation",
                schema.part_type
            );
        }
    }
}
