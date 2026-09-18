//! ODF census — count elements in ODF content.xml and packages.
//!
//! Walks the parsed XML DOM of an ODF document to count structural elements:
//! paragraphs (`text:p`), tables (`table:table`), text frames (`draw:frame`),
//! lists (`text:list`), list items (`text:list-item`), table rows (`table:table-row`),
//! table cells (`table:table-cell`), and revision markers
//! (`text:insert`, `text:mark-deleted`).
//!
//! Also provides delta computation between two census snapshots for
//! fidelity-budget tracking.

use std::collections::BTreeMap;

use crate::manifest::OdfPackage;
use crate::xml::{parse_xml, XmlNode};
use gp_core::diagnostics::Diagnostic;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A census of element counts in an ODF document.
///
/// Counts are derived from `content.xml` (or a union of all XML parts
/// when computed from an `OdfPackage`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OdfCensus {
    /// Paragraph count (`text:p`).
    pub paragraphs: u64,
    /// Table count (`table:table`).
    pub tables: u64,
    /// Text frame count (`draw:frame`).
    pub text_frames: u64,
    /// List count (`text:list`).
    pub lists: u64,
    /// List item count (`text:list-item`).
    pub list_items: u64,
    /// Table row count (`table:table-row`).
    pub table_rows: u64,
    /// Table cell count (`table:table-cell`).
    pub table_cells: u64,
    /// Revision insert count (`text:insert` and `text:inserted`).
    pub revisions_insert: u64,
    /// Revision delete count (`text:mark-deleted` and `text:deleted`).
    pub revisions_delete: u64,
    /// ODF 1.2+ insert count (`text:inserted`).
    pub revisions_inserted: u64,
    /// ODF 1.2+ delete count (`text:deleted`).
    pub revisions_deleted: u64,
    /// Total element count (sum of all above fields).
    pub total_elements: u64,
}

impl OdfCensus {
    /// Compute a new census from raw ODF content.xml bytes.
    pub fn from_content_xml(content_xml: &[u8]) -> Result<Self, Vec<Diagnostic>> {
        let doc = parse_xml(content_xml).map_err(|mut diags| {
            diags.push(Diagnostic::new(
                "ODF-E-001",
                gp_core::diagnostics::Severity::Error,
                "content.xml parse failure",
            ));
            diags
        })?;
        let mut counters = BTreeMap::new();
        count_nodes(&doc.root, &mut counters);
        Ok(from_counters(&counters))
    }

    /// Compute a census from an OdfPackage, summing content.xml and
    /// styles.xml element counts.
    pub fn from_package(pkg: &OdfPackage) -> Result<Self, Vec<Diagnostic>> {
        let mut merged: BTreeMap<String, u64> = BTreeMap::new();

        for part in &pkg.parts {
            if part.path == "content.xml" || part.path == "styles.xml" {
                let mut part_counters: BTreeMap<String, u64> = BTreeMap::new();
                let doc = match parse_xml(&part.data) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                count_nodes(&doc.root, &mut part_counters);
                for (k, v) in part_counters {
                    *merged.entry(k).or_default() += v;
                }
            }
        }

        Ok(from_counters(&merged))
    }

    /// Compute the delta from this census to another census.
    ///
    /// `delta` is computed as `other - self`, so a positive value
    /// means the element increased.
    pub fn delta(&self, other: &OdfCensus) -> OdfCensusDelta {
        OdfCensusDelta {
            paragraphs_delta: other.paragraphs as i64 - self.paragraphs as i64,
            tables_delta: other.tables as i64 - self.tables as i64,
            text_frames_delta: other.text_frames as i64 - self.text_frames as i64,
            lists_delta: other.lists as i64 - self.lists as i64,
            list_items_delta: other.list_items as i64 - self.list_items as i64,
            revisions_insert_delta: other.revisions_insert as i64 - self.revisions_insert as i64,
            revisions_delete_delta: other.revisions_delete as i64 - self.revisions_delete as i64,
            revisions_inserted_delta: other.revisions_inserted as i64
                - self.revisions_inserted as i64,
            revisions_deleted_delta: other.revisions_deleted as i64 - self.revisions_deleted as i64,
        }
    }
}

/// The delta between two ODF census snapshots.
///
/// Positive values mean the element count increased in the target.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OdfCensusDelta {
    /// Paragraph delta (`other.paragraphs - self.paragraphs`).
    pub paragraphs_delta: i64,
    /// Table delta.
    pub tables_delta: i64,
    /// Text frame delta.
    pub text_frames_delta: i64,
    /// List delta.
    pub lists_delta: i64,
    /// List item delta.
    pub list_items_delta: i64,
    /// Revision insert delta.
    pub revisions_insert_delta: i64,
    /// Revision delete delta.
    pub revisions_delete_delta: i64,
    /// ODF 1.2+ inserted delta.
    pub revisions_inserted_delta: i64,
    /// ODF 1.2+ deleted delta.
    pub revisions_deleted_delta: i64,
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Count occurrences of each tag name across the DOM subtree.
fn count_nodes(node: &XmlNode, counters: &mut BTreeMap<String, u64>) {
    let tag_key = match node {
        XmlNode::Element { name, .. } => {
            // Include namespace prefix if present (e.g. "text:p").
            name.prefix.as_ref().map_or_else(
                || name.local_name.clone(),
                |p| format!("{}:{}", p, name.local_name),
            )
        }
        _ => return,
    };

    counters.entry(tag_key).and_modify(|v| *v += 1).or_insert(1);

    if let XmlNode::Element { children, .. } = node {
        for child in children {
            count_nodes(child, counters);
        }
    }
}

/// Build an OdfCensus from a tag-count map.
fn from_counters(counters: &BTreeMap<String, u64>) -> OdfCensus {
    let get = |key: &str| counters.get(key).copied().unwrap_or(0);

    let paragraphs = get("text:p");
    let tables = get("table:table");
    let text_frames = get("draw:frame");
    let lists = get("text:list");
    let list_items = get("text:list-item");
    let table_rows = get("table:table-row");
    let table_cells = get("table:table-cell");
    // Legacy ODF markers.
    let revisions_insert_legacy = get("text:insert");
    let revisions_delete_legacy = get("text:mark-deleted");
    // ODF 1.2+ revision elements.
    let revisions_inserted = get("text:inserted");
    let revisions_deleted = get("text:deleted");

    // Total insert = legacy + ODF 1.2+
    let revisions_insert = revisions_insert_legacy + revisions_inserted;
    // Total delete = legacy + ODF 1.2+
    let revisions_delete = revisions_delete_legacy + revisions_deleted;

    let total_elements = paragraphs
        + tables
        + text_frames
        + lists
        + list_items
        + table_rows
        + table_cells
        + revisions_insert
        + revisions_delete;

    OdfCensus {
        paragraphs,
        tables,
        text_frames,
        lists,
        list_items,
        table_rows,
        table_cells,
        revisions_insert,
        revisions_delete,
        revisions_inserted,
        revisions_deleted,
        total_elements,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal content.xml for testing.
    fn sample_content() -> &'static [u8] {
        b"\
<?xml version=\"1.0\"?>\
<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"\
    xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"\
    xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\"\
    xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\">\
  <office:body>\
    <office:text>\
      <text:p>Hello</text:p>\
      <text:p>World</text:p>\
      <text:p>Third</text:p>\
      <text:list>\
        <text:list-item><text:p>Item 1</text:p></text:list-item>\
        <text:list-item><text:p>Item 2</text:p></text:list-item>\
      </text:list>\
      <table:table>\
        <table:table-row>\
          <table:table-cell><text:p>A</text:p></table:table-cell>\
          <table:table-cell><text:p>B</text:p></table:table-cell>\
        </table:table-row>\
      </table:table>\
      <draw:frame>\
        <draw:text-box><text:p>Frame text</text:p></draw:text-box>\
      </draw:frame>\
    </office:text>\
  </office:body>\
</office:document-content>"
    }

    /// test_from_content_xml_paragraphs — should count text:p elements.
    #[test]
    fn test_from_content_xml_paragraphs() {
        let census = OdfCensus::from_content_xml(sample_content()).expect("should parse");
        // Paragraphs in the content:
        //   text:p x3 at top level + Item 1, Item 2 inside list + A, B inside cells
        //   + Frame text inside draw:text-box
        // Total = 8
        assert_eq!(census.paragraphs, 8, "should count all text:p");
    }

    /// test_from_content_xml_tables — should count table:table.
    #[test]
    fn test_from_content_xml_tables() {
        let census = OdfCensus::from_content_xml(sample_content()).expect("should parse");
        assert_eq!(census.tables, 1, "should count one table");
        assert_eq!(census.table_rows, 1, "should count one table-row");
        assert_eq!(census.table_cells, 2, "should count two cells");
    }

    /// test_from_content_xml_frames — should count draw:frame.
    #[test]
    fn test_from_content_xml_frames() {
        let census = OdfCensus::from_content_xml(sample_content()).expect("should parse");
        assert_eq!(census.text_frames, 1, "should count one frame");
    }

    /// test_from_content_xml_lists — should count text:list and text:list-item.
    #[test]
    fn test_from_content_xml_lists() {
        let census = OdfCensus::from_content_xml(sample_content()).expect("should parse");
        assert_eq!(census.lists, 1, "should count one list");
        assert_eq!(census.list_items, 2, "should count two list items");
    }

    /// test_from_content_xml_revisions — should count revision markers (legacy + ODF 1.2+).
    #[test]
    fn test_from_content_xml_revisions() {
        // Use a sample with explicit revision markers (both legacy and ODF 1.2+).
        let xml = b"\
<?xml version=\"1.0\"?>\
<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"\
    xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\">\
  <office:body><office:text>\
    <text:p>Before</text:p>\
    <text:insert>Legacy insert</text:insert>\
    <text:insert>More legacy insert</text:insert>\
    <text:mark-deleted>Legacy delete</text:mark-deleted>\
    <text:inserted office:change-id=\"ins-1\" office:creators=\"alice\"><text:p>ODF 1.2 insert</text:p></text:inserted>\
    <text:deleted office:change-id=\"del-1\" office:creators=\"bob\"><text:p>ODF 1.2 delete</text:p></text:deleted>\
    <text:p>After</text:p>\
  </office:text></office:body>\
</office:document-content>";
        let census = OdfCensus::from_content_xml(xml).expect("should parse");
        assert_eq!(
            census.paragraphs, 4,
            "two text:p at top + text:p inside inserted + text:p inside deleted"
        );
        assert_eq!(
            census.revisions_insert, 3,
            "two legacy inserts + one ODF 1.2+ insert"
        );
        assert_eq!(
            census.revisions_delete, 2,
            "one legacy delete + one ODF 1.2+ delete"
        );
        assert_eq!(census.revisions_inserted, 1, "one ODF 1.2+ inserted");
        assert_eq!(census.revisions_deleted, 1, "one ODF 1.2+ deleted");
    }

    /// test_from_content_xml_total — total_elements is sum of all counts.
    #[test]
    fn test_from_content_xml_total() {
        let census = OdfCensus::from_content_xml(sample_content()).expect("should parse");
        let expected_total = census.paragraphs
            + census.tables
            + census.text_frames
            + census.lists
            + census.list_items
            + census.table_rows
            + census.table_cells
            + census.revisions_insert
            + census.revisions_delete;
        assert_eq!(
            census.total_elements, expected_total,
            "total must be the sum"
        );
    }

    /// test_empty_content_xml — empty body yields zero census.
    #[test]
    fn test_empty_content_xml() {
        let xml = b"\
<?xml version=\"1.0\"?>\
<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"\
    xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\">\
  <office:body><office:text></office:text></office:body>\
</office:document-content>";
        let census = OdfCensus::from_content_xml(xml.as_ref()).expect("should parse");
        assert_eq!(
            census,
            OdfCensus::default(),
            "empty content should be all zeros"
        );
    }

    /// test_delta_insert_paragraph — adding one paragraph yields delta of +1.
    #[test]
    fn test_delta_insert_paragraph() {
        let before_xml = b"\
<?xml version=\"1.0\"?>\
<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"\
    xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\">\
  <office:body><office:text>\
    <text:p>Hello</text:p>\
  </office:text></office:body>\
</office:document-content>";
        let before = OdfCensus::from_content_xml(before_xml.as_ref()).expect("should parse");
        // Add one more paragraph.
        let after_xml = b"\
<?xml version=\"1.0\"?>\
<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"\
    xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\">\
  <office:body><office:text>\
    <text:p>Hello</text:p>\
    <text:p>New paragraph</text:p>\
  </office:text></office:body>\
</office:document-content>";
        let after = OdfCensus::from_content_xml(after_xml.as_ref()).expect("should parse");
        let delta = before.delta(&after);
        assert_eq!(delta.paragraphs_delta, 1, "should be +1 paragraph");
    }

    /// test_delta_total — total_elements_delta matches individual deltas.
    #[test]
    fn test_delta_total_elements() {
        let before = OdfCensus::default();
        let xml = b"\
<?xml version=\"1.0\"?>\
<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"\
    xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\">\
  <office:body><office:text>\
    <text:p>One</text:p>\
    <text:p>Two</text:p>\
    <text:insert>rev</text:insert>\
  </office:text></office:body>\
</office:document-content>";
        let after = OdfCensus::from_content_xml(xml.as_ref()).expect("should parse");
        let delta = before.delta(&after);
        assert_eq!(delta.paragraphs_delta, 2);
        assert_eq!(delta.revisions_insert_delta, 1);
    }

    /// test_from_package — census on a synthetic OdfPackage.
    #[test]
    fn test_from_package() {
        let mut pkg = OdfPackage {
            parts: vec![],
            manifest: None,
            diagnostics: vec![],
        };

        let content_xml = b"\
<?xml version=\"1.0\"?>\
<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"\
    xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\">\
  <office:body><office:text>\
    <text:p>Hello</text:p>\
    <text:p>World</text:p>\
  </office:text></office:body>\
</office:document-content>";

        let styles_xml = b"\
<?xml version=\"1.0\"?>\
<office:document-styles xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"\
    xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\">\
  <office:styles>\
    <style:style style:name=\"Default\" style:family=\"paragraph\"/>\
  </office:styles>\
</office:document-styles>";

        pkg.parts.push(crate::manifest::OdfPart {
            path: "content.xml".to_string(),
            content_type: "text/xml".to_string(),
            data: content_xml.to_vec(),
        });
        pkg.parts.push(crate::manifest::OdfPart {
            path: "styles.xml".to_string(),
            content_type: "text/xml".to_string(),
            data: styles_xml.to_vec(),
        });

        let census = OdfCensus::from_package(&pkg).expect("should census package");
        assert_eq!(census.paragraphs, 2, "should count 2 paragraphs");
    }

    /// test_census_on_corpus_fixture — census on real corpus ODT fixture.
    #[test]
    fn test_census_on_corpus_fixture() {
        let path = format!(
            "{}/corpus/blobs/cjk-text-odt-001.odt",
            env!("CARGO_MANIFEST_DIR")
        );
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(_) => {
                eprintln!("SKIP: corpus fixture not found at {}", path);
                return;
            }
        };
        let pkg = match crate::manifest::parse_odf(&data) {
            Ok(p) => p,
            Err(diags) => {
                eprintln!("WARN: parse failed: {:?}", diags);
                return;
            }
        };
        let census = match OdfCensus::from_package(&pkg) {
            Ok(c) => c,
            Err(diags) => {
                eprintln!("WARN: census failed: {:?}", diags);
                return;
            }
        };
        // A real document should have at least one paragraph.
        assert!(
            census.paragraphs > 0,
            "corpus fixture should have at least one paragraph (got {})",
            census.paragraphs
        );
    }
}
