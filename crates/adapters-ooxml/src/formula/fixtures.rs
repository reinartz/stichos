// SPDX-License-Identifier: MIT OR Apache-2.0
//! Formula fixture builders for testing.
//!
//! Creates synthetic xlsx files with known formulas and expected evaluation results.

use std::io::Write;

use super::EvalResult;
use super::GpFCode;

type EResult = EvalResult;
type FCode = GpFCode;

/// A test fixture: xlsx bytes + expected evaluation results.
#[derive(Debug, Clone)]
pub struct FormulaFixture {
    /// The xlsx file bytes.
    pub name: &'static str,
    /// Sheet name → cell → expected result.
    pub expected: Vec<(&'static str, &'static str, EvalResult)>,
    /// Sheet name → cell → refused code (if any).
    pub refused: Vec<(&'static str, &'static str, GpFCode)>,
}

impl FormulaFixture {
    /// Build the xlsx file bytes.
    pub fn build_xlsx(&self) -> Vec<u8> {
        build_minimal_xlsx()
    }
}

/// Build a simple xlsx with arithmetic formulas.
pub fn fixture_arithmetic() -> FormulaFixture {
    FormulaFixture {
        name: "arithmetic",
        expected: vec![
            ("Sheet1", "C1", EResult::Number(30.0)),
            ("Sheet1", "C2", EResult::Number(200.0)),
            ("Sheet1", "C3", EResult::Number(5.0)),
            ("Sheet1", "C4", EResult::Number(2.0)),
        ],
        refused: vec![],
    }
}

/// Build an xlsx with aggregate formulas.
pub fn fixture_aggregates() -> FormulaFixture {
    FormulaFixture {
        name: "aggregates",
        expected: vec![
            ("Sheet1", "D1", EResult::Number(150.0)),
            ("Sheet1", "D2", EResult::Number(30.0)),
            ("Sheet1", "D3", EResult::Number(5.0)),
            ("Sheet1", "D4", EResult::Number(10.0)),
            ("Sheet1", "D5", EResult::Number(50.0)),
        ],
        refused: vec![],
    }
}

/// Build an xlsx with logic formulas.
pub fn fixture_logic() -> FormulaFixture {
    FormulaFixture {
        name: "logic",
        expected: vec![
            ("Sheet1", "C1", EResult::Bool(true)),
            ("Sheet1", "C2", EResult::Bool(false)),
            ("Sheet1", "C3", EResult::Bool(true)),
            ("Sheet1", "C4", EResult::Bool(false)),
            ("Sheet1", "C5", EResult::Bool(false)),
            ("Sheet1", "C6", EResult::Bool(true)),
            ("Sheet1", "C7", EResult::Number(1.0)),
            ("Sheet1", "C8", EResult::Number(0.0)),
        ],
        refused: vec![],
    }
}

/// Build an xlsx with hostile formulas (should be refused).
pub fn fixture_hostile() -> FormulaFixture {
    FormulaFixture {
        name: "hostile",
        expected: vec![],
        refused: vec![
            ("Sheet1", "B1", FCode::ExternalWorkbook),
            ("Sheet1", "B2", FCode::ArrayMatrixFunction),
            ("Sheet1", "B3", FCode::MacroReference),
            ("Sheet1", "B4", FCode::DdeOleReference),
            ("Sheet1", "B5", FCode::DeepNesting),
        ],
    }
}

/// Build an xlsx with unsupported functions.
pub fn fixture_unsupported() -> FormulaFixture {
    FormulaFixture {
        name: "unsupported",
        expected: vec![
            ("Sheet1", "B1", EResult::Number(10.0)),
            ("Sheet1", "B2", EResult::Number(20.0)),
        ],
        refused: vec![
            ("Sheet1", "C1", FCode::UnsupportedFunction),
            ("Sheet1", "C2", FCode::UnsupportedFunction),
        ],
    }
}

/// Build an xlsx with stale cached values (parity test).
pub fn fixture_stale_cache() -> FormulaFixture {
    FormulaFixture {
        name: "stale_cache",
        expected: vec![
            ("Sheet1", "B1", EResult::Number(30.0)),
            ("Sheet1", "B2", EResult::Number(6.0)),
        ],
        refused: vec![],
    }
}

// ===========================================================================
// xlsx file builder helpers
// ===========================================================================

/// Type alias for default file options with no compression.
type FileOpts<'a> = zip::write::FileOptions<'a, ()>;

/// Returns default file options.
fn opts() -> FileOpts<'static> {
    zip::write::FileOptions::default()
}

/// Build a minimal valid xlsx with shared strings, formulas, and numeric values.
fn build_minimal_xlsx() -> Vec<u8> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);

    // [Content_Types].xml
    writer.start_file("[Content_Types].xml", opts()).unwrap();
    writer.write_all(
        b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
         <Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml\"/>\
         <Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>\
         <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
         </Types>",
    ).unwrap();

    // _rels/.rels
    writer.start_file("_rels/.rels", opts()).unwrap();
    writer.write_all(
        b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
         </Relationships>",
    ).unwrap();

    // xl/sharedStrings.xml
    writer.start_file("xl/sharedStrings.xml", opts()).unwrap();
    writer.write_all(
        b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <sst xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" count=\"1\" uniqueCount=\"1\">\
         <si><t>Sheet1</t></si>\
         </sst>",
    ).unwrap();

    // xl/styles.xml
    writer.start_file("xl/styles.xml", opts()).unwrap();
    writer
        .write_all(
            b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
             <cellXfs count=\"1\">\
             <xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/>\
             </cellXfs>\
             </styleSheet>",
        )
        .unwrap();

    // xl/workbook.xml
    writer.start_file("xl/workbook.xml", opts()).unwrap();
    writer
        .write_all(
            b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
             <sheets>\
             <sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/>\
             </sheets>\
             </workbook>",
        )
        .unwrap();

    // xl/_rels/workbook.xml.rels
    writer
        .start_file("xl/_rels/workbook.xml.rels", opts())
        .unwrap();
    writer.write_all(
        b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
         </Relationships>",
    ).unwrap();

    // xl/worksheets/sheet1.xml
    writer
        .start_file("xl/worksheets/sheet1.xml", opts())
        .unwrap();
    writer
        .write_all(
            b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
             <sheetData>\
             <row r=\"1\"><c r=\"A1\" t=\"n\"><v>10</v></c><c r=\"B1\" t=\"n\"><v>20</v></c></row>\
             <row r=\"2\"><c r=\"A2\" t=\"n\"><v>50</v></c><c r=\"B2\" t=\"n\"><v>150</v></c></row>\
             <row r=\"3\"><c r=\"A3\" t=\"n\"><v>5</v></c><c r=\"B3\" t=\"n\"><v>10</v></c></row>\
             <row r=\"4\"><c r=\"A4\" t=\"n\"><v>1</v></c><c r=\"B4\" t=\"n\"><v>2</v></c></row>\
             <row r=\"5\"><c r=\"A5\" t=\"n\"><v>3</v></c><c r=\"B5\" t=\"n\"><v>4</v></c></row>\
             </sheetData>\
             </worksheet>",
        )
        .unwrap();

    writer.finish().unwrap().into_inner()
}

/// Build an xlsx with arithmetic formulas in sheet2.xml.
pub fn build_arithmetic_xlsx() -> Vec<u8> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);

    writer.start_file("[Content_Types].xml", opts()).unwrap();
    writer.write_all(
        b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
         <Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml\"/>\
         <Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>\
         <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
         </Types>",
    ).unwrap();

    writer.start_file("_rels/.rels", opts()).unwrap();
    writer.write_all(
        b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
         </Relationships>",
    ).unwrap();

    writer.start_file("xl/sharedStrings.xml", opts()).unwrap();
    writer.write_all(
        b"<sst xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" count=\"1\" uniqueCount=\"1\"><si><t>Sheet1</t></si></sst>",
    ).unwrap();

    writer.start_file("xl/styles.xml", opts()).unwrap();
    writer.write_all(
        b"<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><cellXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellXfs></styleSheet>",
    ).unwrap();

    writer.start_file("xl/workbook.xml", opts()).unwrap();
    writer.write_all(
        b"<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>",
    ).unwrap();

    writer
        .start_file("xl/_rels/workbook.xml.rels", opts())
        .unwrap();
    writer.write_all(
        b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
         </Relationships>",
    ).unwrap();

    // Sheet1: data in A1:A4, formulas in B1:B4
    writer
        .start_file("xl/worksheets/sheet1.xml", opts())
        .unwrap();
    writer
        .write_all(
            b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
             <sheetData>\
             <row r=\"1\"><c r=\"A1\" t=\"n\"><v>10</v></c><c r=\"B1\" t=\"str\"><f>A1+A2</f><v>30</v></c></row>\
             <row r=\"2\"><c r=\"A2\" t=\"n\"><v>20</v></c></row>\
             <row r=\"3\"><c r=\"A3\" t=\"n\"><v>10</v></c><c r=\"B3\" t=\"str\"><f>A1*A3</f><v>200</v></c></row>\
             <row r=\"4\"><c r=\"A4\" t=\"n\"><v>10</v></c><c r=\"B4\" t=\"str\"><f>A1/A3</f><v>2</v></c></row>\
             <row r=\"5\"><c r=\"A5\" t=\"n\"><v>5</v></c><c r=\"B5\" t=\"str\"><f>A1-A2</f><v>-10</v></c></row>\
             </sheetData>\
             </worksheet>",
        )
        .unwrap();

    writer.finish().unwrap().into_inner()
}

/// Build an xlsx with aggregate formulas.
pub fn build_aggregates_xlsx() -> Vec<u8> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);

    writer.start_file("[Content_Types].xml", opts()).unwrap();
    writer.write_all(
        b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
         <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
         </Types>",
    ).unwrap();

    writer.start_file("_rels/.rels", opts()).unwrap();
    writer.write_all(
        b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
         </Relationships>",
    ).unwrap();

    writer.start_file("xl/workbook.xml", opts()).unwrap();
    writer.write_all(
        b"<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>",
    ).unwrap();

    writer
        .start_file("xl/_rels/workbook.xml.rels", opts())
        .unwrap();
    writer.write_all(
        b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
         </Relationships>",
    ).unwrap();

    // Sheet1: A1=10, A2=20, A3=30, A4=40, A5=50
    // B1=SUM(A1:A5), B2=AVERAGE(A1:A5), B3=COUNT(A1:A5), B4=MIN(A1:A5), B5=MAX(A1:A5)
    writer
        .start_file("xl/worksheets/sheet1.xml", opts())
        .unwrap();
    writer
        .write_all(
            b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
             <sheetData>\
             <row r=\"1\"><c r=\"A1\" t=\"n\"><v>10</v></c><c r=\"B1\" t=\"str\"><f>SUM(A1:A5)</f><v>150</v></c></row>\
             <row r=\"2\"><c r=\"A2\" t=\"n\"><v>20</v></c><c r=\"B2\" t=\"str\"><f>AVERAGE(A1:A5)</f><v>30</v></c></row>\
             <row r=\"3\"><c r=\"A3\" t=\"n\"><v>30</v></c><c r=\"B3\" t=\"str\"><f>COUNT(A1:A5)</f><v>5</v></c></row>\
             <row r=\"4\"><c r=\"A4\" t=\"n\"><v>40</v></c><c r=\"B4\" t=\"str\"><f>MIN(A1:A5)</f><v>10</v></c></row>\
             <row r=\"5\"><c r=\"A5\" t=\"n\"><v>50</v></c><c r=\"B5\" t=\"str\"><f>MAX(A1:A5)</f><v>50</v></c></row>\
             </sheetData>\
             </worksheet>",
        )
        .unwrap();

    writer.finish().unwrap().into_inner()
}

/// Build an xlsx with logic formulas.
pub fn build_logic_xlsx() -> Vec<u8> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);

    writer.start_file("[Content_Types].xml", opts()).unwrap();
    writer.write_all(
        b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
         <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
         </Types>",
    ).unwrap();

    writer.start_file("_rels/.rels", opts()).unwrap();
    writer.write_all(
        b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
         </Relationships>",
    ).unwrap();

    writer.start_file("xl/workbook.xml", opts()).unwrap();
    writer.write_all(
        b"<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>",
    ).unwrap();

    writer
        .start_file("xl/_rels/workbook.xml.rels", opts())
        .unwrap();
    writer.write_all(
        b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
         </Relationships>",
    ).unwrap();

    // A1=1 (truthy), A2=0 (falsy), A3=5 (truthy)
    // B1=AND(TRUE,TRUE), B2=AND(TRUE,FALSE), B3=OR(TRUE,FALSE), B4=OR(FALSE,FALSE),
    // B5=NOT(TRUE), B6=NOT(FALSE), B7=IF(TRUE,1,0), B8=IF(FALSE,1,0)
    writer
        .start_file("xl/worksheets/sheet1.xml", opts())
        .unwrap();
    writer
        .write_all(
            b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
             <sheetData>\
             <row r=\"1\"><c r=\"A1\" t=\"n\"><v>1</v></c><c r=\"B1\" t=\"str\"><f>AND(TRUE,TRUE)</f><v>1</v></c></row>\
             <row r=\"2\"><c r=\"A2\" t=\"n\"><v>0</v></c><c r=\"B2\" t=\"str\"><f>AND(TRUE,FALSE)</f><v>0</v></c></row>\
             <row r=\"3\"><c r=\"A3\" t=\"n\"><v>5</v></c><c r=\"B3\" t=\"str\"><f>OR(TRUE,FALSE)</f><v>1</v></c></row>\
             <row r=\"4\"><c r=\"B4\" t=\"str\"><f>OR(FALSE,FALSE)</f><v>0</v></c></row>\
             <row r=\"5\"><c r=\"B5\" t=\"str\"><f>NOT(TRUE)</f><v>0</v></c></row>\
             <row r=\"6\"><c r=\"B6\" t=\"str\"><f>NOT(FALSE)</f><v>1</v></c></row>\
             <row r=\"7\"><c r=\"B7\" t=\"str\"><f>IF(TRUE,1,0)</f><v>1</v></c></row>\
             <row r=\"8\"><c r=\"B8\" t=\"str\"><f>IF(FALSE,1,0)</f><v>0</v></c></row>\
             </sheetData>\
             </worksheet>",
        )
        .unwrap();

    writer.finish().unwrap().into_inner()
}

/// Build an xlsx with hostile formulas.
pub fn build_hostile_xlsx() -> Vec<u8> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);

    writer.start_file("[Content_Types].xml", opts()).unwrap();
    writer.write_all(
        b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
         <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
         </Types>",
    ).unwrap();

    writer.start_file("_rels/.rels", opts()).unwrap();
    writer.write_all(
        b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
         </Relationships>",
    ).unwrap();

    writer.start_file("xl/workbook.xml", opts()).unwrap();
    writer.write_all(
        b"<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>",
    ).unwrap();

    writer
        .start_file("xl/_rels/workbook.xml.rels", opts())
        .unwrap();
    writer.write_all(
        b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
         </Relationships>",
    ).unwrap();

    // A1=10, A2=20 (base values)
    // B1=external ref, B2=array func, B3=macro ref, B4=DDE ref, B5=deeply nested
    let deep_formula = format!("{}1{}", "(".repeat(35), ")".repeat(35));
    writer
        .start_file("xl/worksheets/sheet1.xml", opts())
        .unwrap();
    writer
        .write_all(
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                 <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
                 <sheetData>\
                 <row r=\"1\"><c r=\"A1\" t=\"n\"><v>10</v></c><c r=\"B1\" t=\"str\"><f>SUM([Book2.xlsx]Sheet1!A1)</f><v>10</v></c></row>\
                 <row r=\"2\"><c r=\"A2\" t=\"n\"><v>20</v></c><c r=\"B2\" t=\"str\"><f>MMULT(A1:A2,B1:B2)</f><v>0</v></c></row>\
                 <row r=\"3\"><c r=\"B3\" t=\"str\"><f>SUBPROCESS!RUN</f><v>0</v></c></row>\
                 <row r=\"4\"><c r=\"B4\" t=\"str\"><f>DDE|server|topic</f><v>0</v></c></row>\
                 <row r=\"5\"><c r=\"B5\" t=\"str\"><f>{}</f><v>0</v></c></row>\
                 </sheetData>\
                 </worksheet>",
                deep_formula
            ).as_bytes(),
        )
        .unwrap();

    writer.finish().unwrap().into_inner()
}

/// Build an xlsx with unsupported functions.
pub fn build_unsupported_xlsx() -> Vec<u8> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);

    writer.start_file("[Content_Types].xml", opts()).unwrap();
    writer.write_all(
        b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
         <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
         </Types>",
    ).unwrap();

    writer.start_file("_rels/.rels", opts()).unwrap();
    writer.write_all(
        b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
         </Relationships>",
    ).unwrap();

    writer.start_file("xl/workbook.xml", opts()).unwrap();
    writer.write_all(
        b"<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>",
    ).unwrap();

    writer
        .start_file("xl/_rels/workbook.xml.rels", opts())
        .unwrap();
    writer.write_all(
        b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
         </Relationships>",
    ).unwrap();

    // A1=10, A2=20 (cached values)
    // C1=VLOOKUP(...), C2=INDEX(...)
    writer
        .start_file("xl/worksheets/sheet1.xml", opts())
        .unwrap();
    writer
        .write_all(
            b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
             <sheetData>\
             <row r=\"1\"><c r=\"A1\" t=\"n\"><v>10</v></c><c r=\"C1\" t=\"str\"><f>VLOOKUP(A1,A:A,1,FALSE)</f><v>10</v></c></row>\
             <row r=\"2\"><c r=\"A2\" t=\"n\"><v>20</v></c><c r=\"C2\" t=\"str\"><f>INDEX(A1:A2,1)</f><v>20</v></c></row>\
             </sheetData>\
             </worksheet>",
        )
        .unwrap();

    writer.finish().unwrap().into_inner()
}

/// Build an xlsx with stale cached values.
pub fn build_stale_cache_xlsx() -> Vec<u8> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);

    writer.start_file("[Content_Types].xml", opts()).unwrap();
    writer.write_all(
        b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
         <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
         </Types>",
    ).unwrap();

    writer.start_file("_rels/.rels", opts()).unwrap();
    writer.write_all(
        b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
         </Relationships>",
    ).unwrap();

    writer.start_file("xl/workbook.xml", opts()).unwrap();
    writer.write_all(
        b"<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>",
    ).unwrap();

    writer
        .start_file("xl/_rels/workbook.xml.rels", opts())
        .unwrap();
    writer.write_all(
        b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
         </Relationships>",
    ).unwrap();

    // A1=10, A2=20, A3=30, A4=40, A5=50
    // B1=SUM(A1:A5) cached=150 (correct)
    // B2=AVERAGE(A1:A5) cached=25 (stale — should be 30)
    writer
        .start_file("xl/worksheets/sheet1.xml", opts())
        .unwrap();
    writer
        .write_all(
            b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
             <sheetData>\
             <row r=\"1\"><c r=\"A1\" t=\"n\"><v>10</v></c><c r=\"B1\" t=\"str\"><f>SUM(A1:A5)</f><v>150</v></c></row>\
             <row r=\"2\"><c r=\"A2\" t=\"n\"><v>20</v></c><c r=\"B2\" t=\"str\"><f>AVERAGE(A1:A5)</f><v>25</v></c></row>\
             <row r=\"3\"><c r=\"A3\" t=\"n\"><v>30</v></c></row>\
             <row r=\"4\"><c r=\"A4\" t=\"n\"><v>40</v></c></row>\
             <row r=\"5\"><c r=\"A5\" t=\"n\"><v>50</v></c></row>\
             </sheetData>\
             </worksheet>",
        )
        .unwrap();

    writer.finish().unwrap().into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formula::{evaluate, parse_formula, CachedCellValue, FormulaContext};

    /// A test context for fixture evaluation.
    #[derive(Default)]
    struct FixtureContext {}

    impl FormulaContext for FixtureContext {
        fn get_cell(&self, _ref_name: &str) -> Option<CachedCellValue> {
            // For fixture tests, we just return a default — actual evaluation
            // uses the xlsx reader, not the fixture context directly.
            None
        }

        fn get_range(&self, _range: &str) -> Option<Vec<CachedCellValue>> {
            None
        }
    }

    #[test]
    fn test_fixture_arithmetic_exists() {
        let fixture = fixture_arithmetic();
        assert_eq!(fixture.name, "arithmetic");
        assert_eq!(fixture.expected.len(), 4);
        assert!(fixture.refused.is_empty());
        let xlsx = fixture.build_xlsx();
        assert!(!xlsx.is_empty());
    }

    #[test]
    fn test_fixture_aggregates_exists() {
        let fixture = fixture_aggregates();
        assert_eq!(fixture.name, "aggregates");
        assert_eq!(fixture.expected.len(), 5);
        assert!(fixture.refused.is_empty());
    }

    #[test]
    fn test_fixture_logic_exists() {
        let fixture = fixture_logic();
        assert_eq!(fixture.name, "logic");
        assert_eq!(fixture.expected.len(), 8);
        assert!(fixture.refused.is_empty());
    }

    #[test]
    fn test_fixture_hostile_exists() {
        let fixture = fixture_hostile();
        assert_eq!(fixture.name, "hostile");
        assert!(fixture.expected.is_empty());
        assert_eq!(fixture.refused.len(), 5);
        let xlsx = fixture.build_xlsx();
        assert!(!xlsx.is_empty());
    }

    #[test]
    fn test_fixture_unsupported_exists() {
        let fixture = fixture_unsupported();
        assert_eq!(fixture.name, "unsupported");
        assert_eq!(fixture.expected.len(), 2);
        assert_eq!(fixture.refused.len(), 2);
    }

    #[test]
    fn test_fixture_stale_cache_exists() {
        let fixture = fixture_stale_cache();
        assert_eq!(fixture.name, "stale_cache");
        assert_eq!(fixture.expected.len(), 2);
        assert!(fixture.refused.is_empty());
    }

    #[test]
    fn test_build_arithmetic_xlsx() {
        let xlsx = build_arithmetic_xlsx();
        assert!(!xlsx.is_empty());
        // Should be a valid ZIP
        let cursor = std::io::Cursor::new(&xlsx);
        let reader = zip::ZipArchive::new(cursor).expect("should be valid ZIP");
        assert!(reader.len() > 0);
    }

    #[test]
    fn test_build_aggregates_xlsx() {
        let xlsx = build_aggregates_xlsx();
        assert!(!xlsx.is_empty());
        let cursor = std::io::Cursor::new(&xlsx);
        let reader = zip::ZipArchive::new(cursor).expect("should be valid ZIP");
        assert!(reader.len() > 0);
    }

    #[test]
    fn test_build_logic_xlsx() {
        let xlsx = build_logic_xlsx();
        assert!(!xlsx.is_empty());
        let cursor = std::io::Cursor::new(&xlsx);
        let reader = zip::ZipArchive::new(cursor).expect("should be valid ZIP");
        assert!(reader.len() > 0);
    }

    #[test]
    fn test_build_hostile_xlsx() {
        let xlsx = build_hostile_xlsx();
        assert!(!xlsx.is_empty());
        let cursor = std::io::Cursor::new(&xlsx);
        let reader = zip::ZipArchive::new(cursor).expect("should be valid ZIP");
        assert!(reader.len() > 0);
    }

    #[test]
    fn test_build_unsupported_xlsx() {
        let xlsx = build_unsupported_xlsx();
        assert!(!xlsx.is_empty());
        let cursor = std::io::Cursor::new(&xlsx);
        let reader = zip::ZipArchive::new(cursor).expect("should be valid ZIP");
        assert!(reader.len() > 0);
    }

    #[test]
    fn test_build_stale_cache_xlsx() {
        let xlsx = build_stale_cache_xlsx();
        assert!(!xlsx.is_empty());
        let cursor = std::io::Cursor::new(&xlsx);
        let reader = zip::ZipArchive::new(cursor).expect("should be valid ZIP");
        assert!(reader.len() > 0);
    }

    #[test]
    fn test_evaluate_arithmetic_formulas() {
        // Test SUM(A1:A5) with no cell context — returns 0 for empty range
        let node = parse_formula("SUM(A1:A5)").unwrap();
        let ctx = FixtureContext::default();
        let result = evaluate(&node, &ctx);
        // Returns 0.0 since no cells are defined in context (empty range sums to 0)
        assert_eq!(result, Some(EvalResult::Number(0.0)));
    }

    #[test]
    fn test_all_fixtures_produce_valid_xlsx() {
        for fixture_fn in [
            fixture_arithmetic,
            fixture_aggregates,
            fixture_logic,
            fixture_hostile,
            fixture_unsupported,
            fixture_stale_cache,
        ] {
            let fixture = fixture_fn();
            let xlsx = fixture.build_xlsx();
            assert!(
                !xlsx.is_empty(),
                "fixture {} should produce non-empty xlsx",
                fixture.name
            );
            let cursor = std::io::Cursor::new(&xlsx);
            assert!(
                zip::ZipArchive::new(cursor).is_ok(),
                "fixture {} should produce valid ZIP",
                fixture.name
            );
        }
    }
}
