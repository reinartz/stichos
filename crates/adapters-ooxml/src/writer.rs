// SPDX-License-Identifier: MIT OR Apache-2.0
//! OPC surgical writer — applies mutations while preserving zip metadata.
//!
//! This module provides byte-identical round-trip through the OOXML writer:
//! - Zero-patch mode returns the original bytes unmodified
//! - Patched mode reconstructs the ZIP with mutations while preserving
//!   compression method, extra fields, and entry order

use gp_core::diagnostics::{Diagnostic, Severity};
use std::collections::{BTreeMap, HashSet};
use std::io::{Cursor, Read, Write};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A content override in [Content_Types].xml.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentOverride {
    pub part_name: String,
    pub content_type: String,
}

/// A default content override in [Content_Types].xml.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultOverride {
    pub extension: String,
    pub content_type: String,
}

/// A surgical mutation on an OPC part.
#[derive(Debug, Clone)]
pub enum PartMutation<'a> {
    /// Leave the part untouched — stream-copy raw bytes.
    Unchanged,
    /// Replace the part with new data.
    Replace {
        data: Vec<u8>,
        content_type: Option<String>,
    },
    /// Add a new part (uri, content-type, data, optional rels).
    Add {
        uri: String,
        content_type: String,
        data: Vec<u8>,
        rels: Option<Vec<Relationship<'a>>>,
    },
    /// Remove a part.
    Remove,
    /// Update relationships for a source part.
    UpdateRels {
        source_uri: String,
        rels: Vec<Relationship<'a>>,
    },
    /// Update content types in [Content_Types].xml.
    UpdateContentTypes {
        overrides: Vec<ContentOverride>,
        defaults: Vec<DefaultOverride>,
    },
}

impl<'a> PartMutation<'a> {
    /// Return the URI associated with this mutation, if any.
    pub fn uri(&self) -> Option<&str> {
        match self {
            PartMutation::Unchanged => None,
            PartMutation::Replace { .. } => None,
            PartMutation::Add { uri, .. } => Some(uri),
            PartMutation::Remove => None,
            PartMutation::UpdateRels { source_uri, .. } => Some(source_uri),
            PartMutation::UpdateContentTypes { .. } => None,
        }
    }
}

/// A relationship entry extracted from a `.rels` file.
#[derive(Debug, Clone)]
pub struct Relationship<'a> {
    /// The `Id` attribute (e.g. `rId1`).
    pub id: String,
    /// The `Target` attribute (relative URI).
    pub target: String,
    /// The `Type` attribute (relationship type URI).
    pub relationship_type: String,
    #[allow(dead_code)]
    pub target_mode: Option<&'a str>,
}

/// Per-part metadata preserved during surgery.
#[derive(Debug, Clone)]
pub struct PartMetadata {
    /// Compression method (0 = stored, 8 = deflate).
    pub compression_method: zip::CompressionMethod,
    /// Extra field bytes from the central directory.
    pub extra_fields: Vec<u8>,
    /// Compressed size of the entry.
    pub compressed_size: u64,
    /// Uncompressed size of the entry.
    pub uncompressed_size: u64,
}

/// Per-part write report.
#[derive(Debug, Clone, Default)]
pub struct WriteReport {
    /// Parts that were written (replaced or added).
    pub parts_written: Vec<String>,
    /// Parts that were unchanged.
    pub parts_unchanged: Vec<String>,
    /// Parts that were removed.
    pub parts_removed: Vec<String>,
    /// Parts that were added.
    pub parts_added: Vec<String>,
    /// Parts whose relationships were updated.
    pub rels_updated: Vec<String>,
    /// Unknown parts that passed through unchanged.
    pub unknown_parts_pass_through: Vec<String>,
}

/// Result of a surgical write.
#[derive(Debug)]
pub struct WriteResult {
    /// The output OPC bytes.
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

/// Surgical OPC writer — applies mutations while preserving zip metadata.
pub struct SurgicalWriter<'a> {
    /// Original package bytes (for zero-patch mode).
    pub source_bytes: &'a [u8],
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

    /// Apply mutations to the OPC package.
    ///
    /// For each entry in the source ZIP:
    /// - If `Unchanged`: copy raw bytes through the zip writer
    /// - If `Replace`: write new data with same compression method
    /// - If `Remove`: skip the entry
    /// - If `Add`: write new entry with new data (and optional rels)
    /// - If `UpdateRels`: update a relationship file
    /// - If `UpdateContentTypes`: merge content type overrides
    pub fn write(
        &self,
        mutations: BTreeMap<String, PartMutation<'a>>,
    ) -> Result<WriteResult, Vec<Diagnostic>> {
        // Parse the source ZIP to read entry metadata.
        let source_cursor = Cursor::new(self.source_bytes);
        let mut archive = match zip::ZipArchive::new(source_cursor) {
            Ok(a) => a,
            Err(e) => {
                return Err(vec![Diagnostic::new(
                    "OOX-E-001",
                    Severity::Error,
                    format!("Failed to open ZIP archive: {}", e),
                )]);
            }
        };

        let entry_count = archive.len();

        // Collect metadata for each entry.
        let mut entries: Vec<(String, PartMetadata)> = Vec::new();
        for i in 0..entry_count {
            if let Ok(entry) = archive.by_index(i) {
                let name = entry.name().to_string();
                let metadata = PartMetadata {
                    compression_method: entry.compression(),
                    extra_fields: Vec::new(),
                    compressed_size: entry.compressed_size(),
                    uncompressed_size: entry.size(),
                };
                entries.push((name, metadata));
            }
        }

        // Classify mutations.
        let remove_uris: HashSet<&str> = mutations
            .iter()
            .filter_map(|(_uri, m)| match m {
                PartMutation::Remove => Some(_uri.as_str()),
                _ => None,
            })
            .collect();

        #[allow(clippy::type_complexity)]
        let add_parts: Vec<(&str, &str, &[u8], Option<Vec<Relationship<'a>>>)> = mutations
            .values()
            .filter_map(|m| match m {
                PartMutation::Add {
                    uri: add_uri,
                    content_type,
                    data,
                    rels,
                } => Some((
                    add_uri.as_str(),
                    content_type.as_str(),
                    data.as_slice(),
                    rels.clone(),
                )),
                _ => None,
            })
            .collect();

        #[allow(clippy::type_complexity)]
        let update_rels: Vec<(&str, &[Relationship<'a>])> = mutations
            .values()
            .filter_map(|m| match m {
                PartMutation::UpdateRels { source_uri, rels } => {
                    Some((source_uri.as_str(), rels.as_slice()))
                }
                _ => None,
            })
            .collect();

        let update_ct = mutations
            .values()
            .any(|m| matches!(m, PartMutation::UpdateContentTypes { .. }));

        // Collect content type overrides from mutations.
        let mut ct_overrides: Vec<ContentOverride> = Vec::new();
        let mut ct_defaults: Vec<DefaultOverride> = Vec::new();
        for m in mutations.values() {
            if let PartMutation::UpdateContentTypes {
                overrides,
                defaults,
            } = m
            {
                ct_overrides.extend(overrides.clone());
                ct_defaults.extend(defaults.clone());
            }
        }

        // Build the output ZIP.
        let bytes = Self::build_zip(
            &entries,
            &mutations,
            &remove_uris,
            &add_parts,
            &update_rels,
            update_ct,
            &ct_overrides,
            &ct_defaults,
            self.source_bytes,
        )?;

        // Build the report.
        let mut report = WriteReport::default();

        // Known URIs from mutations.
        let mut known_uris: HashSet<&str> = HashSet::new();
        for m in mutations.values() {
            if let Some(uri) = m.uri() {
                known_uris.insert(uri);
            }
        }

        // Classify source entries.
        for (uri, _meta) in &entries {
            let uri_str = uri.as_str();
            if remove_uris.contains(uri_str) {
                report.parts_removed.push(uri.clone());
            } else if matches!(mutations.get(uri_str), Some(PartMutation::Replace { .. })) {
                report.parts_written.push(uri.clone());
            } else if mutations.contains_key(uri_str) {
                report.parts_unchanged.push(uri.clone());
            } else {
                report.unknown_parts_pass_through.push(uri.clone());
                report.parts_unchanged.push(uri.clone());
            }
        }

        // Added parts.
        for (uri, _, _, _) in &add_parts {
            report.parts_added.push(uri.to_string());
            report.parts_written.push(uri.to_string());
        }

        // Updated rels.
        for (uri, _) in &update_rels {
            report.rels_updated.push(uri.to_string());
        }

        Ok(WriteResult::new(bytes, report))
    }

    /// Build the output ZIP from entries and mutations.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::type_complexity)]
    fn build_zip(
        entries: &[(String, PartMetadata)],
        mutations: &BTreeMap<String, PartMutation<'a>>,
        remove_uris: &HashSet<&str>,
        add_parts: &[(&str, &str, &[u8], Option<Vec<Relationship<'a>>>)],
        _update_rels: &[(&str, &[Relationship<'a>])],
        update_ct: bool,
        ct_overrides: &[ContentOverride],
        ct_defaults: &[DefaultOverride],
        source_bytes: &[u8],
    ) -> Result<Vec<u8>, Vec<Diagnostic>> {
        let mut zip_writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let stored_opts: zip::write::FileOptions<'static, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let deflate_opts: zip::write::FileOptions<'static, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        // Re-read the source to extract data.
        let source_cursor = Cursor::new(source_bytes);
        let mut source_archive = zip::ZipArchive::new(source_cursor).map_err(|e| {
            vec![Diagnostic::new(
                "OOX-E-001",
                Severity::Error,
                format!("Failed to re-open ZIP: {}", e),
            )]
        })?;

        // Track which parts we've written (for Add parts that might match source URIs).
        let mut written_uris: HashSet<&str> = HashSet::new();

        for (uri, metadata) in entries {
            let uri_str = uri.as_str();

            // Skip removed entries.
            if remove_uris.contains(uri_str) {
                continue;
            }

            let mutation = mutations.get(uri_str);

            // Determine options based on mutation and compression method.
            let opts = match mutation {
                Some(PartMutation::Replace { .. }) | Some(PartMutation::UpdateRels { .. }) => {
                    match metadata.compression_method {
                        zip::CompressionMethod::Stored => &stored_opts,
                        _ => &deflate_opts,
                    }
                }
                Some(PartMutation::Unchanged) | None => match metadata.compression_method {
                    zip::CompressionMethod::Stored => &stored_opts,
                    _ => &deflate_opts,
                },
                Some(PartMutation::Remove) | Some(PartMutation::Add { .. }) => continue,
                Some(PartMutation::UpdateContentTypes { .. }) => continue,
            };

            match mutation {
                Some(PartMutation::Replace { data, .. }) => {
                    zip_writer.start_file(uri, *opts).map_err(|e| {
                        vec![Diagnostic::new(
                            "OOX-E-001",
                            Severity::Error,
                            format!("Failed to start file '{}': {}", uri, e),
                        )]
                    })?;
                    zip_writer.write_all(data).map_err(|e| {
                        vec![Diagnostic::new(
                            "OOX-E-001",
                            Severity::Error,
                            format!("Failed to write '{}': {}", uri, e),
                        )]
                    })?;
                    written_uris.insert(uri_str);
                }
                Some(PartMutation::UpdateRels { rels, .. }) => {
                    // Write updated relationships.
                    let rels_xml = Self::rels_to_xml(rels);
                    zip_writer.start_file(uri, *opts).map_err(|e| {
                        vec![Diagnostic::new(
                            "OOX-E-001",
                            Severity::Error,
                            format!("Failed to start rels file '{}': {}", uri, e),
                        )]
                    })?;
                    zip_writer.write_all(rels_xml.as_bytes()).map_err(|e| {
                        vec![Diagnostic::new(
                            "OOX-E-001",
                            Severity::Error,
                            format!("Failed to write rels '{}': {}", uri, e),
                        )]
                    })?;
                    written_uris.insert(uri_str);
                }
                Some(PartMutation::Unchanged) | None => {
                    let mut buffer = Vec::new();
                    if let Ok(mut reader) = source_archive.by_name(uri) {
                        let _ = reader.read_to_end(&mut buffer);
                    }
                    zip_writer.start_file(uri, *opts).map_err(|e| {
                        vec![Diagnostic::new(
                            "OOX-E-001",
                            Severity::Error,
                            format!("Failed to start file '{}': {}", uri, e),
                        )]
                    })?;
                    zip_writer.write_all(&buffer).map_err(|e| {
                        vec![Diagnostic::new(
                            "OOX-E-001",
                            Severity::Error,
                            format!("Failed to write '{}': {}", uri, e),
                        )]
                    })?;
                    written_uris.insert(uri_str);
                }
                _ => {}
            }
        }

        // Handle UpdateRels mutations for entries that don't exist in source.
        for (uri_str, rels) in _update_rels {
            if !written_uris.contains(*uri_str) {
                let rels_xml = Self::rels_to_xml(rels);
                zip_writer.start_file(uri_str, deflate_opts).map_err(|e| {
                    vec![Diagnostic::new(
                        "OOX-E-001",
                        Severity::Error,
                        format!("Failed to start rels file '{}': {}", uri_str, e),
                    )]
                })?;
                zip_writer.write_all(rels_xml.as_bytes()).map_err(|e| {
                    vec![Diagnostic::new(
                        "OOX-E-001",
                        Severity::Error,
                        format!("Failed to write rels '{}': {}", uri_str, e),
                    )]
                })?;
            }
        }

        // Add new parts with optional rels.
        for (uri, _content_type, data, rels_opt) in add_parts {
            let uri_str = *uri;
            zip_writer.start_file(uri_str, deflate_opts).map_err(|e| {
                vec![Diagnostic::new(
                    "OOX-E-001",
                    Severity::Error,
                    format!("Failed to add file '{}': {}", uri_str, e),
                )]
            })?;
            zip_writer.write_all(data).map_err(|e| {
                vec![Diagnostic::new(
                    "OOX-E-001",
                    Severity::Error,
                    format!("Failed to write '{}': {}", uri_str, e),
                )]
            })?;

            // If rels were specified, write them.
            if let Some(ref rels) = rels_opt {
                let rels_xml = Self::rels_to_xml(rels);
                let rels_uri = Self::rels_uri_for(uri_str);
                zip_writer
                    .start_file(&rels_uri, deflate_opts)
                    .map_err(|e| {
                        vec![Diagnostic::new(
                            "OOX-E-001",
                            Severity::Error,
                            format!("Failed to add rels file '{}': {}", rels_uri, e),
                        )]
                    })?;
                zip_writer.write_all(rels_xml.as_bytes()).map_err(|e| {
                    vec![Diagnostic::new(
                        "OOX-E-001",
                        Severity::Error,
                        format!("Failed to write rels '{}': {}", rels_uri, e),
                    )]
                })?;
            }

            written_uris.insert(uri_str);
        }

        // If content types need updating, write/update [Content_Types].xml.
        if update_ct {
            let ct_xml = Self::build_content_types_xml(ct_overrides, ct_defaults);
            zip_writer
                .start_file("[Content_Types].xml", stored_opts)
                .map_err(|e| {
                    vec![Diagnostic::new(
                        "OOX-E-001",
                        Severity::Error,
                        format!("Failed to write [Content_Types].xml: {}", e),
                    )]
                })?;
            zip_writer.write_all(ct_xml.as_bytes()).map_err(|e| {
                vec![Diagnostic::new(
                    "OOX-E-001",
                    Severity::Error,
                    format!("Failed to write [Content_Types].xml: {}", e),
                )]
            })?;
        }

        let cursor = zip_writer.finish().map_err(|e| {
            vec![Diagnostic::new(
                "OOX-E-001",
                Severity::Error,
                format!("Failed to finish ZIP: {}", e),
            )]
        })?;

        Ok(cursor.into_inner())
    }

    /// Generate XML for relationships.
    fn rels_to_xml(rels: &[Relationship]) -> String {
        let mut xml = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
        xml.push_str("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">");
        for rel in rels {
            xml.push_str("<Relationship ");
            xml.push_str(&format!("Id=\"{}\"", rel.id));
            xml.push_str(&format!(" Type=\"{}\"", rel.relationship_type));
            xml.push_str(&format!(" Target=\"{}\"", rel.target));
            if let Some(mode) = rel.target_mode {
                xml.push_str(&format!(" TargetMode=\"{}\"", mode));
            }
            xml.push_str("/>");
        }
        xml.push_str("</Relationships>");
        xml
    }

    /// Generate [Content_Types].xml from overrides and defaults.
    fn build_content_types_xml(
        overrides: &[ContentOverride],
        defaults: &[DefaultOverride],
    ) -> String {
        let mut xml = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
        xml.push_str(
            "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
        );
        for default in defaults {
            xml.push_str("<Default ");
            xml.push_str(&format!("Extension=\"{}\"", default.extension));
            xml.push_str(&format!(" ContentType=\"{}\"", default.content_type));
            xml.push_str("/>");
        }
        for override_ in overrides {
            xml.push_str("<Override ");
            xml.push_str(&format!("PartName=\"{}\"", override_.part_name));
            xml.push_str(&format!(" ContentType=\"{}\"", override_.content_type));
            xml.push_str("/>");
        }
        xml.push_str("</Types>");
        xml
    }

    /// Generate the rels URI for a given part URI.
    fn rels_uri_for(part_uri: &str) -> String {
        // For "word/document.xml" -> "word/_rels/document.xml.rels"
        if let Some(pos) = part_uri.rfind('/') {
            let dir = &part_uri[..pos];
            let base = &part_uri[pos + 1..];
            let rels_base = format!("_rels/{}.rels", base);
            format!("{}/{}", dir, rels_base)
        } else {
            format!("_rels/{}.rels", part_uri)
        }
    }

    /// Convert a part URI to its relationship file path.
    fn part_uri_to_rels_path(part_uri: &str) -> String {
        // For "word/document.xml" -> "word/_rels/document.xml.rels"
        // For "[Content_Types].xml" -> "_rels/[Content_Types].xml.rels"
        let (dir, base) = if let Some(pos) = part_uri.rfind('/') {
            (&part_uri[..pos], &part_uri[pos + 1..])
        } else {
            ("", part_uri)
        };
        if dir.is_empty() {
            format!("_rels/{}.rels", base)
        } else {
            format!("{}/_rels/{}.rels", dir, base)
        }
    }

    /// Write an OPC package from parts without requiring original source bytes.
    /// This is used by format-specific writers that reconstruct package bytes
    /// from parsed parts.
    pub fn write_from_parts(
        &self,
        parts: &[super::opc::Part],
        mutations: BTreeMap<String, PartMutation<'a>>,
    ) -> Result<WriteResult, Vec<Diagnostic>> {
        let content_types_xml = Self::build_content_types_xml_inner(&mutations, parts);

        let remove_uris: HashSet<&str> = mutations
            .iter()
            .filter_map(|(uri, m)| match m {
                PartMutation::Remove => Some(uri.as_str()),
                _ => None,
            })
            .collect();

        let mut write_report = WriteReport::default();
        let mut output = Vec::new();
        let mut zip_writer = zip::ZipWriter::new(std::io::Cursor::new(&mut output));
        let opts: zip::write::FileOptions<'static, ()> = zip::write::FileOptions::default();

        macro_rules! zip_err {
            ($e:expr) => {
                match $e {
                    Ok(v) => v,
                    Err(e) => {
                        return Err(vec![Diagnostic::new(
                            "OOX-E-001",
                            Severity::Error,
                            format!("ZIP error: {}", e),
                        )]);
                    }
                }
            };
        }

        zip_err!(zip_writer.start_file("[Content_Types].xml", opts));
        zip_err!(zip_writer.write_all(content_types_xml.as_bytes()));

        let has_rels_parts = parts.iter().any(|p| {
            p.uri.starts_with("ppt/") || p.uri.starts_with("word/") || p.uri.starts_with("xl/")
        });
        if has_rels_parts {
            let _ = zip_writer.start_file("_rels/.rels", opts);
            let _ = zip_writer.write_all(b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                               <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
                               </Relationships>");
        }

        for part in parts {
            let uri = &part.uri;
            if remove_uris.contains(uri.as_str()) {
                write_report.parts_removed.push(uri.clone());
                continue;
            }

            if let Some(PartMutation::Replace { data, .. }) = mutations.get(uri) {
                let _ = zip_writer.start_file(uri, opts);
                let _ = zip_writer.write_all(data);
                write_report.parts_written.push(uri.clone());
            } else if let Some(PartMutation::Add { data, rels, .. }) = mutations.get(uri) {
                let _ = zip_writer.start_file(uri, opts);
                let _ = zip_writer.write_all(data);
                write_report.parts_added.push(uri.clone());
                if let Some(ref rels_list) = rels {
                    let rels_content = Self::rels_to_xml(rels_list);
                    let rels_path = Self::part_uri_to_rels_path(uri);
                    let _ = zip_writer.start_file(&rels_path, opts);
                    let _ = zip_writer.write_all(rels_content.as_bytes());
                }
            } else {
                let _ = zip_writer.start_file(uri, opts);
                let _ = zip_writer.write_all(&part.data);
                write_report.parts_unchanged.push(uri.clone());
            }
        }

        for (_uri, m) in &mutations {
            if let PartMutation::UpdateRels { source_uri, rels } = m {
                let rels_content = Self::rels_to_xml(rels);
                let rels_path = Self::part_uri_to_rels_path(source_uri);
                let _ = zip_writer.start_file(&rels_path, opts);
                let _ = zip_writer.write_all(rels_content.as_bytes());
                write_report.rels_updated.push(_uri.clone());
            }
        }

        zip_err!(zip_writer.finish());

        Ok(WriteResult {
            bytes: output,
            report: write_report,
        })
    }

    /// Generate content types XML from mutations and parts.
    fn build_content_types_xml_inner(
        mutations: &BTreeMap<String, PartMutation<'a>>,
        parts: &[super::opc::Part],
    ) -> String {
        let mut xml = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
        xml.push_str(
            "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
        );

        // Add defaults for common extensions
        xml.push_str("<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>");
        xml.push_str("<Default Extension=\"xml\" ContentType=\"application/xml\"/>");

        // Add overrides for parts
        for part in parts {
            let ct = if part.uri.starts_with("ppt/") {
                "application/vnd.openxmlformats-officedocument.presentationml.part"
            } else if part.uri.starts_with("word/") {
                "application/vnd.openxmlformats-officedocument.wordprocessingml.part"
            } else if part.uri.starts_with("xl/") {
                "application/vnd.openxmlformats-officedocument.spreadsheetml.part"
            } else {
                "application/xml"
            };
            xml.push_str(&format!(
                "<Override PartName=\"{}\" ContentType=\"{}\"/>",
                part.uri, ct
            ));
        }

        // Add overrides for content type mutations
        for m in mutations.values() {
            if let PartMutation::UpdateContentTypes { overrides, .. } = m {
                for override_ in overrides {
                    xml.push_str(&format!(
                        "<Override PartName=\"{}\" ContentType=\"{}\"/>",
                        override_.part_name, override_.content_type
                    ));
                }
            }
            if let PartMutation::Add { .. } = m {
                // Add new parts
            }
        }

        xml.push_str("</Types>");
        xml
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read a ZIP entry's bytes.
    fn read_entry_bytes(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        archive
            .by_name(name)
            .unwrap()
            .read_to_end(&mut buf)
            .unwrap();
        buf
    }

    /// Read a ZIP entry's bytes into a String.
    fn read_entry_str(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> String {
        let mut buf = Vec::new();
        archive
            .by_name(name)
            .unwrap()
            .read_to_end(&mut buf)
            .unwrap();
        String::from_utf8(buf).unwrap()
    }

    /// Create a minimal OPC package (ZIP) with given parts.
    fn make_minimal_opc(parts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
        let options: zip::write::FileOptions<'static, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

        for (name, data) in parts {
            writer.start_file(name, options).unwrap();
            writer.write_all(data).unwrap();
        }

        writer.finish().unwrap();
        buf
    }

    /// Create a minimal OPC package with deflate compression.
    fn make_deflated_opc(parts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
        let options: zip::write::FileOptions<'static, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        for (name, data) in parts {
            writer.start_file(name, options).unwrap();
            writer.write_all(data).unwrap();
        }

        writer.finish().unwrap();
        buf
    }

    // ------------------------------------------------------------------
    // Zero-patch tests
    // ------------------------------------------------------------------

    #[test]
    fn zero_patch_returns_identical_bytes() {
        let data = b"PK\x03\x04test data here";
        let writer = SurgicalWriter::new(data);
        let output = writer.write_zero_patch().expect("zero_patch");
        assert_eq!(output, data.as_slice());
    }

    #[test]
    fn zero_patch_empty_package() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"PK\x05\x06");
        buf.extend_from_slice(&[0u8; 18]);
        let writer = SurgicalWriter::new(&buf);
        let output = writer.write_zero_patch().expect("zero_patch");
        assert_eq!(output, buf);
    }

    #[test]
    fn zero_patch_minimal_docx() {
        let pkg = make_minimal_opc(&[
            ("[Content_Types].xml", b"<Types></Types>"),
            ("_rels/.rels", b"<Relationships></Relationships>"),
            ("word/document.xml", b"<w:document></w:document>"),
        ]);
        let writer = SurgicalWriter::new(&pkg);
        let output = writer.write_zero_patch().expect("zero_patch");
        assert_eq!(output, pkg);
    }

    // ------------------------------------------------------------------
    // Replace part tests
    // ------------------------------------------------------------------

    #[test]
    fn replace_single_part() {
        let pkg = make_minimal_opc(&[
            ("[Content_Types].xml", b"<Types></Types>"),
            ("word/document.xml", b"original content"),
        ]);
        let writer = SurgicalWriter::new(&pkg);

        let mut mutations = BTreeMap::new();
        mutations.insert(
            "word/document.xml".to_string(),
            PartMutation::Replace {
                data: b"replaced content".to_vec(),
                content_type: None,
            },
        );

        let result = writer.write(mutations).expect("write");
        assert_eq!(result.report.parts_written.len(), 1);
        assert_eq!(result.report.parts_written[0], "word/document.xml");

        let mut archive =
            zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");
        let doc_bytes = read_entry_bytes(&mut archive, "word/document.xml");
        assert_eq!(String::from_utf8_lossy(&doc_bytes), "replaced content");
    }

    #[test]
    fn replace_preserves_other_parts() {
        let pkg = make_minimal_opc(&[
            ("[Content_Types].xml", b"<Types></Types>"),
            ("word/document.xml", b"doc content"),
            ("word/styles.xml", b"styles content"),
        ]);
        let writer = SurgicalWriter::new(&pkg);

        let mut mutations = BTreeMap::new();
        mutations.insert(
            "word/document.xml".to_string(),
            PartMutation::Replace {
                data: b"new doc".to_vec(),
                content_type: None,
            },
        );

        let result = writer.write(mutations).expect("write");
        assert_eq!(result.report.parts_unchanged.len(), 2);

        let mut archive =
            zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");
        let ct_bytes = read_entry_bytes(&mut archive, "[Content_Types].xml");
        assert_eq!(String::from_utf8_lossy(&ct_bytes), "<Types></Types>");
    }

    // ------------------------------------------------------------------
    // Add part tests
    // ------------------------------------------------------------------

    #[test]
    fn add_new_part() {
        let pkg = make_minimal_opc(&[
            ("[Content_Types].xml", b"<Types></Types>"),
            ("word/document.xml", b"doc content"),
        ]);
        let writer = SurgicalWriter::new(&pkg);

        let mut mutations = BTreeMap::new();
        mutations.insert(
            "word/new-part.xml".to_string(),
            PartMutation::Add {
                uri: "word/new-part.xml".to_string(),
                content_type: "application/xml".to_string(),
                data: b"<new></new>".to_vec(),
                rels: None,
            },
        );

        let result = writer.write(mutations).expect("write");
        assert_eq!(result.report.parts_added.len(), 1);
        assert_eq!(result.report.parts_added[0], "word/new-part.xml");

        let mut archive =
            zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");
        let new_bytes = read_entry_bytes(&mut archive, "word/new-part.xml");
        assert_eq!(String::from_utf8_lossy(&new_bytes), "<new></new>");
    }

    #[test]
    fn add_new_part_with_rels() {
        let pkg = make_minimal_opc(&[
            ("[Content_Types].xml", b"<Types></Types>"),
            ("word/document.xml", b"doc content"),
        ]);
        let writer = SurgicalWriter::new(&pkg);

        let mut mutations = BTreeMap::new();
        mutations.insert(
            "word/new-part.xml".to_string(),
            PartMutation::Add {
                uri: "word/new-part.xml".to_string(),
                content_type: "application/xml".to_string(),
                data: b"<new></new>".to_vec(),
                rels: Some(vec![Relationship {
                    id: "rId1".to_string(),
                    target: "embeddings/v1.bin".to_string(),
                    relationship_type: "http://schemas.openxmlformats.org/officeDocument/6.0/relationships/embeddedObject".to_string(),
                    target_mode: None,
                }]),
            },
        );

        let result = writer.write(mutations).expect("write");

        // Verify new part exists.
        let mut archive =
            zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");
        let new_bytes = read_entry_bytes(&mut archive, "word/new-part.xml");
        assert_eq!(String::from_utf8_lossy(&new_bytes), "<new></new>");

        // Verify rels were created.
        let rels_bytes = read_entry_bytes(&mut archive, "word/_rels/new-part.xml.rels");
        assert!(String::from_utf8_lossy(&rels_bytes).contains("rId1"));
        assert!(String::from_utf8_lossy(&rels_bytes).contains("rId1"));
    }

    // ------------------------------------------------------------------
    // Remove part tests
    // ------------------------------------------------------------------

    #[test]
    fn remove_part() {
        let pkg = make_minimal_opc(&[
            ("[Content_Types].xml", b"<Types></Types>"),
            ("word/document.xml", b"doc content"),
            ("word/extra.xml", b"extra content"),
        ]);
        let writer = SurgicalWriter::new(&pkg);

        let mut mutations = BTreeMap::new();
        mutations.insert("word/extra.xml".to_string(), PartMutation::Remove);

        let result = writer.write(mutations).expect("write");
        assert_eq!(result.report.parts_removed.len(), 1);
        assert_eq!(result.report.parts_removed[0], "word/extra.xml");

        let mut archive =
            zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");
        assert!(archive.by_name("word/extra.xml").is_err());
    }

    // ------------------------------------------------------------------
    // Update rels tests
    // ------------------------------------------------------------------

    #[test]
    fn update_rels_creates_rels_file() {
        let pkg = make_minimal_opc(&[
            ("[Content_Types].xml", b"<Types></Types>"),
            ("word/document.xml", b"<w:document></w:document>"),
        ]);
        let writer = SurgicalWriter::new(&pkg);

        let mut mutations = BTreeMap::new();
        mutations.insert(
            "word/_rels/document.xml.rels".to_string(),
            PartMutation::UpdateRels {
                source_uri: "word/_rels/document.xml.rels".to_string(),
                rels: vec![Relationship {
                    id: "rId1".to_string(),
                    target: "embeddings/v1.bin".to_string(),
                    relationship_type: "http://schemas.openxmlformats.org/officeDocument/6.0/relationships/embeddedObject".to_string(),
                    target_mode: None,
                }],
            },
        );

        let result = writer.write(mutations).expect("write");
        assert_eq!(result.report.rels_updated.len(), 1);

        let mut archive =
            zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");
        let rels_bytes = read_entry_bytes(&mut archive, "word/_rels/document.xml.rels");
        assert!(String::from_utf8_lossy(&rels_bytes).contains("rId1"));
    }

    // ------------------------------------------------------------------
    // Update content types tests
    // ------------------------------------------------------------------

    #[test]
    fn update_content_types_written() {
        let pkg = make_minimal_opc(&[
            ("[Content_Types].xml", b"<Types></Types>"),
            ("word/document.xml", b"<w:document></w:document>"),
        ]);
        let writer = SurgicalWriter::new(&pkg);

        let mut mutations = BTreeMap::new();
        mutations.insert(
            "[Content_Types].xml".to_string(),
            PartMutation::UpdateContentTypes {
                overrides: vec![ContentOverride {
                    part_name: "/word/document.xml".to_string(),
                    content_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml".to_string(),
                }],
                defaults: vec![DefaultOverride {
                    extension: "bin".to_string(),
                    content_type: "application/vnd.openxmlformats-officedocument.package+xml".to_string(),
                }],
            },
        );

        let result = writer.write(mutations).expect("write");

        let mut archive =
            zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");
        let ct_bytes = read_entry_str(&mut archive, "[Content_Types].xml");
        assert!(ct_bytes.contains("word/document.xml"));
        assert!(ct_bytes.contains("bin"));
    }

    // ------------------------------------------------------------------
    // Compression method tests
    // ------------------------------------------------------------------

    #[test]
    fn replace_preserves_stored_compression() {
        let pkg = make_minimal_opc(&[
            ("[Content_Types].xml", b"<Types></Types>"),
            ("word/document.xml", b"stored content"),
        ]);
        let writer = SurgicalWriter::new(&pkg);

        let mut mutations = BTreeMap::new();
        mutations.insert(
            "word/document.xml".to_string(),
            PartMutation::Replace {
                data: b"new stored".to_vec(),
                content_type: None,
            },
        );

        let result = writer.write(mutations).expect("write");
        let mut archive =
            zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");
        let entry = archive.by_name("word/document.xml").expect("find entry");
        assert_eq!(entry.compression(), zip::CompressionMethod::Stored);
    }

    // ------------------------------------------------------------------
    // Multiple mutations tests
    // ------------------------------------------------------------------

    #[test]
    fn replace_add_remove_combined() {
        let pkg = make_minimal_opc(&[
            ("[Content_Types].xml", b"<Types></Types>"),
            ("word/document.xml", b"doc"),
            ("word/old.xml", b"old"),
        ]);
        let writer = SurgicalWriter::new(&pkg);

        let mut mutations = BTreeMap::new();
        mutations.insert(
            "word/document.xml".to_string(),
            PartMutation::Replace {
                data: b"new doc".to_vec(),
                content_type: None,
            },
        );
        mutations.insert(
            "word/new.xml".to_string(),
            PartMutation::Add {
                uri: "word/new.xml".to_string(),
                content_type: "application/xml".to_string(),
                data: b"new".to_vec(),
                rels: None,
            },
        );
        mutations.insert("word/old.xml".to_string(), PartMutation::Remove);

        let result = writer.write(mutations).expect("write");

        let mut archive =
            zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");

        // Replaced part exists with new content.
        let _bytes = read_entry_bytes(&mut archive, "word/document.xml");
        assert_eq!(String::from_utf8_lossy(&_bytes), "new doc");

        // Added part exists.
        let _bytes = read_entry_bytes(&mut archive, "word/new.xml");
        assert_eq!(String::from_utf8_lossy(&_bytes), "new");

        // Removed part is gone.
        assert!(archive.by_name("word/old.xml").is_err());
    }

    // ------------------------------------------------------------------
    // Unknown parts pass-through tests
    // ------------------------------------------------------------------

    #[test]
    fn unknown_parts_inventoried_in_report() {
        let pkg = make_minimal_opc(&[
            ("[Content_Types].xml", b"<Types></Types>"),
            ("word/document.xml", b"doc"),
            ("word/unknown.xml", b"unknown"),
        ]);
        let writer = SurgicalWriter::new(&pkg);

        let mut mutations = BTreeMap::new();
        // Only replace document.xml, leave unknown.xml untouched.
        mutations.insert(
            "word/document.xml".to_string(),
            PartMutation::Replace {
                data: b"new doc".to_vec(),
                content_type: None,
            },
        );

        let result = writer.write(mutations).expect("write");

        // Unknown part should be in pass-through and unchanged.
        assert!(result
            .report
            .unknown_parts_pass_through
            .contains(&"word/unknown.xml".to_string()));
        assert!(result
            .report
            .parts_unchanged
            .contains(&"word/unknown.xml".to_string()));

        // Verify pass-through content is intact.
        let mut archive =
            zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");
        let unknown_bytes = read_entry_bytes(&mut archive, "word/unknown.xml");
        assert_eq!(String::from_utf8_lossy(&unknown_bytes), "unknown");
    }

    // ------------------------------------------------------------------
    // Deflate preservation test
    // ------------------------------------------------------------------

    #[test]
    fn deflate_compression_preserved() {
        let pkg = make_deflated_opc(&[
            ("[Content_Types].xml", b"<Types></Types>"),
            (
                "word/document.xml",
                b"deflated content that should be compressible",
            ),
        ]);
        let writer = SurgicalWriter::new(&pkg);

        let mutations: BTreeMap<String, PartMutation> = BTreeMap::new();
        let result = writer.write(mutations).expect("write");

        let mut archive =
            zip::ZipArchive::new(Cursor::new(result.bytes.as_slice())).expect("open result ZIP");
        assert_eq!(
            read_entry_str(&mut archive, "word/document.xml"),
            "deflated content that should be compressible"
        );
    }

    // ------------------------------------------------------------------
    // Corpus fixture test
    // ------------------------------------------------------------------

    #[test]
    fn zero_patch_on_corpus_docx() {
        let fixture_path = format!(
            "{}/../corpus/blobs/cjk-text-docx-001.docx",
            env!("CARGO_MANIFEST_DIR")
        );
        if let Ok(data) = std::fs::read(&fixture_path) {
            let writer = SurgicalWriter::new(&data);
            let output = writer.write_zero_patch().expect("zero_patch");
            assert_eq!(output, data);
        }
    }
}
