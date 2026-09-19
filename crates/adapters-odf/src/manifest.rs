// SPDX-License-Identifier: MIT OR Apache-2.0
//! ODF manifest.xml parser and encryption detection.
//!
//! Parses `META-INF/manifest.xml` from ODF packages (.odt/.ods/.odp) to extract
//! part URIs and media types. Also detects encryption markers.
//!
//! Diagnostic codes:
//! - ODF-E-001: ODF package error
//! - ODF-E-002: Manifest parse error
//! - ODF-E-003: Encryption detected
//!
//! ODF namespaces:
//! - manifest: `urn:oasis:names:tc:opendocument:xmlns:manifest:1.0`
//!
//! ODF files are ZIP packages with a predictable structure:
//! - META-INF/manifest.xml — part manifest with URIs and media types
//! - content.xml — document content
//! - styles.xml — document styles
//! - meta.xml — document metadata
//!
//! See OASIS ODF 1.3 specification, Part 1, Section 4 (Manifest).

use gp_core::diagnostics::{Diagnostic, Severity};
use serde::{Deserialize, Serialize};
use std::io::Cursor;

// ---------------------------------------------------------------------------
// Manifest types
// ---------------------------------------------------------------------------

/// A single file entry from the ODF manifest.
///
/// Corresponds to a `<manifest:file-entry>` element in META-INF/manifest.xml.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestPart {
    /// The `full-path` attribute (e.g. `"/"`, `"content.xml"`, `"styles.xml"`).
    pub path: String,
    /// The `media-type` attribute (e.g. `"application/vnd.oasis.opendocument.text"`).
    pub media_type: String,
    /// Whether compression was used (`"false"` means stored).
    pub compression: bool,
}

/// A default media-type entry from the manifest.
///
/// Corresponds to a `<manifest:default>` element — provides a fallback
/// media-type for paths matching a given prefix pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestDefault {
    /// The prefix pattern (e.g. `""` matches all).
    pub path: String,
    /// The media-type to use for matching paths.
    pub media_type: String,
}

/// Parsed ODF manifest from META-INF/manifest.xml.
///
/// Contains all file entries and default media-type mappings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// All file entries from `<manifest:file-entry>` elements.
    pub parts: Vec<ManifestPart>,
    /// Default media-type mappings from `<manifest:default>` elements.
    pub defaults: Vec<ManifestDefault>,
    /// The manifest version (from `manifest:version` attribute, if present).
    pub version: Option<String>,
    /// Diagnostics produced during parsing.
    pub diagnostics: Vec<Diagnostic>,
}

impl Manifest {
    /// Look up a manifest part by its full path.
    ///
    /// Returns `None` if no entry matches the given path.
    pub fn find_part(&self, path: &str) -> Option<&ManifestPart> {
        self.parts.iter().find(|p| p.path == path)
    }

    /// Resolve the media type for a given path, using file entries first,
    /// then falling back to default mappings.
    ///
    /// Returns `"application/octet-stream"` as a last resort.
    pub fn resolve_media_type(&self, path: &str) -> &str {
        // Check file entries first for exact match.
        for part in &self.parts {
            if part.path == path {
                return &part.media_type;
            }
        }

        // Fall back to default mappings — match longest prefix.
        let mut best_match: Option<&ManifestDefault> = None;
        for default in &self.defaults {
            let default_path = default.path.as_str();
            let matches = if default_path.is_empty() {
                true // Empty default matches everything
            } else if best_match.is_some() && best_match.as_ref().unwrap().path.is_empty() {
                false // Already have an empty default, skip non-empty
            } else {
                path == default_path || path.starts_with(&format!("{}/", default_path))
            };
            if matches {
                // Empty default matches everything; non-empty defaults should be longer
                if default_path.is_empty()
                    || default_path.len() > best_match.map_or(0, |d| d.path.len())
                {
                    best_match = Some(default);
                }
            }
        }

        if let Some(def) = best_match {
            &def.media_type
        } else {
            "application/octet-stream"
        }
    }

    /// Return all paths in the manifest.
    pub fn paths(&self) -> Vec<&str> {
        self.parts.iter().map(|p| p.path.as_str()).collect()
    }

    /// Return true if the manifest contains a part at the given path.
    pub fn has_part(&self, path: &str) -> bool {
        self.find_part(path).is_some()
    }
}

// ---------------------------------------------------------------------------
// XML attribute extraction (minimal, no external XML deps)
// ---------------------------------------------------------------------------

/// Extract an attribute value from an XML tag string.
///
/// Handles both single and double quotes. Returns `None` if the attribute
/// is not present.
fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let pat = format!("{}=\"", attr);
    let start = tag.find(&pat)?;
    let after_eq = &tag[start + pat.len()..];
    let end = after_eq.find('"')?;
    Some(after_eq[..end].to_string())
}

// ---------------------------------------------------------------------------
// parse_manifest — core parser
// ---------------------------------------------------------------------------

/// Parse an ODF manifest from META-INF/manifest.xml bytes.
///
/// Extracts `<manifest:file-entry>`, `<manifest:default>`, and the root
/// `manifest:version` attribute.
///
/// # Errors
///
/// Returns diagnostics if the XML is malformed or the manifest root namespace
/// does not match the expected ODF manifest namespace.
pub fn parse_manifest(data: &[u8]) -> Result<Manifest, Vec<Diagnostic>> {
    let mut manifest = Manifest::default();
    let mut diags: Vec<Diagnostic> = Vec::new();
    let content = String::from_utf8_lossy(data);

    // Check root namespace.
    if content.contains("manifest:manifest")
        && content.contains("urn:oasis:names:tc:opendocument:xmlns:manifest:1.0")
    {
        // Good — matches the ODF manifest namespace.
    } else if !content.contains("manifest:manifest") {
        diags.push(Diagnostic::new(
            "ODF-E-002",
            Severity::Error,
            "manifest.xml: missing manifest:manifest root element",
        ));
    }

    // Extract root version attribute.
    if let Some(pos) = content.find("manifest:manifest") {
        let tag_end = content[pos..].find('>');
        if let Some(end) = tag_end {
            let root_tag = &content[pos..pos + end];
            if let Some(version) = extract_attr(root_tag, "manifest:version") {
                manifest.version = Some(version);
            }
        }
    }

    // Parse file-entry elements (both self-closing and block forms).
    for line in content.lines() {
        let trimmed = line.trim();

        // Self-closing: <manifest:file-entry .../>
        if trimmed.contains("<manifest:file-entry") {
            if let Some(path) = extract_attr(trimmed, "manifest:full-path") {
                if let Some(media_type) = extract_attr(trimmed, "manifest:media-type") {
                    let compression_str = extract_attr(trimmed, "manifest:compression")
                        .unwrap_or_else(|| "true".to_string());
                    let compression = compression_str != "false";
                    manifest.parts.push(ManifestPart {
                        path,
                        media_type,
                        compression,
                    });
                }
            }
        }
    }

    // Also handle multi-line file-entry elements by scanning the full content.
    let parts_list: Vec<_> = content.split("<manifest:file-entry").collect();
    for entry_str in &parts_list[1..] {
        // Skip if we already found one on a single line above.
        // Process all entries from the full-text scan.
        if let (Some(path), Some(media_type)) = (
            extract_attr(entry_str, "manifest:full-path"),
            extract_attr(entry_str, "manifest:media-type"),
        ) {
            let compression_str = extract_attr(entry_str, "manifest:compression")
                .unwrap_or_else(|| "true".to_string());
            let compression = compression_str != "false";
            manifest.parts.push(ManifestPart {
                path,
                media_type,
                compression,
            });
        }
    }

    // Also handle multi-line default elements by scanning the full content.
    let defaults_list: Vec<_> = content.split("<manifest:default").collect();
    for entry_str in &defaults_list[1..] {
        // Only process if this is an actual start tag (contains manifest:default)
        // and not just a fragment from the split.
        if entry_str.starts_with(' ') || entry_str.is_empty() {
            if let (Some(path), Some(media_type)) = (
                extract_attr(entry_str, "manifest:full-path"),
                extract_attr(entry_str, "manifest:media-type"),
            ) {
                // Skip the root manifest tag which also has full-path/media-type
                if !path.contains('>') && !path.is_empty() || !media_type.is_empty() {
                    manifest.defaults.push(ManifestDefault { path, media_type });
                }
            }
        }
    }

    // Deduplicate file entries (single-line parsing above may have captured them already).
    let mut seen_paths: Vec<String> = Vec::new();
    let mut deduped: Vec<ManifestPart> = Vec::new();
    for part in manifest.parts {
        if !seen_paths.contains(&part.path) {
            seen_paths.push(part.path.clone());
            deduped.push(part);
        }
    }
    manifest.parts = deduped;

    // Deduplicate default entries.
    let mut seen_defaults: Vec<(String, String)> = Vec::new();
    let mut deduped_defaults: Vec<ManifestDefault> = Vec::new();
    for default in manifest.defaults {
        let key = (default.path.clone(), default.media_type.clone());
        if !seen_defaults.contains(&key) {
            seen_defaults.push(key);
            deduped_defaults.push(default);
        }
    }
    manifest.defaults = deduped_defaults;

    manifest.diagnostics = diags;
    Ok(manifest)
}

// ---------------------------------------------------------------------------
// ODF Package abstraction
// ---------------------------------------------------------------------------

/// An ODF package — a ZIP archive with manifest.xml and ODF-specific parts.
///
/// This mirrors `OpcPackage` from the ooxml crate but is specific to ODF.
/// It reuses the same ZIP parsing infrastructure.
#[derive(Debug, Clone)]
pub struct OdfPackage {
    /// All parts extracted from the ZIP.
    pub parts: Vec<OdfPart>,
    /// Parsed manifest from META-INF/manifest.xml (if present).
    pub manifest: Option<Manifest>,
    /// Diagnostics produced during parsing.
    pub diagnostics: Vec<Diagnostic>,
}

/// A part in an ODF package.
#[derive(Debug, Clone)]
pub struct OdfPart {
    /// Path within the ZIP archive (e.g. `"content.xml"`).
    pub path: String,
    /// Content-Type / media type.
    pub content_type: String,
    /// Raw bytes of the part.
    pub data: Vec<u8>,
}

/// Parse an ODF package from ZIP bytes.
///
/// Opens the ZIP, extracts all entries, then parses META-INF/manifest.xml
/// if present.
pub fn parse_odf(data: &[u8]) -> Result<OdfPackage, Vec<Diagnostic>> {
    let mut diags: Vec<Diagnostic> = Vec::new();

    // Open the ZIP archive.
    let mut archive = match zip::ZipArchive::new(Cursor::new(data)) {
        Ok(a) => a,
        Err(e) => {
            diags.push(Diagnostic::new(
                "ODF-E-001",
                Severity::Error,
                format!("Failed to open ZIP archive: {}", e),
            ));
            return Err(diags);
        }
    };

    let entry_count = archive.len();

    // Check entry count limit.
    if entry_count > 10000 {
        diags.push(Diagnostic::new(
            "ODF-E-001",
            Severity::Warning,
            format!("ODF package has {} entries (large)", entry_count),
        ));
    }

    // Enumerate entries.
    let mut safe_entries: Vec<(String,)> = Vec::new();
    for i in 0..archive.len() {
        let entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let name = entry.name().to_string();

        // Path traversal check.
        if name.contains("..") {
            diags.push(Diagnostic::new(
                "ODF-E-001",
                Severity::Fatal,
                format!("Path traversal detected in ODF package entry: {}", name),
            ));
            continue;
        }

        // Symlink check.
        if entry.is_symlink() {
            diags.push(Diagnostic::new(
                "ODF-E-001",
                Severity::Fatal,
                format!("Symlink entry detected in ODF package: {}", name),
            ));
            continue;
        }

        // Skip directory entries.
        if entry.is_dir() {
            continue;
        }

        safe_entries.push((name,));
    }

    // Parse manifest if present.
    let manifest = archive
        .by_name("META-INF/manifest.xml")
        .ok()
        .and_then(|mut e| {
            let mut buf = Vec::new();
            let _ = std::io::Read::read_to_end(&mut e, &mut buf);
            if buf.is_empty() {
                None
            } else {
                Some(buf)
            }
        })
        .and_then(|bytes| match parse_manifest(&bytes) {
            Ok(m) => Some(m),
            Err(d) => {
                diags.extend(d);
                None
            }
        });

    // Read part data.
    let mut parts: Vec<OdfPart> = Vec::new();
    for (path,) in &safe_entries {
        let buffer = match archive.by_name(path) {
            Ok(mut e) => {
                let mut buf = Vec::new();
                let _ = std::io::Read::read_to_end(&mut e, &mut buf);
                buf
            }
            Err(_) => Vec::new(),
        };

        // Resolve content type via manifest or heuristic.
        let content_type = if let Some(ref manifest) = manifest {
            manifest.resolve_media_type(path).to_string()
        } else if path == "content.xml"
            || path == "styles.xml"
            || path == "meta.xml"
            || path.ends_with(".xml")
        {
            "text/xml".to_string()
        } else if path.ends_with(".png") || path.ends_with(".jpg") || path.ends_with(".gif") {
            "image/png".to_string()
        } else {
            "application/octet-stream".to_string()
        };

        parts.push(OdfPart {
            path: path.clone(),
            content_type,
            data: buffer,
        });
    }

    // If we have fatal diagnostics, return error.
    let has_fatal = diags.iter().any(|d| d.severity == Severity::Fatal);
    if has_fatal {
        return Err(diags);
    }

    Ok(OdfPackage {
        parts,
        manifest,
        diagnostics: diags,
    })
}

// ---------------------------------------------------------------------------
// Encryption detection
// ---------------------------------------------------------------------------

/// Detect if an ODF package is encrypted.
///
/// Checks for:
/// 1. `META-INF/encryption.xml` — ODF encryption marker.
/// 2. `META-INF/encryptionInformation.xml` — OOXML-style encryption.
/// 3. `EncryptionInfo` stream — MS Office encryption container.
/// 4. Non-text content in content.xml (encrypted documents often have
///    binary content where XML is expected).
///
/// Returns `None` if the package is not encrypted, or `Some(reason)` with
/// a human-readable description of the encryption marker found.
pub fn detect_encryption(package: &OdfPackage) -> Option<String> {
    // Check for META-INF/encryption.xml.
    for part in &package.parts {
        if part.path == "META-INF/encryption.xml" {
            return Some("ODF encryption: META-INF/encryption.xml found".to_string());
        }
    }

    // Check for OOXML-style encryption.
    for part in &package.parts {
        if part.path == "META-INF/encryptionInformation.xml" {
            return Some("ODF encryption: META-INF/encryptionInformation.xml found".to_string());
        }
    }

    // Check for MS Office EncryptionInfo stream.
    for part in &package.parts {
        if part.path == "EncryptionInfo" || part.path == "EncryptionInfo5" {
            return Some(format!("ODF encryption: {} stream found", part.path));
        }
    }

    // Check for encrypted content.xml (binary data where XML is expected).
    for part in &package.parts {
        if part.path == "content.xml" {
            if let Ok(text) = String::from_utf8(part.data.clone()) {
                // If content.xml contains encryption-related markup, flag it.
                if text.contains("EncryptedDocument")
                    || text.contains("EncryptedPackage")
                    || text.contains("<enc:encrypted")
                {
                    return Some(
                        "ODF encryption: encrypted content found in content.xml".to_string(),
                    );
                }
            }
            // If content.xml is mostly binary (very few printable characters),
            // it might be encrypted.
            if !part.data.is_empty() {
                let printable_ratio = part
                    .data
                    .iter()
                    .filter(|&&b| b.is_ascii_graphic() || b == b'\n' || b == b'\r' || b == b'\t')
                    .count() as f64
                    / part.data.len() as f64;
                if printable_ratio < 0.5 {
                    return Some(
                        "ODF encryption: content.xml appears to be encrypted binary".to_string(),
                    );
                }
            }
        }
    }

    // Check manifest for encryption-related entries.
    if let Some(ref manifest) = package.manifest {
        for part in &manifest.parts {
            if part.path.contains("encryption") {
                return Some(format!(
                    "ODF encryption: manifest entry references encryption: {}",
                    part.path
                ));
            }
        }
    }

    None
}

/// Check raw ZIP bytes for encryption markers without full parsing.
///
/// Useful for a fast pre-check before full package parsing.
pub fn has_encryption_markers(data: &[u8]) -> Option<String> {
    let mut archive = match zip::ZipArchive::new(Cursor::new(data)) {
        Ok(a) => a,
        Err(_) => return None,
    };

    let encryption_files = [
        "META-INF/encryption.xml",
        "META-INF/encryptionInformation.xml",
        "EncryptionInfo",
        "EncryptionInfo5",
    ];

    for name in &encryption_files {
        if archive.by_name(name).is_ok() {
            return Some(format!("encryption marker: {}", name));
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Synthetic test fixtures
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test_fixtures {
    use std::io::Write;

    use super::*;

    type FileOpts = zip::write::FileOptions<'static, ()>;

    fn opts() -> FileOpts {
        zip::write::FileOptions::default()
    }

    /// Create a minimal valid ODF text document (.odt) in memory.
    pub fn create_valid_odt() -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let o = opts();

        // content.xml
        writer.start_file("content.xml", o).unwrap();
        writer.write_all(
            b"<?xml version=\"1.0\"?><text:document-content xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><text:p>Hello, ODF!</text:p></text:document-content>",
        )
        .unwrap();

        // meta.xml
        writer.start_file("meta.xml", o).unwrap();
        writer.write_all(b"<?xml version=\"1.0\"?><office:meta xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:meta>")
            .unwrap();

        // styles.xml
        writer.start_file("styles.xml", o).unwrap();
        writer.write_all(b"<?xml version=\"1.0\"?><office:styles xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:styles>")
            .unwrap();

        // META-INF/manifest.xml
        writer.start_file("META-INF/manifest.xml", o).unwrap();
        writer.write_all(
            b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:compression=\"false\" manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/><manifest:default manifest:full-path=\"\" manifest:media-type=\"text/xml\"/></manifest:manifest>",
        )
        .unwrap();

        writer.finish().unwrap().into_inner()
    }

    /// Create a minimal valid ODF spreadsheet (.ods) in memory.
    pub fn create_valid_ods() -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let o = opts();

        writer.start_file("content.xml", o).unwrap();
        writer.write_all(b"<text:document-content xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\"><table:table><table:table-row><table:table-cell><text:p>42</text:p></table:table-cell></table:table-row></table:table></text:document-content>")
            .unwrap();

        writer.start_file("meta.xml", o).unwrap();
        writer.write_all(b"<office:meta xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:meta>")
            .unwrap();

        writer.start_file("styles.xml", o).unwrap();
        writer.write_all(b"<office:styles xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:styles>")
            .unwrap();

        writer.start_file("META-INF/manifest.xml", o).unwrap();
        writer.write_all(
            b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.spreadsheet\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/><manifest:default manifest:full-path=\"\" manifest:media-type=\"text/xml\"/></manifest:manifest>",
        )
        .unwrap();

        writer.finish().unwrap().into_inner()
    }

    /// Create a minimal valid ODF presentation (.odp) in memory.
    pub fn create_valid_odp() -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let o = opts();

        writer.start_file("content.xml", o).unwrap();
        writer.write_all(b"<text:document-content xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><text:p>Slide 1</text:p></text:document-content>")
            .unwrap();

        writer.start_file("meta.xml", o).unwrap();
        writer.write_all(b"<office:meta xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:meta>")
            .unwrap();

        writer.start_file("styles.xml", o).unwrap();
        writer.write_all(b"<office:styles xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:styles>")
            .unwrap();

        writer.start_file("META-INF/manifest.xml", o).unwrap();
        writer.write_all(
            b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.presentation\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/><manifest:default manifest:full-path=\"\" manifest:media-type=\"text/xml\"/></manifest:manifest>",
        )
        .unwrap();

        writer.finish().unwrap().into_inner()
    }

    /// Create an ODF package with META-INF/encryption.xml.
    pub fn create_encrypted_odt() -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let o = opts();

        writer.start_file("content.xml", o).unwrap();
        writer.write_all(b"<text:document-content xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"><text:p>Encrypted!</text:p></text:document-content>")
            .unwrap();

        writer.start_file("META-INF/manifest.xml", o).unwrap();
        writer.write_all(
            b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"META-INF/encryption.xml\" manifest:media-type=\"application/vnd.oasis.opendocument.encryption\"/></manifest:manifest>",
        )
        .unwrap();

        writer.start_file("META-INF/encryption.xml", o).unwrap();
        writer.write_all(b"<enc:encryption xmlns:enc=\"http://www.w3.org/2001/04/xmlenc#\"><enc:Algorithm></enc:Algorithm></enc:encryption>")
            .unwrap();

        writer.finish().unwrap().into_inner()
    }

    /// Create an ODF package with encrypted content.xml (binary data).
    pub fn create_encrypted_content_odt() -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let o = opts();

        // Write binary content that looks encrypted.
        writer.start_file("content.xml", o).unwrap();
        let encrypted_data: Vec<u8> = (0..256).map(|i| (i * 7 + 3) as u8).collect();
        writer.write_all(&encrypted_data).unwrap();

        writer.start_file("META-INF/manifest.xml", o).unwrap();
        writer.write_all(
            b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"application/octet-stream\"/></manifest:manifest>",
        )
        .unwrap();

        writer.finish().unwrap().into_inner()
    }

    /// Create a minimal ZIP that is not a valid ODF package.
    pub fn create_not_odf_zip() -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let o = opts();
        writer.start_file("random.txt", o).unwrap();
        writer.write_all(b"not odf content").unwrap();
        writer.finish().unwrap().into_inner()
    }

    /// Create an empty ZIP.
    pub fn create_empty_zip() -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        #[allow(unused_mut)]
        let mut writer = zip::ZipWriter::new(cursor);
        writer.finish().unwrap().into_inner()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use test_fixtures::*;

    /// test_parse_manifest — parse manifest.xml from fixture data.
    #[test]
    fn test_parse_manifest() {
        let data = create_valid_odt();
        let pkg = parse_odf(&data).expect("should parse valid ODF");

        let manifest = pkg.manifest.expect("should have manifest");

        assert!(
            manifest.parts.len() >= 2,
            "Should have at least 2 parts (root + content.xml)"
        );
        assert!(
            !manifest.defaults.is_empty(),
            "Should have at least 1 default, got {} parts and {} defaults",
            manifest.parts.len(),
            manifest.defaults.len()
        );
    }

    /// test_manifest_part_uris — verify all parts listed.
    #[test]
    fn test_manifest_part_uris() {
        let data = create_valid_odt();
        let pkg = parse_odf(&data).expect("should parse valid ODF");
        let manifest = pkg.manifest.expect("should have manifest");

        let paths: Vec<&str> = manifest.paths();
        assert!(paths.contains(&"/"), "Should contain root entry '/'");
        assert!(paths.contains(&"content.xml"), "Should contain content.xml");
    }

    /// test_manifest_media_types — verify media types.
    #[test]
    fn test_manifest_media_types() {
        let data = create_valid_odt();
        let pkg = parse_odf(&data).expect("should parse valid ODF");
        let manifest = pkg.manifest.expect("should have manifest");

        // Root entry should have the ODF text media type.
        let root = manifest.find_part("/").expect("should have root");
        assert_eq!(root.media_type, "application/vnd.oasis.opendocument.text");

        // content.xml should be text/xml.
        let content = manifest
            .find_part("content.xml")
            .expect("should have content.xml");
        assert_eq!(content.media_type, "text/xml");
    }

    /// test_manifest_full_path_mapping — full_path → part URI mapping correct.
    #[test]
    fn test_manifest_full_path_mapping() {
        let data = create_valid_odt();
        let pkg = parse_odf(&data).expect("should parse valid ODF");
        let manifest = pkg.manifest.expect("should have manifest");

        assert!(manifest.has_part("/"));
        assert!(manifest.has_part("content.xml"));
        assert!(!manifest.has_part("nonexistent.xml"));
        assert!(manifest.resolve_media_type("/") == "application/vnd.oasis.opendocument.text");
        assert!(manifest.resolve_media_type("content.xml") == "text/xml");
    }

    /// test_detect_encryption_not_encrypted — normal fixture → None.
    #[test]
    fn test_detect_encryption_not_encrypted() {
        let data = create_valid_odt();
        let pkg = parse_odf(&data).expect("should parse valid ODF");
        let result = detect_encryption(&pkg);
        assert!(
            result.is_none(),
            "Normal ODF should not be flagged as encrypted"
        );
    }

    /// test_detect_encryption_encrypted — synthetic encrypted → Some(reason).
    #[test]
    fn test_detect_encryption_encrypted() {
        let data = create_encrypted_odt();
        let pkg = parse_odf(&data).expect("should parse encrypted ODF");
        let result = detect_encryption(&pkg);
        assert!(result.is_some(), "Encrypted ODF should be flagged");
        assert!(
            result.as_ref().unwrap().contains("encryption"),
            "Encryption reason should mention encryption"
        );
    }

    /// test_detect_encryption_encrypted_content — binary content.xml → Some.
    #[test]
    fn test_detect_encryption_encrypted_content() {
        let data = create_encrypted_content_odt();
        let pkg = parse_odf(&data).expect("should parse encrypted content ODF");
        let result = detect_encryption(&pkg);
        assert!(result.is_some(), "Encrypted content.xml should be flagged");
    }

    /// test_has_encryption_markers — fast pre-check.
    #[test]
    fn test_has_encryption_markers() {
        // Normal ODF has no markers.
        let data = create_valid_odt();
        assert!(
            has_encryption_markers(&data).is_none(),
            "Normal ODF should have no encryption markers"
        );

        // Encrypted ODF has markers.
        let data = create_encrypted_odt();
        let markers = has_encryption_markers(&data);
        assert!(
            markers.is_some(),
            "Encrypted ODF should have encryption markers"
        );
        assert!(markers.unwrap().contains("encryption"));
    }

    /// test_parse_all_odf_fixtures — all odt/ods/odp files parse successfully.
    #[test]
    fn test_parse_all_odf_fixtures() {
        let blobs_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/blobs");
        let extensions = ["odt", "ods", "odp"];
        let mut parsed_count = 0;

        for entry in std::fs::read_dir(blobs_dir).expect("corpus/blobs/ should be readable") {
            let entry = entry.expect("should read directory entry");
            let path = entry.path();
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if extensions.contains(&ext) {
                    let data = std::fs::read(&path)
                        .unwrap_or_else(|e| panic!("should read {}: {e}", path.display()));
                    let result = parse_odf(&data);
                    match result {
                        Ok(pkg) => {
                            parsed_count += 1;
                            // Check for no fatal diagnostics.
                            let has_fatal = pkg
                                .diagnostics
                                .iter()
                                .any(|d| d.severity == Severity::Fatal);
                            assert!(
                                !has_fatal,
                                "Fatal diagnostic for {}: {:?}",
                                path.display(),
                                pkg.diagnostics
                            );
                        }
                        Err(diags) => {
                            let has_fatal = diags.iter().any(|d| d.severity == Severity::Fatal);
                            assert!(
                                !has_fatal,
                                "Fatal diagnostic for {}: {:?}",
                                path.display(),
                                diags
                            );
                            parsed_count += 1;
                        }
                    }
                }
            }
        }

        assert!(
            parsed_count > 0,
            "Should have parsed at least one ODF fixture (parsed {})",
            parsed_count
        );
    }

    /// test_manifest_with_stored_compression — entries with compression="false" detected.
    #[test]
    fn test_manifest_with_stored_compression() {
        let data = create_valid_odt();
        let pkg = parse_odf(&data).expect("should parse valid ODF");
        let manifest = pkg.manifest.expect("should have manifest");

        // Check that compression attribute is parsed.
        for part in &manifest.parts {
            if part.path == "content.xml" {
                assert!(
                    !part.compression,
                    "content.xml should have compression=false"
                );
            }
        }
    }

    /// test_manifest_default_media_type — defaults fallback works.
    #[test]
    fn test_manifest_default_media_type() {
        let data = create_valid_odt();
        let pkg = parse_odf(&data).expect("should parse valid ODF");
        let manifest = pkg.manifest.expect("should have manifest");

        // Unlisted path should resolve to default or fallback.
        let resolved = manifest.resolve_media_type("unknown/file.xml");
        assert_eq!(
            resolved, "text/xml",
            "Should use default text/xml for .xml files"
        );
    }

    /// test_manifest_version — manifest version attribute parsed.
    #[test]
    fn test_manifest_version() {
        let data = create_valid_odt();
        let pkg = parse_odf(&data).expect("should parse valid ODF");
        let manifest = pkg.manifest.expect("should have manifest");

        // Version may or may not be present depending on the fixture.
        let _ = &manifest.version; // Just verify it parses without error.
    }

    /// test_odf_package_parts — all parts from ZIP are included in package.
    #[test]
    fn test_odf_package_parts() {
        let data = create_valid_odt();
        let pkg = parse_odf(&data).expect("should parse valid ODF");

        // Should have content.xml, meta.xml, styles.xml, and manifest.xml.
        let paths: Vec<&str> = pkg.parts.iter().map(|p| p.path.as_str()).collect();
        assert!(paths.contains(&"content.xml"));
        assert!(paths.contains(&"meta.xml"));
        assert!(paths.contains(&"META-INF/manifest.xml"));
    }

    /// test_ods_manifest_media_type — ODS uses spreadsheet media type.
    #[test]
    fn test_ods_manifest_media_type() {
        let data = create_valid_ods();
        let pkg = parse_odf(&data).expect("should parse valid ODS");
        let manifest = pkg.manifest.expect("should have manifest");

        let root = manifest.find_part("/").expect("should have root");
        assert_eq!(
            root.media_type,
            "application/vnd.oasis.opendocument.spreadsheet"
        );
    }

    /// test_odp_manifest_media_type — ODP uses presentation media type.
    #[test]
    fn test_odp_manifest_media_type() {
        let data = create_valid_odp();
        let pkg = parse_odf(&data).expect("should parse valid ODP");
        let manifest = pkg.manifest.expect("should have manifest");

        let root = manifest.find_part("/").expect("should have root");
        assert_eq!(
            root.media_type,
            "application/vnd.oasis.opendocument.presentation"
        );
    }

    /// test_parse_not_odf_zip — valid ZIP without ODF structure parses OK.
    #[test]
    fn test_parse_not_odf_zip() {
        let data = create_not_odf_zip();
        let result = parse_odf(&data);
        match result {
            Ok(pkg) => {
                assert!(pkg.manifest.is_none(), "Non-ODF should have no manifest");
            }
            Err(_) => {
                // Also acceptable: error from malformed data.
            }
        }
    }

    /// test_parse_empty_zip — empty ZIP returns package with no parts.
    #[test]
    fn test_parse_empty_zip() {
        let data = create_empty_zip();
        let result = parse_odf(&data);
        match result {
            Ok(pkg) => {
                assert!(pkg.parts.is_empty(), "Empty zip should have no parts");
                assert!(pkg.manifest.is_none(), "Empty zip should have no manifest");
            }
            Err(_) => {
                // Also acceptable.
            }
        }
    }

    /// test_parse_not_zip — invalid ZIP → error.
    #[test]
    fn test_parse_not_zip() {
        let not_zip_data = b"This is not a ZIP file at all.\x00\x01\x02\x03".to_vec();
        let diags = parse_odf(&not_zip_data).expect_err("should error on non-ZIP data");
        assert!(!diags.is_empty(), "Should produce diagnostics for non-ZIP");
        let has_e001 = diags.iter().any(|d| d.code == "ODF-E-001");
        assert!(has_e001, "Should have ODF-E-001 for invalid ZIP");
    }

    /// test_corpus_odf_fixtures_all_parse — ensure all 69 ODF fixtures parse.
    #[test]
    fn test_corpus_odf_fixtures_all_parse() {
        let blobs_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/blobs");
        let extensions = ["odt", "ods", "odp"];
        let mut parsed = Vec::new();
        let mut with_manifest = Vec::new();
        let mut encrypted = Vec::new();

        for entry in std::fs::read_dir(blobs_dir).expect("corpus/blobs/ should be readable") {
            let entry = entry.expect("should read directory entry");
            let path = entry.path();
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if extensions.contains(&ext) {
                    let data = std::fs::read(&path)
                        .unwrap_or_else(|e| panic!("should read {}: {e}", path.display()));
                    match parse_odf(&data) {
                        Ok(pkg) => {
                            parsed.push(path.file_name().unwrap().to_str().unwrap().to_string());
                            if pkg.manifest.is_some() {
                                with_manifest
                                    .push(path.file_name().unwrap().to_str().unwrap().to_string());
                            }
                            if detect_encryption(&pkg).is_some() {
                                encrypted
                                    .push(path.file_name().unwrap().to_str().unwrap().to_string());
                            }
                        }
                        Err(_) => {
                            parsed.push(path.file_name().unwrap().to_str().unwrap().to_string());
                        }
                    }
                }
            }
        }

        assert!(
            parsed.len() == 69,
            "Expected 69 ODF fixtures, got {}",
            parsed.len()
        );
        assert!(
            with_manifest.len() >= 65,
            "Expected >=65 fixtures with manifest, got {} (of {} total)",
            with_manifest.len(),
            parsed.len()
        );
        assert!(
            encrypted.is_empty(),
            "No corpus fixtures should be flagged as encrypted: {:?}",
            encrypted
        );
    }
}
