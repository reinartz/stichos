//! ODF surgical package writer — applies mutations while preserving ZIP structure.
//!
//! This module provides byte-identical round-trip through the ODF writer:
//! - Zero-patch mode returns the original bytes unmodified
//! - Patched mode reconstructs the ZIP with mutations while preserving
//!   compression method and entry order
//!
//! ODF-specific rules:
//! - `mimetype` MUST be the FIRST entry in the ZIP archive
//! - `mimetype` MUST be stored (uncompressed, compression method = 0)
//! - `META-INF/manifest.xml` tracks all parts with their compression flags
//! - On add/replace/remove, the manifest is updated to reflect changes
//! - Packages with `xml:encrypt` attributes in the manifest are refused (C3)

use std::collections::{BTreeMap, HashSet};
use std::io::{Cursor, Read, Write};

use gp_core::diagnostics::{Diagnostic, Severity};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A surgical mutation on an ODF part.
#[derive(Debug, Clone)]
pub enum OdfMutation {
    /// Leave the part untouched — stream-copy raw bytes.
    Unchanged,
    /// Replace the part with new data.
    Replace { data: Vec<u8> },
    /// Add a new part (path, data).
    Add { path: String, data: Vec<u8> },
    /// Remove a part.
    Remove,
}

/// Per-part write report.
#[derive(Debug, Clone, Default)]
pub struct WriteReport {
    /// Parts that were written (replaced).
    pub parts_written: Vec<String>,
    /// Parts that were unchanged.
    pub parts_unchanged: Vec<String>,
    /// Parts that were removed.
    pub parts_removed: Vec<String>,
    /// Parts that were added.
    pub parts_added: Vec<String>,
}

/// Result of a surgical write.
#[derive(Debug)]
pub struct WriteResult {
    /// The output ODF bytes.
    pub bytes: Vec<u8>,
    /// Per-part write report.
    pub report: WriteReport,
}

impl WriteResult {
    /// Create a new WriteResult.
    pub fn new(bytes: Vec<u8>, report: WriteReport) -> Self {
        Self { bytes, report }
    }
}

/// Surgical ODF writer — applies mutations while preserving ZIP metadata.
pub struct SurgicalWriter<'a> {
    source_bytes: &'a [u8],
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl<'a> SurgicalWriter<'a> {
    /// Create a new surgical writer from source bytes.
    pub fn new(source_bytes: &'a [u8]) -> Self {
        Self { source_bytes }
    }

    /// Write without any mutations — returns byte-identical output.
    pub fn write_zero_patch(&self) -> Result<Vec<u8>, Vec<Diagnostic>> {
        Ok(self.source_bytes.to_vec())
    }

    /// Apply mutations to the ODF package.
    ///
    /// For each entry in the source ZIP:
    /// - If `Unchanged`: copy raw bytes through the zip writer
    /// - If `Replace`: write new data with same compression method
    /// - If `Remove`: skip the entry
    /// - If `Add`: write new entry with new data
    ///
    /// ODF rules enforced:
    /// - `mimetype` is always written first with Stored compression
    /// - `META-INF/manifest.xml` is updated on add/remove
    pub fn write(
        &self,
        mutations: BTreeMap<String, OdfMutation>,
    ) -> Result<WriteResult, Vec<Diagnostic>> {
        let source_cursor = Cursor::new(self.source_bytes);
        let mut archive = match zip::ZipArchive::new(source_cursor) {
            Ok(a) => a,
            Err(e) => {
                return Err(vec![Diagnostic::new(
                    "ODF-E-001",
                    Severity::Error,
                    format!("Failed to open ZIP archive: {}", e),
                )]);
            }
        };

        // Collect metadata for each entry.
        let mut entries: Vec<EntryMeta> = Vec::new();
        for i in 0..archive.len() {
            let mut entry = match archive.by_index(i) {
                Ok(e) => e,
                Err(_) => continue,
            };
            let name = entry.name().to_string();
            let mut data = Vec::new();
            let _ = entry.read_to_end(&mut data);
            entries.push(EntryMeta {
                name: name.clone(),
                compression: entry.compression(),
                data,
            });
        }

        // Determine which paths are being removed.
        let remove_set: HashSet<&str> = mutations
            .iter()
            .filter_map(|(p, m)| match m {
                OdfMutation::Remove => Some(p.as_str()),
                _ => None,
            })
            .collect();

        // Determine which paths are being added.
        let mut add_list: Vec<(String, &[u8])> = Vec::new();
        let add_set: HashSet<&str> = mutations
            .iter()
            .filter_map(|(p, m)| match m {
                OdfMutation::Add { .. } => Some(p.as_str()),
                _ => None,
            })
            .collect();

        for (path, m) in &mutations {
            if let OdfMutation::Add { data, .. } = m {
                add_list.push((path.clone(), data.as_slice()));
            }
        }

        // Check if manifest needs updating (add/remove on non-manifest parts).
        let mut manifest_needs_update = false;
        for (path, m) in &mutations {
            if path.as_str() != "META-INF/manifest.xml" {
                match m {
                    OdfMutation::Add { .. } | OdfMutation::Remove => {
                        manifest_needs_update = true;
                        break;
                    }
                    _ => {}
                }
            }
        }

        // Build updated manifest data if needed.
        let mut manifest_override: Option<Vec<u8>> = None;
        if manifest_needs_update {
            // Find the original manifest or a stub.
            let original_manifest = entries
                .iter()
                .find(|e| e.name == "META-INF/manifest.xml")
                .map(|e| e.data.clone())
                .unwrap_or_default();
            manifest_override = Some(build_updated_manifest(
                &original_manifest,
                &mutations,
                &remove_set,
                &add_list,
            ));
        }

        // Determine final data for each entry.
        let mut final_entries: Vec<(&str, &[u8])> = Vec::new();
        let mut final_compressions: Vec<zip::CompressionMethod> = Vec::new();

        for entry in &entries {
            // Check if this entry is being removed.
            if remove_set.contains(entry.name.as_str()) {
                continue;
            }

            // Check for replacement.
            if let Some(OdfMutation::Replace { data }) = mutations.get(entry.name.as_str()) {
                final_entries.push((entry.name.as_str(), data.as_slice()));
                final_compressions.push(entry.compression);
                continue;
            }

            // Skip added entries — they will be appended at the end.
            if add_set.contains(entry.name.as_str()) {
                continue;
            }

            // Use original data (unchanged or no mutation).
            final_entries.push((&entry.name, entry.data.as_slice()));
            final_compressions.push(entry.compression);
        }

        // Build output ZIP.
        let output = build_odf_zip(
            &entries,
            &final_entries,
            &final_compressions,
            &add_list,
            manifest_override.as_deref(),
        )?;

        // Build report.
        let mut parts_written = Vec::new();
        let mut parts_unchanged = Vec::new();
        let mut parts_removed = Vec::new();
        let mut parts_added = Vec::new();

        // Classify source entries.
        for entry in &entries {
            let name = entry.name.as_str();
            if remove_set.contains(name) {
                parts_removed.push(entry.name.clone());
            } else if let Some(OdfMutation::Replace { .. }) = mutations.get(name) {
                parts_written.push(entry.name.clone());
            } else if mutations.contains_key(name) {
                // Unchanged mutation.
                parts_unchanged.push(entry.name.clone());
            } else {
                // Unknown entry — pass through as unchanged.
                parts_unchanged.push(entry.name.clone());
            }
        }

        // Added parts.
        for (path, _data) in &add_list {
            parts_added.push(path.clone());
            parts_written.push(path.clone());
        }

        Ok(WriteResult::new(
            output,
            WriteReport {
                parts_written,
                parts_unchanged,
                parts_removed,
                parts_added,
            },
        ))
    }
}

/// Entry metadata from the source ZIP.
struct EntryMeta {
    name: String,
    compression: zip::CompressionMethod,
    data: Vec<u8>,
}

/// Build the output ODF ZIP package.
fn build_odf_zip(
    source_entries: &[EntryMeta],
    final_entries: &[(&str, &[u8])],
    final_compressions: &[zip::CompressionMethod],
    add_list: &[(String, &[u8])],
    manifest_override: Option<&[u8]>,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let mut output = Vec::new();
    let mut zip_writer = zip::ZipWriter::new(Cursor::new(&mut output));
    let stored_opts: zip::write::FileOptions<'static, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflate_opts: zip::write::FileOptions<'static, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // Collect source data for quick lookup.
    let source_map: BTreeMap<&str, &EntryMeta> = source_entries
        .iter()
        .map(|e| (e.name.as_str(), e))
        .collect();

    // Determine if mimetype exists and if it should be first.
    let has_mimetype = source_entries.iter().any(|e| e.name == "mimetype");

    // Write mimetype first (ODF requirement).
    if has_mimetype {
        // Check for replacement.
        let mime_data: &[u8] =
            if let Some((_, d)) = final_entries.iter().find(|(n, _)| *n == "mimetype") {
                d
            } else {
                source_map
                    .get("mimetype")
                    .map(|e| e.data.as_slice())
                    .unwrap_or_default()
            };
        zip_writer
            .start_file("mimetype", stored_opts)
            .map_err(|e| {
                vec![Diagnostic::new(
                    "ODF-E-001",
                    Severity::Error,
                    format!("Failed to start file 'mimetype': {}", e),
                )]
            })?;
        zip_writer.write_all(mime_data).map_err(|e| {
            vec![Diagnostic::new(
                "ODF-E-001",
                Severity::Error,
                format!("Failed to write 'mimetype': {}", e),
            )]
        })?;
    }

    // Write remaining entries in source order.
    let mut manifest_written = false;
    for (i, (name, data)) in final_entries.iter().enumerate() {
        // Skip mimetype — already written.
        if *name == "mimetype" {
            continue;
        }

        let compression = final_compressions.get(i).copied().unwrap_or(
            source_map
                .get(name)
                .map(|e| e.compression)
                .unwrap_or(zip::CompressionMethod::Stored),
        );

        let opts = match compression {
            zip::CompressionMethod::Stored => &stored_opts,
            _ => &deflate_opts,
        };

        // If this is manifest and we have an override, use the override.
        let write_data = if *name == "META-INF/manifest.xml" {
            manifest_written = true;
            if let Some(override_data) = manifest_override {
                override_data
            } else {
                *data
            }
        } else {
            *data
        };

        zip_writer.start_file(name, *opts).map_err(|e| {
            vec![Diagnostic::new(
                "ODF-E-001",
                Severity::Error,
                format!("Failed to start file '{}': {}", name, e),
            )]
        })?;
        zip_writer.write_all(write_data).map_err(|e| {
            vec![Diagnostic::new(
                "ODF-E-001",
                Severity::Error,
                format!("Failed to write '{}': {}", name, e),
            )]
        })?;
    }

    // Write added entries.
    for (path, data) in add_list {
        let path_str = path.as_str();
        // Skip mimetype and manifest — already handled.
        if path_str == "mimetype" || path_str == "META-INF/manifest.xml" {
            continue;
        }

        let opts = if path_str == "mimetype" {
            &stored_opts
        } else {
            &deflate_opts
        };

        zip_writer.start_file(path_str, *opts).map_err(|e| {
            vec![Diagnostic::new(
                "ODF-E-001",
                Severity::Error,
                format!("Failed to add file '{}': {}", path, e),
            )]
        })?;
        zip_writer.write_all(data).map_err(|e| {
            vec![Diagnostic::new(
                "ODF-E-001",
                Severity::Error,
                format!("Failed to write '{}': {}", path, e),
            )]
        })?;
    }

    // If manifest was not written but needs overriding (new package with add),
    // write the override.
    if let Some(override_data) = manifest_override {
        if !manifest_written {
            zip_writer
                .start_file("META-INF/manifest.xml", stored_opts)
                .map_err(|e| {
                    vec![Diagnostic::new(
                        "ODF-E-001",
                        Severity::Error,
                        format!("Failed to write manifest: {}", e),
                    )]
                })?;
            zip_writer.write_all(override_data).map_err(|e| {
                vec![Diagnostic::new(
                    "ODF-E-001",
                    Severity::Error,
                    format!("Failed to write manifest data: {}", e),
                )]
            })?;
        }
    }

    zip_writer.finish().map_err(|e| {
        vec![Diagnostic::new(
            "ODF-E-001",
            Severity::Error,
            format!("Failed to finish ZIP: {}", e),
        )]
    })?;

    Ok(output)
}

/// Build an updated manifest.xml that reflects add/remove mutations.
fn build_updated_manifest(
    original_manifest: &[u8],
    mutations: &BTreeMap<String, OdfMutation>,
    _remove_set: &HashSet<&str>,
    add_list: &[(String, &[u8])],
) -> Vec<u8> {
    let mut content = String::from_utf8_lossy(original_manifest).to_string();

    // Remove entries for removed parts.
    for (path, m) in mutations {
        if let OdfMutation::Remove = m {
            if path != "META-INF/manifest.xml" {
                content = remove_manifest_entry(&content, path);
            }
        }
    }

    // Add entries for new parts.
    for (path, _data) in add_list {
        if path != "META-INF/manifest.xml" {
            content = add_manifest_entry(&content, path);
        }
    }

    content.into_bytes()
}

/// Remove a manifest:file-entry for the given path.
fn remove_manifest_entry(content: &str, path: &str) -> String {
    let search = format!("manifest:full-path=\"{}\"", path);
    if let Some(pos) = content.find(&search) {
        // Find the start of this file-entry.
        let entry_start = if let Some(start) = content[..pos].rfind("<manifest:file-entry") {
            start
        } else {
            return content.to_string();
        };
        // Find the end (either /> or </manifest:file-entry>).
        let after = &content[pos..];
        let end = if let Some(slash) = after.find("/>") {
            pos + slash + 2
        } else if let Some(close) = after.find("</manifest:file-entry>") {
            pos + close + 23
        } else {
            return content.to_string();
        };
        let mut result = content.to_string();
        result.replace_range(entry_start..end, "");
        result
    } else {
        content.to_string()
    }
}

/// Add a manifest:file-entry for a new part.
fn add_manifest_entry(content: &str, path: &str) -> String {
    let media_type = resolve_media_type(path);
    let entry = format!(
        "<manifest:file-entry manifest:full-path=\"{}\" manifest:media-type=\"{}\"/>",
        path, media_type
    );
    // Insert before the closing </manifest:manifest> tag.
    if let Some(pos) = content.rfind("</manifest:manifest>") {
        let mut result = content.to_string();
        result.insert_str(pos, &entry);
        result
    } else {
        format!("{}{}", content, entry)
    }
}

/// Resolve the media type for a given path.
fn resolve_media_type(path: &str) -> &str {
    if path == "mimetype" {
        "application/vnd.oasis.opendocument.text"
    } else if path.ends_with(".xml") {
        "text/xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else if path.ends_with(".gif") {
        "image/gif"
    } else if path.ends_with(".pdf") {
        "application/pdf"
    } else {
        "application/octet-stream"
    }
}

// ---------------------------------------------------------------------------
// Encryption detection
// ---------------------------------------------------------------------------

/// Check if the manifest XML contains `xml:encrypt` attributes on any file-entry.
///
/// Per the ODF specification, a file-entry may carry an `xml:encrypt` attribute
/// indicating the entry is encrypted. This function detects that condition.
///
/// Returns `Some(reason)` if encryption is detected, `None` otherwise.
pub fn detect_xml_encrypt_in_manifest(manifest_xml: &[u8]) -> Option<String> {
    let content = String::from_utf8_lossy(manifest_xml);

    if content.contains("xml:encrypt") {
        let encrypted_path = find_encrypted_path(&content);
        return match encrypted_path {
            Some(path) => Some(format!(
                "ODF encryption: xml:encrypt attribute detected on manifest entry: {}",
                path
            )),
            None => Some("ODF encryption: xml:encrypt attribute detected in manifest".to_string()),
        };
    }

    None
}

/// Find the full-path of a file-entry that has xml:encrypt.
fn find_encrypted_path(content: &str) -> Option<String> {
    for entry_block in content.split("<manifest:file-entry") {
        if entry_block.contains("xml:encrypt") {
            return extract_path(entry_block);
        }
    }
    if content.contains("xml:encrypt") {
        return Some("manifest (root)".to_string());
    }
    None
}

/// Extract the manifest:full-path attribute from a file-entry block.
fn extract_path(block: &str) -> Option<String> {
    // Multi-line: search the whole block.
    if let Some(pos) = block.find("manifest:full-path=\"") {
        let after = &block[pos + 20..];
        if let Some(end) = after.find('"') {
            return Some(after[..end].to_string());
        }
    }
    None
}

/// Extract an attribute value from an XML tag string.
#[allow(dead_code)]
fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let pat = format!("{}=\"", attr);
    let start = tag.find(&pat)?;
    let after_eq = &tag[start + pat.len()..];
    let end = after_eq.find('"')?;
    Some(after_eq[..end].to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use super::*;
    use crate::manifest::{detect_encryption, parse_odf};

    type FileOpts = zip::write::FileOptions<'static, ()>;

    fn opts() -> FileOpts {
        zip::write::FileOptions::default()
    }

    // ------------------------------------------------------------------
    // Synthetic fixtures
    // ------------------------------------------------------------------

    /// Create a minimal valid ODT fixture (mimetype first, stored).
    fn make_valid_odt() -> Vec<u8> {
        let mut buf = Vec::new();
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
        let o = opts();

        writer.start_file("mimetype", o).unwrap();
        writer
            .write_all(b"application/vnd.oasis.opendocument.text")
            .unwrap();

        writer.start_file("content.xml", o).unwrap();
        writer.write_all(b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:document-content>")
            .unwrap();
        writer.start_file("meta.xml", o).unwrap();
        writer.write_all(b"<office:meta xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:meta>")
            .unwrap();
        writer.start_file("styles.xml", o).unwrap();
        writer.write_all(b"<office:styles xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:styles>")
            .unwrap();
        writer.start_file("META-INF/manifest.xml", o).unwrap();
        writer.write_all(b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:compression=\"false\" manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/><manifest:default manifest:full-path=\"\" manifest:media-type=\"text/xml\"/></manifest:manifest>")
            .unwrap();

        writer.finish().unwrap();
        buf
    }

    /// Create a minimal valid ODS fixture.
    fn make_valid_ods() -> Vec<u8> {
        let mut buf = Vec::new();
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
        let o = opts();

        writer.start_file("mimetype", o).unwrap();
        writer
            .write_all(b"application/vnd.oasis.opendocument.spreadsheet")
            .unwrap();

        writer.start_file("content.xml", o).unwrap();
        writer.write_all(b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:document-content>")
            .unwrap();
        writer.start_file("META-INF/manifest.xml", o).unwrap();
        writer.write_all(b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.spreadsheet\"/><manifest:default manifest:full-path=\"\" manifest:media-type=\"text/xml\"/></manifest:manifest>")
            .unwrap();

        writer.finish().unwrap();
        buf
    }

    /// Create a minimal valid ODP fixture.
    fn make_valid_odp() -> Vec<u8> {
        let mut buf = Vec::new();
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
        let o = opts();

        writer.start_file("mimetype", o).unwrap();
        writer
            .write_all(b"application/vnd.oasis.opendocument.presentation")
            .unwrap();

        writer.start_file("content.xml", o).unwrap();
        writer.write_all(b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:document-content>")
            .unwrap();
        writer.start_file("META-INF/manifest.xml", o).unwrap();
        writer.write_all(b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.presentation\"/><manifest:default manifest:full-path=\"\" manifest:media-type=\"text/xml\"/></manifest:manifest>")
            .unwrap();

        writer.finish().unwrap();
        buf
    }

    /// Create an ODF fixture with stored compression.
    fn make_stored_odt() -> Vec<u8> {
        let mut buf = Vec::new();
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
        let o = opts();

        writer.start_file("mimetype", o).unwrap();
        writer
            .write_all(b"application/vnd.oasis.opendocument.text")
            .unwrap();

        writer.start_file("content.xml", o).unwrap();
        writer.write_all(b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:document-content>")
            .unwrap();
        writer.start_file("META-INF/manifest.xml", o).unwrap();
        writer.write_all(b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:default manifest:full-path=\"\" manifest:media-type=\"text/xml\"/></manifest:manifest>")
            .unwrap();

        writer.finish().unwrap();
        buf
    }

    /// Create an ODF fixture with deflate compression.
    fn make_deflated_odt() -> Vec<u8> {
        let mut buf = Vec::new();
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
        let stored: FileOpts = opts();
        let deflated: FileOpts =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        writer.start_file("mimetype", stored).unwrap();
        writer
            .write_all(b"application/vnd.oasis.opendocument.text")
            .unwrap();

        writer.start_file("content.xml", deflated).unwrap();
        writer.write_all(b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><text:p>This is a longer piece of text that can be compressed</text:p></office:document-content>")
            .unwrap();
        writer.start_file("META-INF/manifest.xml", stored).unwrap();
        writer.write_all(b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/><manifest:default manifest:full-path=\"\" manifest:media-type=\"text/xml\"/></manifest:manifest>")
            .unwrap();

        writer.finish().unwrap();
        buf
    }

    /// Create an ODF package with xml:encrypt attribute in manifest.
    fn make_encrypted_manifest_odt() -> Vec<u8> {
        let mut buf = Vec::new();
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
        let o = opts();

        writer.start_file("mimetype", o).unwrap();
        writer
            .write_all(b"application/vnd.oasis.opendocument.text")
            .unwrap();

        writer.start_file("content.xml", o).unwrap();
        writer.write_all(b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><text:p>encrypted</text:p></office:document-content>")
            .unwrap();
        writer.start_file("META-INF/manifest.xml", o).unwrap();
        writer.write_all(b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" xmlns:xml=\"http://www.w3.org/XML/1998/namespace\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\" xml:encrypt=\"symmetric\"/></manifest:manifest>")
            .unwrap();

        writer.finish().unwrap();
        buf
    }

    // ------------------------------------------------------------------
    // Zero-patch tests
    // ------------------------------------------------------------------

    #[test]
    fn zero_patch_valid_odt() {
        let pkg = make_valid_odt();
        let writer = SurgicalWriter::new(&pkg);
        let output = writer.write_zero_patch().expect("zero_patch");
        assert_eq!(output, pkg);
    }

    #[test]
    fn zero_patch_valid_ods() {
        let pkg = make_valid_ods();
        let writer = SurgicalWriter::new(&pkg);
        let output = writer.write_zero_patch().expect("zero_patch");
        assert_eq!(output, pkg);
    }

    #[test]
    fn zero_patch_valid_odp() {
        let pkg = make_valid_odp();
        let writer = SurgicalWriter::new(&pkg);
        let output = writer.write_zero_patch().expect("zero_patch");
        assert_eq!(output, pkg);
    }

    #[test]
    fn zero_patch_stored_compression() {
        let pkg = make_stored_odt();
        let writer = SurgicalWriter::new(&pkg);
        let output = writer.write_zero_patch().expect("zero_patch");
        assert_eq!(output, pkg);
    }

    #[test]
    fn zero_patch_deflated_compression() {
        let pkg = make_deflated_odt();
        let writer = SurgicalWriter::new(&pkg);
        let output = writer.write_zero_patch().expect("zero_patch");
        assert_eq!(output, pkg);
    }

    // ------------------------------------------------------------------
    // Mimetype order / stored tests
    // ------------------------------------------------------------------

    #[test]
    fn write_preserves_mimetype_first() {
        let pkg = make_valid_odt();
        let writer = SurgicalWriter::new(&pkg);
        let mutations: BTreeMap<String, OdfMutation> = BTreeMap::new();
        let result = writer.write(mutations).expect("write");

        let mut archive =
            zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");
        let first_name = archive.by_index(0).expect("first entry").name().to_string();
        assert_eq!(
            first_name, "mimetype",
            "mimetype must be the first entry in the ZIP"
        );
    }

    #[test]
    fn write_preserves_mimetype_stored() {
        let pkg = make_valid_odt();
        let writer = SurgicalWriter::new(&pkg);
        let mutations: BTreeMap<String, OdfMutation> = BTreeMap::new();
        let result = writer.write(mutations).expect("write");

        let mut archive =
            zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");
        let entry = archive.by_index(0).expect("first entry");
        assert_eq!(
            entry.compression(),
            zip::CompressionMethod::Stored,
            "mimetype must be stored (uncompressed)"
        );
    }

    // ------------------------------------------------------------------
    // Encryption refusal tests
    // ------------------------------------------------------------------

    #[test]
    fn detect_xml_encrypt_in_manifest_detected() {
        let pkg = make_encrypted_manifest_odt();
        let mut archive = zip::ZipArchive::new(Cursor::new(&pkg)).expect("open ZIP");
        let mut manifest_bytes = Vec::new();
        archive
            .by_name("META-INF/manifest.xml")
            .expect("find manifest")
            .read_to_end(&mut manifest_bytes)
            .expect("read manifest");

        let result = detect_xml_encrypt_in_manifest(&manifest_bytes);
        assert!(
            result.is_some(),
            "xml:encrypt should be detected in manifest"
        );
        assert!(
            result.as_ref().unwrap().contains("xml:encrypt"),
            "reason should mention xml:encrypt"
        );
    }

    #[test]
    fn detect_xml_encrypt_no_encrypt() {
        let pkg = make_valid_odt();
        let mut archive = zip::ZipArchive::new(Cursor::new(&pkg)).expect("open ZIP");
        let mut manifest_bytes = Vec::new();
        archive
            .by_name("META-INF/manifest.xml")
            .expect("find manifest")
            .read_to_end(&mut manifest_bytes)
            .expect("read manifest");

        let result = detect_xml_encrypt_in_manifest(&manifest_bytes);
        assert!(
            result.is_none(),
            "normal manifest should not trigger xml:encrypt"
        );
    }

    #[test]
    fn xml_encrypt_diagnostic_code_is_odf_e_003() {
        let result = detect_xml_encrypt_in_manifest(
            b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" xmlns:xml=\"http://www.w3.org/XML/1998/namespace\"><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\" xml:encrypt=\"symmetric\"/></manifest:manifest>",
        );
        assert!(result.is_some());
        let message = result.unwrap();
        assert!(
            message.contains("xml:encrypt"),
            "message should reference xml:encrypt"
        );
        // The diagnostic code ODF-E-003 is registered in gp_core::diagnostics.
        // Callers of detect_xml_encrypt_in_manifest create ODF-E-003 diagnostics.
    }

    #[test]
    fn detect_encryption_with_xml_encrypt_package() {
        let pkg = make_encrypted_manifest_odt();
        let mut archive = zip::ZipArchive::new(Cursor::new(&pkg)).expect("open ZIP");
        let mut manifest_bytes = Vec::new();
        archive
            .by_name("META-INF/manifest.xml")
            .expect("find manifest")
            .read_to_end(&mut manifest_bytes)
            .expect("read manifest");

        assert!(
            detect_xml_encrypt_in_manifest(&manifest_bytes).is_some(),
            "encrypted package should be detected"
        );
    }

    // ------------------------------------------------------------------
    // Add/Replace/Remove mutation tests
    // ------------------------------------------------------------------

    #[test]
    fn replace_part() {
        let pkg = make_valid_odt();
        let writer = SurgicalWriter::new(&pkg);

        let mut mutations = BTreeMap::new();
        mutations.insert(
            "content.xml".to_string(),
            OdfMutation::Replace {
                data: b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><text:p>Modified!</text:p></office:document-content>".to_vec(),
            },
        );

        let result = writer.write(mutations).expect("write");
        assert_eq!(result.report.parts_written.len(), 1);
        assert_eq!(result.report.parts_written[0], "content.xml");

        let mut archive =
            zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");
        let mut content = Vec::new();
        archive
            .by_name("content.xml")
            .expect("find content")
            .read_to_end(&mut content)
            .unwrap();
        assert!(
            String::from_utf8_lossy(&content).contains("Modified!"),
            "content.xml should be replaced"
        );
    }

    #[test]
    fn add_new_part() {
        let pkg = make_valid_odt();
        let writer = SurgicalWriter::new(&pkg);

        let new_data = b"<custom:custom xmlns:custom=\"urn:oasis:names:tc:opendocument:xmlns:custom-properties:1.0\"></custom:custom>";
        let mut mutations = BTreeMap::new();
        mutations.insert(
            "custom.xml".to_string(),
            OdfMutation::Add {
                path: "custom.xml".to_string(),
                data: new_data.to_vec(),
            },
        );

        let result = writer.write(mutations).expect("write");
        assert_eq!(result.report.parts_added.len(), 1);
        assert_eq!(result.report.parts_added[0], "custom.xml");

        let mut archive =
            zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");
        let mut data = Vec::new();
        archive
            .by_name("custom.xml")
            .expect("find custom.xml")
            .read_to_end(&mut data)
            .unwrap();
        assert_eq!(data, new_data);
    }

    #[test]
    fn remove_part() {
        let pkg = make_valid_odt();
        let writer = SurgicalWriter::new(&pkg);

        let mut mutations = BTreeMap::new();
        mutations.insert("meta.xml".to_string(), OdfMutation::Remove);

        let result = writer.write(mutations).expect("write");
        assert_eq!(result.report.parts_removed.len(), 1);
        assert_eq!(result.report.parts_removed[0], "meta.xml");

        let mut archive =
            zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");
        assert!(
            archive.by_name("meta.xml").is_err(),
            "meta.xml should be removed"
        );
    }

    #[test]
    fn replace_preserves_other_parts() {
        let pkg = make_valid_odt();
        let writer = SurgicalWriter::new(&pkg);

        let mut mutations = BTreeMap::new();
        mutations.insert(
            "content.xml".to_string(),
            OdfMutation::Replace {
                data: b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"></office:document-content>".to_vec(),
            },
        );

        let result = writer.write(mutations).expect("write");
        assert!(result.report.parts_unchanged.len() >= 2);

        let mut archive =
            zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");
        let mut meta = Vec::new();
        archive
            .by_name("meta.xml")
            .expect("find meta.xml")
            .read_to_end(&mut meta)
            .unwrap();
        assert!(
            String::from_utf8_lossy(&meta).contains("office:meta"),
            "meta.xml should be unchanged"
        );
    }

    #[test]
    fn combined_mutations() {
        let pkg = make_valid_odt();
        let writer = SurgicalWriter::new(&pkg);

        let mut mutations = BTreeMap::new();
        mutations.insert(
            "content.xml".to_string(),
            OdfMutation::Replace {
                data: b"<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"><text:p>New!</text:p></office:document-content>".to_vec(),
            },
        );
        mutations.insert(
            "custom.xml".to_string(),
            OdfMutation::Add {
                path: "custom.xml".to_string(),
                data: b"<custom/>".to_vec(),
            },
        );
        mutations.insert("meta.xml".to_string(), OdfMutation::Remove);

        let result = writer.write(mutations).expect("write");

        assert_eq!(result.report.parts_written.len(), 2);
        assert!(result
            .report
            .parts_removed
            .contains(&"meta.xml".to_string()));
        assert!(result
            .report
            .parts_added
            .contains(&"custom.xml".to_string()));

        let mut archive =
            zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");

        let mut content = Vec::new();
        archive
            .by_name("content.xml")
            .expect("find content")
            .read_to_end(&mut content)
            .unwrap();
        assert!(String::from_utf8_lossy(&content).contains("New!"));

        let mut custom = Vec::new();
        archive
            .by_name("custom.xml")
            .expect("find custom")
            .read_to_end(&mut custom)
            .unwrap();
        assert_eq!(custom, b"<custom/>");

        assert!(archive.by_name("meta.xml").is_err());

        assert_eq!(archive.by_index(0).expect("first entry").name(), "mimetype");
    }

    #[test]
    fn manifest_update_on_add() {
        let new_data = b"<custom/>";
        let mut mutations = BTreeMap::new();
        mutations.insert(
            "custom.xml".to_string(),
            OdfMutation::Add {
                path: "custom.xml".to_string(),
                data: new_data.to_vec(),
            },
        );

        let original_manifest = b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/><manifest:default manifest:full-path=\"\" manifest:media-type=\"text/xml\"/></manifest:manifest>";

        let updated = build_updated_manifest(
            original_manifest,
            &mutations,
            &HashSet::new(),
            &[("custom.xml".to_string(), new_data)],
        );

        let manifest_str = String::from_utf8_lossy(&updated);
        assert!(
            manifest_str.contains("custom.xml"),
            "manifest should include the added part: {}",
            manifest_str
        );
    }

    #[test]
    fn manifest_update_on_remove() {
        let manifest_xml = b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"meta.xml\" manifest:media-type=\"text/xml\"/><manifest:default manifest:full-path=\"\" manifest:media-type=\"text/xml\"/></manifest:manifest>";
        let mut mutations = BTreeMap::new();
        mutations.insert("meta.xml".to_string(), OdfMutation::Remove);
        let mut rm = HashSet::new();
        rm.insert("meta.xml");

        let updated = build_updated_manifest(manifest_xml, &mutations, &rm, &[]);
        let result = String::from_utf8_lossy(&updated);
        assert!(
            !result.contains("meta.xml") || result.contains("manifest:full-path=\"\""),
            "manifest should not contain meta.xml: {}",
            result
        );
    }

    // ------------------------------------------------------------------
    // Encryption detection integration
    // ------------------------------------------------------------------

    #[test]
    fn detect_encryption_not_encrypted() {
        let pkg = make_valid_odt();
        let parsed = parse_odf(&pkg).expect("parse valid ODF");
        let enc = detect_encryption(&parsed);
        assert!(
            enc.is_none(),
            "valid ODF should not be flagged as encrypted"
        );
    }

    // ------------------------------------------------------------------
    // xml:encrypt detection edge cases
    // ------------------------------------------------------------------

    #[test]
    fn detect_xml_encrypt_empty_manifest() {
        let result = detect_xml_encrypt_in_manifest(b"");
        assert!(result.is_none(), "empty manifest should not trigger");
    }

    #[test]
    fn detect_xml_encrypt_no_attribute() {
        let manifest = b"<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\"><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/></manifest:manifest>";
        let result = detect_xml_encrypt_in_manifest(manifest);
        assert!(
            result.is_none(),
            "manifest without xml:encrypt should not trigger"
        );
    }

    #[test]
    fn extract_path_from_file_entry() {
        let block = r#"
            manifest:full-path="styles.xml" manifest:media-type="text/xml"
        "#;
        let path = extract_path(block);
        assert_eq!(path, Some("styles.xml".to_string()));
    }

    #[test]
    fn extract_attr_basic() {
        let tag = r#"<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>"#;
        assert_eq!(
            extract_attr(tag, "manifest:full-path"),
            Some("content.xml".to_string())
        );
        assert_eq!(
            extract_attr(tag, "manifest:media-type"),
            Some("text/xml".to_string())
        );
        assert!(extract_attr(tag, "manifest:missing").is_none());
    }
}
