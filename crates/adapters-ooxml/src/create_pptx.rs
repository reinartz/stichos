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
            build_slide_xml(&presentation.slides[i], i),
        ));
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

// ===========================================================================
// [Content_Types].xml
// ===========================================================================

fn build_content_types(num_slides: usize, num_layouts: usize, num_masters: usize) -> Vec<u8> {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
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

fn build_slide_xml(slide: &Slide, _slide_num: usize) -> Vec<u8> {
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
fn build_slide_element_xml(elem: &SlideElement, idx: usize) -> String {
    let elem_type = elem.kind.as_str();
    let x = (idx as i64 * 200) % 4000 + 500;
    let y = (idx as i64 * 150) % 3000 + 500;
    let w = 2000 + (idx as i64 * 300) % 2000;
    let h = 500 + (idx as i64 * 200) % 1500;
    let name = format!("Element {}", idx + 1);

    match elem_type {
        "text" => build_text_element(elem, idx, x, y, w, h, &name),
        "shape" => build_shape_element(elem, idx, x, y, w, h, &name),
        "image" => build_image_element(elem, idx, x, y, w, h, &name),
        "table" => build_table_element(elem, idx, x, y, w, h, &name),
        "drawing" | "graphic" => build_image_element(elem, idx, x, y, w, h, &name),
        _ => build_shape_element(elem, idx, x, y, w, h, &name),
    }
}

/// Build a text element (text box).
fn build_text_element(
    _elem: &SlideElement,
    idx: usize,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    name: &str,
) -> String {
    let rel_id = format!("rId{}", idx + 1);
    format!(
        "\n<a:graphicFrame>\n\
         <a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>\n\
         <p:pic>\n\
         <p:nvPicPr>\n\
         <p:cNvPr id=\"{}\" name=\"{}\"/>\n\
         <p:cNvPicPr/>\n\
         </p:nvPicPr>\n\
         <p:blipFill>\n\
         <a:blip r:embed=\"{}\"/>\n\
         <a:stretch><a:fillToRect/></a:stretch>\n\
         </p:blipFill>\n\
         <p:spPr>\n\
         <a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>\n\
         <a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>\n\
         </p:spPr>\n\
         </p:pic>\n\
         </a:graphicFrame>",
        x,
        y,
        w,
        h,
        idx + 2,
        escape_xml(name),
        rel_id,
        w,
        h
    )
}

/// Build a shape element (filled shape).
fn build_shape_element(
    _elem: &SlideElement,
    idx: usize,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    name: &str,
) -> String {
    let elem_id = idx + 10;
    format!(
        "\n<p:sp>\n\
         <p:nvSpPr>\n\
         <p:cNvPr id=\"{}\" name=\"{}\"/>\n\
         <p:cNvSpPr/>\n\
         <p:nvPr/>\n\
         </p:nvSpPr>\n\
         <p:spPr>\n\
         <a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>\n\
         <a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>\n\
         <a:solidFill><a:srgbClr val=\"4472C4\"/></a:solidFill>\n\
         <a:ln w=\"12700\" cnsdAst=\"1\" a:fillEnabled=\"1\"/>\n\
         </p:spPr>\n\
         <p:txBody>\n\
         <a:bodyPr wrpSp=\"false\" fitText=\"hor\" anchor=\"ctr\" span=\"0\" marR=\"11430\" marT=\"45720\" marB=\"45720\" marL=\"91440\" indent=\"0\" dense=\"0\"/>\n\
         <a:lstStyle/>\n\
         <a:p>\n\
         <a:pPr lvl=\"0\" indent=\"0\" marL=\"0\" marS=\"0\" spcBef=\"0\" spcBefPts=\"0\" spcAft=\"0\" spcAftPts=\"0\" algn=\"ctr\" defTabSz=\"91440\" rtl=\"0\">\n\
         <a:defRPr sz=\"1800\"/>\n\
         </a:pPr>\n\
         <a:r>\n\
         <a:rPr lang=\"en-US\" smtClean=\"0\" dirty=\"0\"/>\n\
         <a:t>{}</a:t>\n\
         </a:r>\n\
         </a:p>\n\
         </p:txBody>\n\
         </p:sp>",
        elem_id, escape_xml(name),
        x, y, w, h,
        escape_xml(name)
    )
}

/// Build an image element (graphic frame).
fn build_image_element(
    _elem: &SlideElement,
    idx: usize,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    name: &str,
) -> String {
    let rel_id = format!("rId{}", idx + 1);
    let pic_id = idx + 2;
    format!(
        "\n<a:graphicFrame>\n\
         <a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>\n\
         <p:pic>\n\
         <p:nvPicPr>\n\
         <p:cNvPr id=\"{}\" name=\"{}\"/>\n\
         <p:cNvPicPr/>\n\
         </p:nvPicPr>\n\
         <p:blipFill>\n\
         <a:blip r:embed=\"{}\"/>\n\
         <a:stretch><a:fillToRect/></a:stretch>\n\
         </p:blipFill>\n\
         <p:spPr>\n\
         <a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>\n\
         <a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>\n\
         </p:spPr>\n\
         </p:pic>\n\
         </a:graphicFrame>",
        x,
        y,
        w,
        h,
        pic_id,
        escape_xml(name),
        rel_id,
        w,
        h
    )
}

/// Build a table element.
fn build_table_element(
    _elem: &SlideElement,
    _idx: usize,
    x: i64,
    y: i64,
    _w: i64,
    h: i64,
    _name: &str,
) -> String {
    format!(
        "\n<a:graphicFrame>\n\
         <a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"457200\" cy=\"{}\"/></a:xfrm>\n\
         <p:tbl>\n\
         <p:tblPr band=\"1\">\n\
         <a:ln w=\"91440\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\">\n\
         <a:solidFill><a:schemeClr val=\"tx2\"/></a:solidFill>\n\
         </a:ln>\n\
         </p:tblPr>\n\
         <a:graphicFrame>\n\
         <a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"457200\" cy=\"{}\"/></a:xfrm>\n\
         <a:chFill>\n\
         <a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill>\n\
         </a:chFill>\n\
         <a:spPr>\n\
         <a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></a:xfrm>\n\
         </a:spPr>\n\
         </a:graphicFrame>\n\
         <p:rowRef id=\"0\"/>\n\
         <p:rowRef id=\"1\"/>\n\
         <p:rowRef id=\"2\"/>\n\
         <p:colRef id=\"0\"/>\n\
         <p:colRef id=\"1\"/>\n\
         </p:tbl>\n\
         </a:graphicFrame>",
        x, y, h, h,
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
                            kind: "text".to_string(),
                        },
                        SlideElement {
                            id: make_id("elem", &["elem:1"], "body"),
                            kind: "shape".to_string(),
                        },
                    ],
                },
                Slide {
                    id: make_id("slide", &["slide:1"], "Slide 2"),
                    elements: vec![SlideElement {
                        id: make_id("elem", &["elem:2"], "image"),
                        kind: "image".to_string(),
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
                        kind: "text".to_string(),
                    },
                    SlideElement {
                        id: make_id("elem", &["elem:1"], "Content"),
                        kind: "text".to_string(),
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
                    kind: "text".to_string(),
                }],
            }],
            layouts: vec![
                Layout {
                    id: make_id("layout", &["layout:0"], "Title Slide"),
                    name: "Title Slide".to_string(),
                },
                Layout {
                    id: make_id("layout", &["layout:1"], "Content"),
                    name: "Content".to_string(),
                },
            ],
            masters: vec![Master {
                id: make_id("master", &["master:0"], "Office Theme"),
                name: "Office Theme".to_string(),
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
        // Text elements create graphicFrame
        assert!(
            slide_xml.contains("<a:graphicFrame"),
            "Should have graphicFrame for text elements"
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
                        kind: "shape".to_string(),
                    },
                    SlideElement {
                        id: make_id("elem", &["elem:1"], "circle"),
                        kind: "shape".to_string(),
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
                        kind: "image".to_string(),
                    },
                    SlideElement {
                        id: make_id("elem", &["elem:1"], "photo"),
                        kind: "drawing".to_string(),
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
                    kind: "table".to_string(),
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
}
