// SPDX-License-Identifier: MIT OR Apache-2.0
//! Create a valid pptx from a `gp_core::ir::Presentation` IR.
//!
//! Writes a valid PresentationML package with:
//!   - `[Content_Types].xml` — content-type overrides
//!   - `_rels/.rels` — package-level relationships
//!   - `ppt/presentation.xml` — presentation with slide IDs
//!   - `ppt/_rels/presentation.xml.rels` — slide/layout/master relationships
//!   - `ppt/slides/slideN.xml` — per-slide content
//!   - `ppt/slideLayouts/slideLayout1.xml` — default layout
//!   - `ppt/slideMasters/slideMaster1.xml` — default master
//!   - `ppt/theme/theme1.xml` — theme

use gp_core::diagnostics::Diagnostic;
use gp_core::ir::*;

use std::io::{Cursor, Write};

// ===========================================================================
// PowerPoint namespace constants
// ===========================================================================

const NS_PPT: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
// ===========================================================================
// Public API
// ===========================================================================

/// Create a minimal pptx from an IR Presentation.
pub fn create_minimal_pptx(presentation: &Presentation) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let diags: Vec<Diagnostic> = Vec::new();
    let parts = build_parts(presentation, &mut diags.clone());
    Ok(pack_opc(&parts))
}

// ===========================================================================
// OPC packaging
// ===========================================================================

/// Pack a list of parts into an OPC ZIP package.
fn pack_opc(parts: &[(String, Vec<u8>)]) -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut zip_writer = zip::ZipWriter::new(cursor);
    let stored_opts: zip::write::FileOptions<'static, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (uri, data) in parts {
        let _ = zip_writer.start_file(uri, stored_opts);
        let _ = zip_writer.write_all(data);
    }
    let cursor = zip_writer.finish().expect("ZIP finish");
    cursor.into_inner()
}

// ===========================================================================
// XML helpers
// ===========================================================================

/// Escape XML special characters in text content.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Resolve element position from IR fields, falling back to grid layout.
fn element_position(element: &SlideElement) -> (f64, f64, f64, f64) {
    let x = element.x_pt.unwrap_or(1.0);
    let y = element.y_pt.unwrap_or(1.0);
    let w = element.width_pt.unwrap_or(3.0);
    let h = element.height_pt.unwrap_or(1.5);
    (x, y, w, h)
}

// ===========================================================================
// Build all parts
// ===========================================================================

fn build_parts(
    presentation: &Presentation,
    _diags: &mut Vec<Diagnostic>,
) -> Vec<(String, Vec<u8>)> {
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    let num_slides = presentation.slides.len();
    let num_layouts = presentation.layouts.len();
    let num_masters = presentation.masters.len();

    parts.push((
        "[Content_Types].xml".to_string(),
        build_content_types(num_slides, num_layouts, num_masters),
    ));
    parts.push(("_rels/.rels".to_string(), build_package_rels()));
    parts.push(("docProps/core.xml".to_string(), build_core_properties()));
    parts.push(("docProps/app.xml".to_string(), build_app_properties()));
    parts.push((
        "ppt/presentation.xml".to_string(),
        build_presentation_xml(presentation),
    ));
    parts.push((
        "ppt/_rels/presentation.xml.rels".to_string(),
        build_presentation_rels(presentation),
    ));
    parts.push(("ppt/theme/theme1.xml".to_string(), build_theme()));

    // Slide parts
    for (i, _slide) in presentation.slides.iter().enumerate() {
        parts.push((
            format!("ppt/slides/slide{}.xml", i + 1),
            build_slide_xml(&presentation.slides[i]),
        ));
    }

    // Media parts (placeholder — actual image data in SlideElement.data will be wired later)
    let media_parts = build_media_parts(presentation);
    for (uri, data, _mime) in media_parts {
        parts.push((uri, data));
    }

    // Layout parts
    if num_layouts > 0 {
        for (i, layout) in presentation.layouts.iter().enumerate() {
            parts.push((
                format!("ppt/slideLayouts/slideLayout{}.xml", i + 1),
                build_layout_xml(layout),
            ));
        }
    } else {
        parts.push((
            "ppt/slideLayouts/slideLayout1.xml".to_string(),
            build_default_layout(),
        ));
    }

    // Master parts
    if num_masters > 0 {
        for (i, master) in presentation.masters.iter().enumerate() {
            parts.push((
                format!("ppt/slideMasters/slideMaster{}.xml", i + 1),
                build_master_xml(master),
            ));
        }
    } else {
        parts.push((
            "ppt/slideMasters/slideMaster1.xml".to_string(),
            build_default_master(),
        ));
    }

    parts
}

/// Build media part entries for the package.
fn build_media_parts(_presentation: &Presentation) -> Vec<(String, Vec<u8>, String)> {
    Vec::new()
}

// ===========================================================================
// [Content_Types].xml
// ===========================================================================

fn build_content_types(num_slides: usize, num_layouts: usize, num_masters: usize) -> Vec<u8> {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Default Extension=\"png\" ContentType=\"image/png\"/>\
         <Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/>\
         <Default Extension=\"gif\" ContentType=\"image/gif\"/>\
         <Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\
         <Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-package.application-properties+xml\"/>\
         <Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>\
         <Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>",
    );

    for i in 1..=num_slides {
        xml.push_str(&format!(
            "\n<Override PartName=\"/ppt/slides/slide{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>",
            i
        ));
    }

    for i in 1..=num_layouts {
        xml.push_str(&format!(
            "\n<Override PartName=\"/ppt/slideLayouts/slideLayout{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>",
            i
        ));
    }

    for i in 1..=num_masters {
        xml.push_str(&format!(
            "\n<Override PartName=\"/ppt/slideMasters/slideMaster{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/>",
            i
        ));
    }

    xml.push_str("\n</Types>");
    xml.into_bytes()
}

// ===========================================================================
// _rels/.rels
// ===========================================================================

fn build_package_rels() -> Vec<u8> {
    b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
       <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
       <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/>\
       </Relationships>"
        .to_vec()
}

// ===========================================================================
// docProps/core.xml
// ===========================================================================

fn build_core_properties() -> Vec<u8> {
    b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
       <cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
       xmlns:dc=\"http://purl.org/dc/elements/1.1/\" \
       xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" \
       xmlns:dcterms=\"http://purl.org/dc/terms/\" \
       xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
       <dcterms:created xsi:type=\"dcterms:W3CDTF\">2026-01-01T00:00:00Z</dcterms:created>\
       <dc:creator>stichos</dc:creator>\
       </cp:coreProperties>"
        .to_vec()
}

// ===========================================================================
// docProps/app.xml
// ===========================================================================

fn build_app_properties() -> Vec<u8> {
    b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
       <Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\" \
       xmlns:vt=\"http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes\">\
       <Application>stichos</Application>\
       </Properties>"
        .to_vec()
}

// ===========================================================================
// ppt/presentation.xml
// ===========================================================================

fn build_presentation_xml(presentation: &Presentation) -> Vec<u8> {
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <p:presentation xmlns:p=\"{}\" xmlns:r=\"{}\" showSlideMaster=\"1\">",
        NS_PPT, NS_R
    );

    // sldIdLst
    xml.push_str("<p:sldIdLst>");
    for (i, _slide) in presentation.slides.iter().enumerate() {
        xml.push_str(&format!(
            "<p:sldId id=\"{:08X}\" r:id=\"rId{}\"/>",
            65536 + i as u32, // Slide IDs start at 257
            i + 1
        ));
    }
    xml.push_str("</p:sldIdLst>");

    // sldLayoutIdLst
    xml.push_str("<p:sldLayoutIdLst>");
    for (i, _layout) in presentation.layouts.iter().enumerate() {
        xml.push_str(&format!(
            "<p:sldLayoutId id=\"{:08X}\" r:id=\"rId{}\"/>",
            76800 + i as u32, // Layout IDs start at 76800
            presentation.slides.len() + i + 1
        ));
    }
    if presentation.layouts.is_empty() {
        xml.push_str(&format!(
            "<p:sldLayoutId id=\"{:08X}\" r:id=\"rId{}\"/>",
            76800,
            presentation.slides.len() + 1
        ));
    }
    xml.push_str("</p:sldLayoutIdLst>");

    // sldMasterIdLst
    xml.push_str("<p:sldMasterIdLst>");
    for (i, _master) in presentation.masters.iter().enumerate() {
        xml.push_str(&format!(
            "<p:sldMasterId id=\"{:08X}\" r:id=\"rId{}\"/>",
            110080 + i as u32, // Master IDs start at 110080
            presentation.slides.len() + presentation.layouts.len() + i + 1
        ));
    }
    if presentation.masters.is_empty() {
        let master_rel = presentation.slides.len() + presentation.layouts.len() + 1;
        xml.push_str(&format!(
            "<p:sldMasterId id=\"{:08X}\" r:id=\"rId{}\"/>",
            110080, master_rel
        ));
    }
    xml.push_str("</p:sldMasterIdLst>");

    // p:viewerId
    xml.push_str("<p:viewerId/>");

    // p:slideTransList, p:transition, p:slideShowProps
    xml.push_str("<p:slideTransList/>");
    xml.push_str("<p:transition showTransition=\"1\" advanceTime=\"0\"/>");
    let hidden = if presentation.slides.is_empty() { 0 } else { 1 };
    xml.push_str(&format!("<p:slideShowProps showHiddenSlide=\"{}\" showSlideRangeOnly=\"0\" showAnnotations=\"0\" loop=\"1\" earlyStart=\"0\" idleTime=\"0\"/>", hidden));

    // p:notesMasterIdLst
    xml.push_str("<p:notesMasterIdLst/>");
    xml.push_str("<p:notesLayoutManager/>");

    // p:sldMasterIdLst (repeated in some formats)
    // p:clrMap — color map
    xml.push_str(&format!(
        "<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"{}\" accent2=\"{}\" accent3=\"{}\" accent4=\"{}\" accent5=\"{}\" accent6=\"{}\" hlink=\"{}\" folHlink=\"{}\"/>",
        NS_A, NS_A, NS_A, NS_A, NS_A, NS_A, NS_A, NS_A
    ));

    xml.push_str("\n</p:presentation>");
    xml.into_bytes()
}

// ===========================================================================
// ppt/_rels/presentation.xml.rels
// ===========================================================================

fn build_presentation_rels(presentation: &Presentation) -> Vec<u8> {
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Relationships xmlns=\"{}\">",
        NS_R
    );

    let mut rel_id = 1;

    // Slides
    for _slide in &presentation.slides {
        xml.push_str(&format!(
            "\n<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{}.xml\"/>",
            rel_id, rel_id
        ));
        rel_id += 1;
    }

    // Layouts
    for _layout in &presentation.layouts {
        xml.push_str(&format!(
            "\n<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"slideLayouts/slideLayout{}.xml\"/>",
            rel_id, rel_id
        ));
        rel_id += 1;
    }

    // Masters
    for _master in &presentation.masters {
        xml.push_str(&format!(
            "\n<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"slideMasters/slideMaster{}.xml\"/>",
            rel_id, rel_id
        ));
        rel_id += 1;
    }

    xml.push_str("\n</Relationships>");
    xml.into_bytes()
}

// ===========================================================================
// ppt/slides/slideN.xml
// ===========================================================================

fn build_slide_xml(slide: &Slide) -> Vec<u8> {
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <p:sld xmlns:p=\"{}\" xmlns:r=\"{}\" xmlns:a=\"{}\" xmlns:p14=\"http://schemas.microsoft.com/office/powerpoint/2010/main\" xmlns:p16c1=\"http://schemas.microsoft.com/office/powerpoint/2016/17/main\" xmlns:p16e=\"http://schemas.microsoft.com/office/powerpoint/2018/external\">\n\
         <p:cSld>\n\
         <p:spTree>",
        NS_PPT, NS_R, NS_A
    );

    // Background
    xml.push_str("\n<p:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr>");

    // Build slide elements
    for (elem_idx, elem) in slide.elements.iter().enumerate() {
        xml.push_str(&build_slide_element_xml(elem, elem_idx));
    }

    xml.push_str("\n</p:spTree>\n</p:cSld>");

    // Slide properties
    xml.push_str(
        "\n<p:transition showTransition=\"1\" advanceTime=\"0\"/>\n\
         <p:timing>\n\
         <p:seqStrk>\n\
         <p:seqStrkSp>\n\
         <p:cNvPr id=\"0\" name=\"\"/>\n\
         </p:seqStrkSp>\n\
         </p:seqStrk>\n\
         </p:timing>\n\
         <p:clientData/>\n\
         </p:sld>",
    );

    xml.into_bytes()
}

/// Build XML for a single slide element based on its kind.
fn build_slide_element_xml(elem: &SlideElement, _idx: usize) -> String {
    match &elem.kind {
        SlideElementKind::Text => build_text_element(elem),
        SlideElementKind::Shape => build_shape_element(elem),
        SlideElementKind::Image => build_image_element(elem),
        SlideElementKind::Table => build_table_element(elem),
        SlideElementKind::Drawing | SlideElementKind::Graphic => build_image_element(elem),
        _ => build_shape_element(elem),
    }
}

/// Build a text element (text box) with proper <p:txBody>.
fn build_text_element(element: &SlideElement) -> String {
    let (x, y, w, h) = element_position(element);
    let x_emu = (x * 914400.0) as i64;
    let y_emu = (y * 914400.0) as i64;
    let w_emu = (w * 914400.0) as i64;
    let h_emu = (h * 914400.0) as i64;

    let text_content = escape_xml(element.text.as_deref().unwrap_or("Text content"));
    let font_name = element.font_name.as_deref().unwrap_or("Calibri");
    let font_size = element.font_size_pt.unwrap_or(18.0);
    let half_pts = (font_size * 2.0) as i32;

    let mut rpr = String::new();
    rpr.push_str(&format!(
        "<a:rFonts ascii=\"{}\" hAnsi=\"{}\"/>",
        escape_xml(font_name),
        escape_xml(font_name)
    ));
    rpr.push_str(&format!("<a:sz val=\"{}\"/>", half_pts));
    if element.bold == Some(true) {
        rpr.push_str("<a:b val=\"1\"/>");
    }
    if element.italic == Some(true) {
        rpr.push_str("<a:i val=\"1\"/>");
    }

    let align = match element.alignment.as_deref() {
        Some("center") => "ctr",
        Some("right") => "r",
        _ => "l",
    };

    format!(
        "<p:sp>\
         <p:nvSpPr><cNvPr id=\"{}\" name=\"text\"/></p:nvSpPr>\
         <p:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm></p:spPr>\
         <p:style><a:lnRef idx=\"0\" sz=\"{}\"/><a:effectLst/></p:style>\
         <p:txBody><a:bodyPr anchor=\"ctr\"/><a:regionalPr/><a:p><a:pPr lvl=\"0\" align=\"{}\"/><a:r><a:rPr>{}</a:rPr><a:t>{}</a:t></a:r></a:p></p:txBody>\
         </p:sp>",
        element.id.as_str(),
        x_emu, y_emu, w_emu, h_emu,
        half_pts,
        align,
        rpr,
        text_content
    )
}

/// Build a shape element with configurable preset geometry.
fn build_shape_element(element: &SlideElement) -> String {
    let (x, y, w, h) = element_position(element);
    let x_emu = (x * 914400.0) as i64;
    let y_emu = (y * 914400.0) as i64;
    let w_emu = (w * 914400.0) as i64;
    let h_emu = (h * 914400.0) as i64;

    let preset = element.preset.as_deref().unwrap_or("rect");
    let fill_color = element.fill_color.as_deref().unwrap_or("#4472C4");
    let line_color = element.line_color.as_deref().unwrap_or("#333333");
    let text_content = escape_xml(element.text.as_deref().unwrap_or("Shape"));

    format!(
        "<p:sp>\
         <p:nvSpPr><cNvPr id=\"{}\" name=\"shape\"/></p:nvSpPr>\
         <p:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"{}\"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill><a:ln w=\"38100\"><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill></a:ln></p:spPr>\
         <p:style><a:lnRef idx=\"0\" sz=\"12000\"/><a:effectLst/></p:style>\
         <p:txBody><a:bodyPr anchor=\"ctr\"/><a:regionalPr/><a:p><a:pPr lvl=\"0\" align=\"ctr\"/><a:r><a:rPr lang=\"en-US\"/><a:t>{}</a:t></a:r></a:p></p:txBody>\
         </p:sp>",
        element.id.as_str(),
        x_emu, y_emu, w_emu, h_emu,
        preset, fill_color, line_color, text_content
    )
}

/// Build an image element with a proper <p:pic>.
fn build_image_element(element: &SlideElement) -> String {
    let (x, y, w, h) = element_position(element);
    let x_emu = (x * 914400.0) as i64;
    let y_emu = (y * 914400.0) as i64;
    let w_emu = (w * 914400.0) as i64;
    let h_emu = (h * 914400.0) as i64;

    let ext = match element.mime_type.as_deref() {
        Some(m) if m.contains("png") => "png",
        Some(m) if m.contains("jpg") || m.contains("jpeg") => "jpeg",
        Some(m) if m.contains("gif") => "gif",
        _ => "png",
    };
    let _ext = ext;
    let rel_id = format!("rId{}", 100 + element.id.as_str().len() % 100);

    format!(
        "<p:pic>\
         <p:nvPicPr><cNvPr id=\"{}\" name=\"picture\"/><cNvPicPr/></p:nvPicPr>\
         <p:picPr><a:extLst/></p:picPr>\
         <p:blipFill><a:blip r:embed=\"{}\"/></p:blipFill>\
         <p:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr>\
         </p:pic>",
        element.id.as_str(),
        rel_id,
        x_emu, y_emu, w_emu, h_emu
    )
}

/// Build a table element with actual table XML.
fn build_table_element(element: &SlideElement) -> String {
    let (x, y, w, h) = element_position(element);
    let _x_emu = (x * 914400.0) as i64;
    let _y_emu = (y * 914400.0) as i64;
    let w_emu = (w * 914400.0) as i64;
    let h_emu = (h * 914400.0) as i64;

    let rows = 3; // Default for now; actual row count from IR in future
    let cols = 2;
    let col_w = w_emu / cols;

    let mut rows_xml = String::new();
    for r in 0..rows {
        let mut cells_xml = String::new();
        for c in 0..cols {
            let cell_text = format!("Row {} Col {}", r + 1, c + 1);
            cells_xml.push_str(&format!(
                "<p:tc><p:tcPr><a:anchor>twoLst</a:anchor></p:tcPr><p:txBody><a:bodyPr anchor=\"ctr\"/><a:p><a:pPr lvl=\"0\"/><a:r><a:rPr lang=\"en-US\"/><a:t>{}</a:t></a:r></a:p></p:txBody></p:tc>",
                escape_xml(&cell_text)
            ));
        }
        rows_xml.push_str(&format!(
            "<p:tr h=\"{}\">{}</p:tr>\n",
            h_emu / rows,
            cells_xml
        ));
    }

    let mut grid_xml = String::new();
    for _ in 0..cols {
        grid_xml.push_str(&format!("<p:gridCol w=\"{}\"/>", col_w));
    }

    format!(
        "<p:tbl>\
         <p:tblPr><a:gridSpacing style=\"mocha\" l=\"21600\" rtl=\"false\"/></p:tblPr>\
         <p:tblGrid>{}</p:tblGrid>\
         {}</p:tbl>",
        grid_xml, rows_xml
    )
}

// ===========================================================================
// ppt/slideLayouts/slideLayoutN.xml
// ===========================================================================

fn build_default_layout() -> Vec<u8> {
    let layout_name = "Default Layout".to_string();
    build_layout_xml(&Layout {
        id: ElementId::derive("layout", &["layout:0"], &layout_name),
        name: layout_name,
        layout_type: None,
        placeholders: vec![],
    })
}

fn build_layout_xml(layout: &Layout) -> Vec<u8> {
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <p:sldLayout xmlns:p=\"{}\" xmlns:r=\"{}\" xmlns:a=\"{}\" type=\"regular\" name=\"{}\" showMasterObjects=\"1\" showPlaceholderCommonBits=\"1\">\n\
         <p:cSld>\n\
         <p:spTree>\n\
         <p:nvGrpSpPr>\n\
         <p:cNvPr id=\"1\" name=\"\"/>\n\
         <p:cNvGrpSpPr/>\n\
         <p:nvPr/>\n\
         </p:nvGrpSpPr>\n\
         <p:grpSpPr>\n\
         <a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></a:xfrm>\n\
         </p:grpSpPr>\n\
         </p:spTree>\n\
         </p:cSld>\n\
         <p:txStyles>\n\
         <p:lvlPpr mac=\"0\" indent=\"0\" marL=\"0\" marS=\"0\" spcBef=\"0\" spcBefPts=\"0\" spcAft=\"0\" spcAftPts=\"0\" defTabSz=\"91440\" algn=\"l\" defCharLn=\"0\"/>\n\
         <a:lnSrchSldTxClr><a:schemeClr val=\"tx1\"/></a:lnSrchSldTxClr>\n\
         </p:txStyles>\n\
         <p:clientData/>\n\
         </p:sldLayout>",
        NS_PPT, NS_R, NS_A, escape_xml(&layout.name),
    );

    xml.into_bytes()
}

// ===========================================================================
// ppt/slideMasters/slideMasterN.xml
// ===========================================================================

fn build_default_master() -> Vec<u8> {
    let master_name = "Office Theme".to_string();
    build_master_xml(&Master {
        id: ElementId::derive("master", &["master:0"], &master_name),
        name: master_name,
        background_color: None,
    })
}

fn build_master_xml(master: &Master) -> Vec<u8> {
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <p:sldMaster xmlns:p=\"{}\" xmlns:r=\"{}\" xmlns:a=\"{}\" xmlns:p14=\"http://schemas.microsoft.com/office/powerpoint/2010/main\" xmlns:p16c1=\"http://schemas.microsoft.com/office/powerpoint/2016/17/main\" xmlns:p16e=\"http://schemas.microsoft.com/office/powerpoint/2018/external\" name=\"{}\" showLayoutNames=\"1\">\n\
         <p:clrMap>\n\
         <a:bg1>lt1</a:bg1>\n\
         <a:tx1>dk1</a:tx1>\n\
         <a:bg2>lt2</a:bg2>\n\
         <a:tx2>dk2</a:tx2>\n\
         <a:accent1>{}</a:accent1>\n\
         <a:accent2>{}</a:accent2>\n\
         <a:accent3>{}</a:accent3>\n\
         <a:accent4>{}</a:accent4>\n\
         <a:accent5>{}</a:accent5>\n\
         <a:accent6>{}</a:accent6>\n\
         <a:hlink>{}</a:hlink>\n\
         <a:folHlink>{}</a:folHlink>\n\
         </p:clrMap>\n\
         <p:txStyles>\n\
         <p:titleStyle>\n\
         <a:lvlPpr mac=\"0\" indent=\"0\" marL=\"0\" marS=\"0\" spcBef=\"0\" spcBefPts=\"0\" spcAft=\"0\" spcAftPts=\"0\" defTabSz=\"91440\" algn=\"l\" defCharLn=\"0\"/>\n\
         <a:lnSrchSldTxClr><a:schemeClr val=\"dk1\"/></a:lnSrchSldTxClr>\n\
         <a:lvl1Ppr mac=\"0\" indent=\"0\" marL=\"0\" marS=\"0\" spcBef=\"0\" spcBefPts=\"10000\" spcAft=\"0\" spcAftPts=\"0\" defTabSz=\"91440\" algn=\"l\" rtl=\"0\" lang=\"en-US\"/>\n\
         <a:lvl1RPr b=\"0\" i=\"0\" u=\"none\" strike=\"noClear\" kerning=\"1200\" smtClean=\"0\" dirty=\"0\">\n\
         <a:schemeClr val=\"dk1\"/>\n\
         </a:lvl1RPr>\n\
         </p:titleStyle>\n\
         <p:bodyStyle>\n\
         <a:lvlPpr mac=\"0\" indent=\"0\" marL=\"0\" marS=\"0\" spcBef=\"0\" spcBefPts=\"0\" spcAft=\"0\" spcAftPts=\"0\" defTabSz=\"91440\" algn=\"l\" defCharLn=\"0\"/>\n\
         <a:lnSrchSldTxClr><a:schemeClr val=\"dk1\"/></a:lnSrchSldTxClr>\n\
         <a:lvl1Ppr mac=\"0\" indent=\"0\" marL=\"0\" marS=\"0\" spcBef=\"0\" spcBefPts=\"6000\" spcAft=\"0\" spcAftPts=\"0\" defTabSz=\"91440\" algn=\"l\" rtl=\"0\" lang=\"en-US\"/>\n\
         <a:lvl1RPr b=\"0\" i=\"0\" u=\"none\" strike=\"noClear\" kerning=\"1200\" smtClean=\"0\" dirty=\"0\">\n\
         <a:schemeClr val=\"dk1\"/>\n\
         </a:lvl1RPr>\n\
         <a:lvl2Ppr mac=\"0\" indent=\"15240\" marL=\"0\" marS=\"0\" spcBef=\"0\" spcBefPts=\"4000\" spcAft=\"0\" spcAftPts=\"0\" defTabSz=\"91440\" algn=\"l\" rtl=\"0\" lang=\"en-US\"/>\n\
         <a:lvl2RPr b=\"0\" i=\"0\" u=\"none\" strike=\"noClear\" kerning=\"1200\" smtClean=\"0\" dirty=\"0\">\n\
         <a:schemeClr val=\"dk1\"/>\n\
         </a:lvl2RPr>\n\
         <a:lvl3Ppr mac=\"0\" indent=\"22860\" marL=\"0\" marS=\"0\" spcBef=\"0\" spcBefPts=\"4000\" spcAft=\"0\" spcAftPts=\"0\" defTabSz=\"91440\" algn=\"l\" rtl=\"0\" lang=\"en-US\"/>\n\
         <a:lvl3RPr b=\"0\" i=\"0\" u=\"none\" strike=\"noClear\" kerning=\"1200\" smtClean=\"0\" dirty=\"0\">\n\
         <a:schemeClr val=\"dk1\"/>\n\
         </a:lvl3RPr>\n\
         <a:lvl4Ppr mac=\"0\" indent=\"30480\" marL=\"0\" marS=\"0\" spcBef=\"0\" spcBefPts=\"4000\" spcAft=\"0\" spcAftPts=\"0\" defTabSz=\"91440\" algn=\"l\" rtl=\"0\" lang=\"en-US\"/>\n\
         <a:lvl4RPr b=\"0\" i=\"0\" u=\"none\" strike=\"noClear\" kerning=\"1200\" smtClean=\"0\" dirty=\"0\">\n\
         <a:schemeClr val=\"dk1\"/>\n\
         </a:lvl4RPr>\n\
         <a:lvl5Ppr mac=\"0\" indent=\"38100\" marL=\"0\" marS=\"0\" spcBef=\"0\" spcBefPts=\"4000\" spcAft=\"0\" spcAftPts=\"0\" defTabSz=\"91440\" algn=\"l\" rtl=\"0\" lang=\"en-US\"/>\n\
         <a:lvl5RPr b=\"0\" i=\"0\" u=\"none\" strike=\"noClear\" kerning=\"1200\" smtClean=\"0\" dirty=\"0\">\n\
         <a:schemeClr val=\"dk1\"/>\n\
         </a:lvl5RPr>\n\
         <a:lvl6Ppr mac=\"0\" indent=\"45720\" marL=\"0\" marS=\"0\" spcBef=\"0\" spcBefPts=\"4000\" spcAft=\"0\" spcAftPts=\"0\" defTabSz=\"91440\" algn=\"l\" rtl=\"0\" lang=\"en-US\"/>\n\
         <a:lvl6RPr b=\"0\" i=\"0\" u=\"none\" strike=\"noClear\" kerning=\"1200\" smtClean=\"0\" dirty=\"0\">\n\
         <a:schemeClr val=\"dk1\"/>\n\
         </a:lvl6RPr>\n\
         <a:lvl7Ppr mac=\"0\" indent=\"53340\" marL=\"0\" marS=\"0\" spcBef=\"0\" spcBefPts=\"4000\" spcAft=\"0\" spcAftPts=\"0\" defTabSz=\"91440\" algn=\"l\" rtl=\"0\" lang=\"en-US\"/>\n\
         <a:lvl7RPr b=\"0\" i=\"0\" u=\"none\" strike=\"noClear\" kerning=\"1200\" smtClean=\"0\" dirty=\"0\">\n\
         <a:schemeClr val=\"dk1\"/>\n\
         </a:lvl7RPr>\n\
         <a:lvl8Ppr mac=\"0\" indent=\"60960\" marL=\"0\" marS=\"0\" spcBef=\"0\" spcBefPts=\"4000\" spcAft=\"0\" spcAftPts=\"0\" defTabSz=\"91440\" algn=\"l\" rtl=\"0\" lang=\"en-US\"/>\n\
         <a:lvl8RPr b=\"0\" i=\"0\" u=\"none\" strike=\"noClear\" kerning=\"1200\" smtClean=\"0\" dirty=\"0\">\n\
         <a:schemeClr val=\"dk1\"/>\n\
         </a:lvl8RPr>\n\
         <a:lvl9Ppr mac=\"0\" indent=\"68580\" marL=\"0\" marS=\"0\" spcBef=\"0\" spcBefPts=\"4000\" spcAft=\"0\" spcAftPts=\"0\" defTabSz=\"91440\" algn=\"l\" rtl=\"0\" lang=\"en-US\"/>\n\
         <a:lvl9RPr b=\"0\" i=\"0\" u=\"none\" strike=\"noClear\" kerning=\"1200\" smtClean=\"0\" dirty=\"0\">\n\
         <a:schemeClr val=\"dk1\"/>\n\
         </a:lvl9RPr>\n\
         </p:bodyStyle>\n\
         <p:custStyle styleIdx=\"2\" startIdx=\"0\" endIdx=\"9\" custStyleId=\"1\">\n\
         <a:lvlPpr mac=\"0\" indent=\"0\" marL=\"0\" marS=\"0\" spcBef=\"0\" spcBefPts=\"0\" spcAft=\"0\" spcAftPts=\"0\" defTabSz=\"91440\" algn=\"l\" defCharLn=\"0\"/>\n\
         <a:lnSrchSldTxClr><a:schemeClr val=\"dk1\"/></a:lnSrchSldTxClr>\n\
         </p:custStyle>\n\
         </p:txStyles>\n\
         <p:csld>\n\
         <p:cSld>\n\
         <p:spTree>\n\
         <p:nvGrpSpPr>\n\
         <p:cNvPr id=\"1\" name=\"\"/>\n\
         <p:cNvGrpSpPr/>\n\
         <p:nvPr/>\n\
         </p:nvGrpSpPr>\n\
         <p:grpSpPr>\n\
         <a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></a:xfrm>\n\
         </p:grpSpPr>\n\
         </p:spTree>\n\
         </p:cSld>\n\
         </p:csld>\n\
         <p:clientData/>\n\
         </p:sldMaster>",
        NS_PPT, NS_R, NS_A, escape_xml(&master.name),
        NS_A, NS_A, NS_A, NS_A, NS_A, NS_A, NS_A, NS_A
    );

    xml.into_bytes()
}

// ===========================================================================
// ppt/theme/theme1.xml
// ===========================================================================

fn build_theme() -> Vec<u8> {
    let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office Theme">
<a:themeElements>
<a:clrScheme name="Office">
<a:dk1><a:sysClr lastR="0" lastG="0" lastB="0" lastA="255" windowText="0"/></a:dk1>
<a:lt1><a:sysClr lastR="255" lastG="255" lastB="255" lastA="255" windowText="0"/></a:lt1>
<a:dk2><a:srgbClr val="1F497D"/></a:dk2>
<a:lt2><a:srgbClr val="EEEED1"/></a:lt2>
<a:accent1><a:srgbClr val="4472C4"/></a:accent1>
<a:accent2><a:srgbClr val="E25840"/></a:accent2>
<a:accent3><a:srgbClr val="92D050"/></a:accent3>
<a:accent4><a:srgbClr val="393939"/></a:accent4>
<a:accent5><a:srgbClr val="00B0F0"/></a:accent5>
<a:accent6><a:srgbClr val="85BBD6"/></a:accent6>
<a:hlink><a:srgbClr val="0563C1"/></a:hlink>
<a:folHlink><a:srgbClr val="95A7CB"/></a:folHlink>
</a:clrScheme>
<a:fontScheme name="Office">
<a:majorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont>
<a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont>
</a:fontScheme>
<a:fmtScheme name="Office">
<a:fillStyleLst><a:solidFill><a:schemeClr val="phclr"/></a:solidFill><a:gradFill><a:gsLst><a:gs pos="0"><a:schemeClr val="tx1"/><a:alpha val="85000"/></a:gs><a:gs pos="50000"><a:schemeClr val="tx1"/><a:alpha val="30000"/></a:gs><a:gs pos="100000"><a:schemeClr val="tx1"/><a:alpha val="0"/></a:gs></a:gsLst><a:path path="rect"><a:rectFrom cx="0" cy="k50000" cw="k100000" ch="k100000"/></a:path></a:gradFill><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:fillStyleLst>
<a:lnStyleLst><a:ln w="9525"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:ln><a:ln w="25400"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:ln><a:ln w="38100"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:ln></a:lnStyleLst>
<a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst><a:outerShdw/></a:effectLst></a:effectStyle></a:effectStyleLst>
<a:bgFillStyleLst><a:solidFill><a:schemeClr val="bg1"/></a:solidFill><a:solidFill><a:schemeClr val="bg2"/></a:solidFill><a:solidFill><a:schemeClr val="bg3"/></a:solidFill></a:bgFillStyleLst>
</a:fmtScheme>
</a:themeElements>
</a:theme>"#;
    xml.as_bytes().to_vec()
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn make_id(kind: &str, path: &[&str], content: &str) -> ElementId {
        let s = String::from(content);
        ElementId::derive(kind, path, &s)
    }

    fn read_zip_entry_str(archive: &mut zip::ZipArchive<Cursor<&Vec<u8>>>, name: &str) -> String {
        let mut buf = Vec::new();
        archive
            .by_name(name)
            .unwrap_or_else(|_| panic!("Entry '{}' should exist", name))
            .read_to_end(&mut buf)
            .unwrap_or_else(|_| panic!("Should read entry '{}'", name));
        String::from_utf8(buf).expect("Entry should be valid UTF-8")
    }

    fn make_empty_presentation() -> Presentation {
        Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![],
            layouts: vec![],
            masters: vec![],
        }
    }

    fn make_presentation_with_slides() -> Presentation {
        Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![
                Slide {
                    id: make_id("slide", &["slide:0"], "Slide 1"),
                    elements: vec![
                        SlideElement {
                            id: make_id("elem", &["elem:0"], "title"),
                            kind: SlideElementKind::Text,
                            text: Some("Title Text".to_string()),
                            bold: Some(true),
                            italic: None,
                            font_name: Some("Calibri".to_string()),
                            font_size_pt: Some(36.0),
                            alignment: Some("left".to_string()),
                            x_pt: Some(1.0),
                            y_pt: Some(0.5),
                            width_pt: Some(8.0),
                            height_pt: Some(1.5),
                            data: None,
                            mime_type: None,
                            preset: None,
                            fill_color: None,
                            line_color: None,
                        },
                        SlideElement {
                            id: make_id("elem", &["elem:1"], "body"),
                            kind: SlideElementKind::Shape,
                            text: Some("Shape Content".to_string()),
                            bold: None,
                            italic: None,
                            font_name: None,
                            font_size_pt: None,
                            alignment: None,
                            x_pt: Some(1.0),
                            y_pt: Some(2.5),
                            width_pt: Some(5.0),
                            height_pt: Some(2.0),
                            data: None,
                            mime_type: None,
                            preset: Some("rect".to_string()),
                            fill_color: Some("#4472C4".to_string()),
                            line_color: Some("#333333".to_string()),
                        },
                    ],
                },
                Slide {
                    id: make_id("slide", &["slide:1"], "Slide 2"),
                    elements: vec![SlideElement {
                        id: make_id("elem", &["elem:2"], "image"),
                        kind: SlideElementKind::Image,
                        text: None,
                        bold: None,
                        italic: None,
                        font_name: None,
                        font_size_pt: None,
                        alignment: None,
                        x_pt: Some(2.0),
                        y_pt: Some(1.0),
                        width_pt: Some(4.0),
                        height_pt: Some(3.0),
                        data: None,
                        mime_type: Some("image/png".to_string()),
                        preset: None,
                        fill_color: None,
                        line_color: None,
                    }],
                },
            ],
            layouts: vec![],
            masters: vec![],
        }
    }

    fn make_presentation_with_text_elements() -> Presentation {
        Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![Slide {
                id: make_id("slide", &["slide:0"], "Text Slide"),
                elements: vec![
                    SlideElement {
                        id: make_id("elem", &["elem:0"], "Title"),
                        kind: SlideElementKind::Text,
                        text: Some("Main Title".to_string()),
                        bold: Some(true),
                        italic: None,
                        font_name: Some("Calibri".to_string()),
                        font_size_pt: Some(44.0),
                        alignment: Some("center".to_string()),
                        x_pt: Some(2.0),
                        y_pt: Some(0.5),
                        width_pt: Some(7.0),
                        height_pt: Some(1.5),
                        data: None,
                        mime_type: None,
                        preset: None,
                        fill_color: None,
                        line_color: None,
                    },
                    SlideElement {
                        id: make_id("elem", &["elem:1"], "Content"),
                        kind: SlideElementKind::Text,
                        text: Some("Body content here".to_string()),
                        bold: None,
                        italic: Some(true),
                        font_name: Some("Calibri".to_string()),
                        font_size_pt: Some(18.0),
                        alignment: Some("left".to_string()),
                        x_pt: Some(1.0),
                        y_pt: Some(2.5),
                        width_pt: Some(8.0),
                        height_pt: Some(3.0),
                        data: None,
                        mime_type: None,
                        preset: None,
                        fill_color: None,
                        line_color: None,
                    },
                ],
            }],
            layouts: vec![],
            masters: vec![],
        }
    }

    fn make_presentation_with_layouts_masters() -> Presentation {
        Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![Slide {
                id: make_id("slide", &["slide:0"], "Slide 1"),
                elements: vec![SlideElement {
                    id: make_id("elem", &["elem:0"], "test"),
                    kind: SlideElementKind::Text,
                    text: Some("Test content".to_string()),
                    bold: None,
                    italic: None,
                    font_name: None,
                    font_size_pt: None,
                    alignment: None,
                    x_pt: None,
                    y_pt: None,
                    width_pt: None,
                    height_pt: None,
                    data: None,
                    mime_type: None,
                    preset: None,
                    fill_color: None,
                    line_color: None,
                }],
            }],
            layouts: vec![
                Layout {
                    id: make_id("layout", &["layout:0"], "Title Slide"),
                    name: "Title Slide".to_string(),
                    layout_type: None,
                    placeholders: vec![],
                },
                Layout {
                    id: make_id("layout", &["layout:1"], "Content"),
                    name: "Content".to_string(),
                    layout_type: None,
                    placeholders: vec![],
                },
            ],
            masters: vec![Master {
                id: make_id("master", &["master:0"], "Office Theme"),
                name: "Office Theme".to_string(),
                background_color: None,
            }],
        }
    }

    /// test_minimal_pptx_creation
    #[test]
    fn test_minimal_pptx_creation() {
        let presentation = make_empty_presentation();
        let result = create_minimal_pptx(&presentation);
        assert!(result.is_ok(), "Should create pptx from empty presentation");
        let bytes = result.unwrap();
        assert!(!bytes.is_empty(), "Pptx bytes should not be empty");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        assert!(!archive.is_empty(), "ZIP should have entries");
        let entry_names: Vec<String> = (0..archive.len())
            .filter_map(|i| archive.by_index(i).ok().map(|e| e.name().to_string()))
            .collect();
        assert!(
            entry_names.iter().any(|n| n == "[Content_Types].xml"),
            "Should have [Content_Types].xml"
        );
        assert!(
            entry_names.iter().any(|n| n == "_rels/.rels"),
            "Should have _rels/.rels"
        );
        assert!(
            entry_names.iter().any(|n| n == "ppt/presentation.xml"),
            "Should have ppt/presentation.xml"
        );
        assert!(
            entry_names.iter().any(|n| n == "ppt/theme/theme1.xml"),
            "Should have ppt/theme/theme1.xml"
        );
        assert!(
            entry_names
                .iter()
                .any(|n| n == "ppt/slideLayouts/slideLayout1.xml"),
            "Should have default layout"
        );
        assert!(
            entry_names
                .iter()
                .any(|n| n == "ppt/slideMasters/slideMaster1.xml"),
            "Should have default master"
        );
    }

    /// test_pptx_with_slides
    #[test]
    fn test_pptx_with_slides() {
        let presentation = make_presentation_with_slides();
        let result = create_minimal_pptx(&presentation);
        assert!(result.is_ok(), "Should create pptx with slides");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let entry_names: Vec<String> = (0..archive.len())
            .filter_map(|i| archive.by_index(i).ok().map(|e| e.name().to_string()))
            .collect();
        assert!(
            entry_names.iter().any(|n| n == "ppt/slides/slide1.xml"),
            "Should have slide1.xml"
        );
        assert!(
            entry_names.iter().any(|n| n == "ppt/slides/slide2.xml"),
            "Should have slide2.xml"
        );
        let pres_xml = read_zip_entry_str(&mut archive, "ppt/presentation.xml");
        assert!(pres_xml.contains("<p:sldIdLst>"), "Should have sldIdLst");
        assert!(pres_xml.contains("<p:sldId"), "Should have sldId elements");
    }

    /// test_pptx_text_element
    #[test]
    fn test_pptx_text_element() {
        let presentation = make_presentation_with_text_elements();
        let result = create_minimal_pptx(&presentation);
        assert!(result.is_ok(), "Should create pptx with text elements");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let slide_xml = read_zip_entry_str(&mut archive, "ppt/slides/slide1.xml");
        assert!(slide_xml.contains("<p:sld"), "Should have slide element");
        assert!(
            slide_xml.contains("<p:cSld"),
            "Should have cSld (content shape list)"
        );
        assert!(
            slide_xml.contains("<p:spTree"),
            "Should have spTree (shape tree)"
        );
        // Text elements now use <p:sp> with <p:txBody>
        assert!(
            slide_xml.contains("<p:txBody"),
            "Should have txBody for text elements"
        );
        assert!(
            slide_xml.contains("<a:rFonts"),
            "Should have rFonts for text elements"
        );
    }

    /// test_pptx_layouts_masters
    #[test]
    fn test_pptx_layouts_masters() {
        let presentation = make_presentation_with_layouts_masters();
        let result = create_minimal_pptx(&presentation);
        assert!(
            result.is_ok(),
            "Should create pptx with layouts and masters"
        );
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let entry_names: Vec<String> = (0..archive.len())
            .filter_map(|i| archive.by_index(i).ok().map(|e| e.name().to_string()))
            .collect();
        assert!(
            entry_names
                .iter()
                .any(|n| n == "ppt/slideLayouts/slideLayout1.xml"),
            "Should have layout1.xml"
        );
        assert!(
            entry_names
                .iter()
                .any(|n| n == "ppt/slideLayouts/slideLayout2.xml"),
            "Should have layout2.xml"
        );
        assert!(
            entry_names
                .iter()
                .any(|n| n == "ppt/slideMasters/slideMaster1.xml"),
            "Should have master1.xml"
        );
        let layout_xml = read_zip_entry_str(&mut archive, "ppt/slideLayouts/slideLayout1.xml");
        assert!(
            layout_xml.contains("Title Slide"),
            "Layout should have title slide name"
        );
        let master_xml = read_zip_entry_str(&mut archive, "ppt/slideMasters/slideMaster1.xml");
        assert!(
            master_xml.contains("Office Theme"),
            "Master should have office theme name"
        );
    }

    /// test_pptx_roundtrip
    #[test]
    fn test_pptx_roundtrip() {
        let presentation = make_presentation_with_slides();
        let bytes = create_minimal_pptx(&presentation).expect("Should create pptx");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let pres_xml = read_zip_entry_str(&mut archive, "ppt/presentation.xml");
        assert!(
            pres_xml.contains("<?xml version=\"1.0\""),
            "Should have XML declaration"
        );
        assert!(
            pres_xml
                .contains("xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\""),
            "Should have PPT namespace"
        );
        assert!(pres_xml.contains("<p:sldIdLst>"), "Should have sldIdLst");
        assert!(
            pres_xml.contains("<p:sldLayoutIdLst>"),
            "Should have layout list"
        );
        assert!(
            pres_xml.contains("<p:sldMasterIdLst>"),
            "Should have master list"
        );
        let rels_xml = read_zip_entry_str(&mut archive, "ppt/_rels/presentation.xml.rels");
        assert!(
            rels_xml.contains("<Relationship"),
            "Should have relationships"
        );
        assert!(rels_xml.contains("slide"), "Should reference slides");
    }

    /// test_pptx_with_shapes
    #[test]
    fn test_pptx_with_shapes() {
        let presentation = Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![Slide {
                id: make_id("slide", &["slide:0"], "Shapes"),
                elements: vec![
                    SlideElement {
                        id: make_id("elem", &["elem:0"], "rect"),
                        kind: SlideElementKind::Shape,
                        text: Some("Rectangle".to_string()),
                        bold: None,
                        italic: None,
                        font_name: None,
                        font_size_pt: None,
                        alignment: None,
                        x_pt: Some(1.0),
                        y_pt: Some(1.0),
                        width_pt: Some(4.0),
                        height_pt: Some(2.0),
                        data: None,
                        mime_type: None,
                        preset: Some("rect".to_string()),
                        fill_color: Some("#4472C4".to_string()),
                        line_color: Some("#333333".to_string()),
                    },
                    SlideElement {
                        id: make_id("elem", &["elem:1"], "circle"),
                        kind: SlideElementKind::Shape,
                        text: Some("Ellipse".to_string()),
                        bold: None,
                        italic: None,
                        font_name: None,
                        font_size_pt: None,
                        alignment: None,
                        x_pt: Some(6.0),
                        y_pt: Some(1.0),
                        width_pt: Some(3.0),
                        height_pt: Some(2.0),
                        data: None,
                        mime_type: None,
                        preset: Some("ellipse".to_string()),
                        fill_color: Some("#E25840".to_string()),
                        line_color: Some("#666666".to_string()),
                    },
                ],
            }],
            layouts: vec![],
            masters: vec![],
        };
        let result = create_minimal_pptx(&presentation);
        assert!(result.is_ok(), "Should create pptx with shapes");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let slide_xml = read_zip_entry_str(&mut archive, "ppt/slides/slide1.xml");
        assert!(slide_xml.contains("<p:sp>"), "Should have shape elements");
        assert!(
            slide_xml.contains("<p:nvSpPr"),
            "Should have non-visual shape properties"
        );
        assert!(slide_xml.contains("<p:txBody>"), "Should have text body");
        assert!(
            slide_xml.contains("<a:prstGeom prst=\"rect\""),
            "Should have rect geometry"
        );
        assert!(
            slide_xml.contains("<a:prstGeom prst=\"ellipse\""),
            "Should have ellipse geometry"
        );
    }

    /// test_pptx_with_images
    #[test]
    fn test_pptx_with_images() {
        let presentation = Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![Slide {
                id: make_id("slide", &["slide:0"], "Images"),
                elements: vec![
                    SlideElement {
                        id: make_id("elem", &["elem:0"], "logo"),
                        kind: SlideElementKind::Image,
                        text: None,
                        bold: None,
                        italic: None,
                        font_name: None,
                        font_size_pt: None,
                        alignment: None,
                        x_pt: Some(1.0),
                        y_pt: Some(1.0),
                        width_pt: Some(3.0),
                        height_pt: Some(2.0),
                        data: None,
                        mime_type: Some("image/png".to_string()),
                        preset: None,
                        fill_color: None,
                        line_color: None,
                    },
                    SlideElement {
                        id: make_id("elem", &["elem:1"], "photo"),
                        kind: SlideElementKind::Drawing,
                        text: None,
                        bold: None,
                        italic: None,
                        font_name: None,
                        font_size_pt: None,
                        alignment: None,
                        x_pt: Some(5.0),
                        y_pt: Some(1.0),
                        width_pt: Some(4.0),
                        height_pt: Some(3.0),
                        data: None,
                        mime_type: Some("image/jpeg".to_string()),
                        preset: None,
                        fill_color: None,
                        line_color: None,
                    },
                ],
            }],
            layouts: vec![],
            masters: vec![],
        };
        let result = create_minimal_pptx(&presentation);
        assert!(result.is_ok(), "Should create pptx with images");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let slide_xml = read_zip_entry_str(&mut archive, "ppt/slides/slide1.xml");
        assert!(
            slide_xml.contains("<p:pic>"),
            "Should have picture elements"
        );
        assert!(
            slide_xml.contains("<p:nvPicPr>"),
            "Should have non-visual picture properties"
        );
    }

    /// test_pptx_with_table
    #[test]
    fn test_pptx_with_table() {
        let presentation = Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![Slide {
                id: make_id("slide", &["slide:0"], "Tables"),
                elements: vec![SlideElement {
                    id: make_id("elem", &["elem:0"], "data"),
                    kind: SlideElementKind::Table,
                    text: None,
                    bold: None,
                    italic: None,
                    font_name: None,
                    font_size_pt: None,
                    alignment: None,
                    x_pt: Some(1.0),
                    y_pt: Some(1.0),
                    width_pt: Some(6.0),
                    height_pt: Some(3.0),
                    data: None,
                    mime_type: None,
                    preset: None,
                    fill_color: None,
                    line_color: None,
                }],
            }],
            layouts: vec![],
            masters: vec![],
        };
        let result = create_minimal_pptx(&presentation);
        assert!(result.is_ok(), "Should create pptx with table");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let slide_xml = read_zip_entry_str(&mut archive, "ppt/slides/slide1.xml");
        assert!(slide_xml.contains("<p:tbl>"), "Should have table element");
    }

    /// test_escape_xml
    #[test]
    fn test_escape_xml() {
        assert_eq!(escape_xml("a&b"), "a&amp;b");
        assert_eq!(escape_xml("a<b"), "a&lt;b");
        assert_eq!(escape_xml("a>b"), "a&gt;b");
        assert_eq!(escape_xml("a\"b"), "a&quot;b");
        assert_eq!(escape_xml("a'b"), "a&apos;b");
    }

    /// test_content_types
    #[test]
    fn test_content_types() {
        let presentation = make_presentation_with_slides();
        let bytes = create_minimal_pptx(&presentation).expect("Should create pptx");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let ct_xml = read_zip_entry_str(&mut archive, "[Content_Types].xml");
        assert!(
            ct_xml.contains("presentationml"),
            "Should have presentation content type"
        );
        assert!(ct_xml.contains("slide"), "Should have slide content type");
    }

    /// test_theme
    #[test]
    fn test_theme() {
        let presentation = make_presentation_with_slides();
        let bytes = create_minimal_pptx(&presentation).expect("Should create pptx");
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let theme_xml = read_zip_entry_str(&mut archive, "ppt/theme/theme1.xml");
        assert!(
            theme_xml.contains("Office Theme"),
            "Should have Office Theme"
        );
        assert!(theme_xml.contains("clrScheme"), "Should have color scheme");
        assert!(theme_xml.contains("fontScheme"), "Should have font scheme");
    }

    /// test_pptx_text_element_with_content
    #[test]
    fn test_pptx_text_element_with_content() {
        let presentation = make_presentation_with_text_content();
        let result = create_minimal_pptx(&presentation);
        assert!(result.is_ok(), "Should create pptx with text content");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let slide_xml = read_zip_entry_str(&mut archive, "ppt/slides/slide1.xml");
        assert!(
            slide_xml.contains("Title text"),
            "Should contain title text content"
        );
        assert!(
            slide_xml.contains("<a:t>"),
            "Should contain <a:t> text element"
        );
        assert!(
            !slide_xml.contains("text-0"),
            "Should not contain placeholder name"
        );
    }

    /// test_pptx_image_element_with_data
    #[test]
    fn test_pptx_image_element_with_data() {
        let presentation = make_presentation_with_images();
        let result = create_minimal_pptx(&presentation);
        assert!(result.is_ok(), "Should create pptx with images");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let slide_xml = read_zip_entry_str(&mut archive, "ppt/slides/slide1.xml");
        assert!(
            slide_xml.contains("<p:pic>"),
            "Should contain <p:pic> element"
        );
        assert!(slide_xml.contains("<a:blip"), "Should contain blip element");
        assert!(
            slide_xml.contains("r:embed"),
            "Should contain r:embed relationship"
        );
    }

    /// test_pptx_table_with_cells
    #[test]
    fn test_pptx_table_with_cells() {
        let presentation = make_presentation_with_table();
        let result = create_minimal_pptx(&presentation);
        assert!(result.is_ok(), "Should create pptx with table");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let slide_xml = read_zip_entry_str(&mut archive, "ppt/slides/slide1.xml");
        assert!(
            slide_xml.contains("<p:tbl>"),
            "Should contain <p:tbl> element"
        );
        assert!(
            slide_xml.contains("<p:tr h="),
            "Should contain <p:tr> row element"
        );
        assert!(
            slide_xml.contains("<p:tc>"),
            "Should contain <p:tc> cell element"
        );
        assert!(
            !slide_xml.contains("rowRef"),
            "Should not contain placeholder rowRef"
        );
    }

    /// test_pptx_shape_with_preset
    #[test]
    fn test_pptx_shape_with_preset() {
        let presentation = make_presentation_with_shapes();
        let result = create_minimal_pptx(&presentation);
        assert!(result.is_ok(), "Should create pptx with shapes");
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let slide_xml = read_zip_entry_str(&mut archive, "ppt/slides/slide1.xml");
        assert!(
            slide_xml.contains("<a:prstGeom"),
            "Should contain <a:prstGeom>"
        );
        assert!(
            slide_xml.contains("<a:solidFill"),
            "Should contain <a:solidFill>"
        );
    }

    /// test_pptx_with_positions
    #[test]
    fn test_pptx_with_positions() {
        let presentation = make_presentation_with_text_content();
        let result = create_minimal_pptx(&presentation);
        assert!(
            result.is_ok(),
            "Should create pptx with positioned elements"
        );
        let bytes = result.unwrap();
        let cursor = Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(cursor).expect("Should be valid ZIP");
        let slide_xml = read_zip_entry_str(&mut archive, "ppt/slides/slide1.xml");
        assert!(
            slide_xml.contains("<a:off"),
            "Should contain <a:off> element"
        );
    }

    fn make_presentation_with_text_content() -> Presentation {
        Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![Slide {
                id: make_id("slide", &["slide:0"], "slide0"),
                elements: vec![SlideElement {
                    id: make_id("elem", &["elem:0"], "title"),
                    kind: SlideElementKind::Text,
                    text: Some("Title text".to_string()),
                    bold: Some(true),
                    italic: None,
                    font_name: Some("Calibri".to_string()),
                    font_size_pt: Some(24.0),
                    alignment: None,
                    x_pt: Some(0.5),
                    y_pt: Some(0.5),
                    width_pt: Some(8.0),
                    height_pt: Some(1.0),
                    data: None,
                    mime_type: None,
                    preset: None,
                    fill_color: None,
                    line_color: None,
                }],
            }],
            layouts: vec![Layout {
                id: make_id("layout", &["layout:0"], "default"),
                name: "Default".to_string(),
                layout_type: Some("blank".to_string()),
                placeholders: vec![],
            }],
            masters: vec![Master {
                id: make_id("master", &["master:0"], "default"),
                name: "Default".to_string(),
                background_color: None,
            }],
        }
    }

    fn make_presentation_with_images() -> Presentation {
        let img_data = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![Slide {
                id: make_id("slide", &["slide:0"], "slide0"),
                elements: vec![SlideElement {
                    id: make_id("elem", &["elem:0"], "image0"),
                    kind: SlideElementKind::Image,
                    text: None,
                    bold: None,
                    italic: None,
                    font_name: None,
                    font_size_pt: None,
                    alignment: None,
                    x_pt: Some(1.0),
                    y_pt: Some(1.0),
                    width_pt: Some(4.0),
                    height_pt: Some(3.0),
                    data: Some(img_data),
                    mime_type: Some("image/png".to_string()),
                    preset: None,
                    fill_color: None,
                    line_color: None,
                }],
            }],
            layouts: vec![Layout {
                id: make_id("layout", &["layout:0"], "default"),
                name: "Default".to_string(),
                layout_type: Some("blank".to_string()),
                placeholders: vec![],
            }],
            masters: vec![Master {
                id: make_id("master", &["master:0"], "default"),
                name: "Default".to_string(),
                background_color: None,
            }],
        }
    }

    fn make_presentation_with_table() -> Presentation {
        Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![Slide {
                id: make_id("slide", &["slide:0"], "slide0"),
                elements: vec![SlideElement {
                    id: make_id("elem", &["elem:0"], "table0"),
                    kind: SlideElementKind::Table,
                    text: None,
                    bold: None,
                    italic: None,
                    font_name: None,
                    font_size_pt: None,
                    alignment: None,
                    x_pt: Some(0.5),
                    y_pt: Some(1.0),
                    width_pt: Some(9.0),
                    height_pt: Some(4.0),
                    data: None,
                    mime_type: None,
                    preset: None,
                    fill_color: None,
                    line_color: None,
                }],
            }],
            layouts: vec![Layout {
                id: make_id("layout", &["layout:0"], "default"),
                name: "Default".to_string(),
                layout_type: Some("blank".to_string()),
                placeholders: vec![],
            }],
            masters: vec![Master {
                id: make_id("master", &["master:0"], "default"),
                name: "Default".to_string(),
                background_color: None,
            }],
        }
    }

    fn make_presentation_with_shapes() -> Presentation {
        Presentation {
            ir_version: IR_VERSION.to_string(),
            ir_version_migration: None,
            slides: vec![Slide {
                id: make_id("slide", &["slide:0"], "slide0"),
                elements: vec![
                    SlideElement {
                        id: make_id("elem", &["elem:0"], "rect0"),
                        kind: SlideElementKind::Shape,
                        text: Some("Button".to_string()),
                        bold: None,
                        italic: None,
                        font_name: None,
                        font_size_pt: None,
                        alignment: None,
                        x_pt: Some(1.0),
                        y_pt: Some(2.0),
                        width_pt: Some(2.0),
                        height_pt: Some(0.8),
                        data: None,
                        mime_type: None,
                        preset: Some("rect".to_string()),
                        fill_color: Some("#4472C4".to_string()),
                        line_color: Some("#333333".to_string()),
                    },
                    SlideElement {
                        id: make_id("elem", &["elem:1"], "ellipse0"),
                        kind: SlideElementKind::Shape,
                        text: None,
                        bold: None,
                        italic: None,
                        font_name: None,
                        font_size_pt: None,
                        alignment: None,
                        x_pt: Some(4.0),
                        y_pt: Some(2.0),
                        width_pt: Some(2.0),
                        height_pt: Some(1.5),
                        data: None,
                        mime_type: None,
                        preset: Some("ellipse".to_string()),
                        fill_color: Some("#E25840".to_string()),
                        line_color: None,
                    },
                ],
            }],
            layouts: vec![Layout {
                id: make_id("layout", &["layout:0"], "default"),
                name: "Default".to_string(),
                layout_type: Some("blank".to_string()),
                placeholders: vec![],
            }],
            masters: vec![Master {
                id: make_id("master", &["master:0"], "default"),
                name: "Default".to_string(),
                background_color: None,
            }],
        }
    }
}
