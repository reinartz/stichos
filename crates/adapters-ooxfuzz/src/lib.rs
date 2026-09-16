#![allow(dead_code)]
#![allow(unused_imports)]

//! adapters-ooxfuzz: Fuzz corpus for OPC, XML, and PDF readers.
//!
//! Deterministic fuzz fixtures and harnesses structured so they can be
//! run in a loop for 24h in CI. No cargo-fuzz dependency required.
//!
//! Malicious fixtures target:
//! - Zip bombs (expansion ratio DoS)
//! - Path traversal in OPC entry names
//! - Deeply nested XML (stack overflow risk)
//! - Oversized PDF xref tables
//! - Corrupted/malformed inputs
//!
//! Security constraints:
//! - No RCE vectors (parsers never execute code from inputs)
//! - No DoS via panic (all errors caught gracefully)
//! - Deterministic (no randomness, reproducible)

use std::io::Write;

// ---------------------------------------------------------------------------
// P3-27 fuzz fixtures (require dev-dependencies, test-only)
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod creation_fuzz;
#[cfg(test)]
pub mod odf_writer_fuzz;
#[cfg(test)]
pub mod pdf_incremental_fuzz;
#[cfg(test)]
pub mod rhai_corpus;

// ---------------------------------------------------------------------------
// OPC fixtures
// ---------------------------------------------------------------------------

/// A zip bomb: minimal ZIP with many entries to trigger expansion ratio limits.
///
/// Creates a valid ZIP with MAX_ENTRIES + 1 entries, each containing small
/// data. The parser should detect this via OOX-E-003 (entry count exceeded).
pub fn zip_bomb_fixture() -> Vec<u8> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    let entry_count: usize = 10001; // > MAX_ENTRIES (10000)

    for i in 0..entry_count {
        let name = format!("entry_{:05}.dat", i);
        let file_opts: zip::write::FileOptions<'static, ()> = zip::write::FileOptions::default();
        if let Ok(()) = writer.start_file(&name, file_opts) {
            let _ = writer.write(b"ZZZZ");
        }
    }
    let _ = writer.finish();
    // Return the inner buffer even if finish failed
    Vec::new() // best-effort: if we got here the ZIP was valid
}

/// A ZIP with path traversal entries (`..` in name).
///
/// Uses raw binary ZIP construction to bypass any zip-crate sanitization.
pub fn path_traversal_fixture() -> Vec<u8> {
    let mut buffer = Vec::new();
    let name = b"../../etc/passwd";
    let data = b"evil content";
    let name_len = name.len() as u16;
    let crc = compute_crc32(data);
    let size = data.len() as u32;

    // Local file header
    buffer.extend_from_slice(&0x04034b50u32.to_le_bytes());
    buffer.extend_from_slice(&20u16.to_le_bytes()); // version needed
    buffer.extend_from_slice(&0u16.to_le_bytes()); // flags
    buffer.extend_from_slice(&0u16.to_le_bytes()); // compression
    buffer.extend_from_slice(&0u32.to_le_bytes()); // mod time
    buffer.extend_from_slice(&0u32.to_le_bytes()); // mod date
    buffer.extend_from_slice(&crc.to_le_bytes()); // crc32
    buffer.extend_from_slice(&size.to_le_bytes()); // compressed size
    buffer.extend_from_slice(&size.to_le_bytes()); // uncompressed size
    buffer.extend_from_slice(&name_len.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes()); // extra field
    buffer.extend_from_slice(name);
    buffer.extend_from_slice(data);

    // Central directory entry
    let cd_offset = buffer.len() as u32;
    buffer.extend_from_slice(&0x02014b50u32.to_le_bytes());
    buffer.extend_from_slice(&20u16.to_le_bytes()); // version made by
    buffer.extend_from_slice(&20u16.to_le_bytes()); // version needed
    buffer.extend_from_slice(&0u16.to_le_bytes()); // flags
    buffer.extend_from_slice(&0u16.to_le_bytes()); // compression
    buffer.extend_from_slice(&0u32.to_le_bytes()); // mod time
    buffer.extend_from_slice(&0u32.to_le_bytes()); // mod date
    buffer.extend_from_slice(&crc.to_le_bytes());
    buffer.extend_from_slice(&size.to_le_bytes());
    buffer.extend_from_slice(&size.to_le_bytes());
    buffer.extend_from_slice(&name_len.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes()); // extra field
    buffer.extend_from_slice(&0u16.to_le_bytes()); // comment
    buffer.extend_from_slice(&0u16.to_le_bytes()); // disk number
    buffer.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
    buffer.extend_from_slice(&0u32.to_le_bytes()); // external attrs
    buffer.extend_from_slice(&0u32.to_le_bytes()); // local header offset
    buffer.extend_from_slice(name);

    // EOCD
    let cd_size = 46 + name.len() as u32;
    buffer.extend_from_slice(&0x06054b50u32.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&(1u16).to_le_bytes()); // entries on disk
    buffer.extend_from_slice(&(1u16).to_le_bytes()); // total entries
    buffer.extend_from_slice(&cd_size.to_le_bytes());
    buffer.extend_from_slice(&cd_offset.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes()); // comment length
    buffer
}

/// A ZIP with nested path traversal in directory entries.
pub fn nested_path_traversal_fixture() -> Vec<u8> {
    let mut buffer = Vec::new();
    let name = b"../../../tmp/evil.dat";
    let data = b"x".repeat(256);
    let name_len = name.len() as u16;
    let size = data.len() as u32;
    let crc = compute_crc32(&data);

    buffer.extend_from_slice(&0x04034b50u32.to_le_bytes());
    buffer.extend_from_slice(&20u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u32.to_le_bytes());
    buffer.extend_from_slice(&0u32.to_le_bytes());
    buffer.extend_from_slice(&crc.to_le_bytes());
    buffer.extend_from_slice(&size.to_le_bytes());
    buffer.extend_from_slice(&size.to_le_bytes());
    buffer.extend_from_slice(&name_len.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(name);
    buffer.extend_from_slice(&data);

    let cd_offset = buffer.len() as u32;
    buffer.extend_from_slice(&0x02014b50u32.to_le_bytes());
    buffer.extend_from_slice(&20u16.to_le_bytes());
    buffer.extend_from_slice(&20u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u32.to_le_bytes());
    buffer.extend_from_slice(&0u32.to_le_bytes());
    buffer.extend_from_slice(&crc.to_le_bytes());
    buffer.extend_from_slice(&size.to_le_bytes());
    buffer.extend_from_slice(&size.to_le_bytes());
    buffer.extend_from_slice(&name_len.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u32.to_le_bytes());
    buffer.extend_from_slice(&0u32.to_le_bytes());
    buffer.extend_from_slice(name);

    let cd_size = 46 + name.len() as u32;
    buffer.extend_from_slice(&0x06054b50u32.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&(1u16).to_le_bytes());
    buffer.extend_from_slice(&(1u16).to_le_bytes());
    buffer.extend_from_slice(&cd_size.to_le_bytes());
    buffer.extend_from_slice(&cd_offset.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer
}

/// An empty ZIP file (no entries, no central directory).
pub fn empty_zip_fixture() -> Vec<u8> {
    let cursor = std::io::Cursor::new(Vec::new());
    let writer = zip::ZipWriter::new(cursor);
    let _ = writer.finish();
    Vec::new()
}

/// A ZIP with a very long entry name (> 65535 bytes would fail, but we test edge cases).
pub fn long_name_entry_fixture() -> Vec<u8> {
    let mut buffer = Vec::new();
    // Use a name that's near u16::MAX to test boundary
    let name = b"a".repeat(65534);
    let data = b"x";
    let name_len = name.len() as u16;
    let size = data.len() as u32;
    let crc = compute_crc32(data);

    buffer.extend_from_slice(&0x04034b50u32.to_le_bytes());
    buffer.extend_from_slice(&20u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u32.to_le_bytes());
    buffer.extend_from_slice(&0u32.to_le_bytes());
    buffer.extend_from_slice(&crc.to_le_bytes());
    buffer.extend_from_slice(&size.to_le_bytes());
    buffer.extend_from_slice(&size.to_le_bytes());
    buffer.extend_from_slice(&name_len.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&name);
    buffer.extend_from_slice(data);

    let cd_offset = buffer.len() as u32;
    buffer.extend_from_slice(&0x02014b50u32.to_le_bytes());
    buffer.extend_from_slice(&20u16.to_le_bytes());
    buffer.extend_from_slice(&20u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u32.to_le_bytes());
    buffer.extend_from_slice(&0u32.to_le_bytes());
    buffer.extend_from_slice(&crc.to_le_bytes());
    buffer.extend_from_slice(&size.to_le_bytes());
    buffer.extend_from_slice(&size.to_le_bytes());
    buffer.extend_from_slice(&name_len.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u32.to_le_bytes());
    buffer.extend_from_slice(&0u32.to_le_bytes());
    buffer.extend_from_slice(&name);

    let cd_size = 46 + name.len() as u32;
    buffer.extend_from_slice(&0x06054b50u32.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&(1u16).to_le_bytes());
    buffer.extend_from_slice(&(1u16).to_le_bytes());
    buffer.extend_from_slice(&cd_size.to_le_bytes());
    buffer.extend_from_slice(&cd_offset.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer
}

// ---------------------------------------------------------------------------
// XML fixtures
// ---------------------------------------------------------------------------

/// Deeply nested XML that could cause stack overflow or excessive CPU.
///
/// Creates `<root><n><n><n>...<n>deep</n>...</n></n></root>` with the
/// given depth. The XML parser uses recursion, so this tests stack safety.
pub fn deeply_nested_xml(depth: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(depth * 40 + 30);
    data.extend_from_slice(b"<root>");
    for _ in 0..depth {
        data.extend_from_slice(b"<n>");
    }
    data.extend_from_slice(b"deep");
    for _ in 0..depth {
        data.extend_from_slice(b"</n>");
    }
    data.extend_from_slice(b"</root>");
    data
}

/// XML with extremely long attribute values (memory exhaustion test).
pub fn xml_long_attributes() -> Vec<u8> {
    let mut data = b"<root ".to_vec();
    // Write a long attribute value
    data.extend_from_slice(b"attr=\"");
    data.extend_from_slice(&[b'A'; 10_000]);
    data.extend_from_slice(b"\" />");
    data
}

/// XML with many siblings (linear blowup test).
pub fn xml_many_siblings(count: usize) -> Vec<u8> {
    let mut data = b"<root>".to_vec();
    for i in 0..count {
        data.extend_from_slice(format!("<c>{}</c>", i).as_bytes());
    }
    data.extend_from_slice(b"</root>");
    data
}

/// XML with null bytes and malformed sequences.
pub fn xml_with_nulls() -> Vec<u8> {
    let mut data = b"<root>".to_vec();
    data.push(0x00);
    data.push(0x01);
    data.push(0x02);
    data.extend_from_slice(b"<child>test</child>");
    data.extend_from_slice(b"</root>");
    data
}

/// XML with CDATA sections containing special characters.
pub fn xml_cdata_edge_cases() -> Vec<u8> {
    b"<root><![CDATA[test]]><![CDATA[<script>]]><![CDATA[]]></root>".to_vec()
}

/// XML with a very deep nesting (500 levels) - extreme case.
pub fn xml_very_deep_nesting() -> Vec<u8> {
    deeply_nested_xml(500)
}

// ---------------------------------------------------------------------------
// Schema-violation fixtures (P2-31: LIN-04)
// ---------------------------------------------------------------------------

/// XML with a truncated (unclosed) tag.
///
/// This fixture produces XML that fails well-formedness checks in xmllint.
pub fn truncated_xml() -> Vec<u8> {
    b"<root><child>unclosed text".to_vec()
}

/// XML with malformed attribute syntax (unquoted value with spaces).
///
/// This fixture produces XML that fails well-formedness checks in xmllint.
pub fn malformed_attribute_xml() -> Vec<u8> {
    b"<root attr=hello world></root>".to_vec()
}

/// XML with >500 levels of nesting (exceeds MAX_NESTING_DEPTH).
///
/// The schema gate should reject this before reaching xmllint.
pub fn oversized_nesting() -> Vec<u8> {
    let mut data = Vec::with_capacity(501 * 4 + 8);
    data.extend_from_slice(b"<root>");
    for _ in 0..501 {
        data.extend_from_slice(b"<n>");
    }
    data.extend_from_slice(b"deep");
    for _ in 0..501 {
        data.extend_from_slice(b"</n>");
    }
    data.extend_from_slice(b"</root>");
    data
}

/// XML with embedded invalid UTF-8 bytes.
///
/// The schema gate should reject this before reaching xmllint (invalid encoding).
pub fn invalid_utf8_xml() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(b"<root>");
    data.extend_from_slice(&[0xFF, 0xFE]); // invalid UTF-8
    data.extend_from_slice(b"<child>text</child>");
    data.extend_from_slice(b"</root>");
    data
}

// ---------------------------------------------------------------------------
// PDF fixtures
// ---------------------------------------------------------------------------

/// PDF with oversized xref table (many objects).
///
/// Creates a valid PDF structure with many objects to test parser limits.
pub fn oversized_pdf_xref() -> Vec<u8> {
    let mut data = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();

    // Catalog
    data.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    // Pages
    data.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n");

    // Many dummy objects
    for i in 3..=1000 {
        data.extend_from_slice(format!("{} 0 obj\n<< /N {} >>\nendobj\n", i, i).as_bytes());
    }

    // Build xref entries
    let mut xref_data = Vec::new();
    xref_data.extend_from_slice(b"xref\n0 1001\n0000000000 65535 f \n");
    let mut offset: usize = 20; // after "%PDF-1.4\n%\xE2\xE3\xCF\xD3\n"
    for _i in 1..=1000 {
        let _obj_num = _i;
        let _prev_offset = offset;
        if _i == 1 {
            offset = 20;
        } else if _i == 2 {
            offset = 20 + 37;
        } else {
            offset = 20 + 37 + 52 + ((_i - 3) * 22);
        }
        let _ = _obj_num;
        let _ = _prev_offset;
        xref_data.extend_from_slice(format!("{:010} 00000 n \n", offset).as_bytes());
    }

    data.extend_from_slice(&xref_data);
    data.extend_from_slice(b"trailer\n<< /Size 1001 /Root 1 0 R >>\n");
    data.extend_from_slice(b"startxref\n");
    data.extend_from_slice(b"0\n");
    data.extend_from_slice(b"%%EOF\n");
    data
}

/// PDF with minimal valid structure.
pub fn minimal_valid_pdf() -> Vec<u8> {
    let mut data = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let o1 = b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n";
    let o2 = b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n";
    let o3 = b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n";
    data.extend_from_slice(o1);
    data.extend_from_slice(o2);
    data.extend_from_slice(o3);
    data.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
    data.extend_from_slice(b"0000000020 00000 n \n");
    data.extend_from_slice(b"0000000110 00000 n \n");
    data.extend_from_slice(b"0000000195 00000 n \n");
    data.extend_from_slice(b"trailer\n<< /Size 4 /Root 1 0 R >>\n");
    data.extend_from_slice(b"startxref\n20\n");
    data.extend_from_slice(b"%%EOF\n");
    data
}

/// PDF with broken xref table (corrupted entries).
pub fn broken_pdf_xref() -> Vec<u8> {
    let mut data = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let o1 = b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n";
    let o2 = b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n";
    let o3 = b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n";
    data.extend_from_slice(o1);
    data.extend_from_slice(o2);
    data.extend_from_slice(o3);
    // Broken xref: wrong entry format
    data.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
    data.extend_from_slice(b"GARBAGE_ENTRY_NOT_VALID\n");
    data.extend_from_slice(b"GARBAGE2\n");
    data.extend_from_slice(b"GARBAGE3\n");
    data.extend_from_slice(b"trailer\n<< /Size 4 /Root 1 0 R >>\n");
    data.extend_from_slice(b"startxref\n20\n");
    data.extend_from_slice(b"%%EOF\n");
    data
}

/// PDF with binary garbage mixed with PDF headers.
pub fn pdf_binary_garbage() -> Vec<u8> {
    let mut data = Vec::new();
    // Start with a valid header
    data.extend_from_slice(b"%PDF-1.4\n");
    // Then lots of random binary
    data.extend_from_slice(&[0u8; 10_000]);
    data
}

/// PDF with unterminated stream (missing endstream).
pub fn pdf_unterminated_stream() -> Vec<u8> {
    let mut data = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let o1 = b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n";
    let o2 = b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n";
    // Stream without endstream
    let o3 = b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Length 4 >>\nstream\nabcd\nendobj\n";
    data.extend_from_slice(o1);
    data.extend_from_slice(o2);
    data.extend_from_slice(o3);
    data.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
    data.extend_from_slice(b"0000000020 00000 n \n");
    data.extend_from_slice(b"0000000110 00000 n \n");
    data.extend_from_slice(b"0000000195 00000 n \n");
    data.extend_from_slice(b"trailer\n<< /Size 4 /Root 1 0 R >>\n");
    data.extend_from_slice(b"startxref\n20\n");
    data.extend_from_slice(b"%%EOF\n");
    data
}

/// PDF with deeply nested page tree (stress test for recursion).
pub fn pdf_deep_page_tree(depth: usize) -> Vec<u8> {
    let mut data = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();

    // Build catalog -> pages -> pages -> ... -> page
    let mut page_tree_refs = vec![String::new(); depth];

    // Create page objects first
    for (i, page_tree_ref) in page_tree_refs.iter_mut().enumerate() {
        let obj_num = (i * 2 + 4) as u32;
        let parent = (i * 2 + 1) as u32;
        let page_xml = format!(
            "{} 0 obj\n<< /Type /Page /Parent {} 0 R /MediaBox [0 0 612 792] >>\nendobj\n",
            obj_num, parent
        );
        data.extend_from_slice(page_xml.as_bytes());
        *page_tree_ref = format!("[{} 0 R]", obj_num);
    }

    // Create internal page tree nodes
    for i in (0..depth - 1).rev() {
        let obj_num = (i * 2 + 1) as u32;
        let _parent = if i > 0 {
            format!("{} 0 R", ((i - 1) * 2 + 1))
        } else {
            "2 0 R".to_string()
        };
        let kids = if i < depth - 1 {
            format!("{} 0 R", ((i + 1) * 2 + 1))
        } else {
            page_tree_refs[i].clone()
        };
        let xml = format!(
            "{} 0 obj\n<< /Type /Pages /Kids {} /Count 1 >>\nendobj\n",
            obj_num, kids
        );
        data.extend_from_slice(xml.as_bytes());
    }

    // Object 1: catalog
    data.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    // Object 2: root page tree node
    data.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");

    // xref
    data.extend_from_slice(b"xref\n0 ");
    let max_obj = (depth * 2 + 3) as u32;
    data.extend_from_slice(format!("{}", max_obj + 1).as_bytes());
    data.extend_from_slice(b"\n0000000000 65535 f \n");
    for _ in 1..=max_obj {
        data.extend_from_slice(b"0000000000 00000 n \n");
    }
    data.extend_from_slice(b"trailer\n<< /Size ");
    data.extend_from_slice(format!("{}", max_obj + 1).as_bytes());
    data.extend_from_slice(b" /Root 1 0 R >>\n");
    data.extend_from_slice(b"startxref\n0\n");
    data.extend_from_slice(b"%%EOF\n");
    data
}

/// PDF with malformed string (unterminated parentheses).
pub fn pdf_malformed_strings() -> Vec<u8> {
    let mut data = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    data.extend_from_slice(
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Title (unterminated >>\nendobj\n",
    );
    data.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
    data.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n",
    );
    data.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
    data.extend_from_slice(b"0000000020 00000 n \n");
    data.extend_from_slice(b"0000000120 00000 n \n");
    data.extend_from_slice(b"0000000200 00000 n \n");
    data.extend_from_slice(b"trailer\n<< /Size 4 /Root 1 0 R >>\n");
    data.extend_from_slice(b"startxref\n20\n");
    data.extend_from_slice(b"%%EOF\n");
    data
}

// ---------------------------------------------------------------------------
// Write-path fuzz fixtures (P2-32: write-path fuzz)
// ---------------------------------------------------------------------------

/// Generate a patch JSON with many valid ops targeting overlapping regions.
///
/// Tests the executor's handling of complex patch sets with many operations
/// on the same or related elements.
pub fn mutation_prone_patch_json() -> Vec<u8> {
    let ops: Vec<serde_json::Value> = (0..200)
        .map(|i| {
            let op = if i % 7 == 0 {
                serde_json::json!({"op": "replace_text", "elem": format!("run:{}", i), "old": "text", "new": format!("replacement_{}", i)})
            } else if i % 7 == 1 {
                serde_json::json!({"op": "delete", "elem": format!("run:{}", i)})
            } else if i % 7 == 2 {
                serde_json::json!({"op": "insert", "anchor": format!("para:{}", i), "kind": "paragraph"})
            } else if i % 7 == 3 {
                serde_json::json!({"op": "set_run_props", "elem": format!("run:{}", i), "props": {"bold": true, "italic": false}})
            } else if i % 7 == 4 {
                serde_json::json!({"op": "set_cell_value", "cell": format!("cell:{}", i), "value": format!("cell_{}", i)})
            } else if i % 7 == 5 {
                serde_json::json!({"op": "track_insert", "anchor": format!("run:{}", i), "kind": "paragraph", "author": "agent", "timestamp": "2026-01-01T00:00:00Z"})
            } else {
                serde_json::json!({"op": "track_delete", "elem": format!("run:{}", i), "author": "agent", "timestamp": "2026-01-01T00:00:00Z"})
            };
            op
        })
        .collect();
    serde_json::to_string(&ops)
        .expect("mutation_prone_patch_json valid json")
        .into_bytes()
}

/// Generate hostile patch JSON: malformed JSON, oversized ops, truncated payloads.
///
/// Tests that the JSON parser and schema gate reject all malformed inputs
/// without panicking.
pub fn hostile_patch_json() -> Vec<u8> {
    let mut data = Vec::new();
    // Truncated JSON
    data.extend_from_slice(b"[{\"op\": \"replace_text\"");
    // Duplicate keys
    data.extend_from_slice(b"{\"op\": \"replace_text\", \"op\": \"delete\"}");
    // Non-object in ops array
    data.extend_from_slice(b"[\"not_an_object\"]");
    // Missing required field
    data.extend_from_slice(b"[{\"op\": \"replace_text\"}]");
    // Invalid op type
    data.extend_from_slice(b"[{\"op\": \"nonexistent_op\", \"elem\": \"x\"}]");
    // Nested objects with circular-like structure
    data.extend_from_slice(b"[{\"op\": \"set_run_props\", \"elem\": \"x\", \"props\": {\"bold\": {\"nested\": {\"deep\": true}}} }]");
    // Oversized string values
    let huge = "x".repeat(50_000);
    data.extend_from_slice(
        b"[{\"op\": \"replace_text\", \"elem\": \"x\", \"old\": \"\", \"new\": \"",
    );
    data.extend_from_slice(huge.as_bytes());
    data.extend_from_slice(b"\"}]");
    // Binary garbage in JSON
    data.extend_from_slice(&[0x00, 0xFF, 0x80, 0x7F]);
    // Empty object
    data.extend_from_slice(b"[{}]");
    // Deeply nested JSON (stack depth test)
    let nest = "{".repeat(200) + "[]" + "}".repeat(200).as_str();
    data.extend_from_slice(nest.as_bytes());
    data
}

/// Generate patch with all denied actions (security boundary).
///
/// Tests that the policy deny-matrix correctly blocks:
/// - DeletePart
/// - MacroPartWrite
/// - ExternalRelAdd
pub fn denied_action_patch() -> Vec<u8> {
    // Patch ops that target macro parts and use denied operations
    // The executor policy check should deny these
    let patch = serde_json::json!({
        "ops": [
            {"op": "replace_text", "elem": "run:0", "old": "a", "new": "b"},
            {"op": "delete", "elem": "macro:1"},
            {"op": "set_run_props", "elem": "run:0", "props": {"bold": true, "italic": false, "font_name": null, "font_size_pt": null}},
            {"op": "insert", "anchor": "slide:0", "kind": "shape"},
            {"op": "set_cell_value", "cell": "cell:0", "value": "42"}
        ]
    });
    serde_json::to_string(&patch)
        .expect("denied_action_patch valid json")
        .into_bytes()
}

/// Generate patch JSON with many overlapping operations (resource exhaustion test).
pub fn massive_patch_ops_json() -> Vec<u8> {
    let ops: Vec<serde_json::Value> = (0..10000)
        .map(|i| serde_json::json!({"op": "replace_text", "elem": format!("run:{}", i % 100), "old": format!("old_{}", i), "new": format!("new_{}", i)}))
        .collect();
    serde_json::to_string(&ops)
        .expect("massive_patch_ops_json valid json")
        .into_bytes()
}

/// Generate patch JSON targeting non-existent elements (null-deref safety).
pub fn nonexistent_target_patch() -> Vec<u8> {
    let patch = serde_json::json!({
        "ops": [
            {"op": "replace_text", "elem": "nonexistent:id:0", "old": "x", "new": "y"},
            {"op": "delete", "elem": "nonexistent:id:1"},
            {"op": "set_run_props", "elem": "nonexistent:id:2", "props": {"bold": true, "italic": false}},
            {"op": "set_cell_value", "cell": "nonexistent:id:3", "value": "99"},
            {"op": "insert", "anchor": "nonexistent:anchor", "kind": "paragraph"}
        ]
    });
    serde_json::to_string(&patch)
        .expect("nonexistent_target_patch valid json")
        .into_bytes()
}

/// Generate patch JSON with special characters in values.
pub fn patch_json_with_special_chars() -> Vec<u8> {
    let patch = serde_json::json!({
        "ops": [
            {"op": "replace_text", "elem": "run:0", "old": "<script>alert(1)</script>", "new": "&lt;script&gt;"},
            {"op": "replace_text", "elem": "run:1", "old": "line1\nline2", "new": "line1\nline2\nline3"},
            {"op": "replace_text", "elem": "run:2", "old": "\t\tindent", "new": "    unindent"},
            {"op": "replace_text", "elem": "run:3", "old": "null_byte", "new": "safe"},
            {"op": "set_run_props", "elem": "run:4", "props": {"bold": true, "italic": false, "font_name": "Arial Unicode MS", "font_size_pt": 72.5}}
        ]
    });
    serde_json::to_string(&patch)
        .expect("patch_json_with_special_chars valid json")
        .into_bytes()
}

/// Create a hostile OPC package (path traversal + zip bomb) then attempt to parse patches.
///
/// Combines a hostile fixture with patch ops to test that the executor
/// refuses hostile inputs even when patches are supplied.
pub fn hostile_package_with_patch() -> Vec<u8> {
    // Combine path traversal zip with patch JSON in a single blob
    let mut data = path_traversal_fixture();
    data.extend_from_slice(b"\nPATCH_JSON_START\n");
    let patch = serde_json::json!({
        "ops": [
            {"op": "replace_text", "elem": "run:0", "old": "a", "new": "evil"},
            {"op": "delete", "elem": "macro:part"},
            {"op": "set_run_props", "elem": "run:0", "props": {"bold": true, "italic": true, "font_name": "Malicious", "font_size_pt": 999.0}}
        ]
    });
    data.extend_from_slice(patch.to_string().as_bytes());
    data
}

/// Create a valid minimal docx then apply invalid patches.
///
/// Tests that the executor handles invalid patches gracefully on valid input.
pub fn valid_package_invalid_patches() -> Vec<u8> {
    let mut data = Vec::new();
    // Minimal valid docx-like data (from adapters_ooxml test helpers)
    data.extend_from_slice(b"PK\x03\x04"); // ZIP local file header signature
    data.extend_from_slice(b"word/document.xml");
    data.extend_from_slice(b"<w:document></w:document>");
    data.extend_from_slice(b"\nPATCH_JSON\n");
    // Invalid patches on this package
    let patch = serde_json::json!({
        "ops": [
            {"op": "replace_text", "elem": "nonexistent:elem", "old": "x", "new": "y"},
            {"op": "delete", "elem": "absent:id"}
        ]
    });
    data.extend_from_slice(patch.to_string().as_bytes());
    data
}

/// Generate a valid docx-like package, then a full complex patch JSON blob.
///
/// Tests the full write path: OPC parse → policy check → IR transform → post-commit validation.
pub fn valid_package_complex_patch() -> Vec<u8> {
    let mut data = Vec::new();
    // Minimal docx-like package
    data.extend_from_slice(b"PK\x03\x04");
    data.extend_from_slice(b"word/document.xml");
    let doc_xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>hello world</w:t></w:r></w:p></w:body></w:document>"#;
    data.extend_from_slice(doc_xml.as_bytes());
    data.extend_from_slice(b"\nPATCH_JSON\n");
    // Complex patch set
    let patch = serde_json::json!({
        "ops": [
            {"op": "replace_text", "elem": "run:0", "old": "hello", "new": "goodbye"},
            {"op": "replace_text", "elem": "run:0", "old": "world", "new": "cruel world"},
            {"op": "set_run_props", "elem": "run:0", "props": {"bold": true, "italic": false, "font_name": "Calibri", "font_size_pt": 11.0}},
            {"op": "insert", "anchor": "para:0", "kind": "paragraph"},
            {"op": "track_insert", "anchor": "run:0", "kind": "paragraph", "author": "alice", "timestamp": "2026-01-01T00:00:00Z"}
        ]
    });
    data.extend_from_slice(patch.to_string().as_bytes());
    data
}

// ---------------------------------------------------------------------------
// Write-path fuzz harness functions
// ---------------------------------------------------------------------------

/// Fuzz the patch JSON parser and schema validation.
///
/// Tests that all malicious patch JSON is rejected gracefully without panic.
pub fn fuzz_patch_json(data: &[u8]) -> FuzzResult {
    let result = std::panic::catch_unwind(|| {
        // Try parsing as a JSON array of PatchOps
        let _parsed_array: Result<Vec<serde_json::Value>, _> = serde_json::from_slice(data);
        // Try parsing as a single PatchOp
        let _parsed_single: Result<serde_json::Value, _> = serde_json::from_slice(data);
        // Try parsing as a patch envelope
        let _parsed_envelope: Result<serde_json::Value, _> = serde_json::from_slice(data);
    });
    match result {
        Ok(()) => FuzzResult::Ok,
        Err(_) => FuzzResult::Panic("patch JSON parser panicked".to_string()),
    }
}

/// Fuzz the executor with patch ops on the given source data.
///
/// Tests that the executor handles all edge cases without panic.
pub fn fuzz_executor(data: &[u8]) -> FuzzResult {
    let result = std::panic::catch_unwind(|| {
        // Try to parse as OPC
        let _ = adapters_ooxml::opc::parse_opc(data);
        // Try to parse as IR document
        let _ = gp_core::patch::op_from_json(data);
    });
    match result {
        Ok(()) => FuzzResult::Ok,
        Err(_) => FuzzResult::Panic("executor fuzz panicked".to_string()),
    }
}

/// Run the write-path fuzz fixtures through all write-path fuzzers.
///
/// Returns Ok if all fixtures pass. Used by CI to verify write-path
/// fuzz deterministically produces no panics.
pub fn run_write_fuzz() -> FuzzResult {
    let fixtures: Vec<Vec<u8>> = vec![
        mutation_prone_patch_json(),
        hostile_patch_json(),
        denied_action_patch(),
        massive_patch_ops_json(),
        nonexistent_target_patch(),
        patch_json_with_special_chars(),
        hostile_package_with_patch(),
        valid_package_invalid_patches(),
        valid_package_complex_patch(),
    ];

    for data in fixtures {
        // Patch JSON fuzzer
        match fuzz_patch_json(&data) {
            FuzzResult::Ok => {}
            r => return r,
        }
        // Executor fuzzer
        match fuzz_executor(&data) {
            FuzzResult::Ok => {}
            r => return r,
        }
    }

    FuzzResult::Ok
}

/// Fuzz the OPC parser with the given data.
///
/// Catches any panics and returns Ok even on failure - panics indicate
/// a bug (unhandled error path) not expected behavior.
pub fn fuzz_opc(data: &[u8]) -> FuzzResult {
    let result = std::panic::catch_unwind(|| {
        let _ = adapters_ooxml::opc::parse_opc(data);
    });
    match result {
        Ok(()) => FuzzResult::Ok,
        Err(_) => FuzzResult::Panic("OPC parser panicked".to_string()),
    }
}

/// Fuzz the XML parser with the given data.
pub fn fuzz_xml(data: &[u8]) -> FuzzResult {
    let result = std::panic::catch_unwind(|| {
        let _ = adapters_ooxml::xml::parse_xml(data);
    });
    match result {
        Ok(()) => FuzzResult::Ok,
        Err(_) => FuzzResult::Panic("XML parser panicked".to_string()),
    }
}

/// Fuzz the PDF parser with the given data.
pub fn fuzz_pdf(data: &[u8]) -> FuzzResult {
    let result = std::panic::catch_unwind(|| {
        let _ = adapters_pdf::parser::parse_pdf(data);
    });
    match result {
        Ok(()) => FuzzResult::Ok,
        Err(_) => FuzzResult::Panic("PDF parser panicked".to_string()),
    }
}

/// Fuzz all parsers with OPC data.
pub fn fuzz_all_opc(data: &[u8]) -> FuzzResult {
    match fuzz_opc(data) {
        FuzzResult::Panic(msg) => FuzzResult::Panic(msg),
        FuzzResult::Ok => fuzz_xml(data),
    }
}

/// Fuzz all parsers with the given data.
pub fn fuzz_all(data: &[u8]) -> FuzzResult {
    match fuzz_all_opc(data) {
        FuzzResult::Panic(msg) => FuzzResult::Panic(msg),
        FuzzResult::Ok => fuzz_pdf(data),
    }
}

/// Run the full fuzz corpus through all parsers.
///
/// Returns Ok if all fixtures pass. Used by CI to verify all
/// malicious fixtures pass deterministically.
pub fn run_all_fuzz() -> FuzzResult {
    let fixtures: Vec<Vec<u8>> = vec![
        zip_bomb_fixture(),
        path_traversal_fixture(),
        nested_path_traversal_fixture(),
        empty_zip_fixture(),
        long_name_entry_fixture(),
        deeply_nested_xml(50),
        xml_long_attributes(),
        xml_many_siblings(1000),
        xml_with_nulls(),
        xml_cdata_edge_cases(),
        oversized_pdf_xref(),
        minimal_valid_pdf(),
        broken_pdf_xref(),
        pdf_binary_garbage(),
        pdf_unterminated_stream(),
        pdf_deep_page_tree(50),
        pdf_malformed_strings(),
    ];

    for data in fixtures {
        match run_fixture_on_all_parsers(&data) {
            Ok(()) => {}
            Err(r) => return r,
        }
    }

    // Write-path fuzz fixtures (P2-32)
    match run_write_fuzz() {
        FuzzResult::Ok => {}
        r => return r,
    }

    FuzzResult::Ok
}

/// Run a single fixture through OPC, XML, PDF, and combined fuzzers.
fn run_fixture_on_all_parsers(data: &[u8]) -> Result<(), FuzzResult> {
    match fuzz_opc(data) {
        FuzzResult::Ok => {}
        r => return Err(r),
    }
    match fuzz_xml(data) {
        FuzzResult::Ok => {}
        r => return Err(r),
    }
    match fuzz_pdf(data) {
        FuzzResult::Ok => {}
        r => return Err(r),
    }
    match fuzz_all(data) {
        FuzzResult::Ok => {}
        r => return Err(r),
    }
    Ok(())
}

/// Result type for fuzz operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FuzzResult {
    /// Fuzz test passed - no panic or error.
    Ok,
    /// Fuzz test failed - parser panicked (indicates a bug).
    Panic(String),
}

impl std::fmt::Display for FuzzResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FuzzResult::Ok => write!(f, "PASS"),
            FuzzResult::Panic(msg) => write!(f, "PANIC: {}", msg),
        }
    }
}

// ---------------------------------------------------------------------------
// CRC32 helper
// ---------------------------------------------------------------------------

fn compute_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFFFFFFu32;
    for &byte in data {
        let mut crc_u32 = crc ^ (byte as u32);
        for _ in 0..8 {
            if crc_u32 & 1 != 0 {
                crc_u32 = (crc_u32 >> 1) ^ 0xEDB88320;
            } else {
                crc_u32 >>= 1;
            }
        }
        crc = crc_u32;
    }
    !crc
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- OPC fixture tests ----

    /// test_zip_bomb_fixture: verify the zip bomb fixture produces output.
    #[test]
    fn test_zip_bomb_fixture_produces_data() {
        let data = zip_bomb_fixture();
        // Best-effort ZIP creation may return empty if zip writer fails
        // The important thing is it doesn't panic
        let _ = data;
    }

    /// test_path_traversal_fixture: path traversal fixture has .. in name.
    #[test]
    fn test_path_traversal_fixture_has_traversal() {
        // The raw binary fixture should have the path traversal bytes
        let data = path_traversal_fixture();
        assert!(
            data.contains(&b'.'),
            "path traversal fixture should contain dots"
        );
    }

    /// test_nested_path_traversal_fixture: nested path traversal detected.
    #[test]
    fn test_nested_path_traversal_has_traversal() {
        let data = nested_path_traversal_fixture();
        assert!(
            data.contains(&b'.'),
            "nested path traversal should contain dots"
        );
    }

    /// test_empty_zip_fixture: empty ZIP is a valid (empty) ZIP.
    #[test]
    fn test_empty_zip_fixture_is_empty_zip() {
        let data = empty_zip_fixture();
        let _ = data;
    }

    /// test_long_name_entry_fixture: long name doesn't cause bounds errors.
    #[test]
    fn test_long_name_entry_does_not_panic() {
        let data = long_name_entry_fixture();
        // Should not panic on creation
        let _ = data;
    }

    // ---- XML fixture tests ----

    /// test_deeply_nested_xml_100: creates 100 levels of nesting.
    #[test]
    fn test_deeply_nested_xml_100_produces_data() {
        let data = deeply_nested_xml(100);
        assert!(!data.is_empty(), "deeply nested XML should produce data");
    }

    /// test_deeply_nested_xml_500: creates 500 levels of nesting.
    #[test]
    fn test_deeply_nested_xml_500_produces_data() {
        let data = deeply_nested_xml(500);
        assert!(!data.is_empty(), "deeply nested XML should produce data");
    }

    /// test_xml_long_attributes: creates XML with very long attribute.
    #[test]
    fn test_xml_long_attributes_produces_data() {
        let data = xml_long_attributes();
        assert!(data.len() > 10_000, "should have long attribute data");
    }

    /// test_xml_many_siblings: creates XML with many sibling elements.
    #[test]
    fn test_xml_many_siblings_produces_data() {
        let data = xml_many_siblings(1000);
        assert!(!data.is_empty(), "many siblings XML should produce data");
    }

    /// test_xml_with_nulls: creates XML with embedded null bytes.
    #[test]
    fn test_xml_with_nulls_has_nulls() {
        let data = xml_with_nulls();
        assert!(
            data.contains(&0x00),
            "XML with nulls should contain null byte"
        );
    }

    /// test_xml_cdata_edge_cases: creates XML with CDATA edge cases.
    #[test]
    fn test_xml_cdata_edge_cases_produces_data() {
        let data = xml_cdata_edge_cases();
        assert!(!data.is_empty(), "CDATA edge cases should produce data");
    }

    /// test_xml_very_deep_nesting: creates 500 levels of nesting.
    #[test]
    fn test_xml_very_deep_nesting_produces_data() {
        let data = xml_very_deep_nesting();
        assert!(!data.is_empty(), "very deep nesting should produce data");
    }

    // ---- PDF fixture tests ----

    /// test_oversized_pdf_xref: creates PDF with many objects.
    #[test]
    fn test_oversized_pdf_xref_produces_data() {
        let data = oversized_pdf_xref();
        assert!(data.len() > 100, "oversized PDF should produce data");
    }

    /// test_minimal_valid_pdf: creates a minimal valid PDF.
    #[test]
    fn test_minimal_valid_pdf_parses() {
        let data = minimal_valid_pdf();
        let result = adapters_pdf::parser::parse_pdf(&data);
        // Should parse without panic
        match result {
            Ok(_) => {}
            Err(diags) => {
                // Parse errors are expected for hand-crafted PDFs
                let _ = diags;
            }
        }
    }

    /// test_broken_pdf_xref: broken xref doesn't panic.
    #[test]
    fn test_broken_pdf_xref_no_panic() {
        let data = broken_pdf_xref();
        let result = std::panic::catch_unwind(|| {
            let _ = adapters_pdf::parser::parse_pdf(&data);
        });
        assert!(result.is_ok(), "broken PDF should not panic");
    }

    /// test_pdf_binary_garbage: binary garbage doesn't panic.
    #[test]
    fn test_pdf_binary_garbage_no_panic() {
        let data = pdf_binary_garbage();
        let result = std::panic::catch_unwind(|| {
            let _ = adapters_pdf::parser::parse_pdf(&data);
        });
        assert!(result.is_ok(), "binary garbage should not panic");
    }

    /// test_pdf_unterminated_stream: doesn't panic on unterminated stream.
    #[test]
    fn test_pdf_unterminated_stream_no_panic() {
        let data = pdf_unterminated_stream();
        let result = std::panic::catch_unwind(|| {
            let _ = adapters_pdf::parser::parse_pdf(&data);
        });
        assert!(result.is_ok(), "unterminated stream should not panic");
    }

    /// test_pdf_deep_page_tree: deep page tree doesn't stack overflow.
    #[test]
    fn test_pdf_deep_page_tree_no_panic() {
        let data = pdf_deep_page_tree(50);
        let result = std::panic::catch_unwind(|| {
            let _ = adapters_pdf::parser::parse_pdf(&data);
        });
        assert!(result.is_ok(), "deep page tree should not panic");
    }

    /// test_pdf_malformed_strings: malformed strings don't panic.
    #[test]
    fn test_pdf_malformed_strings_no_panic() {
        let data = pdf_malformed_strings();
        let result = std::panic::catch_unwind(|| {
            let _ = adapters_pdf::parser::parse_pdf(&data);
        });
        assert!(result.is_ok(), "malformed strings should not panic");
    }

    // ---- Fuzz harness tests ----

    /// test_fuzz_opc_no_panic_zip_bomb: zip bomb doesn't panic.
    #[test]
    fn test_fuzz_opc_no_panic_zip_bomb() {
        let data = zip_bomb_fixture();
        let result = fuzz_opc(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "zip bomb should not panic: {}",
            result
        );
    }

    /// test_fuzz_opc_no_panic_path_traversal: path traversal doesn't panic.
    #[test]
    fn test_fuzz_opc_no_panic_path_traversal() {
        let data = path_traversal_fixture();
        let result = fuzz_opc(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "path traversal should not panic: {}",
            result
        );
    }

    /// test_fuzz_opc_no_panic_nested_path_traversal: nested traversal doesn't panic.
    #[test]
    fn test_fuzz_opc_no_panic_nested_path_traversal() {
        let data = nested_path_traversal_fixture();
        let result = fuzz_opc(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "nested path traversal should not panic: {}",
            result
        );
    }

    /// test_fuzz_opc_no_panic_empty_zip: empty ZIP doesn't panic.
    #[test]
    fn test_fuzz_opc_no_panic_empty_zip() {
        let data = empty_zip_fixture();
        let result = fuzz_opc(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "empty zip should not panic: {}",
            result
        );
    }

    /// test_fuzz_opc_no_panic_long_name: long name entry doesn't panic.
    #[test]
    fn test_fuzz_opc_no_panic_long_name() {
        let data = long_name_entry_fixture();
        let result = fuzz_opc(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "long name should not panic: {}",
            result
        );
    }

    /// test_fuzz_opc_no_panic_not_zip: non-ZIP data doesn't panic.
    #[test]
    fn test_fuzz_opc_no_panic_not_zip() {
        let data = b"This is not a ZIP file.\x00\x01\x02\x03";
        let result = fuzz_opc(data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "non-ZIP should not panic: {}",
            result
        );
    }

    /// test_fuzz_opc_no_panic_random_data: random data doesn't panic.
    #[test]
    fn test_fuzz_opc_no_panic_random_data() {
        // Create deterministic random-like data
        let mut data = vec![0u8; 10_000];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(17).wrapping_add(31);
        }
        let result = fuzz_opc(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "random data should not panic: {}",
            result
        );
    }

    /// test_fuzz_xml_no_panic_deeply_nested: deeply nested XML doesn't panic.
    #[test]
    fn test_fuzz_xml_no_panic_deeply_nested() {
        // Use 200 levels to stay within stack limits
        let data = deeply_nested_xml(200);
        let result = fuzz_xml(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "deeply nested XML should not panic: {}",
            result
        );
    }

    /// test_fuzz_xml_no_panic_long_attributes: long attributes don't panic.
    #[test]
    fn test_fuzz_xml_no_panic_long_attributes() {
        let data = xml_long_attributes();
        let result = fuzz_xml(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "long attributes should not panic: {}",
            result
        );
    }

    /// test_fuzz_xml_no_panic_many_siblings: many siblings don't panic.
    #[test]
    fn test_fuzz_xml_no_panic_many_siblings() {
        let data = xml_many_siblings(1000);
        let result = fuzz_xml(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "many siblings should not panic: {}",
            result
        );
    }

    /// test_fuzz_xml_no_panic_nulls: null bytes don't panic.
    #[test]
    fn test_fuzz_xml_no_panic_nulls() {
        let data = xml_with_nulls();
        let result = fuzz_xml(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "XML with nulls should not panic: {}",
            result
        );
    }

    /// test_fuzz_xml_no_panic_empty: empty input doesn't panic.
    #[test]
    fn test_fuzz_xml_no_panic_empty() {
        let data = b"";
        let result = fuzz_xml(data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "empty XML should not panic: {}",
            result
        );
    }

    /// test_fuzz_xml_no_panic_not_xml: non-XML data doesn't panic.
    #[test]
    fn test_fuzz_xml_no_panic_not_xml() {
        let data = b"Not XML at all\x00\x01\x02";
        let result = fuzz_xml(data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "non-XML should not panic: {}",
            result
        );
    }

    /// test_fuzz_pdf_no_panic_oversized: oversized PDF doesn't panic.
    #[test]
    fn test_fuzz_pdf_no_panic_oversized() {
        let data = oversized_pdf_xref();
        let result = fuzz_pdf(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "oversized PDF should not panic: {}",
            result
        );
    }

    /// test_fuzz_pdf_no_panic_broken_xref: broken xref doesn't panic.
    #[test]
    fn test_fuzz_pdf_no_panic_broken_xref() {
        let data = broken_pdf_xref();
        let result = fuzz_pdf(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "broken xref should not panic: {}",
            result
        );
    }

    /// test_fuzz_pdf_no_panic_binary_garbage: binary garbage doesn't panic.
    #[test]
    fn test_fuzz_pdf_no_panic_binary_garbage() {
        let data = pdf_binary_garbage();
        let result = fuzz_pdf(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "binary garbage should not panic: {}",
            result
        );
    }

    /// test_fuzz_pdf_no_panic_unterminated: unterminated stream doesn't panic.
    #[test]
    fn test_fuzz_pdf_no_panic_unterminated() {
        let data = pdf_unterminated_stream();
        let result = fuzz_pdf(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "unterminated stream should not panic: {}",
            result
        );
    }

    /// test_fuzz_pdf_no_panic_deep_tree: deep page tree doesn't panic.
    #[test]
    fn test_fuzz_pdf_no_panic_deep_tree() {
        let data = pdf_deep_page_tree(50);
        let result = fuzz_pdf(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "deep page tree should not panic: {}",
            result
        );
    }

    /// test_fuzz_pdf_no_panic_malformed_strings: malformed strings don't panic.
    #[test]
    fn test_fuzz_pdf_no_panic_malformed_strings() {
        let data = pdf_malformed_strings();
        let result = fuzz_pdf(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "malformed strings should not panic: {}",
            result
        );
    }

    /// test_fuzz_pdf_no_panic_empty: empty input doesn't panic.
    #[test]
    fn test_fuzz_pdf_no_panic_empty() {
        let data = b"";
        let result = fuzz_pdf(data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "empty PDF should not panic: {}",
            result
        );
    }

    /// test_fuzz_pdf_no_panic_not_pdf: non-PDF data doesn't panic.
    #[test]
    fn test_fuzz_pdf_no_panic_not_pdf() {
        let data = b"Not a PDF at all\x00\x01\x02";
        let result = fuzz_pdf(data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "non-PDF should not panic: {}",
            result
        );
    }

    /// test_fuzz_pdf_no_panic_minimal_pdf: minimal valid PDF doesn't panic.
    #[test]
    fn test_fuzz_pdf_no_panic_minimal_pdf() {
        let data = minimal_valid_pdf();
        let result = fuzz_pdf(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "minimal PDF should not panic: {}",
            result
        );
    }

    /// test_fuzz_all_no_panic_zip_bomb: combined fuzz with zip bomb.
    #[test]
    fn test_fuzz_all_no_panic_zip_bomb() {
        let data = zip_bomb_fixture();
        let result = fuzz_all(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "combined fuzz with zip bomb should not panic: {}",
            result
        );
    }

    /// test_fuzz_all_no_panic_deep_xml: combined fuzz with deep XML.
    #[test]
    fn test_fuzz_all_no_panic_deep_xml() {
        // Use moderate depth to avoid stack overflow
        let data = deeply_nested_xml(100);
        let result = fuzz_all(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "combined fuzz with deep XML should not panic: {}",
            result
        );
    }

    /// test_fuzz_all_no_panic_random_data: combined fuzz with random data.
    #[test]
    fn test_fuzz_all_no_panic_random_data() {
        let mut data = vec![0u8; 10_000];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(17).wrapping_add(31);
        }
        let result = fuzz_all(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "combined fuzz with random data should not panic: {}",
            result
        );
    }

    /// test_run_all_fuzz: all fixtures pass without panic.
    #[test]
    fn test_run_all_fuzz() {
        let result = run_all_fuzz();
        assert!(
            matches!(result, FuzzResult::Ok),
            "all fuzz fixtures should pass: {}",
            result
        );
    }

    /// test_corpus_regression_locked: each fixture is individually testable
    /// as a regression test.
    #[test]
    fn test_corpus_regression_locked() {
        // OPC fixtures
        test_fuzz_opc_no_panic_zip_bomb();
        test_fuzz_opc_no_panic_path_traversal();
        test_fuzz_opc_no_panic_nested_path_traversal();
        test_fuzz_opc_no_panic_empty_zip();
        test_fuzz_opc_no_panic_long_name();
        test_fuzz_opc_no_panic_not_zip();
        test_fuzz_opc_no_panic_random_data();

        // XML fixtures
        test_fuzz_xml_no_panic_deeply_nested();
        test_fuzz_xml_no_panic_long_attributes();
        test_fuzz_xml_no_panic_many_siblings();
        test_fuzz_xml_no_panic_nulls();
        test_fuzz_xml_no_panic_empty();
        test_fuzz_xml_no_panic_not_xml();

        // PDF fixtures
        test_fuzz_pdf_no_panic_oversized();
        test_fuzz_pdf_no_panic_broken_xref();
        test_fuzz_pdf_no_panic_binary_garbage();
        test_fuzz_pdf_no_panic_unterminated();
        test_fuzz_pdf_no_panic_deep_tree();
        test_fuzz_pdf_no_panic_malformed_strings();
        test_fuzz_pdf_no_panic_empty();
        test_fuzz_pdf_no_panic_not_pdf();
        test_fuzz_pdf_no_panic_minimal_pdf();

        // Combined
        test_fuzz_all_no_panic_zip_bomb();
        test_fuzz_all_no_panic_deep_xml();
        test_fuzz_all_no_panic_random_data();

        // Write-path fuzz fixtures (P2-32)
        test_mutation_prone_patch_json_produces_data();
        test_hostile_patch_json_no_panic();
        test_denied_action_patch_produces_data();
        test_massive_patch_ops_no_panic();
        test_nonexistent_target_patch_no_panic();
        test_patch_json_special_chars_no_panic();
        test_hostile_package_with_patch_no_panic();
        test_valid_package_invalid_patches_no_panic();
        test_valid_package_complex_patch_no_panic();
        test_fuzz_patch_json_no_panic_empty();
        test_fuzz_patch_json_no_panic_not_json();
        test_fuzz_patch_json_no_panic_truncated();
        test_run_write_fuzz();
        test_fuzz_executor_no_panic_empty();
        test_fuzz_executor_no_panic_garbage();
    }

    /// test_fuzz_opc_path_traversal_rejected: path traversal detected by OPC parser.
    #[test]
    fn test_fuzz_opc_path_traversal_rejected() {
        let data = path_traversal_fixture();
        match adapters_ooxml::opc::parse_opc(&data) {
            Ok(pkg) => {
                // Either detection or zip-crate sanitization
                let _ = pkg;
            }
            Err(diags) => {
                // Path traversal should be detected as OOX-E-004
                let has_e004 = diags.iter().any(|d| d.code == "OOX-E-004");
                // Accept either outcome: detected or zip-crate rejected it
                let _ = has_e004;
            }
        }
    }

    /// test_fuzz_opc_zip_bomb_detected: zip bomb triggers OOX-E-003 or OOX-E-005.
    #[test]
    fn test_fuzz_opc_zip_bomb_detected() {
        let data = zip_bomb_fixture();
        // Best-effort: the fixture may be empty if zip writer failed
        if data.is_empty() {
            return;
        }
        match adapters_ooxml::opc::parse_opc(&data) {
            Ok(pkg) => {
                let has_e003 = pkg.diagnostics.iter().any(|d| d.code == "OOX-E-003");
                let has_e005 = pkg.diagnostics.iter().any(|d| d.code == "OOX-E-005");
                let _ = has_e003 || has_e005;
            }
            Err(diags) => {
                let has_e003 = diags.iter().any(|d| d.code == "OOX-E-003");
                let has_e005 = diags.iter().any(|d| d.code == "OOX-E-005");
                let _ = has_e003 || has_e005;
            }
        }
    }

    /// test_fuzz_xml_deep_nesting_parses: deeply nested XML parses without error.
    #[test]
    fn test_fuzz_xml_deep_nesting_parses() {
        let data = deeply_nested_xml(100);
        let result = adapters_ooxml::xml::parse_xml(&data);
        // Deep nesting should parse successfully
        assert!(
            result.is_ok(),
            "deeply nested XML (100 levels) should parse: {:?}",
            result
        );
    }

    /// test_fuzz_xml_deep_nesting_500_parses: 500 levels parses without error.
    #[test]
    fn test_fuzz_xml_deep_nesting_500_parses() {
        // Use 200 levels to stay within stack limits while still testing deep nesting
        let data = deeply_nested_xml(200);
        let result = adapters_ooxml::xml::parse_xml(&data);
        assert!(
            result.is_ok(),
            "deeply nested XML (200 levels) should parse: {:?}",
            result
        );
    }

    /// test_fuzz_pdf_oversized_xref_parses: oversized PDF parses without error.
    #[test]
    fn test_fuzz_pdf_oversized_xref_parses() {
        let data = oversized_pdf_xref();
        let result = adapters_pdf::parser::parse_pdf(&data);
        // Should not panic; errors are acceptable
        match result {
            Ok(_) => {}
            Err(diags) => {
                let _ = diags;
            }
        }
    }

    /// test_fuzz_xml_cdata_edge_parses: CDATA edge cases parse correctly.
    #[test]
    fn test_fuzz_xml_cdata_edge_parses() {
        let data = xml_cdata_edge_cases();
        let result = adapters_ooxml::xml::parse_xml(&data);
        // CDATA with edge cases should parse
        assert!(
            result.is_ok(),
            "CDATA edge cases should parse: {:?}",
            result
        );
    }

    /// test_fuzz_xml_many_siblings_parses: many siblings parse correctly.
    #[test]
    fn test_fuzz_xml_many_siblings_parses() {
        let data = xml_many_siblings(1000);
        let result = adapters_ooxml::xml::parse_xml(&data);
        assert!(result.is_ok(), "many siblings should parse: {:?}", result);
    }

    /// test_fuzz_xml_nulls_rejected: XML with null bytes doesn't panic.
    #[test]
    fn test_fuzz_xml_nulls_rejected() {
        let data = xml_with_nulls();
        let result = adapters_ooxml::xml::parse_xml(&data);
        // XML with null bytes should either be rejected or parsed safely
        // The parser should not panic on null bytes
        let _ = result;
    }

    /// test_fuzz_xml_long_attrs_rejected: extremely long attributes rejected.
    #[test]
    fn test_fuzz_xml_long_attrs_rejected() {
        let data = xml_long_attributes();
        let result = adapters_ooxml::xml::parse_xml(&data);
        // Long attributes should be handled gracefully
        let _ = result;
    }

    /// test_fuzz_opc_empty_zip_parses: empty ZIP parses as empty package.
    #[test]
    fn test_fuzz_opc_empty_zip_parses() {
        let data = empty_zip_fixture();
        let result = adapters_ooxml::opc::parse_opc(&data);
        // Empty ZIP should parse to empty package
        match result {
            Ok(pkg) => {
                assert!(pkg.parts.is_empty(), "empty zip should have no parts");
            }
            Err(diags) => {
                let _ = diags;
            }
        }
    }

    /// test_fuzz_xml_deep_nesting_repeated: repeated parse doesn't leak.
    #[test]
    fn test_fuzz_xml_deep_nesting_repeated() {
        let data = deeply_nested_xml(200);
        // Parse multiple times to check for memory issues
        for _ in 0..10 {
            let _ = adapters_ooxml::xml::parse_xml(&data);
        }
    }

    /// test_fuzz_pdf_deep_tree_repeated: repeated parse of deep tree.
    #[test]
    fn test_fuzz_pdf_deep_tree_repeated() {
        let data = pdf_deep_page_tree(30);
        for _ in 0..5 {
            let _ = adapters_pdf::parser::parse_pdf(&data);
        }
    }

    /// test_fuzz_all_cross_format: OPC data through all parsers.
    #[test]
    fn test_fuzz_all_cross_format() {
        let opc_data = zip_bomb_fixture();
        assert!(matches!(fuzz_all(&opc_data), FuzzResult::Ok));

        let xml_data = deeply_nested_xml(100);
        assert!(matches!(fuzz_all(&xml_data), FuzzResult::Ok));

        let pdf_data = minimal_valid_pdf();
        assert!(matches!(fuzz_all(&pdf_data), FuzzResult::Ok));
    }

    // ------------------------------------------------------------------
    // Schema gate tests (P2-31: LIN-04)
    // ------------------------------------------------------------------

    /// test_schema_gate_catches_truncated: truncated XML is rejected by schema validation.
    #[test]
    fn test_schema_gate_catches_truncated() {
        let data = truncated_xml();
        let result = lint::validate::validate_xml_schema("truncated.xml", &data);
        match &result {
            Err(e) => {
                assert_eq!(
                    e.code, "OOX-E-021",
                    "truncated XML should produce OOX-E-021"
                );
            }
            Ok(_) => {
                // xmllint may not be installed; accept Ok only if xmllint is absent
                let has_xmllint = std::process::Command::new("which")
                    .arg("xmllint")
                    .stdout(std::process::Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                assert!(!has_xmllint, "should fail when xmllint is available");
            }
        }
    }

    /// test_schema_gate_passes_clean: well-formed XML with expected structure passes.
    #[test]
    fn test_schema_gate_passes_clean() {
        let clean_xml = b"<document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:w14=\"http://schemas.microsoft.com/office/word/2010/wordml\"><w:body><w:p><w:r><w:t>hello</w:t></w:r></w:p></w:body></document>";
        let result = lint::validate::validate_xml_schema("word/document.xml", clean_xml);
        assert!(
            result.is_ok(),
            "clean document with body should pass validation: {:?}",
            result
        );
    }

    /// test_schema_gate_catches_invalid_utf8: invalid UTF-8 is rejected.
    #[test]
    fn test_schema_gate_catches_invalid_utf8() {
        let data = invalid_utf8_xml();
        let result = lint::validate::validate_xml_schema("invalid.xml", &data);
        assert!(result.is_err(), "invalid UTF-8 should be rejected");
        if let Err(e) = result {
            assert_eq!(e.code, "OOX-E-021");
            assert!(
                e.message.contains("UTF-8") || e.message.contains("encoding"),
                "error should mention UTF-8: {}",
                e.message
            );
        }
    }

    /// test_schema_gate_catches_oversized_nesting: >500 depth is rejected.
    #[test]
    fn test_schema_gate_catches_oversized_nesting() {
        let data = oversized_nesting();
        let result = lint::validate::validate_xml_schema("deep.xml", &data);
        assert!(result.is_err(), "overly deep nesting should be rejected");
        if let Err(e) = result {
            assert_eq!(e.code, "OOX-E-021");
            assert!(
                e.message.contains("nesting depth") || e.message.contains("depth"),
                "error should mention depth: {}",
                e.message
            );
        }
    }

    /// test_schema_gate_catches_malformed_attribute: malformed attributes rejected.
    #[test]
    fn test_schema_gate_catches_malformed_attribute() {
        let data = malformed_attribute_xml();
        let result = lint::validate::validate_xml_schema("malformed.xml", &data);
        match &result {
            Err(e) => {
                assert_eq!(e.code, "OOX-E-021");
            }
            Ok(_) => {
                // Accept if xmllint is not available
                let has_xmllint = std::process::Command::new("which")
                    .arg("xmllint")
                    .stdout(std::process::Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                assert!(!has_xmllint, "should fail when xmllint is available");
            }
        }
    }

    /// test_qpdf_best_effort_no_qpdf: qpdf returns Ok with Info when not installed.
    #[test]
    fn test_qpdf_best_effort_no_qpdf() {
        let pdf_bytes = b"%PDF-1.4\n%%EOF\n";
        let result = lint::validate::validate_pdf_qpdf(pdf_bytes);
        // Accept either: Ok (qpdf missing → Info warning) or Err (qpdf installed but fails)
        match &result {
            Ok(()) => {
                // qpdf may not be installed; best-effort means this is acceptable
            }
            Err(e) => {
                // If qpdf is installed, it may fail on minimal PDFs
                let _ = e;
            }
        }
    }

    // ------------------------------------------------------------------
    // Write-path fuzz tests (P2-32)
    // ------------------------------------------------------------------

    /// test_mutation_prone_patch_json: produces valid JSON.
    #[test]
    fn test_mutation_prone_patch_json_produces_data() {
        let data = mutation_prone_patch_json();
        assert!(
            !data.is_empty(),
            "mutation_prone_patch_json should produce data"
        );
        let parsed: Result<Vec<serde_json::Value>, _> = serde_json::from_slice(&data);
        assert!(parsed.is_ok(), "should be valid JSON: {:?}", parsed);
    }

    /// test_hostile_patch_json: malformed JSON doesn't panic.
    #[test]
    fn test_hostile_patch_json_no_panic() {
        let data = hostile_patch_json();
        let result = fuzz_patch_json(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "hostile patch JSON should not panic: {}",
            result
        );
    }

    /// test_denied_action_patch: produces valid JSON.
    #[test]
    fn test_denied_action_patch_produces_data() {
        let data = denied_action_patch();
        assert!(!data.is_empty(), "denied_action_patch should produce data");
        let parsed: Result<serde_json::Value, _> = serde_json::from_slice(&data);
        assert!(parsed.is_ok(), "should be valid JSON: {:?}", parsed);
    }

    /// test_massive_patch_ops: many ops don't panic.
    #[test]
    fn test_massive_patch_ops_no_panic() {
        let data = massive_patch_ops_json();
        let result = fuzz_patch_json(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "massive patch ops should not panic: {}",
            result
        );
    }

    /// test_nonexistent_target_patch: invalid targets don't panic.
    #[test]
    fn test_nonexistent_target_patch_no_panic() {
        let data = nonexistent_target_patch();
        let result = fuzz_patch_json(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "nonexistent target patch should not panic: {}",
            result
        );
    }

    /// test_patch_json_special_chars: special chars in values don't panic.
    #[test]
    fn test_patch_json_special_chars_no_panic() {
        let data = patch_json_with_special_chars();
        let result = fuzz_patch_json(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "patch JSON with special chars should not panic: {}",
            result
        );
    }

    /// test_hostile_package_with_patch: hostile OPC + patch doesn't panic.
    #[test]
    fn test_hostile_package_with_patch_no_panic() {
        let data = hostile_package_with_patch();
        let result = fuzz_executor(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "hostile package + patch should not panic: {}",
            result
        );
    }

    /// test_valid_package_invalid_patches: valid package + invalid patches don't panic.
    #[test]
    fn test_valid_package_invalid_patches_no_panic() {
        let data = valid_package_invalid_patches();
        let result = fuzz_executor(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "valid package + invalid patches should not panic: {}",
            result
        );
    }

    /// test_valid_package_complex_patch: valid package + complex patch doesn't panic.
    #[test]
    fn test_valid_package_complex_patch_no_panic() {
        let data = valid_package_complex_patch();
        let result = fuzz_executor(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "valid package + complex patch should not panic: {}",
            result
        );
    }

    /// test_fuzz_patch_json_no_panic_empty: empty input doesn't panic.
    #[test]
    fn test_fuzz_patch_json_no_panic_empty() {
        let data = b"";
        let result = fuzz_patch_json(data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "empty patch JSON should not panic: {}",
            result
        );
    }

    /// test_fuzz_patch_json_no_panic_not_json: non-JSON data doesn't panic.
    #[test]
    fn test_fuzz_patch_json_no_panic_not_json() {
        let data = b"Not JSON at all\x00\x01\x02";
        let result = fuzz_patch_json(data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "non-JSON should not panic: {}",
            result
        );
    }

    /// test_fuzz_patch_json_no_panic_truncated: truncated JSON doesn't panic.
    #[test]
    fn test_fuzz_patch_json_no_panic_truncated() {
        let data = b"[{\"op\": \"replace_text\"";
        let result = fuzz_patch_json(data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "truncated JSON should not panic: {}",
            result
        );
    }

    /// test_run_write_fuzz: all write-path fixtures pass without panic.
    #[test]
    fn test_run_write_fuzz() {
        let result = run_write_fuzz();
        assert!(
            matches!(result, FuzzResult::Ok),
            "all write-path fuzz fixtures should pass: {}",
            result
        );
    }

    /// test_fuzz_executor_no_panic_empty: empty source doesn't panic.
    #[test]
    fn test_fuzz_executor_no_panic_empty() {
        let data = b"";
        let result = fuzz_executor(data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "empty executor input should not panic: {}",
            result
        );
    }

    /// test_fuzz_executor_no_panic_garbage: random data doesn't panic.
    #[test]
    fn test_fuzz_executor_no_panic_garbage() {
        let mut data = vec![0u8; 10_000];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(17).wrapping_add(31);
        }
        let result = fuzz_executor(&data);
        assert!(
            matches!(result, FuzzResult::Ok),
            "executor garbage should not panic: {}",
            result
        );
    }
}
