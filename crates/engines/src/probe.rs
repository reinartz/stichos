//! Probe framework: real conversion tests to verify engine capabilities.
//!
//! This module provides a probe system that exercises engine conversion
//! paths with embedded fixtures, records results, and caches them for
//! repeated use.
//!
//! # Architecture
//!
//! - [`ProbeFixture`] — embedded test document with source/target format tags.
//! - [`ProbeEngine`] — trait for engines that support probing (default impl
//!   on any `Engine` via `probe_engine()`).
//! - [`ProbeResult`] — JSON-serialisable probe outcome.
//! - [`ProbeCache`] — persistent L1 cache that survives across probe runs.
//! - [`ProbeNMinusOne`] — N/N-1 fallback strategy that tries current version
//!   then rolls back to N-1 if the current probe fails.
//!
//! # Usage
//!
//! ```no_run
//! use engines::probe::{probe_engine, ProbeCache, ProbeNMinusOne};
//! use engines::libreoffice::LibreOfficeEngine;
//!
//! let engine = LibreOfficeEngine::new("/usr/bin/soffice").unwrap();
//! let result = probe_engine(&engine);
//! let cache = ProbeCache::load_or_default();
//! let n1 = ProbeNMinusOne::new(&engine, None, None);
//! ```

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use gp_core::diagnostics::Diagnostic;
use gp_core::diagnostics::Severity;
use gp_core::traits::CapabilityEntry;
use gp_core::traits::CapabilityMatrix;
use gp_core::traits::Engine;

// ======================================================================
// Types
// ======================================================================

/// A probe fixture: an embedded document used to test real conversion.
///
/// The fixture carries the raw bytes of a document in `source_format`
/// encoding, and the engine is expected to produce `target_format` output.
#[derive(Debug, Clone)]
pub struct ProbeFixture {
    /// The format of the embedded data (e.g. `"docx"`).
    pub source_format: String,
    /// The expected output format (e.g. `"pdf"`).
    pub target_format: String,
    /// A human-readable name for the fixture.
    pub name: String,
    /// The document bytes.
    pub data: Vec<u8>,
}

/// Result of a single fixture conversion attempt during probing.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConversionResult {
    /// Fixture name.
    pub fixture: String,
    /// Source format.
    pub source: String,
    /// Target format.
    pub target: String,
    /// Whether the conversion succeeded.
    pub works: bool,
    /// Byte length of the output on success, 0 on failure.
    pub output_bytes: usize,
}

/// Outcome of probing an engine with a set of fixtures.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProbeOutcome {
    /// Individual conversion results.
    pub conversions: Vec<ConversionResult>,
    /// Overall success ratio as a fraction 0.0..=1.0.
    pub pass_ratio: f64,
}

/// A single entry in the probe cache (keyed by engine_id + source + target).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProbeCacheEntry {
    /// Whether that conversion path works.
    pub works: bool,
    /// When this cache entry was recorded (unix epoch seconds).
    pub timestamp: u64,
}

/// Persistent probe cache stored on disk.
///
/// On `load()` the cache reads from the cache file; on `save()` it
/// writes back. The on-disk format is a JSON object.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProbeCache {
    /// Cached results keyed by `engine_id:source:target`.
    pub entries: HashMap<String, ProbeCacheEntry>,
    /// Path to the on-disk cache file (None = in-memory only).
    pub path: Option<String>,
}

/// Result of a full N/N-1 probe run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NMinusOneResult {
    /// The engine that was probed.
    pub engine_id: String,
    /// Version that was probed.
    pub version: String,
    /// Which version ultimately passed (current or N-1).
    pub working_version: String,
    /// Whether the current version passed.
    pub current_passed: bool,
    /// Whether the N-1 version was tried and passed.
    pub n_minus_one_passed: bool,
    /// Conversion results for the version that worked.
    pub results: ProbeOutcome,
}

// ======================================================================
// Probe fixtures — embedded minimal documents
// ======================================================================

/// IEEE CRC-32 (streaming, bitwise — no external dep). Matches zlib's crc32.,
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Build a minimal valid ZIP archive (STORE compression, real CRC).
fn build_minimal_zip(names: &[&[u8]], data: &[&[u8]]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut local_offsets: Vec<u32> = Vec::new();

    // Local file headers
    let mut offset: u32 = 0;
    for (i, name) in names.iter().enumerate() {
        let content = data[i];
        let name_bytes = name.to_vec();
        let name_len = name_bytes.len() as u16;
        let content_len = content.len() as u32;
        let crc = crc32(content);

        result.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]); // "PK\x03\x04" local header
        result.extend_from_slice(&[0x14, 0x00]); // version needed 2.0
        result.extend_from_slice(&[0x00, 0x00]); // flags
        result.extend_from_slice(&[0x00, 0x00]); // method: store
        result.extend_from_slice(&[0u8; 4]); // time + date
        result.extend_from_slice(&crc.to_le_bytes()); // CRC32
        result.extend_from_slice(&content_len.to_le_bytes());
        result.extend_from_slice(&content_len.to_le_bytes());
        result.extend_from_slice(&name_len.to_le_bytes());
        result.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        result.extend_from_slice(&name_bytes);
        result.extend_from_slice(content);

        local_offsets.push(offset);
        offset += 30 + name_len as u32 + content_len;
    }

    // Central directory
    let cd_offset = result.len() as u32;
    for (i, name) in names.iter().enumerate() {
        let name_bytes = name.to_vec();
        let name_len = name_bytes.len() as u16;
        let content = data[i];
        let crc = crc32(content);

        result.extend_from_slice(&[0x50, 0x4B, 0x01, 0x02]); // "PK\x01\x02" central dir
        result.extend_from_slice(&[0x14, 0x00]); // version made by
        result.extend_from_slice(&[0x14, 0x00]); // version needed
        result.extend_from_slice(&[0x00, 0x00]); // flags
        result.extend_from_slice(&[0x00, 0x00]); // method: store
        result.extend_from_slice(&[0u8; 4]); // time + date
        result.extend_from_slice(&crc.to_le_bytes()); // CRC32
        result.extend_from_slice(&(content.len() as u32).to_le_bytes());
        result.extend_from_slice(&(content.len() as u32).to_le_bytes());
        result.extend_from_slice(&name_len.to_le_bytes());
        result.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        result.extend_from_slice(&0u16.to_le_bytes()); // comment length
        result.extend_from_slice(&0u16.to_le_bytes()); // disk number start
        result.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        result.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        result.extend_from_slice(&local_offsets[i].to_le_bytes());
        result.extend_from_slice(&name_bytes);
    }

    // End of central directory
    let cd_size = (result.len() as u32) - cd_offset;
    let num_entries = names.len() as u16;
    result.extend_from_slice(&[0x50, 0x4B, 0x05, 0x06]); // "PK\x05\x06" EOCD
    result.extend_from_slice(&0u16.to_le_bytes()); // disk number
    result.extend_from_slice(&0u16.to_le_bytes()); // central dir disk
    result.extend_from_slice(&num_entries.to_le_bytes());
    result.extend_from_slice(&num_entries.to_le_bytes());
    result.extend_from_slice(&cd_size.to_le_bytes());
    result.extend_from_slice(&cd_offset.to_le_bytes());
    result.extend_from_slice(&0u16.to_le_bytes()); // comment length

    result
}

/// Build a minimal valid DOCX bytes (ZIP archive with OPC structure).
fn minimal_docx_bytes() -> Vec<u8> {
    let content_types = b"[Content_Types].xml";
    let content_types_data = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
        <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
        <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
        <Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
        </Types>"
    .as_slice();
    let rels_xml = b"_rels/.rels";
    let rels_data = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
        </Relationships>"
    .as_slice();
    let document_xml = b"word/document.xml";
    let document_data = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
        <w:body>\
        <w:p>\
        <w:r>\
        <w:t>Hello from Stichos probe.</w:t>\
        </w:r>\
        </w:p>\
        </w:body>\
        </w:document>"
        .as_slice();

    build_minimal_zip(
        &[content_types, rels_xml, document_xml],
        &[content_types_data, rels_data, document_data],
    )
}

/// Build a minimal valid PPTX bytes (ZIP archive).
fn minimal_pptx_bytes() -> Vec<u8> {
    let content_types = b"[Content_Types].xml";
    let content_types_data = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
        <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
        <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
        <Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>\
        <Override PartName=\"/ppt/slides/slide1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>\
        </Types>"
    .as_slice();
    let rels_xml = b"_rels/.rels";
    let rels_data = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/>\
        </Relationships>"
    .as_slice();
    let presentation_xml = b"ppt/presentation.xml";
    let presentation_data = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <p:presentation xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\
        <p:sldIdLst>\
        <p:sldId id=\"1\" relationshipId=\"rId1\"/>\
        </p:sldIdLst>\
        </p:presentation>"
        .as_slice();
    let presentation_rels = b"ppt/_rels/presentation.xml.rels";
    let presentation_rels_data = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide1.xml\"/>\
        </Relationships>"
    .as_slice();
    let slide_xml = b"ppt/slides/slide1.xml";
    let slide_data = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <p:sld xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\
        <p:cSld>\
        <p:spTree/>\
        </p:cSld>\
        </p:sld>"
        .as_slice();

    build_minimal_zip(
        &[
            content_types,
            rels_xml,
            presentation_xml,
            presentation_rels,
            slide_xml,
        ],
        &[
            content_types_data,
            rels_data,
            presentation_data,
            presentation_rels_data,
            slide_data,
        ],
    )
}

/// Build a minimal valid XLSX bytes (ZIP archive).
fn minimal_xlsx_bytes() -> Vec<u8> {
    let content_types = b"[Content_Types].xml";
    let content_types_data = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
        <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
        <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
        <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
        <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
        </Types>"
    .as_slice();
    let rels_xml = b"_rels/.rels";
    let rels_data = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
        </Relationships>"
    .as_slice();
    let workbook_xml = b"xl/workbook.xml";
    let workbook_data = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
        <sheets>\
        <sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/>\
        </sheets>\
        </workbook>"
    .as_slice();
    let workbook_rels = b"xl/_rels/workbook.xml.rels";
    let workbook_rels_data = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
        <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
        </Relationships>"
    .as_slice();
    let sheet_xml = b"xl/worksheets/sheet1.xml";
    let sheet_data = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
        <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
        <dimension ref=\"A1\"/>\
        <sheetData/>\
        </worksheet>"
        .as_slice();

    build_minimal_zip(
        &[
            content_types,
            rels_xml,
            workbook_xml,
            workbook_rels,
            sheet_xml,
        ],
        &[
            content_types_data,
            rels_data,
            workbook_data,
            workbook_rels_data,
            sheet_data,
        ],
    )
}

/// Build a minimal valid ODT bytes (ZIP archive).
///
/// Note: LibreOffice 26 rejects manifest `full-path` values with a leading
/// slash, so entries are written as `content.xml`, not `/content.xml`.
fn minimal_odt_bytes() -> Vec<u8> {
    let content_types = b"mimetype";
    let content_types_data = b"application/vnd.oasis.opendocument.text".as_slice();
    let manifest_xml = b"META-INF/manifest.xml";
    let manifest_data = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
        <manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.4\">\
        <manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.4\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/>\
        <manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/>\
        </manifest:manifest>"
    .as_slice();
    let content_xml = b"content.xml";
    let content_data = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
        <office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
        xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
        xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" office:version=\"1.4\">\
        <office:body>\
        <office:text>\
        <text:p>Probe ODT document.</text:p>\
        </office:text>\
        </office:body>\
        </office:document-content>"
        .as_slice();

    build_minimal_zip(
        &[content_types, manifest_xml, content_xml],
        &[content_types_data, manifest_data, content_data],
    )
}

/// Build a minimal valid HTML document with some text content.
fn minimal_html_bytes() -> Vec<u8> {
    b"\
<!DOCTYPE html>
<html lang=\"en\">
<head><meta charset=\"UTF-8\"><title>Stichos Probe</title></head>
<body><h1>Chromium Probe</h1><p>Minimal HTML document for engine probing.</p></body>
</html>\
"
    .to_vec()
}

/// Build a minimal valid PDF v1.4 bytes.
fn minimal_pdf_bytes() -> Vec<u8> {
    let raw = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\nxref\n0 4\n0000000000 65535 f \n0000000009 00000 n \n0000000058 00000 n \n0000000115 00000 n \ntrailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n190\n%%EOF";
    raw.to_vec()
}

/// Build a minimal valid 1x1 white PNG bytes.
fn minimal_png_bytes() -> Vec<u8> {
    vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR: 13 bytes
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1x1
        0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, // bit depth 8, RGB
        0xDE, // IHDR CRC
        0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, // IDAT: 12 bytes
        0x08, 0xD7, 0x63, 0x60, 0x60, 0x60, 0x60, 0x60, 0x60, 0x60, 0x60, 0x20, 0x52,
        0x4D, // IDAT data
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, // IEND
        0xAE, 0x42, 0x60, 0x82, // IEND CRC
    ]
}

/// Return the standard probe fixtures for engine capability testing.
pub fn standard_fixtures() -> Vec<ProbeFixture> {
    vec![
        ProbeFixture {
            source_format: "docx".into(),
            target_format: "pdf".into(),
            name: "minimal_docx_to_pdf".into(),
            data: minimal_docx_bytes(),
        },
        ProbeFixture {
            source_format: "pptx".into(),
            target_format: "pdf".into(),
            name: "minimal_pptx_to_pdf".into(),
            data: minimal_pptx_bytes(),
        },
        ProbeFixture {
            source_format: "xlsx".into(),
            target_format: "pdf".into(),
            name: "minimal_xlsx_to_pdf".into(),
            data: minimal_xlsx_bytes(),
        },
        ProbeFixture {
            source_format: "odt".into(),
            target_format: "pdf".into(),
            name: "minimal_odt_to_pdf".into(),
            data: minimal_odt_bytes(),
        },
        ProbeFixture {
            source_format: "html".into(),
            target_format: "pdf".into(),
            name: "minimal_html_to_pdf".into(),
            data: minimal_html_bytes(),
        },
        ProbeFixture {
            source_format: "pdf".into(),
            target_format: "png".into(),
            name: "minimal_pdf_to_png".into(),
            data: minimal_pdf_bytes(),
        },
        ProbeFixture {
            source_format: "png".into(),
            target_format: "pdf".into(),
            name: "minimal_png_to_pdf".into(),
            data: minimal_png_bytes(),
        },
    ]
}

/// Return a subset of fixtures focused on document → PDF (most common case).
pub fn document_to_pdf_fixtures() -> Vec<ProbeFixture> {
    standard_fixtures()
        .into_iter()
        .filter(|f| f.target_format == "pdf")
        .collect()
}

/// Return fixtures for a specific source format.
pub fn fixtures_for_source(source: &str) -> Vec<ProbeFixture> {
    standard_fixtures()
        .into_iter()
        .filter(|f| f.source_format == source)
        .collect()
}

/// Return fixtures for a specific target format.
pub fn fixtures_for_target(target: &str) -> Vec<ProbeFixture> {
    standard_fixtures()
        .into_iter()
        .filter(|f| f.target_format == target)
        .collect()
}

// ======================================================================
// Probe cache
// ======================================================================

impl ProbeCache {
    /// Create a new empty probe cache.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            path: None,
        }
    }

    /// Create a new probe cache with a persistent on-disk path.
    pub fn with_path<P: AsRef<Path>>(path: P) -> Self {
        Self {
            entries: HashMap::new(),
            path: Some(path.as_ref().to_string_lossy().to_string()),
        }
    }

    /// Load cache from disk, or return an empty cache if the file does not
    /// exist or cannot be parsed.
    pub fn load_or_default() -> Self {
        let home = std::env::temp_dir()
            .join("stichos")
            .join("probe-cache.json");
        let cache = Self::with_path(&home);
        cache.load().unwrap_or(cache)
    }

    /// Load the cache from disk.
    pub fn load(&self) -> Result<Self, Vec<Diagnostic>> {
        let path = match &self.path {
            Some(p) => p.clone(),
            None => return Ok(Self::new()),
        };
        let mut file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => {
                return Ok(Self {
                    entries: HashMap::new(),
                    path: Some(path),
                })
            }
        };
        let mut contents = String::new();
        let _ = file.read_to_string(&mut contents);
        serde_json::from_str(&contents).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to parse probe cache: {}", e),
            )]
        })
    }

    /// Save the cache to disk.
    pub fn save(&self) -> Result<(), Vec<Diagnostic>> {
        let path = match &self.path {
            Some(p) => p.clone(),
            None => return Ok(()),
        };
        if let Some(parent) = Path::new(&path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    vec![Diagnostic::new(
                        "ENG-E-001",
                        Severity::Error,
                        format!("failed to create cache dir: {}", e),
                    )]
                })?;
            }
        }
        let contents = serde_json::to_string_pretty(self).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to serialise probe cache: {}", e),
            )]
        })?;
        std::fs::write(&path, contents).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to write probe cache: {}", e),
            )]
        })?;
        Ok(())
    }

    /// Generate a deterministic cache key for a given engine, source, target.
    pub fn key(engine_id: &str, source: &str, target: &str) -> String {
        format!("{}:{}:{}", engine_id, source, target)
    }

    /// Look up a cached result for the given engine/source/target.
    pub fn get(&self, engine_id: &str, source: &str, target: &str) -> Option<bool> {
        self.entries
            .get(&Self::key(engine_id, source, target))
            .map(|entry| entry.works)
    }

    /// Store a result in the cache.
    pub fn set(&mut self, engine_id: &str, source: &str, target: &str, works: bool) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.entries.insert(
            Self::key(engine_id, source, target),
            ProbeCacheEntry {
                works,
                timestamp: ts,
            },
        );
    }

    /// Return the number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return true if the cache has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for ProbeCache {
    fn default() -> Self {
        Self::new()
    }
}

// ======================================================================
// Probe engine
// ======================================================================

/// Probe an engine using the standard fixtures.
///
/// This runs every built-in fixture through the engine and returns a
/// [`ProbeOutcome`] describing which conversions succeeded.
pub fn probe_engine<E: Engine>(engine: &E) -> ProbeOutcome {
    let fixtures = standard_fixtures();
    probe_with(engine, &fixtures)
}

/// Probe an engine with a specific set of fixtures.
pub fn probe_with<E: Engine>(engine: &E, fixtures: &[ProbeFixture]) -> ProbeOutcome {
    let conversions: Vec<ConversionResult> =
        fixtures.iter().map(|f| try_convert(engine, f)).collect();
    let total = conversions.len();
    let passed = conversions.iter().filter(|c| c.works).count();
    let pass_ratio = if total == 0 {
        0.0
    } else {
        passed as f64 / total as f64
    };

    ProbeOutcome {
        conversions,
        pass_ratio,
    }
}

/// Try a real conversion through the engine for a single fixture.
///
/// 1. Attempt conversion via `engine.convert()`.
/// 2. If the engine returns Ok, the conversion worked.
/// 3. If the engine returns Err, the conversion failed.
/// 4. Returns a [`ConversionResult`] summarising the outcome.
fn try_convert<E: Engine>(engine: &E, fixture: &ProbeFixture) -> ConversionResult {
    let result = engine.convert(
        &fixture.data,
        &fixture.source_format,
        &fixture.target_format,
    );
    match result {
        Ok(bytes) => ConversionResult {
            fixture: fixture.name.clone(),
            source: fixture.source_format.clone(),
            target: fixture.target_format.clone(),
            works: true,
            output_bytes: bytes.len(),
        },
        Err(_) => ConversionResult {
            fixture: fixture.name.clone(),
            source: fixture.source_format.clone(),
            target: fixture.target_format.clone(),
            works: false,
            output_bytes: 0,
        },
    }
}

// ======================================================================
// Probe cache integration
// ======================================================================

/// Probe an engine with cache-aware lookup.
///
/// Checks the cache first; only runs fixtures that are not already cached.
/// Un-cached results are written back to the cache.
pub fn probe_engine_cached<E: Engine>(engine: &E, cache: &mut ProbeCache) -> ProbeOutcome {
    let fixtures = standard_fixtures();
    let conversions: Vec<ConversionResult> = fixtures
        .iter()
        .map(|f| {
            let key = ProbeCache::key(engine.engine_id(), &f.source_format, &f.target_format);
            let cached = cache.entries.get(&key);

            let result = match cached {
                Some(entry) => ConversionResult {
                    fixture: f.name.clone(),
                    source: f.source_format.clone(),
                    target: f.target_format.clone(),
                    works: entry.works,
                    output_bytes: 0,
                },
                None => try_convert(engine, f),
            };

            // Record uncached results back into the cache
            if cached.is_none() {
                cache.set(
                    engine.engine_id(),
                    &f.source_format,
                    &f.target_format,
                    result.works,
                );
            }

            result
        })
        .collect();

    let total = conversions.len();
    let passed = conversions.iter().filter(|c| c.works).count();
    let pass_ratio = if total == 0 {
        0.0
    } else {
        passed as f64 / total as f64
    };

    ProbeOutcome {
        conversions,
        pass_ratio,
    }
}

// ======================================================================
// N / N-1 fallback
// ======================================================================

/// N/N-1 probe strategy.
///
/// First probes the current engine version; if all conversions fail,
/// tries the N-1 (previous) version engine.  Returns whichever version
/// produced a passing probe, or the current version's result as a best
/// effort.
pub struct ProbeNMinusOne<'e, E: Engine> {
    /// The current (N) version of the engine.
    current: &'e E,
    /// Optional previous (N-1) version engine.
    previous: Option<E>,
    /// Version string of the N-1 engine.
    n_minus_one_version: Option<String>,
}

impl<'e, E: Engine> ProbeNMinusOne<'e, E> {
    /// Create a new N/N-1 probe strategy.
    ///
    /// `n_minus_one_engine` is the previous version engine, and
    /// `n_minus_one_version` is its version string (e.g. `"25.0.0"`).
    pub fn new(
        current: &'e E,
        n_minus_one_engine: Option<E>,
        n_minus_one_version: Option<String>,
    ) -> Self {
        Self {
            current,
            previous: n_minus_one_engine,
            n_minus_one_version,
        }
    }

    /// Run the N/N-1 probe strategy with standard fixtures.
    pub fn probe(&self) -> NMinusOneResult {
        self.probe_with(&standard_fixtures())
    }

    /// Run the N/N-1 probe strategy with a specific fixture set.
    pub fn probe_with(&self, fixtures: &[ProbeFixture]) -> NMinusOneResult {
        let current_version = self
            .current
            .version()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let current_passed = self.check_version(fixtures, self.current);

        if current_passed {
            let results = probe_with(self.current, fixtures);
            return NMinusOneResult {
                engine_id: self.current.engine_id().to_string(),
                version: current_version.clone(),
                working_version: current_version.clone(),
                current_passed: true,
                n_minus_one_passed: false,
                results,
            };
        }

        // Current version failed — try N-1
        if let Some(ref prev) = self.previous {
            let prev_version = self
                .n_minus_one_version
                .clone()
                .or_else(|| prev.version().map(|v| v.to_string()))
                .unwrap_or_else(|| "unknown".to_string());
            let prev_passed = self.check_version(fixtures, prev);

            if prev_passed {
                let results = probe_with(prev, fixtures);
                return NMinusOneResult {
                    engine_id: prev.engine_id().to_string(),
                    version: prev_version.clone(),
                    working_version: prev_version.clone(),
                    current_passed: false,
                    n_minus_one_passed: true,
                    results,
                };
            }
        }

        // Neither version passed — return current with best effort
        let results = probe_with(self.current, fixtures);
        NMinusOneResult {
            engine_id: self.current.engine_id().to_string(),
            version: current_version,
            working_version: "none".to_string(),
            current_passed: false,
            n_minus_one_passed: false,
            results,
        }
    }

    /// Check if any conversion in the fixture set is claimable.
    fn check_version(&self, fixtures: &[ProbeFixture], engine: &E) -> bool {
        fixtures
            .iter()
            .any(|f| engine.can_convert(&f.source_format, &f.target_format))
    }
}

// ======================================================================
// ProbeResult conversion
// ======================================================================

/// Convert a ProbeOutcome into a CapabilityMatrix suitable for the Probe trait.
pub fn outcome_to_matrix(
    outcome: &ProbeOutcome,
    engine_id: &str,
    version: &str,
) -> CapabilityMatrix {
    CapabilityMatrix {
        engine: engine_id.to_string(),
        version: Some(version.to_string()),
        conversions: outcome
            .conversions
            .iter()
            .map(|c| CapabilityEntry {
                source: c.source.clone(),
                target: c.target.clone(),
                works: c.works,
            })
            .collect(),
    }
}

// ======================================================================
// Tests
// ======================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockEngine;

    /// test_probe_engine_available: mock engine → conversions marked as working.
    #[test]
    fn test_probe_engine_available() {
        let engine = MockEngine::new();
        let outcome = probe_engine(&engine);

        assert!(!outcome.conversions.is_empty(), "should have conversions");
        let docx_pdf = outcome
            .conversions
            .iter()
            .find(|c| c.source == "docx" && c.target == "pdf")
            .expect("should have docx→pdf");
        assert!(docx_pdf.works, "mock engine conversion should succeed");
        assert!(docx_pdf.output_bytes > 0, "should have output bytes");
    }

    /// test_probe_engine_unavailable: unavailable mock engine → all fail.
    #[test]
    fn test_probe_engine_unavailable() {
        let mut engine = MockEngine::new();
        engine.available = false;

        let outcome = probe_engine(&engine);
        assert!(
            outcome.conversions.iter().all(|c| !c.works),
            "unavailable engine should have all conversions fail"
        );
        assert_eq!(
            outcome.pass_ratio, 0.0,
            "pass ratio should be 0 for unavailable engine"
        );
    }

    /// test_probe_cache_get_set: cache get/set round-trip works.
    #[test]
    fn test_probe_cache_get_set() {
        let mut cache = ProbeCache::new();
        assert!(cache.is_empty());

        let key = ProbeCache::key("mock", "docx", "pdf");
        assert!(!cache.entries.contains_key(&key));

        cache.set("mock", "docx", "pdf", true);
        assert_eq!(cache.len(), 1);

        assert_eq!(cache.get("mock", "docx", "pdf"), Some(true));
        assert_eq!(cache.get("mock", "xlsx", "pdf"), None);

        cache.set("mock", "xlsx", "pdf", false);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get("mock", "xlsx", "pdf"), Some(false));
    }

    /// test_probe_cache_key_generation: keys are deterministic.
    #[test]
    fn test_probe_cache_key_generation() {
        let k1 = ProbeCache::key("libreoffice", "docx", "pdf");
        let k2 = ProbeCache::key("libreoffice", "docx", "pdf");
        let k3 = ProbeCache::key("pandoc", "docx", "pdf");
        let k4 = ProbeCache::key("libreoffice", "xlsx", "pdf");

        assert_eq!(k1, k2, "same params should produce same key");
        assert_ne!(k3, k1, "different engine should produce different key");
        assert_ne!(k4, k1, "different source should produce different key");

        assert_eq!(k1, "libreoffice:docx:pdf");
        assert_eq!(k3, "pandoc:docx:pdf");
        assert_eq!(k4, "libreoffice:xlsx:pdf");
    }

    /// test_probe_cache_serializes: cache round-trip via JSON works.
    #[test]
    fn test_probe_cache_serializes() {
        let mut cache = ProbeCache::new();
        cache.set("mock", "docx", "pdf", true);
        cache.set("mock", "xlsx", "pdf", false);
        cache.set("libreoffice", "pptx", "pdf", true);

        let json = serde_json::to_string(&cache).expect("cache should serialize");
        let back: ProbeCache = serde_json::from_str(&json).expect("cache should deserialize");

        assert_eq!(cache.entries.len(), back.entries.len());
        for key in cache.entries.keys() {
            assert!(
                back.entries.contains_key(key),
                "deserialized cache should contain key {}",
                key
            );
            assert_eq!(cache.entries[key], back.entries[key],);
        }
    }

    /// test_n_n_minus_one_fallback: probe current then fallback.
    #[test]
    fn test_n_n_minus_one_fallback() {
        let current = MockEngine::new();
        let mut n_minus_one = MockEngine::new();
        n_minus_one.id = "mock-n1".into();
        n_minus_one.version = "0.0.0-n1".into();

        let probe = ProbeNMinusOne::new(&current, Some(n_minus_one), Some("0.0.0-n1".into()));
        let result = probe.probe();

        assert!(
            result.current_passed,
            "current version should pass with mock engine"
        );
        assert!(
            !result.n_minus_one_passed,
            "N-1 should not be tried when current passes"
        );
        assert_eq!(result.working_version, "0.0.0-mock");
        assert!(!result.results.conversions.is_empty());
    }

    /// test_n_n_minus_one_fallback_when_current_fails: current fails, N-1 works.
    #[test]
    fn test_n_n_minus_one_fallback_when_current_fails() {
        let mut current = MockEngine::new();
        current.available = false;

        let mut n_minus_one = MockEngine::new();
        n_minus_one.id = "mock-n1".into();
        n_minus_one.version = "0.0.0-n1".into();

        let probe = ProbeNMinusOne::new(&current, Some(n_minus_one), Some("0.0.0-n1".into()));
        let result = probe.probe();

        assert!(
            !result.current_passed,
            "current version should fail (unavailable)"
        );
        assert!(
            result.n_minus_one_passed,
            "N-1 version should be tried and pass"
        );
        assert_eq!(result.working_version, "0.0.0-n1");
        assert!(result.results.conversions.iter().any(|c| c.works));
    }

    /// test_n_n_minus_one_no_previous: no N-1 engine provided.
    #[test]
    fn test_n_n_minus_one_no_previous() {
        let current = MockEngine::new();

        let probe = ProbeNMinusOne::new(&current, None, None);
        let result = probe.probe();

        assert!(result.current_passed);
        assert!(!result.n_minus_one_passed);
        assert_eq!(result.working_version, "0.0.0-mock");
    }

    /// test_probe_with_custom_fixtures: probe with a subset of fixtures.
    #[test]
    fn test_probe_with_custom_fixtures() {
        let engine = MockEngine::new();
        let fixtures = vec![
            ProbeFixture {
                source_format: "docx".into(),
                target_format: "pdf".into(),
                name: "test_docx".into(),
                data: b"docx data".to_vec(),
            },
            ProbeFixture {
                source_format: "xlsx".into(),
                target_format: "pdf".into(),
                name: "test_xlsx".into(),
                data: b"xlsx data".to_vec(),
            },
        ];

        let outcome = probe_with(&engine, &fixtures);
        assert_eq!(outcome.conversions.len(), 2);
        assert_eq!(outcome.pass_ratio, 1.0);
    }

    /// test_fixtures_for_source: filter fixtures by source format.
    #[test]
    fn test_fixtures_for_source() {
        let docx_fixtures = fixtures_for_source("docx");
        assert_eq!(docx_fixtures.len(), 1);
        assert_eq!(docx_fixtures[0].source_format, "docx");
        assert_eq!(docx_fixtures[0].name, "minimal_docx_to_pdf");

        let pdf_fixtures = fixtures_for_source("pdf");
        assert_eq!(pdf_fixtures.len(), 1);
        assert_eq!(pdf_fixtures[0].source_format, "pdf");
    }

    /// test_fixtures_for_target: filter fixtures by target format.
    #[test]
    fn test_fixtures_for_target() {
        let pdf_fixtures = fixtures_for_target("pdf");
        assert_eq!(pdf_fixtures.len(), 6);
        for f in &pdf_fixtures {
            assert_eq!(f.target_format, "pdf");
        }
    }

    /// test_document_to_pdf_fixtures: document→PDF subset.
    #[test]
    fn test_document_to_pdf_fixtures() {
        let fixtures = document_to_pdf_fixtures();
        assert_eq!(fixtures.len(), 6);
        for f in &fixtures {
            assert_eq!(f.target_format, "pdf");
        }
    }

    /// test_standard_fixtures_completeness: all standard fixtures exist.
    #[test]
    fn test_standard_fixtures_completeness() {
        let fixtures = standard_fixtures();
        assert_eq!(fixtures.len(), 7);
        let names: Vec<&str> = fixtures.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"minimal_docx_to_pdf"));
        assert!(names.contains(&"minimal_pptx_to_pdf"));
        assert!(names.contains(&"minimal_xlsx_to_pdf"));
        assert!(names.contains(&"minimal_odt_to_pdf"));
        assert!(names.contains(&"minimal_html_to_pdf"));
        assert!(names.contains(&"minimal_pdf_to_png"));
        assert!(names.contains(&"minimal_png_to_pdf"));
    }

    /// test_minimal_docx_bytes_produces_valid_zip: minimal DOCX is a valid ZIP.
    #[test]
    fn test_minimal_docx_bytes_produces_valid_zip() {
        let data = minimal_docx_bytes();
        assert!(!data.is_empty());
        assert_eq!(
            &data[0..4],
            &[0x50, 0x4B, 0x03, 0x04],
            "should start with PK\\x03\\x04"
        );
    }

    /// test_minimal_pptx_bytes_produces_valid_zip: minimal PPTX is a valid ZIP.
    #[test]
    fn test_minimal_pptx_bytes_produces_valid_zip() {
        let data = minimal_pptx_bytes();
        assert!(!data.is_empty());
        assert_eq!(
            &data[0..4],
            &[0x50, 0x4B, 0x03, 0x04],
            "should start with PK\\x03\\x04"
        );
    }

    /// test_minimal_xlsx_bytes_produces_valid_zip: minimal XLSX is a valid ZIP.
    #[test]
    fn test_minimal_xlsx_bytes_produces_valid_zip() {
        let data = minimal_xlsx_bytes();
        assert!(!data.is_empty());
        assert_eq!(
            &data[0..4],
            &[0x50, 0x4B, 0x03, 0x04],
            "should start with PK\\x03\\x04"
        );
    }

    /// test_minimal_odt_bytes_produces_valid_zip: minimal ODT is a valid ZIP.
    #[test]
    fn test_minimal_odt_bytes_produces_valid_zip() {
        let data = minimal_odt_bytes();
        assert!(!data.is_empty());
        assert_eq!(
            &data[0..4],
            &[0x50, 0x4B, 0x03, 0x04],
            "should start with PK\\x03\\x04"
        );
    }

    /// test_minimal_pdf_is_valid: minimal PDF starts with %PDF.
    #[test]
    fn test_minimal_pdf_is_valid() {
        let data = minimal_pdf_bytes();
        assert!(!data.is_empty());
        assert!(data.starts_with(b"%PDF"));
    }

    /// test_minimal_png_is_valid: minimal PNG starts with PNG signature.
    #[test]
    fn test_minimal_png_is_valid() {
        let data = minimal_png_bytes();
        assert!(!data.is_empty());
        assert_eq!(
            &data[0..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
            "should start with PNG signature"
        );
    }

    /// test_outcome_to_matrix: conversion result → capability matrix.
    #[test]
    fn test_outcome_to_matrix() {
        let engine = MockEngine::new();
        let outcome = probe_engine(&engine);
        let matrix = outcome_to_matrix(&outcome, "mock", "0.0.0-mock");

        assert_eq!(matrix.engine, "mock");
        assert_eq!(matrix.version, Some("0.0.0-mock".to_string()));
        assert_eq!(matrix.conversions.len(), outcome.conversions.len());
    }

    /// test_probe_cache_save_load: cache can be saved to disk and reloaded.
    #[test]
    fn test_probe_cache_save_load() {
        let tmp = std::env::temp_dir().join("stichos-probe-cache-test.json");
        let _ = std::fs::remove_file(&tmp);

        let mut cache = ProbeCache::with_path(&tmp);
        cache.set("mock", "docx", "pdf", true);
        cache.set("mock", "xlsx", "pdf", false);

        cache.save().expect("cache save should succeed");
        assert!(tmp.exists(), "cache file should exist");

        let loaded = ProbeCache::with_path(&tmp);
        let reloaded = loaded.load().expect("cache load should succeed");
        assert_eq!(reloaded.entries.len(), 2);
        assert_eq!(reloaded.get("mock", "docx", "pdf"), Some(true));
        assert_eq!(reloaded.get("mock", "xlsx", "pdf"), Some(false));

        let _ = std::fs::remove_file(&tmp);
    }

    /// test_probe_cache_load_or_default: load_or_default returns a valid cache.
    #[test]
    fn test_probe_cache_load_or_default() {
        let cache = ProbeCache::load_or_default();
        assert!(cache.path.is_some(), "should have a default path");
    }

    /// test_probe_outcome_pass_ratio: pass_ratio is computed correctly.
    #[test]
    fn test_probe_outcome_pass_ratio() {
        let engine = MockEngine::new();
        let outcome = probe_engine(&engine);
        assert!(
            (outcome.pass_ratio - 1.0).abs() < f64::EPSILON,
            "all mock conversions should succeed → ratio = 1.0"
        );
    }

    /// test_probe_unavailable_outcome_pass_ratio: unavailable engine has 0 ratio.
    #[test]
    fn test_probe_unavailable_outcome_pass_ratio() {
        let mut engine = MockEngine::new();
        engine.available = false;
        let outcome = probe_engine(&engine);
        assert_eq!(outcome.pass_ratio, 0.0);
    }

    /// test_probe_engine_cached: cache integration works.
    #[test]
    fn test_probe_engine_cached() {
        let engine = MockEngine::new();
        let mut cache = ProbeCache::new();

        let outcome = probe_engine_cached(&engine, &mut cache);
        assert!(!outcome.conversions.is_empty());

        assert!(!cache.is_empty(), "cache should have entries after probing");

        let outcome2 = probe_engine_cached(&engine, &mut cache);
        assert_eq!(outcome.conversions.len(), outcome2.conversions.len());
    }

    /// test_probe_fixture_is_cloneable: fixtures are clone-safe.
    #[test]
    fn test_probe_fixture_is_cloneable() {
        let fixture = ProbeFixture {
            source_format: "docx".into(),
            target_format: "pdf".into(),
            name: "test".into(),
            data: b"test data".to_vec(),
        };
        let cloned = fixture.clone();
        assert_eq!(fixture.source_format, cloned.source_format);
        assert_eq!(fixture.target_format, cloned.target_format);
        assert_eq!(fixture.name, cloned.name);
        assert_eq!(fixture.data, cloned.data);
    }

    /// test_conversion_result_serializes: ConversionResult round-trips.
    #[test]
    fn test_conversion_result_serializes() {
        let cr = ConversionResult {
            fixture: "test".into(),
            source: "docx".into(),
            target: "pdf".into(),
            works: true,
            output_bytes: 1024,
        };
        let json = serde_json::to_string(&cr).expect("should serialize");
        let back: ConversionResult = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(cr, back);
    }

    /// test_probe_outcome_serializes: ProbeOutcome round-trips via JSON.
    #[test]
    fn test_probe_outcome_serializes() {
        let engine = MockEngine::new();
        let outcome = probe_engine(&engine);
        let json = serde_json::to_string(&outcome).expect("should serialize");
        let back: ProbeOutcome = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(outcome.conversions.len(), back.conversions.len());
        assert!(
            (outcome.pass_ratio - back.pass_ratio).abs() < f64::EPSILON,
            "pass ratios should match"
        );
    }

    /// test_n_minus_one_result_serializes: NMinusOneResult round-trips.
    #[test]
    fn test_n_minus_one_result_serializes() {
        let engine = MockEngine::new();
        let probe = ProbeNMinusOne::new(&engine, None, None);
        let result = probe.probe();

        let json = serde_json::to_string(&result).expect("should serialize");
        let back: NMinusOneResult = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(result.engine_id, back.engine_id);
        assert_eq!(result.version, back.version);
        assert_eq!(result.working_version, back.working_version);
        assert_eq!(result.current_passed, back.current_passed);
    }

    /// test_empty_engine_id_key: empty engine ID produces valid cache key.
    #[test]
    fn test_empty_engine_id_key() {
        let key = ProbeCache::key("", "docx", "pdf");
        assert_eq!(key, ":docx:pdf");
    }

    /// test_special_characters_in_key: special chars don't break key generation.
    #[test]
    fn test_special_characters_in_key() {
        let key = ProbeCache::key("libreoffice/26.2.6", "doc+xf", "pdf@v2");
        assert_eq!(key, "libreoffice/26.2.6:doc+xf:pdf@v2");
    }
}
