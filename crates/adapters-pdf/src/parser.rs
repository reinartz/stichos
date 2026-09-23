// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::needless_else)]
use gp_core::diagnostics::*;
use gp_core::ir::{
    ElementId, PdfAnnotation, PdfDocument, PdfMetadata, PdfOutlineItem, PdfPage, TaggedNode,
};
use std::collections::{BTreeMap, HashMap};

pub type PdfName = String;

#[derive(Debug, Clone, PartialEq)]
pub enum PdfObject {
    Null,
    Boolean(bool),
    Integer(i64),
    Real(f64),
    Name(String),
    String(Vec<u8>),
    Array(Vec<PdfObject>),
    Dict(BTreeMap<PdfName, PdfObject>),
    Stream {
        dict: BTreeMap<PdfName, PdfObject>,
        data: Vec<u8>,
    },
    Reference {
        object_number: u32,
        generation: u32,
    },
}

#[allow(dead_code)]
impl PdfObject {
    pub fn dict_get(&self, key: &str) -> Option<&PdfObject> {
        if let PdfObject::Dict(m) = self {
            return m.get(key);
        }
        None
    }
    pub fn as_dict(&self) -> Option<&BTreeMap<PdfName, PdfObject>> {
        if let PdfObject::Dict(m) = self {
            return Some(m);
        }
        None
    }
    pub fn as_array(&self) -> Option<&Vec<PdfObject>> {
        if let PdfObject::Array(a) = self {
            return Some(a);
        }
        None
    }
    pub fn ref_obj_num(&self) -> Option<u32> {
        if let PdfObject::Reference { object_number, .. } = self {
            return Some(*object_number);
        }
        None
    }
    pub fn as_stream_dict(&self) -> Option<&BTreeMap<PdfName, PdfObject>> {
        if let PdfObject::Stream { dict, .. } = self {
            return Some(dict);
        }
        None
    }
}

#[derive(Debug, Clone)]
pub struct PdfDocumentData {
    pub version: String,
    pub objects: HashMap<u32, PdfObject>,
    pub pages: Vec<PdfPage>,
    pub metadata: Option<PdfMetadata>,
    pub has_encryption: bool,
    pub diagnostics: Vec<Diagnostic>,
    /// C13: detected signature info (populated during parse, not re-detected)
    pub signatures: Vec<gp_core::ir::SignatureInfo>,
}

impl PdfDocumentData {
    fn to_pdf_document(&self) -> PdfDocument {
        let (annotations, outlines, tagged) = self.extract_all();
        PdfDocument {
            id: ElementId::content_anchored(
                "pdf",
                &["pdf:0"],
                &format!("{}-{}-pages", self.version, self.pages.len()),
            ),
            pages: self.pages.clone(),
            metadata: self.metadata.clone(),
            annotations,
            outlines,
            form_fields: vec![],
            tagged_structure: tagged,
            signatures: self.signatures.clone(),
        }
    }

    fn extract_all(&self) -> (Vec<PdfAnnotation>, Vec<PdfOutlineItem>, Vec<TaggedNode>) {
        let annotations = extract_annotations_from_doc(self);
        let outlines = extract_outline_from_doc(self);
        let tagged = extract_tagged_from_doc(self);
        (annotations, outlines, tagged)
    }
}

pub fn parse_pdf(data: &[u8]) -> Result<gp_core::ir::PdfDocument, Vec<Diagnostic>> {
    let doc = parse_pdf_data(data)?;
    Ok(doc.to_pdf_document())
}

/// Count the number of pages in a PDF by parsing it with the built-in parser.
///
/// Uses the existing `parse_pdf_data` and returns the length of the page tree.
pub fn page_count(data: &[u8]) -> Result<usize, Vec<Diagnostic>> {
    let doc = parse_pdf_data(data)?;
    Ok(doc.pages.len())
}

pub fn parse_pdf_data(data: &[u8]) -> Result<PdfDocumentData, Vec<Diagnostic>> {
    // C13: detect signatures in raw bytes early (P5-11)
    let signatures = super::incremental::detect_signatures_raw(data);

    // Safety: timeout to prevent indefinite hangs on malformed PDFs
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(5);
    let check_timeout = || {
        if start.elapsed() > timeout {
            Some(Diagnostic::new(
                "PDF-E-005",
                Severity::Warning,
                "PDF parsing timed out after 30 seconds".to_string(),
            ))
        } else {
            None
        }
    };
    let mut diagnostics = Vec::new();
    let version = match parse_header(data) {
        Ok(v) => v,
        Err(_e) => {
            diagnostics.push(_e);
            return Err(diagnostics);
        }
    };
    let mut objects: HashMap<u32, PdfObject> = HashMap::new();

    let obj_results = scan_indirect_objects(data);
    if let Some(diag) = check_timeout() {
        diagnostics.push(diag);
        return Err(diagnostics);
    }
    for res in obj_results {
        match res {
            Ok((num, _gen, obj)) => {
                if let PdfObject::Stream { dict, data: sd } = &obj {
                    let decoded = try_decompress(dict, sd);
                    objects.insert(
                        num,
                        PdfObject::Stream {
                            dict: dict.clone(),
                            data: decoded,
                        },
                    );
                } else {
                    objects.insert(num, obj);
                }
            }
            Err(d) => diagnostics.push(d),
        }
    }

    let xref_offsets = find_xref_offsets(data);
    if let Some(diag) = check_timeout() {
        diagnostics.push(diag);
        return Err(diagnostics);
    }
    let mut xref_iter_count = 0u32;
    for offset in &xref_offsets {
        let entries_result = parse_xref_entries(data, *offset);
        if let Ok(entries) = &entries_result {
            // Cap xref resolution entries to avoid pathological slowness on
            // PDFs with tens of thousands of xref entries (e.g. documents
            // with many object streams).  The byte-scanner + page-tree walk
            // already gives us everything we need for inspection.
            const MAX_XREF_ENTRIES: u32 = 100;
            for &(obj_num, byte_off) in entries.iter() {
                if xref_iter_count >= MAX_XREF_ENTRIES {
                    break;
                }
                if let Some(diag) = check_timeout() {
                    diagnostics.push(diag);
                    return Err(diagnostics);
                }
                xref_iter_count += 1;
                if xref_iter_count > 200_000 {
                    break;
                }
                // Bound total objects resolved from xref tables so adversarial
                // inputs cannot cause unbounded work; must be >= the largest
                // legitimate xref table (LO PDFs routinely exceed 500 objects).
                if objects.len() > 200_000 {
                    break;
                }
                if let Some(diag) = check_timeout() {
                    diagnostics.push(diag);
                    return Err(diagnostics);
                }
                // The xref table names the byte offset of each object's `N M obj`
                // header. Parse from there so objects the byte scanner could not
                // reach (long dicts, streams) resolve correctly; this also
                // replaces garbage entries produced by scanning binary streams
                // that merely resemble object syntax.
                if byte_off > 0 {
                    if let Ok(mut o) = parse_indirect_object_at(data, byte_off) {
                        if let PdfObject::Stream { dict, data: sd } = &o {
                            let decoded = try_decompress(dict, sd);
                            o = PdfObject::Stream {
                                dict: dict.clone(),
                                data: decoded,
                            };
                        }
                        objects.insert(obj_num, o);
                    }
                }
            }
        }
    }

    let trailer = match find_trailer(data, &xref_offsets) {
        Ok(t) => t,
        Err(d) => {
            diagnostics.push(d);
            return Err(diagnostics);
        }
    };

    // Extract what we need from trailer before it gets moved to walk_page_tree
    let (trailer_has_encrypt, trailer_root_num) = match trailer {
        Some(ref t) => (
            t.get("Encrypt").is_some(),
            t.get("Root").and_then(|r| r.ref_obj_num()),
        ),
        None => (false, None),
    };
    let has_encryption = {
        let mut enc = trailer_has_encrypt;
        if !enc {
            if let Some(rn) = trailer_root_num {
                if let Some(ro) = objects.get(&rn) {
                    if let Some(rd) = ro.as_stream_dict().or_else(|| ro.as_dict()) {
                        if rd.get("Encrypt").is_some() {
                            enc = true;
                        }
                    }
                }
            }
        }
        enc
    };
    if has_encryption {
        diagnostics.push(Diagnostic::new(
            "PDF-E-003",
            Severity::Warning,
            "Encryption detected - content not decrypted".to_string(),
        ));
    }

    if let Some(diag) = check_timeout() {
        diagnostics.push(diag);
        return Err(diagnostics);
    }
    let (pages, metadata) = walk_page_tree(&objects, trailer.as_ref());
    Ok(PdfDocumentData {
        version,
        objects,
        pages,
        metadata,
        has_encryption,
        diagnostics,
        signatures,
    })
}

fn parse_header(data: &[u8]) -> Result<String, Diagnostic> {
    let start = match data.windows(5).position(|w| w == b"%PDF-") {
        Some(p) => p,
        None => {
            return Err(Diagnostic::new(
                "PDF-E-001",
                Severity::Error,
                "Not a PDF: missing %PDF- header".to_string(),
            ))
        }
    };
    let after = &data[start + 5..];
    let end = after
        .iter()
        .position(|&b| b == b'\n' || b == b'\r' || b == b' ' || b == b'%')
        .unwrap_or(after.len());
    let limit = end.min(4);
    let version = std::str::from_utf8(&after[..limit])
        .unwrap_or("")
        .trim()
        .to_string();
    if version.is_empty() {
        return Err(Diagnostic::new(
            "PDF-E-001",
            Severity::Error,
            "Empty PDF version in header".to_string(),
        ));
    }
    Ok(format!("%PDF-{}", version))
}

fn find_xref_offsets(data: &[u8]) -> Vec<usize> {
    let mut offsets = Vec::new();
    let mut pos = 0;
    let mut iterations = 0u64;
    const MAX_XREF_ITERATIONS: u64 = 10_000_000;
    while pos < data.len() {
        iterations += 1;
        if iterations > MAX_XREF_ITERATIONS {
            break;
        }
        if let Some(p) = data[pos..].windows(4).position(|w| w == b"xref") {
            let abs = pos + p;
            if abs == 0 || (abs > 0 && (data[abs - 1] == b'\n' || data[abs - 1] == b'\r')) {
                offsets.push(abs);
            }
            pos = abs + 4;
        } else {
            break;
        }
    }
    offsets
}

fn parse_xref_entries(data: &[u8], offset: usize) -> Result<Vec<(u32, usize)>, Diagnostic> {
    let mut entries = Vec::new();
    let mut pos = offset + 4;
    skip_ws(data, &mut pos);
    let mut iterations = 0u64;
    const MAX_XREF_ENTRY_ITERATIONS: u64 = 10_000_000;
    loop {
        iterations += 1;
        if iterations > MAX_XREF_ENTRY_ITERATIONS {
            break;
        }
        if pos >= data.len() || data[pos..].starts_with(b"trailer") {
            break;
        }
        let start_num = match read_int(data, &mut pos) {
            Ok(n) => n as u32,
            Err(_) => break,
        };
        skip_ws(data, &mut pos);
        let count = match read_int(data, &mut pos) {
            Ok(n) => n as usize,
            Err(_) => break,
        };
        skip_ws(data, &mut pos);
        if count > 100_000 {
            break;
        }
        for i in 0..count {
            if pos + 20 > data.len() {
                break;
            }
            let entry = std::str::from_utf8(&data[pos..pos + 17]).unwrap_or("");
            if let Ok(byte_off) = entry[0..10].trim().parse::<u32>() {
                if byte_off > 0 {
                    entries.push((start_num + i as u32, byte_off as usize));
                }
            }
            pos += 20;
        }
    }
    Ok(entries)
}

fn find_trailer(
    data: &[u8],
    _xref: &[usize],
) -> Result<Option<BTreeMap<PdfName, PdfObject>>, Diagnostic> {
    let mut pos = 0;
    let mut iterations = 0u64;
    const MAX_TRAILER_ITERATIONS: u64 = 10_000_000;
    while let Some(p) = data[pos..].windows(7).position(|w| w == b"trailer") {
        iterations += 1;
        if iterations > MAX_TRAILER_ITERATIONS {
            break;
        }
        let mut after = pos + p + 7;
        skip_ws(data, &mut after);
        if after >= data.len() {
            break;
        }
        if let Ok((PdfObject::Dict(d), _)) = parse_obj(data, &mut after) {
            return Ok(Some(d));
        }
        // Advance past the failed parse to avoid infinite loop on malformed PDFs
        if after <= pos + p + 7 {
            after = pos + p + 8;
        }
        pos = after;
        if pos >= data.len() {
            break;
        }
    }
    Err(Diagnostic::new(
        "PDF-E-001",
        Severity::Error,
        "No trailer dictionary found".to_string(),
    ))
}

fn scan_indirect_objects(data: &[u8]) -> Vec<Result<(u32, u32, PdfObject), Diagnostic>> {
    // Safety: prevent parsing of very large files that could cause OOM or long scans
    if data.len() > 50 * 1024 * 1024 {
        return vec![Err(Diagnostic::new(
            "PDF-E-004",
            Severity::Warning,
            "File too large for scanning (max 50MB)".to_string(),
        ))];
    }

    let mut results = Vec::new();
    let mut pos = 0;
    let mut iteration_count = 0u32;
    let max_iterations = 100_000u32;
    while pos < data.len() {
        iteration_count += 1;
        if iteration_count > max_iterations {
            break;
        }
        if let Some(end_pos) = data[pos..].windows(6).position(|w| w == b"endobj") {
            let abs_end = pos + end_pos;
            let search_start = abs_end.saturating_sub(100).min(abs_end);
            let mut found = false;
            // Search backwards from abs_end for " N M obj" followed by whitespace.
            // "obj" must be at least 3 bytes before "endobj" and at least 5 bytes
            // into the data to have room for "N M obj" prefix.
            let upper = abs_end.saturating_sub(4);
            let lower = search_start.max(3);
            // Only search if there's a valid range
            if lower <= upper {
                for obj_abs in (lower..=upper).rev() {
                    // Look for "obj" preceded by space and followed by whitespace
                    if &data[obj_abs..obj_abs + 3] != b"obj" {
                        continue;
                    }
                    if obj_abs > 0 && data[obj_abs - 1] != b' ' {
                        continue;
                    }
                    let after = obj_abs + 3;
                    if after >= abs_end {
                        continue;
                    }
                    let ch = data[after];
                    if !matches!(ch, b' ' | b'\t' | b'\n' | b'\r') {
                        continue;
                    }
                    // Found "obj" at obj_abs. Extract "N M" from obj_abs-5..obj_abs
                    if obj_abs < 5 {
                        continue;
                    }
                    let num_str = std::str::from_utf8(&data[obj_abs - 5..obj_abs]).unwrap_or("");
                    let parts: Vec<&str> = num_str.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let (Ok(num), Ok(gen)) =
                            (parts[0].parse::<u32>(), parts[1].parse::<u32>())
                        {
                            let obj_data = &data[after..abs_end];
                            match parse_object_payload(obj_data) {
                                Ok(o) => results.push(Ok((num, gen, o))),
                                Err(e) => results.push(Err(e)),
                            }
                            pos = abs_end + 1;
                            found = true;
                            break;
                        }
                    }
                }
            }
            if !found {
                pos = abs_end + 1;
            }
            continue;
        }
        // No more "endobj" matches anywhere in the remainder of the file,
        // so continuing the byte-by-byte fallback would re-scan the tail for
        // each byte (O(n^2)). Stop here.
        break;
    }
    results
}

fn find_stream_kw(data: &[u8]) -> Option<usize> {
    let mut pos = 0;
    let limit = (data.len().saturating_sub(6)).min(8192); // don't scan more than 8KB
    while pos < limit {
        if data[pos..].starts_with(b"stream") && pos > 0 {
            let prev = data[pos - 1];
            if matches!(prev, b'\n' | b'\r' | b' ' | b'\t') {
                let after = pos + 6;
                if after >= data.len() || matches!(data[after], b'\n' | b'\r' | b' ') {
                    return Some(pos);
                }
            }
        }
        pos += 1;
    }
    None
}

/// Parse the body of an indirect object: the bytes that follow the `N M obj`
/// header and precede `endobj`. Streams are recognized when a `stream` keyword
/// follows the dict; the raw data between the keyword's end-of-line and
/// `endstream` is retained.
fn parse_object_payload(obj_data: &[u8]) -> Result<PdfObject, Diagnostic> {
    // Safety: hard limit to prevent infinite loops inside stream parsing
    let scan_limit = obj_data.len().min(1024 * 1024); // 1MB max
    let obj_data = &obj_data[..scan_limit];
    if let Some(sk) = find_stream_kw(obj_data) {
        let mut dp = 0;
        // Safety: limit dict parsing to before the stream keyword
        let dict_max = sk.min(obj_data.len());
        match parse_dict(obj_data, &mut dp, Some(dict_max)) {
            Ok((dict, _)) => {
                let mut sd = sk + 6;
                while sd < obj_data.len() {
                    if sd + 1 < obj_data.len() && obj_data[sd] == b'\r' && obj_data[sd + 1] == b'\n'
                    {
                        sd += 2;
                        break;
                    }
                    if obj_data[sd] == b'\n' {
                        sd += 1;
                        break;
                    }
                    sd += 1;
                    if sd > sk + 64 {
                        break;
                    }
                }
                // Find "endstream" keyword to end the stream data
                let endstream_pos = obj_data[sd..].windows(9).position(|w| w == b"endstream");
                let sd_end = match endstream_pos {
                    Some(p) => sd + p,
                    None => obj_data.len(),
                };
                let sd_data = &obj_data[sd..sd_end];
                Ok(PdfObject::Stream {
                    dict,
                    data: sd_data.to_vec(),
                })
            }
            Err(_e) => parse_object_payload_non_stream(obj_data),
        }
    } else {
        parse_object_payload_non_stream(obj_data)
    }
}

/// Parse a non-stream (or unparseable-as-stream) object body.
fn parse_object_payload_non_stream(obj_data: &[u8]) -> Result<PdfObject, Diagnostic> {
    let mut pp = 0;
    parse_obj(obj_data, &mut pp).map(|(o, _)| o)
}

/// Parse the indirect object at a known byte offset (from the xref table).
///
/// Unlike `parse_obj` at the raw offset, this skips the `N M obj` header so the
/// object's actual payload is parsed instead of a bare `Integer(N)`. Objects the
/// byte scanner cannot reach — long dictionaries whose `obj` header is far from
/// their `endobj`, or streams — are resolved correctly from here.
fn parse_indirect_object_at(data: &[u8], offset: usize) -> Result<PdfObject, Diagnostic> {
    // Safety: bound each xref-object resolution to 10ms to prevent
    // pathological slowness on PDFs with malformed or extremely large
    // objects.  If an object cannot be parsed quickly we skip it and
    // rely on the byte-scanner results + page-tree walk for the
    // information we need.
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_millis(10);
    let check = || start.elapsed() > timeout;

    let mut pos = offset;
    skip_ws(data, &mut pos);
    if pos >= data.len() {
        return Err(Diagnostic::new(
            "PDF-E-001",
            Severity::Error,
            "xref offset past end of file".to_string(),
        ));
    }
    read_num_str(data, &mut pos);
    skip_ws(data, &mut pos);
    read_num_str(data, &mut pos);
    skip_ws(data, &mut pos);
    if pos >= data.len() || !data[pos..].starts_with(b"obj") {
        return Err(Diagnostic::new(
            "PDF-E-001",
            Severity::Error,
            "xref offset does not point at an indirect object header".to_string(),
        ));
    }
    if check() {
        return Err(Diagnostic::new(
            "PDF-E-006",
            Severity::Warning,
            format!("xref object at {} timed out", offset),
        ));
    }
    let body_start = pos + 3;
    // Quick pre-scan for "/Type /ObjStm" to skip object streams early.
    let scan_limit = (body_start + 2048).min(data.len());
    let body_snippet = &data[body_start..scan_limit];
    if body_snippet.len() > 20
        && body_snippet
            .windows(21)
            .any(|w| w == b"/Type /ObjStm" || w == b"/Type/ObjStm")
    {
        return Err(Diagnostic::new(
            "PDF-I-003",
            Severity::Info,
            format!("skipping object stream at offset {}", offset),
        ));
    }
    let endobj_rel = data[body_start..].windows(6).position(|w| w == b"endobj");
    let body_end = match endobj_rel {
        Some(p) => body_start + p,
        None => data.len(),
    };
    if check() {
        return Err(Diagnostic::new(
            "PDF-E-006",
            Severity::Warning,
            format!("xref object at {} timed out finding endobj", offset),
        ));
    }
    parse_object_payload(&data[body_start..body_end])
}

#[allow(dead_code)]
fn parse_object_at(data: &[u8], offset: usize) -> Result<PdfObject, Diagnostic> {
    let mut pos = offset;
    skip_ws(data, &mut pos);
    parse_obj(data, &mut pos).map(|(o, _)| o)
}

fn parse_obj(data: &[u8], pos: &mut usize) -> Result<(PdfObject, usize), Diagnostic> {
    skip_ws(data, pos);
    if *pos >= data.len() {
        return Err(Diagnostic::new(
            "PDF-E-001",
            Severity::Error,
            "Unexpected end of data".to_string(),
        ));
    }
    match data[*pos] {
        b'/' => parse_name(data, pos),
        b'(' => parse_str(data, pos),
        b'<' => {
            if *pos + 1 < data.len() && data[*pos + 1] == b'<' {
                *pos += 2;
                parse_dict(data, pos, None).map(|(d, p)| (PdfObject::Dict(d), p))
            } else {
                parse_hex_str(data, pos)
            }
        }
        b'[' => parse_arr(data, pos),
        b'n' if data[*pos..].starts_with(b"null") => {
            *pos += 4;
            Ok((PdfObject::Null, *pos))
        }
        b't' if data[*pos..].starts_with(b"true") => {
            *pos += 4;
            Ok((PdfObject::Boolean(true), *pos))
        }
        b'f' if data[*pos..].starts_with(b"false") => {
            *pos += 5;
            Ok((PdfObject::Boolean(false), *pos))
        }
        _ => parse_num_or_ref(data, pos),
    }
}

fn parse_num_or_ref(data: &[u8], pos: &mut usize) -> Result<(PdfObject, usize), Diagnostic> {
    let start = *pos;
    let (ns, c1) = read_num_str(data, pos);
    skip_ws(data, pos);
    if ns.is_empty() {
        return Err(Diagnostic::new(
            "PDF-E-001",
            Severity::Error,
            "Expected number".to_string(),
        ));
    }
    let mut ps = *pos;
    skip_ws(data, &mut ps);
    let (ns2, _) = read_num_str(data, &mut ps);
    skip_ws(data, &mut ps);
    let rp = ps;
    if !ns2.is_empty() && data.get(rp).map(|&b| b == b'R').unwrap_or(false) {
        *pos = rp + 1;
        if let (Ok(n1), Ok(n2)) = (
            ns.split_whitespace().next().unwrap_or("").parse::<u32>(),
            ns2.split_whitespace().next().unwrap_or("").parse::<u32>(),
        ) {
            return Ok((
                PdfObject::Reference {
                    object_number: n1,
                    generation: n2,
                },
                *pos,
            ));
        }
    }
    *pos = start + c1;
    let t = ns.split_whitespace().next().unwrap_or("");
    if let Ok(i) = t.parse::<i64>() {
        Ok((PdfObject::Integer(i), *pos))
    } else if let Ok(f) = t.parse::<f64>() {
        Ok((PdfObject::Real(f), *pos))
    } else {
        Err(Diagnostic::new(
            "PDF-E-001",
            Severity::Error,
            format!("Bad number: {}", t),
        ))
    }
}

fn read_num_str(data: &[u8], pos: &mut usize) -> (String, usize) {
    let start = *pos;
    while *pos < data.len() {
        match data[*pos] {
            b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E' => *pos += 1,
            _ => break,
        }
    }
    (
        std::str::from_utf8(&data[start..*pos])
            .unwrap_or("")
            .to_string(),
        *pos - start,
    )
}

fn parse_name(data: &[u8], pos: &mut usize) -> Result<(PdfObject, usize), Diagnostic> {
    if data[*pos] != b'/' {
        return Err(Diagnostic::new(
            "PDF-E-001",
            Severity::Error,
            "Expected '/'".to_string(),
        ));
    }
    *pos += 1;
    let start = *pos;
    while *pos < data.len() {
        match data[*pos] {
            b'#' => {
                *pos += 3;
            }
            b' ' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'\n' | b'\r' => {
                break
            }
            _ => {
                *pos += 1;
            }
        }
    }
    let raw = std::str::from_utf8(&data[start..*pos]).unwrap_or("");
    Ok((PdfObject::Name(raw.replace('#', "")), *pos))
}

fn parse_str(data: &[u8], pos: &mut usize) -> Result<(PdfObject, usize), Diagnostic> {
    if data[*pos] != b'(' {
        return Err(Diagnostic::new(
            "PDF-E-001",
            Severity::Error,
            "Expected '('".to_string(),
        ));
    }
    *pos += 1;
    let start = *pos;
    let mut esc = false;
    let mut depth = 0;
    while *pos < data.len() {
        if esc {
            esc = false;
            *pos += 1;
            continue;
        }
        match data[*pos] {
            b'\\' => esc = true,
            b'(' => depth += 1,
            b')' if depth == 0 => {
                *pos += 1;
                return Ok((PdfObject::String(data[start..*pos - 1].to_vec()), *pos));
            }
            b')' => depth -= 1,
            _ => {
                *pos += 1;
            }
        }
    }
    Err(Diagnostic::new(
        "PDF-E-001",
        Severity::Error,
        "Unterminated string".to_string(),
    ))
}

fn parse_hex_str(data: &[u8], pos: &mut usize) -> Result<(PdfObject, usize), Diagnostic> {
    if data[*pos] != b'<' {
        return Err(Diagnostic::new(
            "PDF-E-001",
            Severity::Error,
            "Expected '<'".to_string(),
        ));
    }
    *pos += 1;
    let start = *pos;
    while *pos < data.len() && data[*pos] != b'>' {
        *pos += 1;
    }
    if *pos < data.len() {
        *pos += 1;
    }
    let hex = std::str::from_utf8(&data[start..*pos - 1])
        .unwrap_or("")
        .replace(|c: char| c.is_whitespace(), "");
    let mut r = Vec::new();
    for i in (0..hex.len()).step_by(2) {
        if i + 1 < hex.len() {
            if let Ok(b) = u8::from_str_radix(&hex[i..i + 2], 16) {
                r.push(b);
            }
        }
    }
    Ok((PdfObject::String(r), *pos))
}

fn parse_arr(data: &[u8], pos: &mut usize) -> Result<(PdfObject, usize), Diagnostic> {
    if data[*pos] != b'[' {
        return Err(Diagnostic::new(
            "PDF-E-001",
            Severity::Error,
            "Expected '['".to_string(),
        ));
    }
    *pos += 1;
    let mut items = Vec::new();
    loop {
        skip_ws(data, pos);
        if *pos >= data.len() || data[*pos] == b']' {
            if *pos < data.len() {
                *pos += 1;
            }
            return Ok((PdfObject::Array(items), *pos));
        }
        let (o, np) = parse_obj(data, pos)?;
        items.push(o);
        *pos = np;
    }
}

fn parse_dict(
    data: &[u8],
    pos: &mut usize,
    max: Option<usize>,
) -> Result<(BTreeMap<PdfName, PdfObject>, usize), Diagnostic> {
    let mut map = BTreeMap::new();
    let mut iterations = 0usize;
    let max_iterations = max.unwrap_or(data.len());
    loop {
        if let Some(m) = max {
            if *pos >= m {
                break;
            }
        }
        if *pos >= data.len() {
            break;
        }
        // Safety: limit iterations to prevent infinite loops on malformed PDFs
        iterations += 1;
        if iterations > max_iterations {
            break;
        }
        skip_ws(data, pos);
        if *pos >= data.len() {
            break;
        }
        if *pos + 1 < data.len() && data[*pos] == b'>' && data[*pos + 1] == b'>' {
            *pos += 2;
            return Ok((map, *pos));
        }
        if data[*pos] != b'/' {
            *pos += 1;
            continue;
        }
        let (k, np) = parse_name(data, pos)?;
        *pos = np;
        skip_ws(data, pos);
        if *pos >= data.len() {
            break;
        }
        let (v, np) = parse_obj(data, pos)?;
        let key = if let PdfObject::Name(n) = k { n } else { break };
        map.insert(key, v);
        *pos = np;
    }
    Ok((map, *pos))
}

fn read_int(data: &[u8], pos: &mut usize) -> Result<i64, Diagnostic> {
    skip_ws(data, pos);
    let start = *pos;
    if *pos < data.len() && (data[*pos] == b'+' || data[*pos] == b'-') {
        *pos += 1;
    }
    let ds = *pos;
    while *pos < data.len() && data[*pos].is_ascii_digit() {
        *pos += 1;
    }
    if *pos == ds {
        return Err(Diagnostic::new(
            "PDF-E-001",
            Severity::Error,
            "Expected integer".to_string(),
        ));
    }
    let s = std::str::from_utf8(&data[start..*pos]).unwrap_or("").trim();
    s.parse::<i64>()
        .map_err(|_| Diagnostic::new("PDF-E-001", Severity::Error, format!("Bad int: {}", s)))
}

fn walk_page_tree(
    objects: &HashMap<u32, PdfObject>,
    trailer: Option<&BTreeMap<PdfName, PdfObject>>,
) -> (Vec<PdfPage>, Option<PdfMetadata>) {
    let mut pages = Vec::new();
    let mut metadata = None;

    if let Some(t) = trailer {
        if let Some(root) = t.get("Root").and_then(|r| r.ref_obj_num()) {
            if let Some(root_obj) = objects.get(&root) {
                if let Some(rd) = root_obj.as_stream_dict().or_else(|| root_obj.as_dict()) {
                    if let Some(pages_ref) = rd.get("Pages").and_then(|p| p.ref_obj_num()) {
                        walk_pages(objects, pages_ref, 1, &mut pages);
                    } else {
                    }
                    // Check root object for /Info
                    if let Some(info_ref) = rd.get("Info").and_then(|i| i.ref_obj_num()) {
                        if let Some(info_obj) = objects.get(&info_ref) {
                            metadata = extract_metadata(info_obj);
                        }
                    }
                    // Also check trailer for /Info (some PDFs put it there)
                    if metadata.is_none() {
                        if let Some(t) = trailer {
                            if let Some(info_ref) = t.get("Info").and_then(|i| i.ref_obj_num()) {
                                if let Some(info_obj) = objects.get(&info_ref) {
                                    metadata = extract_metadata(info_obj);
                                }
                            }
                        }
                    }
                }
            } else {
            }
        } else {
        }
    } else {
    }
    (pages, metadata)
}
fn walk_pages(
    objects: &HashMap<u32, PdfObject>,
    obj_num: u32,
    page_num: usize,
    pages: &mut Vec<PdfPage>,
) {
    let obj = match objects.get(&obj_num) {
        Some(o) => o,
        None => return,
    };
    let dict = match obj {
        PdfObject::Stream { dict, .. } | PdfObject::Dict(dict) => dict,
        _ => return,
    };
    match dict.get("Type").and_then(|t| {
        if let PdfObject::Name(n) = t {
            Some(n.as_str())
        } else {
            None
        }
    }) {
        Some("Pages") => {
            if let Some(kids) = dict.get("Kids") {
                walk_kids(objects, kids, page_num, pages);
            }
        }
        Some("Page") => {
            pages.push(mk_page(obj_num, page_num, dict));
        }
        _ => {}
    }
}

fn walk_kids(
    objects: &HashMap<u32, PdfObject>,
    kids: &PdfObject,
    sp: usize,
    pages: &mut Vec<PdfPage>,
) {
    let kids_arr = match kids.as_array() {
        Some(a) => a,
        None => return,
    };
    let mut pn = sp;
    for kid in kids_arr {
        let kn = match kid.ref_obj_num() {
            Some(n) => n,
            None => continue,
        };
        let is_page = objects
            .get(&kn)
            .map(|o| {
                o.as_stream_dict().or_else(|| o.as_dict()).and_then(|d| {
                    d.get("Type").and_then(|t| {
                        if let PdfObject::Name(n) = t {
                            Some(n.as_str())
                        } else {
                            None
                        }
                    })
                }) == Some("Page")
            })
            .unwrap_or(false);
        if is_page {
            if let Some(obj) = objects.get(&kn) {
                if let Some(dict) = obj.as_stream_dict().or_else(|| obj.as_dict()) {
                    pages.push(mk_page(kn, pn, dict));
                }
            }
            pn += 1;
        } else {
            walk_pages(objects, kn, pn, pages);
        }
    }
}

fn mk_page(om: u32, pn: usize, dict: &BTreeMap<PdfName, PdfObject>) -> PdfPage {
    let mb = dict
        .get("MediaBox")
        .and_then(|m| m.as_array())
        .and_then(|a| {
            if a.len() >= 4 {
                Some([a2f(&a[0]), a2f(&a[1]), a2f(&a[2]), a2f(&a[3])])
            } else {
                None
            }
        });
    let (w, h) = match mb {
        Some([x0, y0, x1, y1]) => ((x1 - x0).abs(), (y1 - y0).abs()),
        _ => (612.0, 792.0),
    };
    PdfPage {
        id: ElementId::content_anchored(
            "pdf_page",
            &["page", &pn.to_string()],
            &format!(
                "{}:{}:{}:{}:{}",
                om,
                pn,
                w,
                h,
                dict_get_text(dict, "Title").unwrap_or_default()
            ),
        ),
        number: pn,
        width_pt: w,
        height_pt: h,
        text_with_bbox: vec![],
    }
}

fn extract_metadata(info: &PdfObject) -> Option<PdfMetadata> {
    let d = info.as_stream_dict().or_else(|| info.as_dict())?;
    Some(PdfMetadata {
        title: dict_get_text(d, "Title"),
        author: dict_get_text(d, "Author"),
        subject: dict_get_text(d, "Subject"),
        creator: None,
        description: None,
        modified: None,
        keywords: None,
    })
}

fn dict_get_text(d: &BTreeMap<PdfName, PdfObject>, key: &str) -> Option<String> {
    match d.get(key) {
        Some(PdfObject::String(s)) => std::str::from_utf8(s).ok().map(|s| s.to_string()),
        Some(PdfObject::Name(n)) => Some(n.clone()),
        _ => None,
    }
}

fn a2f(o: &PdfObject) -> f64 {
    match o {
        PdfObject::Integer(i) => *i as f64,
        PdfObject::Real(r) => *r,
        _ => 0.0,
    }
}

/// Convert a PDF object to f64 (for numeric values in arrays).
pub(crate) fn pdf_num(o: &PdfObject) -> f64 {
    match o {
        PdfObject::Integer(i) => *i as f64,
        PdfObject::Real(r) => *r,
        _ => 0.0,
    }
}

/// Resolve a PDF reference to the actual object in the document.
pub(crate) fn resolve_reference<'a>(
    obj: &'a PdfObject,
    objects: &'a std::collections::HashMap<u32, PdfObject>,
) -> &'a PdfObject {
    if let PdfObject::Reference { object_number, .. } = obj {
        objects.get(object_number).unwrap_or(obj)
    } else {
        obj
    }
}

/// Find the root catalog object in the document.
pub(crate) fn find_root_object(doc: &PdfDocumentData) -> Option<&PdfObject> {
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

fn try_decompress(_d: &BTreeMap<PdfName, PdfObject>, data: &[u8]) -> Vec<u8> {
    data.to_vec()
}

fn skip_ws(data: &[u8], pos: &mut usize) {
    while *pos < data.len() {
        match data[*pos] {
            b' ' | b'\t' | b'\n' | b'\r' => *pos += 1,
            b'%' => {
                *pos += 1;
                while *pos < data.len() && data[*pos] != b'\n' && data[*pos] != b'\r' {
                    *pos += 1;
                }
            }
            _ => break,
        }
    }
}

pub struct PdfReader;
impl PdfReader {
    pub fn new() -> Self {
        Self
    }
    pub fn read(&self, data: &[u8]) -> Result<gp_core::ir::IRDocument, Vec<Diagnostic>> {
        // parse_pdf_data does all the heavy lifting (object scanning, xref resolution,
        // page tree walk). We call it once and derive both the PdfDocument and
        // PdfDocumentData from the same parsed result to avoid double-parsing.
        let doc_data = parse_pdf_data(data)?;
        let pd = doc_data.to_pdf_document();
        let id = ElementId::content_anchored(
            "pdf_document",
            &["document:0"],
            &format!("{}-{}-pages", pd.id.as_str(), pd.pages.len()),
        );

        // Extract word-level text with bounding boxes, grouped by page number.
        // This gives inspect_document real coordinates instead of empty text trees.
        let text_by_page: std::collections::HashMap<usize, Vec<gp_core::ir::TextWithBbox>> =
            crate::text::extract_text_grouped_by_page(&doc_data);

        Ok(gp_core::ir::IRDocument {
            ir_version: gp_core::ir::IR_VERSION.to_string(),
            root: gp_core::ir::TaggedNode {
                id: id.clone(),
                tag: "pdf".to_string(),
                alt_text: None,
                bbox: None,
                children: pd
                    .pages
                    .iter()
                    .map(|pg| gp_core::ir::TaggedNode {
                        id: pg.id.clone(),
                        tag: "page".to_string(),
                        alt_text: Some(format!(
                            "Page {}: {:.0}x{:.0}pt",
                            pg.number, pg.width_pt, pg.height_pt
                        )),
                        bbox: Some([0.0, 0.0, pg.width_pt, pg.height_pt]),
                        children: text_by_page
                            .get(&pg.number)
                            .into_iter()
                            .flatten()
                            .map(|t| gp_core::ir::TaggedNode {
                                id: t.id.clone(),
                                tag: "text".to_string(),
                                alt_text: Some(t.text.clone()),
                                bbox: Some(t.bbox),
                                children: vec![],
                            })
                            .collect(),
                    })
                    .collect(),
            },
        })
    }
}
impl Default for PdfReader {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// PDF annotation extraction
// ---------------------------------------------------------------------------

fn extract_annotations_from_doc(doc: &PdfDocumentData) -> Vec<PdfAnnotation> {
    let mut annotations = Vec::new();
    for page in &doc.pages {
        let page_num = page.number;
        for obj in doc.objects.values() {
            if let Some(dict) = obj.as_stream_dict().or_else(|| obj.as_dict()) {
                // Only process page objects that have MediaBox (leaf pages)
                if !dict.contains_key("MediaBox") {
                    continue;
                }
                if let Some(PdfObject::Name(name)) = dict.get("Type") {
                    if name != "Page" {
                        continue;
                    }
                } else {
                    continue;
                }
                if let Some(PdfObject::Array(annot_arr)) = dict.get("Annots") {
                    for annot_ref in annot_arr {
                        let annot_obj = resolve_reference(annot_ref, &doc.objects);
                        if let Some(a) = parse_single_annotation(annot_obj, page_num) {
                            annotations.push(a);
                        }
                    }
                }
            }
        }
    }
    annotations
}

fn parse_single_annotation(obj: &PdfObject, page_num: usize) -> Option<PdfAnnotation> {
    let dict = obj.as_stream_dict().or_else(|| obj.as_dict())?;
    let annotation_type = match dict.get("Subtype") {
        Some(PdfObject::Name(n)) => n.clone(),
        Some(_) => "Unknown".to_string(),
        None => "Unknown".to_string(),
    };
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
    let contents = match dict.get("Contents") {
        Some(PdfObject::String(s)) => std::str::from_utf8(s).ok().map(|s| s.to_string()),
        Some(PdfObject::Name(n)) => Some(n.clone()),
        _ => None,
    };
    let id = ElementId::content_anchored(
        "pdf_annotation",
        &["annotation", &annotation_type, &page_num.to_string()],
        &format!("page:{}:type:{}", page_num, annotation_type),
    );
    Some(PdfAnnotation {
        id,
        annotation_type,
        page_number: page_num,
        rect,
        contents,
        kind: None,
        author: None,
        date: None,
        quote: None,
        state: None,
        resolved: None,
    })
}

// ---------------------------------------------------------------------------
// PDF outline extraction
// ---------------------------------------------------------------------------

fn extract_outline_from_doc(doc: &PdfDocumentData) -> Vec<PdfOutlineItem> {
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
    let page_map: HashMap<u32, usize> = doc
        .pages
        .iter()
        .flat_map(|p| {
            // Find the object number for this page by matching MediaBox
            for (num, obj) in &doc.objects {
                if let Some(dict) = obj.as_stream_dict().or_else(|| obj.as_dict()) {
                    if let Some(PdfObject::Name(name)) = dict.get("Type") {
                        if name == "Page" {
                            if let Some(PdfObject::Array(mb)) = dict.get("MediaBox") {
                                if mb.len() >= 4
                                    && (pdf_num(&mb[0]) - 0.0).abs() < 1.0
                                    && (pdf_num(&mb[1]) - 0.0).abs() < 1.0
                                    && (pdf_num(&mb[2]) - p.width_pt).abs() < 0.1
                                    && (pdf_num(&mb[3]) - p.height_pt).abs() < 0.1
                                {
                                    return Some((*num, p.number));
                                }
                            }
                        }
                    }
                }
            }
            None
        })
        .collect();

    // Walk the outline tree from /First
    let mut result = Vec::new();
    if let Some(first) = outlines_dict.get("First") {
        walk_outline_node(first, &doc.objects, &page_map, &mut result);
    }
    result
}

fn walk_outline_node(
    node_obj: &PdfObject,
    objects: &HashMap<u32, PdfObject>,
    page_map: &HashMap<u32, usize>,
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
            if !parts.is_empty() {
                Some(parts.join(" "))
            } else {
                None
            }
        }
        _ => None,
    };

    // Get destination string
    let dest = match dict.get("Dest") {
        Some(PdfObject::String(s)) => Some(std::str::from_utf8(s).unwrap_or("").to_string()),
        Some(PdfObject::Name(n)) => Some(n.clone()),
        _ => None,
    };

    let id = ElementId::content_anchored(
        "pdf_outline",
        &["outline", &title.replace(' ', "_")],
        &format!("title:{}:page:{}", title, page_number),
    );

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

    // Walk siblings via /Next
    if let Some(next) = dict.get("Next") {
        walk_outline_node(next, objects, page_map, result);
    }
}

fn dest_page_number(
    dest: &PdfObject,
    objects: &HashMap<u32, PdfObject>,
    page_map: &HashMap<u32, usize>,
) -> usize {
    match dest {
        PdfObject::Array(arr) => {
            if arr.is_empty() {
                return 0;
            }
            let first = &arr[0];
            if let PdfObject::Reference { object_number, .. } = first {
                let target = *object_number;
                if let Some(&pn) = page_map.get(&target) {
                    return pn;
                }
                // Try to find by scanning page objects
                for (obj_num, obj) in objects {
                    if let Some(dict) = obj.as_stream_dict().or_else(|| obj.as_dict()) {
                        if let Some(PdfObject::Name(name)) = dict.get("Type") {
                            if name == "Page" && *obj_num == target {
                                if let Some(&pn) = page_map.get(obj_num) {
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

// ---------------------------------------------------------------------------
// Tagged structure extraction
// ---------------------------------------------------------------------------

fn extract_tagged_from_doc(doc: &PdfDocumentData) -> Vec<TaggedNode> {
    let root_obj = find_root_object(doc);
    let root_dict = match root_obj.and_then(|o| o.as_stream_dict().or_else(|| o.as_dict())) {
        Some(d) => d,
        None => return Vec::new(),
    };

    let struct_tree_root = match root_dict.get("StructTreeRoot") {
        Some(st) => st,
        None => return Vec::new(),
    };

    let st_obj = resolve_reference(struct_tree_root, &doc.objects);
    let st_dict = match st_obj.as_stream_dict().or_else(|| st_obj.as_dict()) {
        Some(d) => d,
        None => return Vec::new(),
    };

    let mut result = Vec::new();
    if let Some(k) = st_dict.get("K") {
        parse_struct_element(k, &doc.objects, &mut result);
    }
    result
}

fn parse_struct_element(
    obj: &PdfObject,
    objects: &HashMap<u32, PdfObject>,
    result: &mut Vec<TaggedNode>,
) {
    match obj {
        PdfObject::Name(tag_name) => {
            // A single structure type name
            let id = ElementId::content_anchored(
                "struct",
                &["struct", tag_name],
                &format!("tag:{}", tag_name),
            );
            result.push(TaggedNode {
                id,
                tag: tag_name.clone(),
                alt_text: None,
                bbox: None,
                children: Vec::new(),
            });
        }
        PdfObject::Array(arr) => {
            // Could be [ /P [children] ] or [ children... ] or a single ref
            if arr.is_empty() {
                return;
            }
            // Check if first element is a name (structure type)
            if let PdfObject::Name(ref tag_name) = arr[0] {
                // [ /TagName child1 child2 ... ]
                let mut children = Vec::new();
                for child in arr.iter().skip(1) {
                    parse_struct_element(child, objects, &mut children);
                }
                let id = ElementId::content_anchored(
                    "struct",
                    &["struct", tag_name],
                    &format!("tag:{}", tag_name),
                );
                // Extract alternative text if present
                let alt_text = extract_alt_text(arr, objects);
                result.push(TaggedNode {
                    id,
                    tag: tag_name.clone(),
                    alt_text,
                    bbox: None,
                    children,
                });
            } else {
                // Array of children without tag at top level
                for child in arr {
                    parse_struct_element(child, objects, result);
                }
            }
        }
        PdfObject::Reference { object_number, .. } => {
            // A reference to a structure element
            if let Some(dict_obj) = objects.get(object_number) {
                if let Some(dict) = dict_obj.as_stream_dict().or_else(|| dict_obj.as_dict()) {
                    if let Some(PdfObject::Name(tag_name)) = dict.get("S") {
                        // Structure element: /S = type, /K = children, optional /Alt
                        let mut children = Vec::new();
                        if let Some(k) = dict.get("K") {
                            parse_struct_element(k, objects, &mut children);
                        }
                        let alt_text = extract_alt_from_dict(dict, objects);
                        let id = ElementId::content_anchored(
                            "struct",
                            &["struct", tag_name],
                            &format!("obj:{}:tag:{}", object_number, tag_name),
                        );
                        result.push(TaggedNode {
                            id,
                            tag: tag_name.clone(),
                            alt_text,
                            bbox: None,
                            children,
                        });
                    }
                }
            }
        }
        PdfObject::Dict(dict) => {
            // Inline dictionary structure element
            if let Some(PdfObject::Name(tag_name)) = dict.get("S") {
                let mut children = Vec::new();
                if let Some(k) = dict.get("K") {
                    parse_struct_element(k, objects, &mut children);
                }
                let alt_text = extract_alt_from_dict(dict, objects);
                let id = ElementId::content_anchored(
                    "struct",
                    &["struct", tag_name],
                    &format!("tag:{}", tag_name),
                );
                result.push(TaggedNode {
                    id,
                    tag: tag_name.clone(),
                    alt_text,
                    bbox: None,
                    children,
                });
            }
        }
        _ => {}
    }
}

fn extract_alt_text(arr: &[PdfObject], objects: &HashMap<u32, PdfObject>) -> Option<String> {
    for child in arr.iter().skip(1) {
        if let PdfObject::Dict(dict) = child {
            if let Some(alt) = dict.get("Alt") {
                return match alt {
                    PdfObject::String(s) => std::str::from_utf8(s).ok().map(|s| s.to_string()),
                    PdfObject::Name(n) => Some(n.clone()),
                    _ => None,
                };
            }
            if let Some(at) = dict.get("ActualText") {
                return match at {
                    PdfObject::String(s) => std::str::from_utf8(s).ok().map(|s| s.to_string()),
                    PdfObject::Name(n) => Some(n.clone()),
                    _ => None,
                };
            }
        }
        // Check for /ActualText as a sibling reference
        if let PdfObject::Reference { object_number, .. } = child {
            if let Some(dict_obj) = objects.get(object_number) {
                if let Some(dict) = dict_obj.as_stream_dict().or_else(|| dict_obj.as_dict()) {
                    if let Some(PdfObject::Name(s)) = dict.get("S") {
                        if s == "ActualText" || s == "Alt" {
                            if let Some(PdfObject::String(k)) = dict.get("K") {
                                return std::str::from_utf8(k).ok().map(|s| s.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

fn extract_alt_from_dict(
    dict: &BTreeMap<String, PdfObject>,
    _objects: &HashMap<u32, PdfObject>,
) -> Option<String> {
    if let Some(alt) = dict.get("Alt") {
        return match alt {
            PdfObject::String(s) => std::str::from_utf8(s).ok().map(|s| s.to_string()),
            PdfObject::Name(n) => Some(n.clone()),
            _ => None,
        };
    }
    if let Some(at) = dict.get("ActualText") {
        return match at {
            PdfObject::String(s) => std::str::from_utf8(s).ok().map(|s| s.to_string()),
            PdfObject::Name(n) => Some(n.clone()),
            _ => None,
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_pdf() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        let o1 = b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n";
        let o2 = b"2 0 obj\n<< /Type /Pages /Kids [4 0 R 5 0 R] /Count 2 >>\nendobj\n";
        let o3 = b"3 0 obj\n<< /Length 44 >>\nendobj\n";
        let o4 = b"4 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 3 0 R >>\nendobj\n";
        let o5 = b"5 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 3 0 R >>\nendobj\n";
        let o6 =
            b"6 0 obj\n<< /Title (Test PDF) /Author (Test Author) /Subject (Testing) >>\nendobj\n";
        b.extend_from_slice(o1);
        b.extend_from_slice(o2);
        b.extend_from_slice(o3);
        b.extend_from_slice(o4);
        b.extend_from_slice(o5);
        b.extend_from_slice(o6);
        let s = [
            20,
            20 + o1.len(),
            20 + o1.len() + o2.len(),
            20 + o1.len() + o2.len() + o3.len(),
            20 + o1.len() + o2.len() + o3.len() + o4.len(),
            20 + o1.len() + o2.len() + o3.len() + o4.len() + o5.len(),
        ];
        b.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
        for off in &s {
            b.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
        }
        b.extend_from_slice(b"trailer\n<< /Size 7 /Root 1 0 R /Info 6 0 R >>\n");
        b.extend_from_slice(b"startxref\n");
        b.extend_from_slice(format!("{}", s[0]).as_bytes());
        b.extend_from_slice(b"\n%%EOF\n");
        b
    }

    fn nested_pdf() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        let objs: Vec<Vec<u8>> = vec![
            b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_vec(),
            b"2 0 obj\n<< /Type /Pages /Kids [3 0 R 6 0 R] /Count 3 >>\nendobj\n".to_vec(),
            b"3 0 obj\n<< /Type /Pages /Kids [4 0 R 5 0 R] /Count 2 >>\nendobj\n".to_vec(),
            b"4 0 obj\n<< /Type /Page /Parent 3 0 R /MediaBox [0 0 595 842] >>\nendobj\n".to_vec(),
            b"5 0 obj\n<< /Type /Page /Parent 3 0 R /MediaBox [0 0 612 792] >>\nendobj\n".to_vec(),
            b"6 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 1008] >>\nendobj\n".to_vec(),
        ];
        for o in &objs {
            b.extend_from_slice(o);
        }
        let s: Vec<usize> = {
            let mut v = vec![20];
            for i in 0..objs.len() {
                if i + 1 < objs.len() {
                    v.push(v.last().unwrap() + objs[i].len());
                }
            }
            v
        };
        b.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
        for off in &s {
            b.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
        }
        b.extend_from_slice(b"trailer\n<< /Size 7 /Root 1 0 R >>\n");
        b.extend_from_slice(b"startxref\n");
        b.extend_from_slice(format!("{}", s[0]).as_bytes());
        b.extend_from_slice(b"\n%%EOF\n");
        b
    }

    fn minimal_pdf() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        let o1 = b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n";
        let o2 = b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n";
        let o3 = b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n";
        b.extend_from_slice(o1);
        b.extend_from_slice(o2);
        b.extend_from_slice(o3);
        b.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
        b.extend_from_slice(format!("{:010} 00000 n \n", 20).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", 20 + o1.len()).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", 20 + o1.len() + o2.len()).as_bytes());
        b.extend_from_slice(b"trailer\n<< /Size 4 /Root 1 0 R >>\n");
        b.extend_from_slice(b"startxref\n20\n%%EOF\n");
        b
    }

    fn rotated_pdf() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        let o1 = b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n";
        let o2 = b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n";
        let o3 = b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Rotate 90 >>\nendobj\n";
        b.extend_from_slice(o1);
        b.extend_from_slice(o2);
        b.extend_from_slice(o3);
        b.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
        b.extend_from_slice(format!("{:010} 00000 n \n", 20).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", 20 + o1.len()).as_bytes());
        b.extend_from_slice(format!("{:010} 00000 n \n", 20 + o1.len() + o2.len()).as_bytes());
        b.extend_from_slice(b"trailer\n<< /Size 4 /Root 1 0 R >>\n");
        b.extend_from_slice(b"startxref\n20\n%%EOF\n");
        b
    }

    fn encrypted_pdf() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        let o1 = b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Encrypt 7 0 R >>\nendobj\n";
        let o2 = b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n";
        let o3 = b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n";
        let o7 = b"7 0 obj\n<< /Filter /Standard /V 2 /Length 40 >>\nendobj\n";
        b.extend_from_slice(o1);
        b.extend_from_slice(o2);
        b.extend_from_slice(o3);
        b.extend_from_slice(o7);
        b.extend_from_slice(b"xref\n0 8\n0000000000 65535 f \n");
        let off = [
            20,
            20 + o1.len(),
            20 + o1.len() + o2.len(),
            20 + o1.len() + o2.len() + o3.len(),
        ];
        for o in &off {
            b.extend_from_slice(format!("{:010} 00000 n \n", o).as_bytes());
        }
        b.extend_from_slice(
            format!(
                "{:010} 00000 n \n",
                off[0] + o1.len() + o2.len() + o3.len() + o7.len()
            )
            .as_bytes(),
        );
        b.extend_from_slice(b"trailer\n<< /Size 8 /Root 1 0 R >>\n");
        b.extend_from_slice(b"startxref\n20\n%%EOF\n");
        b
    }

    fn stream_pdf() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        b.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        b.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [5 0 R] /Count 1 >>\nendobj\n");
        b.extend_from_slice(b"3 0 obj\n<< /Length 20 /Filter /FlateDecode >>\nstream\nhello compressed data\nendstream\nendobj\n");
        b.extend_from_slice(
            b"4 0 obj\n<< /Length 14 >>\nstream\nhello uncompressed\nendstream\nendobj\n",
        );
        b.extend_from_slice(b"5 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n");
        let s = [20, 74, 157, 240, 357];
        b.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
        for o in &s {
            b.extend_from_slice(format!("{:010} 00000 n \n", o).as_bytes());
        }
        b.extend_from_slice(b"trailer\n<< /Size 6 /Root 1 0 R >>\n");
        b.extend_from_slice(b"startxref\n20\n%%EOF\n");
        b
    }

    #[test]
    fn test_parse_pdf_header() {
        let r = parse_pdf(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\ntrailer\n<< /Size 2 /Root 1 0 R >>\n");
        assert!(r.is_ok(), "valid header: {:?}", r.err());
    }

    #[test]
    fn test_parse_pdf_header_invalid() {
        let r = parse_pdf(b"%not-pdf\nSome content\n");
        assert!(r.is_err());
        let diags = r.unwrap_err();
        assert!(diags.iter().any(|d| d.code == "PDF-E-001"));
    }

    #[test]
    fn test_parse_pdf_header_versions() {
        for v in &["1.0", "1.1", "1.2", "1.3", "1.4", "1.5", "1.6", "1.7"] {
            let s = format!("%%PDF-{}\n", v);
            let data = s.as_bytes();
            let _ = parse_pdf(data);
        }
    }

    #[test]
    fn test_parse_pdf_objects() {
        let r = parse_pdf_data(&simple_pdf());
        assert!(r.is_ok(), "simple PDF: {:?}", r.err());
        let doc = r.unwrap();
        assert!(!doc.objects.is_empty(), "objects: {}", doc.objects.len());
    }

    #[test]
    fn test_parse_xref_table() {
        let r = parse_pdf_data(&simple_pdf());
        assert!(r.is_ok(), "xref: {:?}", r.err());
        let doc = r.unwrap();
        assert!(doc.objects.contains_key(&1), "obj1");
        assert!(doc.objects.contains_key(&2), "obj2");
    }

    #[test]
    fn test_parse_page_tree() {
        let r = parse_pdf_data(&simple_pdf());
        assert!(r.is_ok(), "pagetree: {:?}", r.err());
        let doc = r.unwrap();
        assert_eq!(doc.pages[0].number, 1);
        assert_eq!(doc.pages[1].number, 2);
        assert!((doc.pages[0].width_pt - 612.0).abs() < 0.01);
        assert!((doc.pages[0].height_pt - 792.0).abs() < 0.01);
        assert!((doc.pages[1].width_pt - 595.0).abs() < 0.01);
        assert!((doc.pages[1].height_pt - 842.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_page_tree_nested() {
        let r = parse_pdf_data(&nested_pdf());
        assert!(r.is_ok(), "nested: {:?}", r.err());
        let doc = r.unwrap();
        assert_eq!(doc.pages.len(), 3, "nested pages: {}", doc.pages.len());
    }

    #[test]
    fn test_parse_all_pdf_fixtures() {
        let d1 = parse_pdf_data(&simple_pdf());
        assert!(d1.is_ok(), "simple: {:?}", d1.err());
        let d2 = parse_pdf_data(&nested_pdf());
        assert!(d2.is_ok(), "nested: {:?}", d2.err());
        let d3 = parse_pdf_data(&minimal_pdf());
        assert!(d3.is_ok(), "minimal: {:?}", d3.err());
        let d4 = parse_pdf_data(&rotated_pdf());
        assert!(d4.is_ok(), "rotated: {:?}", d4.err());
    }

    #[test]
    fn test_parse_corrupt_pdf() {
        assert!(parse_pdf(b"not a pdf\n").is_err());
        assert!(parse_pdf(b"").is_err());
        assert!(parse_pdf(b"%PDF-1.4\n%EOF\n").is_err());
    }

    #[test]
    fn test_pdf_codec_integration() {
        let r = PdfReader::new().read(&simple_pdf());
        assert!(r.is_ok(), "codec: {:?}", r.err());
        let ir = r.unwrap();
        assert_eq!(ir.ir_version, "1.0.0");
        assert_eq!(ir.root.tag, "pdf");
        assert_eq!(
            ir.root.children.len(),
            2,
            "IR pages: {}",
            ir.root.children.len()
        );
        for c in &ir.root.children {
            assert_eq!(c.tag, "page");
        }
    }

    #[test]
    fn test_parse_pdf_with_streams() {
        let r = parse_pdf_data(&stream_pdf());
        assert!(r.is_ok(), "streams: {:?}", r.err());
        let doc = r.unwrap();
        assert!(doc.objects.len() >= 5, "objects: {}", doc.objects.len());
    }

    #[test]
    fn test_parse_pdf_encrypted() {
        let r = parse_pdf_data(&encrypted_pdf());
        assert!(r.is_ok(), "encrypted: {:?}", r.err());
        let doc = r.unwrap();
        assert!(doc.has_encryption, "encryption flag");
        assert!(
            doc.diagnostics.iter().any(|d| d.code == "PDF-E-003"),
            "diag: {:?}",
            doc.diagnostics
        );
        assert!(doc
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Warning));
    }

    #[test]
    fn test_parse_pdf_metadata() {
        let r = parse_pdf_data(&simple_pdf());
        assert!(r.is_ok(), "meta: {:?}", r.err());
        let doc = r.unwrap();
        assert!(doc.metadata.is_some(), "has metadata");
        let m = doc.metadata.unwrap();
        assert_eq!(m.title, Some("Test PDF".to_string()));
        assert_eq!(m.author, Some("Test Author".to_string()));
        assert_eq!(m.subject, Some("Testing".to_string()));
    }

    #[test]
    fn test_parse_pdf_rotation() {
        let r = parse_pdf_data(&rotated_pdf());
        assert!(r.is_ok(), "rotation: {:?}", r.err());
        let doc = r.unwrap();
        assert_eq!(doc.pages.len(), 1);
        assert_eq!(doc.pages[0].number, 1);
    }

    #[test]
    fn test_pdf_object_types() {
        assert!(matches!(
            parse_obj(b"null", &mut 0).unwrap().0,
            PdfObject::Null
        ));
        let (o, _) = parse_obj(b"true", &mut 0).unwrap();
        assert!(matches!(o, PdfObject::Boolean(true)));
        let (o, _) = parse_obj(b"false", &mut 0).unwrap();
        assert!(matches!(o, PdfObject::Boolean(false)));
        let (o, _) = parse_obj(b"3.14", &mut 0).unwrap();
        let expected: f64 = "3.14".parse().unwrap();
        assert!(matches!(o, PdfObject::Real(v) if (v - expected).abs() < 1e-9));
        let (o, _) = parse_obj(b"/Type", &mut 0).unwrap();
        assert!(matches!(o, PdfObject::Name(ref n) if n == "Type"));
        let (o, _) = parse_obj(b"(hello world)", &mut 0).unwrap();
        assert!(matches!(o, PdfObject::String(ref s) if s == b"hello world"));
        let (o, _) = parse_obj(b"<48656C6C6F>", &mut 0).unwrap();
        assert!(matches!(o, PdfObject::String(ref s) if s == b"Hello"));
        let (o, _) = parse_obj(b"[1 2 (three)]", &mut 0).unwrap();
        assert!(matches!(o, PdfObject::Array(ref a) if a.len() == 3));
        let (o, _) = parse_obj(b"<< /Type /Pages >>", &mut 0).unwrap();
        assert!(matches!(o, PdfObject::Dict(ref d) if d.contains_key("Type")));
        let (o, _) = parse_obj(b"1 0 R", &mut 0).unwrap();
        assert!(matches!(
            o,
            PdfObject::Reference {
                object_number: 1,
                generation: 0
            }
        ));
        let (o, _) = parse_obj(b"123 456 R", &mut 0).unwrap();
        assert!(matches!(
            o,
            PdfObject::Reference {
                object_number: 123,
                generation: 456
            }
        ));
    }

    #[test]
    fn test_pdf_stream_object() {
        let mut b = Vec::new();
        b.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        b.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        b.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        b.extend_from_slice(
            b"3 0 obj\n<< /Length 26 >>\nstream\nhello stream content!!\nendstream\nendobj\n",
        );
        b.extend_from_slice(
            b"4 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n",
        );
        let s = [20, 74, 157, 240];
        b.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for o in &s {
            b.extend_from_slice(format!("{:010} 00000 n \n", o).as_bytes());
        }
        b.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\n");
        b.extend_from_slice(b"startxref\n20\n%%EOF\n");
        let r = parse_pdf_data(&b);
        assert!(r.is_ok(), "stream obj: {:?}", r.err());
        let doc = r.unwrap();
        assert!(doc.objects.len() >= 3);
        if let Some(PdfObject::Stream { data, .. }) = doc.objects.get(&3) {
            assert_eq!(data, b"hello stream content!!\n", "stream data match");
        }
    }

    // Mimics the layout LibreOffice writes: object dictionaries whose body is
    // longer than the scanner's 100-byte backward search window (so the
    // `obj` header is unreachable from `endobj`), a flate stream whose binary
    // payload contains a spurious `endobj` byte sequence, and a standard xref
    // table. Object recovery must come from the xref fill-in pass, not the
    // byte scanner.
    fn lo_style_pdf() -> Vec<u8> {
        let objs: Vec<Vec<u8>> = vec![
            b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Lang (de-DE) /MarkInfo << /Marked true >> /StructTreeRoot 6 0 R /Outlines 5 0 R /Names 7 0 R /Metadata 8 0 R >>\nendobj\n".to_vec(),
            b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 595 842] /Resources << /Font << /F1 9 0 R >> /ProcSet [/PDF /Text] >> >>\nendobj\n".to_vec(),
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources 2 0 R /Contents 4 0 R /CropBox [0 0 595 842] /Rotate 0 >>\nendobj\n".to_vec(),
            // flate-like stream whose raw bytes happen to contain "endobj"
            b"4 0 obj\n<< /Length 36 /Filter /FlateDecode >>\nstream\nx\x9c\x05endobj\x01\x02\x03\xff\xfe\x00BT\x9c\x16\nendstream\nendobj\n".to_vec(),
            b"5 0 obj\n<< /Type /Outlines /Count 0 >>\nendobj\n".to_vec(),
            b"6 0 obj\n<< /Type /StructTreeRoot /ParentTree << /Nums [0 [3 0 R]] >> >>\nendobj\n".to_vec(),
            b"7 0 obj\n<< /Dests << >> >>\nendobj\n".to_vec(),
            b"8 0 obj\n<< /Type /Metadata /Subtype /XML /Length 24 >>\nstream\n<metadata>hello</metadata>\nendstream\nendobj\n".to_vec(),
            b"9 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n".to_vec(),
        ];
        let mut body = Vec::new();
        body.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        let mut offsets = Vec::new();
        for o in &objs {
            offsets.push(body.len());
            body.extend_from_slice(o);
        }
        let xref_off = body.len();
        body.extend_from_slice(b"xref\n0 10\n0000000000 65535 f \n");
        for off in &offsets {
            body.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
        }
        body.extend_from_slice(b"trailer\n<< /Size 10 /Root 1 0 R >>\n");
        body.extend_from_slice(b"startxref\n");
        body.extend_from_slice(format!("{}", xref_off).as_bytes());
        body.extend_from_slice(b"\n%%EOF\n");
        body
    }

    #[test]
    fn test_parse_lo_style_long_dict_pdf() {
        let r = parse_pdf_data(&lo_style_pdf());
        assert!(r.is_ok(), "lo-style: {:?}", r.err());
        let doc = r.unwrap();
        assert_eq!(doc.pages.len(), 1, "lo-style pages: {}", doc.pages.len());
        // xref fill-in must recover the real Catalog dict, not scanner garbage.
        if let Some(PdfObject::Dict(d)) = doc.objects.get(&1) {
            let is_catalog = d
                .get("Type")
                .is_some_and(|v| matches!(v, PdfObject::Name(n) if n == "Catalog"));
            assert!(is_catalog, "obj 1 Type lost: {:?}", doc.objects.get(&1));
        } else {
            panic!("catalog object 1 lost: {:?}", doc.objects.get(&1));
        }
        // The spurious "endobj" inside the stream must not fragment the page.
        assert!(doc.objects.contains_key(&3), "page obj 3 present");
    }

    #[test]
    fn test_scan_stops_at_last_endobj_without_tail_rescan() {
        // A tail with no further "endobj" used to trigger one full-file rescan
        // per byte after the last object (O(n^2)). A big tail keeps the test
        // discriminative even under debug builds; a fixed scanner finishes in
        // milliseconds. Correctness still flows through parse_pdf_data (the
        // xref fill-in recovers the long-body objects the scan misses).
        let mut b = lo_style_pdf();
        let tail_len = 400_000usize;
        for i in 0..tail_len {
            b.push((i % 26) as u8 + b'a');
        }
        let t = std::time::Instant::now();
        let r = scan_indirect_objects(&b);
        let elapsed = t.elapsed();
        assert!(
            !r.is_empty(),
            "scanner must still find the short-body objects"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "scan of {}KB file took {:?}",
            (b.len() as f64 / 1024.0) as usize,
            elapsed
        );
        let doc = parse_pdf_data(&b).expect("parse with tail");
        assert_eq!(doc.pages.len(), 1, "page tree recovered via xref fill-in");
    }

    // ------------------------------------------------------------------
    // page_count tests (P3-20)
    // ------------------------------------------------------------------

    #[test]
    fn test_page_count_single_page() {
        let r = page_count(&lo_style_pdf());
        assert!(r.is_ok(), "page_count should succeed: {:?}", r.err());
        assert_eq!(r.unwrap(), 1);
    }

    #[test]
    fn test_page_count_invalid_pdf() {
        let r = page_count(b"not a pdf file");
        assert!(r.is_err(), "page_count should fail for invalid PDF");
    }

    #[test]
    fn test_page_count_empty() {
        let r = page_count(b"");
        assert!(r.is_err(), "page_count should fail for empty input");
    }

    // ------------------------------------------------------------------
    // P6-32: saturating_sub regression for find_stream_kw overflow
    // ------------------------------------------------------------------

    #[test]
    fn test_find_stream_kw_empty() {
        // empty input should not panic
        let _result = scan_indirect_objects(b"");
    }

    #[test]
    fn test_find_stream_kw_short_data() {
        // Data shorter than 6 bytes would previously cause saturating subtraction
        let short = b"1 0 obj";
        let _result = scan_indirect_objects(short);
    }
}
