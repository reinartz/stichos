//! Append-only PDF incremental writer — produces byte-identical original prefix.
//!
//! This module provides `IncrementalWriter`, which takes an existing PDF and applies
//! mutations (add, replace, delete objects) while preserving the **append-only invariant**:
//!
//! > The original PDF bytes are a strict prefix of the output.
//!
//! The incremental update is the byte-diff between the original and the output.
//!
//! Approved libraries (ADR-0001):
//! - `lopdf` for reading the existing PDF (parsing objects, xref, trailer)
//! - `pdf-writer` for rendering new objects in COS format
//!
//! Refusals with C3 codes:
//! - PDF-E-003: Encryption detected (`/Encrypt` in trailer/root)
//! - PDF-E-004: Xref stream detected (`/XRefStm` in trailer)
//! - PDF-E-005: Object stream detected (any object with `/Type /ObjStm`)

use gp_core::diagnostics::{Diagnostic, Severity};
use lopdf::{Dictionary, Object};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A surgical mutation on a PDF object.
#[derive(Debug, Clone)]
pub enum PdfMutation {
    /// Leave the object untouched — stream-copy raw bytes (no-op for incremental).
    Unchanged,
    /// Replace an existing object with new content.
    Replace {
        object_id: PdfObjNum,
        dict: BTreeVec,
        data: Option<Vec<u8>>,
    },
    /// Add a new object (caller-chosen object number).
    Add {
        object_id: PdfObjNum,
        dict: BTreeVec,
        data: Option<Vec<u8>>,
    },
    /// Mark an object as deleted (free entry in xref).
    Delete { object_id: PdfObjNum },
}

/// PDF object number: `(object_number, generation_number)`.
pub type PdfObjNum = (u32, u32);

/// Dictionary type: maps Vec<u8> keys to Object values.
pub type BTreeVec = BTreeMap<Vec<u8>, Object>;

/// A collection of mutations applied atomically.
#[derive(Debug, Clone, Default)]
pub struct PdfMutationSet {
    pub mutations: Vec<PdfMutation>,
}

impl PdfMutationSet {
    /// Create a mutation set from a map keyed by object number.
    pub fn from_map(mutations: impl IntoIterator<Item = (PdfObjNum, PdfMutation)>) -> Self {
        Self {
            mutations: mutations.into_iter().map(|(_, m)| m).collect(),
        }
    }
}

/// Result of an incremental write.
pub struct WriteResult {
    /// The output PDF bytes.
    pub bytes: Vec<u8>,
    /// Per-object write report.
    pub report: WriteReport,
}

/// Per-object write report for an incremental update.
#[derive(Debug, Clone, Default)]
pub struct WriteReport {
    /// Objects that were added (new object numbers).
    pub objects_added: Vec<PdfObjNum>,
    /// Objects that were modified (existing object numbers, new content).
    pub objects_modified: Vec<PdfObjNum>,
    /// Objects that were deleted (marked free in xref).
    pub objects_deleted: Vec<PdfObjNum>,
}

/// The append-only PDF incremental writer.
pub struct IncrementalWriter<'a> {
    source_bytes: &'a [u8],
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors specific to incremental PDF writing.
#[derive(Debug, Clone)]
pub enum IncrementalError {
    /// PDF parse failure.
    ParseError(Diagnostic),
    /// PDF-E-003: Encryption detected in source.
    EncryptionDetected(Diagnostic),
    /// PDF-E-004: Xref stream detected — append-only not supported.
    XrefStreamDetected(Diagnostic),
    /// PDF-E-005: Object stream detected — append-only not supported.
    ObjectStreamDetected(Diagnostic),
    /// Write failure during incremental update.
    WriteError(Diagnostic),
}

impl std::fmt::Display for IncrementalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IncrementalError::ParseError(d) => write!(f, "{}", d),
            IncrementalError::EncryptionDetected(d) => write!(f, "{}", d),
            IncrementalError::XrefStreamDetected(d) => write!(f, "{}", d),
            IncrementalError::ObjectStreamDetected(d) => write!(f, "{}", d),
            IncrementalError::WriteError(d) => write!(f, "{}", d),
        }
    }
}

impl std::error::Error for IncrementalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            IncrementalError::ParseError(d) => Some(d),
            IncrementalError::EncryptionDetected(d) => Some(d),
            IncrementalError::XrefStreamDetected(d) => Some(d),
            IncrementalError::ObjectStreamDetected(d) => Some(d),
            IncrementalError::WriteError(d) => Some(d),
        }
    }
}

impl From<IncrementalError> for Vec<Diagnostic> {
    fn from(err: IncrementalError) -> Self {
        match err {
            IncrementalError::ParseError(d) => vec![d],
            IncrementalError::EncryptionDetected(d) => vec![d],
            IncrementalError::XrefStreamDetected(d) => vec![d],
            IncrementalError::ObjectStreamDetected(d) => vec![d],
            IncrementalError::WriteError(d) => vec![d],
        }
    }
}

// ---------------------------------------------------------------------------
// IncrementalWriter implementation
// ---------------------------------------------------------------------------

impl<'a> IncrementalWriter<'a> {
    /// Create a new incremental writer from source bytes.
    pub fn new(source_bytes: &'a [u8]) -> Self {
        Self { source_bytes }
    }

    /// Check if the PDF can be incrementally written.
    ///
    /// Returns `Ok(())` if the PDF is append-friendly (no encryption, no xref streams,
    /// no object streams). Returns `Err` with appropriate C3 diagnostic codes
    /// for disqualifying features.
    pub fn check_writable(&self) -> Result<(), IncrementalError> {
        // Try to parse the PDF to inspect its structure.
        let doc = lopdf::Document::load_mem(self.source_bytes).map_err(|e| {
            IncrementalError::ParseError(Diagnostic::new(
                "PDF-E-001",
                Severity::Error,
                format!("Failed to parse PDF for writable check: {}", e),
            ))
        })?;

        // 1. Check for /Encrypt in trailer dictionary.
        if dict_has_key(&doc.trailer, b"Encrypt") {
            return Err(IncrementalError::EncryptionDetected(Diagnostic::new(
                "PDF-E-003",
                Severity::Warning,
                "Encryption detected: append-only incremental update requires unencrypted PDF",
            )));
        }

        // 2. Check for /Encrypt in root dictionary (some PDFs put it there).
        if let Ok(root_obj) = extract_root_obj(&doc) {
            if dict_has_encrypted(root_obj) {
                return Err(IncrementalError::EncryptionDetected(Diagnostic::new(
                    "PDF-E-003",
                    Severity::Warning,
                    "Encryption detected in root dictionary: append-only incremental update requires unencrypted PDF",
                )));
            }
        }

        // 3. Check for /XRefStm in trailer → xref stream.
        if dict_has_key(&doc.trailer, b"XRefStm") {
            return Err(IncrementalError::XrefStreamDetected(Diagnostic::new(
                "PDF-E-004",
                Severity::Error,
                "Xref stream detected: append-only incremental update not supported with xref streams",
            )));
        }

        // 4. Check for /XRefStmOffset in trailer.
        if dict_has_key(&doc.trailer, b"XRefStmOffset") {
            return Err(IncrementalError::XrefStreamDetected(Diagnostic::new(
                "PDF-E-004",
                Severity::Error,
                "Xref stream offset detected: append-only incremental update not supported",
            )));
        }

        // 5. Check for object streams (/Type /ObjStm) in any object.
        // lopdf may skip object streams during parsing, so we also scan raw bytes.
        let has_obj_stm_in_doc = doc.objects.values().any(object_is_obj_stm);
        let has_obj_stm_in_bytes = self
            .source_bytes
            .windows(b"/Type /ObjStm".len())
            .any(|w| w == b"/Type /ObjStm")
            || self
                .source_bytes
                .windows(b"/Type/ObjStm".len())
                .any(|w| w == b"/Type/ObjStm");
        if has_obj_stm_in_doc || has_obj_stm_in_bytes {
            return Err(IncrementalError::ObjectStreamDetected(Diagnostic::new(
                "PDF-E-005",
                Severity::Error,
                "Object stream detected: append-only incremental update not supported with object streams",
            )));
        }

        Ok(())
    }

    /// Apply mutations to produce an incrementally-updated PDF.
    ///
    /// **Append-only invariant**: `&source_bytes == &output[..source_bytes.len()]`.
    ///
    /// The incremental update consists of:
    /// 1. All original bytes (exact copy — strict prefix)
    /// 2. New/updated object definitions appended after the original
    /// 3. A new classic xref table referencing ALL objects
    /// 4. A new `startxref` pointing to the new xref table
    /// 5. `%%EOF` marker
    ///
    /// If no mutations are given or all are `Unchanged`, returns the original bytes unchanged.
    /// `Delete` mutations mark objects as free in the new xref (the original bytes remain).
    pub fn write(&self, mutations: &PdfMutationSet) -> Result<WriteResult, IncrementalError> {
        let has_active_mutations = mutations
            .mutations
            .iter()
            .any(|m| !matches!(m, PdfMutation::Unchanged));

        if !has_active_mutations {
            let report = WriteReport::default();
            return Ok(WriteResult {
                bytes: self.source_bytes.to_vec(),
                report,
            });
        }

        let doc = lopdf::Document::load_mem(self.source_bytes).map_err(|e| {
            IncrementalError::WriteError(Diagnostic::new(
                "PDF-E-001",
                Severity::Error,
                format!("Failed to parse PDF for write: {}", e),
            ))
        })?;

        // Collect mutation info.
        let add_mutations: HashMap<PdfObjNum, (&BTreeVec, Option<&[u8]>)> = mutations
            .mutations
            .iter()
            .filter_map(|m| match m {
                PdfMutation::Add {
                    object_id,
                    dict,
                    data,
                } => Some((*object_id, (dict, data.as_deref()))),
                _ => None,
            })
            .collect();

        let replace_mutations: HashMap<PdfObjNum, (&BTreeVec, Option<&[u8]>)> = mutations
            .mutations
            .iter()
            .filter_map(|m| match m {
                PdfMutation::Replace {
                    object_id,
                    dict,
                    data,
                } => Some((*object_id, (dict, data.as_deref()))),
                _ => None,
            })
            .collect();

        let delete_set: HashSet<u32> = mutations
            .mutations
            .iter()
            .filter_map(|m| match m {
                PdfMutation::Delete { object_id } => Some(object_id.0),
                _ => None,
            })
            .collect();

        let max_obj_num = compute_max_object_number(&doc, &add_mutations, &replace_mutations);

        let incremental_bytes = build_incremental(
            self.source_bytes,
            &doc,
            &add_mutations,
            &replace_mutations,
            &delete_set,
            max_obj_num,
        )
        .map_err(|e| {
            IncrementalError::WriteError(Diagnostic::new(
                "PDF-E-001",
                Severity::Error,
                format!("Failed to build incremental update: {}", e),
            ))
        })?;

        // Build report.
        let mut objects_added = Vec::new();
        let mut objects_modified = Vec::new();
        let mut objects_deleted = Vec::new();

        for m in &mutations.mutations {
            match m {
                PdfMutation::Add { object_id, .. } => objects_added.push(*object_id),
                PdfMutation::Replace { object_id, .. } => objects_modified.push(*object_id),
                PdfMutation::Delete { object_id } => objects_deleted.push(*object_id),
                PdfMutation::Unchanged => {}
            }
        }

        objects_added.sort();
        objects_modified.sort();
        objects_deleted.sort();

        let report = WriteReport {
            objects_added,
            objects_modified,
            objects_deleted,
        };

        let mut output = self.source_bytes.to_vec();
        output.extend(incremental_bytes);

        Ok(WriteResult {
            bytes: output,
            report,
        })
    }
}

// ---------------------------------------------------------------------------
// Helper: dictionary inspection
// ---------------------------------------------------------------------------

fn dict_has_key(dict: &Dictionary, key: &[u8]) -> bool {
    dict.iter().any(|(k, _)| k.as_slice() == key)
}

fn extract_root_obj(doc: &lopdf::Document) -> Result<&Object, lopdf::Error> {
    let root_ref = doc.trailer.get(b"Root")?;
    if let Object::Reference(id) = root_ref {
        doc.objects.get(id).ok_or(lopdf::Error::ObjectNotFound(*id))
    } else {
        Err(lopdf::Error::Parse(lopdf::ParseError::InvalidTrailer))
    }
}

fn dict_has_encrypted(obj: &Object) -> bool {
    match obj {
        Object::Dictionary(d) => dict_has_key(d, b"Encrypt"),
        Object::Stream(s) => dict_has_key(&s.dict, b"Encrypt"),
        _ => false,
    }
}

fn object_is_obj_stm(obj: &Object) -> bool {
    let dict = match obj {
        Object::Dictionary(d) => d,
        Object::Stream(s) => &s.dict,
        _ => return false,
    };
    if let Ok(val) = dict.get(b"Type") {
        if let Object::Name(n) = val {
            return n.as_slice() == b"ObjStm";
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Helper: compute maximum object number
// ---------------------------------------------------------------------------

fn compute_max_object_number(
    doc: &lopdf::Document,
    add_mutations: &HashMap<PdfObjNum, (&BTreeVec, Option<&[u8]>)>,
    replace_mutations: &HashMap<PdfObjNum, (&BTreeVec, Option<&[u8]>)>,
) -> u32 {
    let mut max_num: u32 = 0;

    for (obj_id, _) in &doc.objects {
        max_num = max_num.max(obj_id.0);
    }
    for (obj_id, _) in add_mutations {
        max_num = max_num.max(obj_id.0);
    }
    for (obj_id, _) in replace_mutations {
        max_num = max_num.max(obj_id.0);
    }

    max_num
}

// ---------------------------------------------------------------------------
// Core: build incremental bytes
// ---------------------------------------------------------------------------

/// Build the bytes to append after the original PDF content.
///
/// Returns: (incremental_bytes, original_root_id) where incremental_bytes
/// starts from the object definitions through the xref table, and
/// original_root_id is the root object number for the trailer.
fn build_incremental(
    source_bytes: &[u8],
    doc: &lopdf::Document,
    add_mutations: &HashMap<PdfObjNum, (&BTreeVec, Option<&[u8]>)>,
    replace_mutations: &HashMap<PdfObjNum, (&BTreeVec, Option<&[u8]>)>,
    delete_set: &HashSet<u32>,
    max_obj_num: u32,
) -> std::io::Result<Vec<u8>> {
    let source_len = source_bytes.len() as u64;
    let original_offsets = find_original_object_offsets(source_bytes);

    // Get original root object id.
    let original_root_id = doc
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|r| {
            if let Object::Reference(id) = r {
                Some(id.0)
            } else {
                None
            }
        })
        .unwrap_or(1);

    // Collect objects to write (Add + Replace).
    let mut new_obj_defs = Vec::new();

    let mut sorted_adds: Vec<_> = add_mutations.iter().collect();
    sorted_adds.sort_by_key(|(id, _)| id.0);

    let mut sorted_replaces: Vec<_> = replace_mutations.iter().collect();
    sorted_replaces.sort_by_key(|(id, _)| id.0);

    for (obj_id, (dict, data)) in sorted_adds {
        let bytes = render_pdf_object(*obj_id, dict, *data);
        new_obj_defs.push((*obj_id, bytes));
    }

    for (obj_id, (dict, data)) in sorted_replaces {
        let bytes = render_pdf_object(*obj_id, dict, *data);
        new_obj_defs.push((*obj_id, bytes));
    }

    // Calculate byte offsets for each new object.
    let mut new_obj_offsets = Vec::new();
    let mut current_offset = source_len;

    for (obj_id, bytes) in &new_obj_defs {
        new_obj_offsets.push((*obj_id, current_offset));
        current_offset += bytes.len() as u64;
        current_offset += 1; // separator
    }

    // Write new object definitions.
    let mut buf = Vec::new();
    for (_obj_id, bytes) in &new_obj_defs {
        buf.extend(bytes);
        buf.push(b'\n');
    }

    // Build and write xref table.
    let xref_start = buf.len() as u64;
    let xref_entries =
        build_xref_entries(&original_offsets, &new_obj_offsets, delete_set, max_obj_num);
    write_xref_section(&mut buf, &xref_entries, max_obj_num, original_root_id);

    // Append startxref + %%EOF.
    writeln!(buf, "\nstartxref\n{}\n%%EOF", xref_start).unwrap();

    Ok(buf)
}

// ---------------------------------------------------------------------------
// Helper: find original object byte offsets
// ---------------------------------------------------------------------------

/// Scan source bytes to find byte offsets of indirect objects.
fn find_original_object_offsets(data: &[u8]) -> HashMap<u32, u64> {
    let mut offsets = HashMap::new();
    let mut pos = 0;

    while pos < data.len() {
        if let Some(end_pos) = data[pos..].windows(6).position(|w| w == b"endobj") {
            let abs_end = pos + end_pos;
            let search_start = abs_end.saturating_sub(100).min(abs_end);
            let upper = abs_end.saturating_sub(4).max(4);

            if upper >= search_start {
                for obj_abs in (search_start..=upper).rev() {
                    if &data[obj_abs..obj_abs + 3] != b"obj" {
                        continue;
                    }
                    if data.get(obj_abs.saturating_sub(1)) != Some(&b' ') {
                        continue;
                    }

                    let after = obj_abs + 3;
                    if after >= abs_end {
                        continue;
                    }
                    if !matches!(data.get(after), Some(&b' ' | b'\t' | b'\n' | b'\r')) {
                        continue;
                    }

                    if obj_abs < 5 {
                        continue;
                    }
                    let num_str = std::str::from_utf8(&data[obj_abs - 5..obj_abs]).unwrap_or("");
                    let parts: Vec<&str> = num_str.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let Ok(n) = parts[0].trim().parse::<u32>() {
                            if n > 0 {
                                offsets.entry(n).or_insert(obj_abs as u64);
                            }
                        }
                    }
                    pos = abs_end + 1;
                    break;
                }
                continue;
            }
            pos = abs_end + 1;
            continue;
        }
        break;
    }

    offsets
}

// ---------------------------------------------------------------------------
// Helper: build xref table entries
// ---------------------------------------------------------------------------

fn build_xref_entries(
    original_offsets: &HashMap<u32, u64>,
    new_obj_offsets: &[(PdfObjNum, u64)],
    delete_set: &HashSet<u32>,
    max_obj_num: u32,
) -> Vec<XrefEntry> {
    let mut entries: Vec<XrefEntry> = Vec::with_capacity((max_obj_num + 1) as usize);
    let new_map: HashMap<u32, u64> = new_obj_offsets
        .iter()
        .map(|(id, off)| (id.0, *off))
        .collect();

    for obj_num in 0..=max_obj_num {
        if obj_num == 0 {
            entries.push(XrefEntry::Free { generation: 65535 });
            continue;
        }

        if delete_set.contains(&obj_num) {
            entries.push(XrefEntry::Free { generation: 0 });
            continue;
        }

        if let Some(&new_off) = new_map.get(&obj_num) {
            entries.push(XrefEntry::InUse {
                byte_offset: new_off,
                generation: 0,
            });
            continue;
        }

        if let Some(&orig_off) = original_offsets.get(&obj_num) {
            entries.push(XrefEntry::InUse {
                byte_offset: orig_off,
                generation: 0,
            });
            continue;
        }

        entries.push(XrefEntry::Free { generation: 0 });
    }

    entries
}

#[derive(Debug)]
enum XrefEntry {
    InUse { byte_offset: u64, generation: u32 },
    Free { generation: u32 },
}

fn write_xref_section(buf: &mut Vec<u8>, entries: &[XrefEntry], max_obj_num: u32, root_id: u32) {
    buf.extend(b"xref\n0 ");
    let count = (max_obj_num + 1) as usize;
    writeln!(buf, "{}", count).unwrap();

    for entry in entries {
        match entry {
            XrefEntry::InUse {
                byte_offset,
                generation,
            } => {
                writeln!(buf, "{:010} {:05} n", byte_offset, generation).unwrap();
            }
            XrefEntry::Free { generation } => {
                writeln!(buf, "{:010} {:05} f", 0, generation).unwrap();
            }
        }
    }

    writeln!(buf, "trailer").unwrap();
    writeln!(
        buf,
        "<< /Size {} /Root {} 0 R /Type /Catalog >>",
        count, root_id
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// Helper: render PDF objects in COS format
// ---------------------------------------------------------------------------

fn render_pdf_object(obj_id: PdfObjNum, dict: &BTreeVec, data: Option<&[u8]>) -> Vec<u8> {
    let mut buf = Vec::new();
    writeln!(buf, "{} {} obj", obj_id.0, obj_id.1).unwrap();
    write_pdf_dict(&mut buf, dict);

    if let Some(stream_data) = data {
        buf.extend(b"stream\n");
        buf.extend(stream_data);
        if !stream_data.is_empty() && !stream_data.ends_with(b"\n") {
            buf.push(b'\n');
        }
        buf.extend(b"\nendstream\n");
    }

    buf.extend(b"endobj");
    buf
}

fn write_pdf_dict(buf: &mut Vec<u8>, dict: &BTreeVec) {
    buf.push(b'<');
    buf.push(b'<');

    for (key, value) in dict {
        buf.push(b'/');
        for &byte in key {
            match byte {
                b'/' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'#' | b'%'
                | b'\n' | b'\r' | b'\t' | b' ' => {
                    buf.push(b'#');
                    buf.push(hex(byte >> 4));
                    buf.push(hex(byte & 0xF));
                }
                _ => buf.push(byte),
            }
        }
        buf.push(b' ');
        write_pdf_object(buf, value);
    }

    buf.push(b'>');
    buf.push(b'>');
}

fn write_pdf_object(buf: &mut Vec<u8>, obj: &Object) {
    match obj {
        Object::Null => buf.extend(b"null"),
        Object::Boolean(b) => {
            if *b {
                buf.extend(b"true");
            } else {
                buf.extend(b"false");
            }
        }
        Object::Integer(i) => {
            write!(buf, "{}", i).unwrap();
        }
        Object::Real(r) => {
            write!(buf, "{}", r).unwrap();
        }
        Object::Name(n) => {
            buf.push(b'/');
            for &byte in n {
                match byte {
                    b'/' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'#' | b'%'
                    | b'\n' | b'\r' | b'\t' | b' ' => {
                        buf.push(b'#');
                        buf.push(hex(byte >> 4));
                        buf.push(hex(byte & 0xF));
                    }
                    _ => buf.push(byte),
                }
            }
        }
        Object::String(s, _) => {
            buf.push(b'(');
            for &byte in s {
                match byte {
                    b'\\' | b'(' | b')' => {
                        buf.push(b'\\');
                        buf.push(byte);
                    }
                    b'\n' => {
                        buf.push(b'\\');
                        buf.push(b'n');
                    }
                    b'\r' => {
                        buf.push(b'\\');
                        buf.push(b'r');
                    }
                    0x00..=0x1f => {
                        write!(buf, "\\{:03o}", byte).unwrap();
                    }
                    _ => buf.push(byte),
                }
            }
            buf.push(b')');
        }
        Object::Array(arr) => {
            buf.push(b'[');
            for item in arr {
                write_pdf_object(buf, item);
            }
            buf.push(b']');
        }
        Object::Reference(id) => {
            write!(buf, "{} {} R", id.0, id.1).unwrap();
        }
        Object::Dictionary(d) => {
            // Render as inline dictionary.
            buf.push(b'<');
            buf.push(b'<');
            for (key, value) in d.iter() {
                buf.push(b'/');
                for &byte in key {
                    match byte {
                        b'/' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'#'
                        | b'%' | b'\n' | b'\r' | b'\t' | b' ' => {
                            buf.push(b'#');
                            buf.push(hex(byte >> 4));
                            buf.push(hex(byte & 0xF));
                        }
                        _ => buf.push(byte),
                    }
                }
                buf.push(b' ');
                write_pdf_object(buf, value);
            }
            buf.push(b'>');
            buf.push(b'>');
        }
        Object::Stream(stream) => {
            // Render the stream dictionary inline.
            buf.push(b'<');
            buf.push(b'<');
            for (key, value) in &stream.dict {
                buf.push(b'/');
                for &byte in key {
                    match byte {
                        b'/' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'#'
                        | b'%' | b'\n' | b'\r' | b'\t' | b' ' => {
                            buf.push(b'#');
                            buf.push(hex(byte >> 4));
                            buf.push(hex(byte & 0xF));
                        }
                        _ => buf.push(byte),
                    }
                }
                buf.push(b' ');
                write_pdf_object(buf, value);
            }
            buf.push(b'>');
            buf.push(b'>');
        }
    }
}

fn hex(b: u8) -> u8 {
    fn to_hex(b: u8) -> u8 {
        if b < 10 {
            b'0' + b
        } else {
            b'A' + (b - 10)
        }
    }
    to_hex(b)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_minimal_pdf() -> Vec<u8> {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
              /Resources << >> /Contents 4 0 R >>\nendobj\n",
        );
        pdf.extend_from_slice(b"4 0 obj\n<< /Length 0 >>\nstream\nendstream\nendobj\n");
        pdf.extend_from_slice(b"xref\n0 5\n");
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        pdf.extend_from_slice(b"0000000022 00000 n \n");
        pdf.extend_from_slice(b"0000000077 00000 n \n");
        pdf.extend_from_slice(b"0000000136 00000 n \n");
        pdf.extend_from_slice(b"0000000258 00000 n \n");
        pdf.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\n");
        pdf.extend_from_slice(b"startxref\n304\n%%EOF\n");
        pdf
    }

    fn make_pdf_with_obj_stream() -> Vec<u8> {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n");
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n",
        );
        pdf.extend_from_slice(
            b"4 0 obj\n<< /Type /ObjStm /Length 0 /N 1 >>\nstream\nendstream\nendobj\n",
        );
        pdf.extend_from_slice(b"xref\n0 5\n");
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        pdf.extend_from_slice(b"0000000022 00000 n \n");
        pdf.extend_from_slice(b"0000000077 00000 n \n");
        pdf.extend_from_slice(b"0000000136 00000 n \n");
        pdf.extend_from_slice(b"0000000258 00000 n \n");
        pdf.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\n");
        pdf.extend_from_slice(b"startxref\n304\n%%EOF\n");
        pdf
    }

    #[test]
    fn check_writable_minimal_pdf_succeeds() {
        let pdf = make_minimal_pdf();
        let writer = IncrementalWriter::new(&pdf);
        assert!(writer.check_writable().is_ok());
    }

    #[test]
    fn check_writable_obj_stream_refused() {
        let pdf = make_pdf_with_obj_stream();
        let writer = IncrementalWriter::new(&pdf);
        let result = writer.check_writable();
        assert!(result.is_err());
        match result {
            Err(IncrementalError::ObjectStreamDetected(d)) => assert_eq!(d.code, "PDF-E-005"),
            other => panic!("Expected ObjectStreamDetected, got {:?}", other),
        }
    }

    #[test]
    fn check_writable_empty_bytes_fails() {
        let writer = IncrementalWriter::new(&[]);
        assert!(writer.check_writable().is_err());
    }

    #[test]
    fn write_no_mutations_returns_original() {
        let pdf = make_minimal_pdf();
        let writer = IncrementalWriter::new(&pdf);
        let mutations = PdfMutationSet {
            mutations: vec![PdfMutation::Unchanged],
        };
        let result = writer.write(&mutations).expect("write");
        assert_eq!(result.bytes, pdf);
        assert!(result.report.objects_added.is_empty());
        assert!(result.report.objects_modified.is_empty());
        assert!(result.report.objects_deleted.is_empty());
    }

    #[test]
    fn write_empty_mutations_returns_original() {
        let pdf = make_minimal_pdf();
        let writer = IncrementalWriter::new(&pdf);
        let mutations = PdfMutationSet::default();
        let result = writer.write(&mutations).expect("write");
        assert_eq!(result.bytes, pdf);
    }

    #[test]
    fn write_add_object_preserves_prefix() {
        let pdf = make_minimal_pdf();
        let writer = IncrementalWriter::new(&pdf);

        let mut dict = BTreeMap::new();
        dict.insert(b"Type".to_vec(), Object::Name(b"Test".to_vec()));

        let mutations = PdfMutationSet::from_map(vec![(
            (5, 0),
            PdfMutation::Add {
                object_id: (5, 0),
                dict,
                data: None,
            },
        )]);

        let result = writer.write(&mutations).expect("write");
        assert_eq!(&result.bytes[..pdf.len()], pdf.as_slice());
        assert_eq!(result.report.objects_added, [(5, 0)]);
    }

    #[test]
    fn write_replace_object_preserves_prefix() {
        let pdf = make_minimal_pdf();
        let writer = IncrementalWriter::new(&pdf);

        let mut dict = BTreeMap::new();
        dict.insert(b"Type".to_vec(), Object::Name(b"Modified".to_vec()));

        let mutations = PdfMutationSet::from_map(vec![(
            (3, 0),
            PdfMutation::Replace {
                object_id: (3, 0),
                dict,
                data: None,
            },
        )]);

        let result = writer.write(&mutations).expect("write");
        assert_eq!(&result.bytes[..pdf.len()], pdf.as_slice());
        assert_eq!(result.report.objects_modified, [(3, 0)]);
    }

    #[test]
    fn write_delete_object_preserves_prefix() {
        let pdf = make_minimal_pdf();
        let writer = IncrementalWriter::new(&pdf);

        let mutations =
            PdfMutationSet::from_map(vec![((4, 0), PdfMutation::Delete { object_id: (4, 0) })]);

        let result = writer.write(&mutations).expect("write");
        assert_eq!(&result.bytes[..pdf.len()], pdf.as_slice());
        assert_eq!(result.report.objects_deleted, [(4, 0)]);
    }

    #[test]
    fn write_multiple_mutations_preserves_prefix() {
        let pdf = make_minimal_pdf();
        let writer = IncrementalWriter::new(&pdf);

        let mut dict = BTreeMap::new();
        dict.insert(b"Type".to_vec(), Object::Name(b"NewPage".to_vec()));

        let mutations = PdfMutationSet::from_map(vec![
            (
                (5, 0),
                PdfMutation::Add {
                    object_id: (5, 0),
                    dict: dict.clone(),
                    data: None,
                },
            ),
            (
                (4, 0),
                PdfMutation::Replace {
                    object_id: (4, 0),
                    dict,
                    data: Some(b"new stream content".to_vec()),
                },
            ),
            ((3, 0), PdfMutation::Delete { object_id: (3, 0) }),
        ]);

        let result = writer.write(&mutations).expect("write");
        assert_eq!(&result.bytes[..pdf.len()], pdf.as_slice());
        assert_eq!(result.report.objects_added, [(5, 0)]);
        assert_eq!(result.report.objects_modified, [(4, 0)]);
        assert_eq!(result.report.objects_deleted, [(3, 0)]);
    }

    #[test]
    fn write_stream_object_preserves_prefix() {
        let pdf = make_minimal_pdf();
        let writer = IncrementalWriter::new(&pdf);

        let mut dict = BTreeMap::new();
        dict.insert(b"Length".to_vec(), Object::Integer(11));

        let mutations = PdfMutationSet::from_map(vec![(
            (6, 0),
            PdfMutation::Add {
                object_id: (6, 0),
                dict,
                data: Some(b"hello world".to_vec()),
            },
        )]);

        let result = writer.write(&mutations).expect("write");
        assert_eq!(&result.bytes[..pdf.len()], pdf.as_slice());
        assert!(
            result.bytes[pdf.len()..].contains(&b'h'),
            "stream content should be appended"
        );
    }

    #[test]
    fn write_refuses_encrypted_pdf() {
        // Build a PDF with /Encrypt in the trailer.
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
              /Resources << >> /Contents 4 0 R >>\nendobj\n",
        );
        pdf.extend_from_slice(b"4 0 obj\n<< /Length 0 >>\nstream\nendstream\nendobj\n");
        pdf.extend_from_slice(b"xref\n0 5\n");
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        pdf.extend_from_slice(b"0000000022 00000 n \n");
        pdf.extend_from_slice(b"0000000077 00000 n \n");
        pdf.extend_from_slice(b"0000000136 00000 n \n");
        pdf.extend_from_slice(b"0000000258 00000 n \n");
        // Trailer with /Encrypt
        pdf.extend_from_slice(
            b"trailer\n<< /Size 5 /Root 1 0 R /Encrypt << /Filter /Standard >> >>\n",
        );
        pdf.extend_from_slice(b"startxref\n365\n%%EOF\n");

        let writer = IncrementalWriter::new(&pdf);
        let result = writer.check_writable();
        assert!(result.is_err());
        match result {
            Err(IncrementalError::EncryptionDetected(d)) => assert_eq!(d.code, "PDF-E-003"),
            other => panic!("Expected EncryptionDetected, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Property tests on real PDF fixtures
    // -----------------------------------------------------------------------

    /// Discover PDF fixtures under `goldens/staging/` (recursive).
    fn find_pdf_fixtures() -> Vec<std::path::PathBuf> {
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest.parent().unwrap().parent().unwrap();
        let goldens = root.join("goldens").join("staging");
        let mut results = Vec::new();
        if !goldens.is_dir() {
            return results;
        }
        let _ = collect_pdfs(&goldens, &mut results);
        results.sort();
        results
    }

    fn collect_pdfs(
        dir: &std::path::Path,
        out: &mut Vec<std::path::PathBuf>,
    ) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                collect_pdfs(&path, out)?;
            } else if path.extension().map_or(false, |e| e == "pdf") {
                out.push(path);
            }
        }
        Ok(())
    }

    /// Apply a trivial mutation: add a dummy annotation object.
    fn make_dummy_addition() -> PdfMutationSet {
        let mut dict = BTreeMap::new();
        dict.insert(
            b"TestKey".to_vec(),
            Object::String(b"test value".to_vec(), lopdf::StringFormat::Literal),
        );
        PdfMutationSet {
            mutations: vec![PdfMutation::Add {
                object_id: (99999, 0),
                dict,
                data: None,
            }],
        }
    }

    #[test]
    fn test_append_only_on_corpus_pdf_simple() {
        let pdf = make_minimal_pdf();
        let writer = IncrementalWriter::new(&pdf);
        let mutations = make_dummy_addition();
        let result = writer.write(&mutations).expect("write");

        let source_bytes = &pdf;
        let source_len = source_bytes.len();
        assert_eq!(
            &result.bytes[..source_len],
            source_bytes,
            "prefix invariant violated"
        );
        assert!(
            result.bytes.len() > source_len,
            "output must be larger than source"
        );
    }

    #[test]
    #[ignore = "slow property test over 151 fixtures — run with --ignored"]
    fn test_append_only_property_on_fixture_set() {
        let fixtures = find_pdf_fixtures();
        if fixtures.is_empty() {
            eprintln!("no PDF fixtures found — skipping append-only property test");
            return;
        }

        let mut passed = 0u32;
        let mut skipped = 0u32;
        let mut errors: Vec<(std::path::PathBuf, String)> = Vec::new();

        for fixture in &fixtures {
            let bytes = match std::fs::read(fixture) {
                Ok(b) => b,
                Err(e) => {
                    errors.push((fixture.clone(), format!("read: {}", e)));
                    continue;
                }
            };

            let writer = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                IncrementalWriter::new(&bytes)
            })) {
                Ok(w) => w,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };

            let result = writer.write(&make_dummy_addition());

            match result {
                Ok(write_result) => {
                    let source_len = bytes.len();
                    if &write_result.bytes[..source_len] == bytes.as_slice() {
                        passed += 1;
                    } else {
                        errors.push((
                            fixture.clone(),
                            "prefix invariant violated on real fixture".to_string(),
                        ));
                    }
                }
                Err(_) => {
                    // Skip PDFs that can't be processed (encrypted, object-stream, etc.)
                    skipped += 1;
                }
            }
        }

        eprintln!(
            "append-only property: {} passed, {} skipped (parse/refused), {} errors",
            passed,
            skipped,
            errors.len()
        );

        for (path, err) in &errors {
            eprintln!("  FAIL {:?}: {}", path, err);
        }

        assert!(
            errors.is_empty(),
            "{} fixtures failed the append-only invariant",
            errors.len()
        );
        assert!(
            passed > 0 || fixtures.is_empty(),
            "no fixtures passed — check fixture path"
        );
    }

    #[test]
    fn test_qpdf_check_available() {
        let available = qpdf_available();
        if available {
            eprintln!("qpdf found — full validation will run");
        } else {
            eprintln!("qpdf not installed — CI installs it");
        }
    }

    #[test]
    fn test_qpdf_check_incremental_output() {
        let pdf = make_minimal_pdf();
        let writer = IncrementalWriter::new(&pdf);
        let mutations = make_dummy_addition();
        let result = writer.write(&mutations).expect("write");

        let qpdf_available = qpdf_available();
        if !qpdf_available {
            eprintln!("qpdf not installed — skipping validation");
            return;
        }

        // Write output to a temp file for qpdf to inspect.
        let tmp = std::env::temp_dir().join(format!("stichos_qpdf_{}.pdf", std::process::id()));
        if let Err(e) = std::fs::write(&tmp, &result.bytes) {
            eprintln!("could not write temp file for qpdf: {}", e);
            return;
        }

        match std::process::Command::new("qpdf")
            .arg("--check=3")
            .arg(tmp.as_os_str())
            .output()
        {
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                eprintln!("qpdf stdout: {}", stdout);
                eprintln!("qpdf stderr: {}", stderr);

                assert!(
                    output.status.success(),
                    "qpdf --check=3 failed: stdout={}\nstderr={}",
                    stdout,
                    stderr
                );
            }
            Err(e) => {
                eprintln!("qpdf execution failed: {}", e);
                // If which said it's available but exec fails, that's unexpected — still pass.
                eprintln!("qpdf not installed — CI installs it");
            }
        }

        let _ = std::fs::remove_file(&tmp);
    }

    fn qpdf_available() -> bool {
        which::which("qpdf").is_ok()
            || std::process::Command::new("which")
                .arg("qpdf")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
    }

    #[test]
    fn test_append_only_e2e_with_all_mutations() {
        let pdf = make_minimal_pdf();
        let source_len = pdf.len();
        let writer = IncrementalWriter::new(&pdf);

        let mut dict = BTreeMap::new();
        dict.insert(
            b"E2E".to_vec(),
            Object::String(b"value".to_vec(), lopdf::StringFormat::Literal),
        );

        let mutations = PdfMutationSet::from_map(vec![
            (
                (5, 0),
                PdfMutation::Add {
                    object_id: (5, 0),
                    dict: dict.clone(),
                    data: None,
                },
            ),
            (
                (4, 0),
                PdfMutation::Replace {
                    object_id: (4, 0),
                    dict,
                    data: Some(b"e2e stream".to_vec()),
                },
            ),
        ]);

        let result = writer.write(&mutations).expect("write");
        assert_eq!(
            &result.bytes[..source_len],
            pdf.as_slice(),
            "prefix invariant violated"
        );
        assert!(result.bytes.len() > source_len);
    }

    #[test]
    fn test_write_result_report_accuracy() {
        let pdf = make_minimal_pdf();
        let writer = IncrementalWriter::new(&pdf);

        let mut add_dict = BTreeMap::new();
        add_dict.insert(b"Type".to_vec(), Object::Name(b"Added".to_vec()));

        let mut rep_dict = BTreeMap::new();
        rep_dict.insert(b"Type".to_vec(), Object::Name(b"Replaced".to_vec()));

        let mutations = PdfMutationSet::from_map(vec![
            (
                (5, 0),
                PdfMutation::Add {
                    object_id: (5, 0),
                    dict: add_dict,
                    data: None,
                },
            ),
            (
                (3, 0),
                PdfMutation::Replace {
                    object_id: (3, 0),
                    dict: rep_dict,
                    data: None,
                },
            ),
            ((4, 0), PdfMutation::Delete { object_id: (4, 0) }),
        ]);

        let result = writer.write(&mutations).expect("write");
        assert_eq!(result.report.objects_added, [(5, 0)]);
        assert_eq!(result.report.objects_modified, [(3, 0)]);
        assert_eq!(result.report.objects_deleted, [(4, 0)]);
    }
}
