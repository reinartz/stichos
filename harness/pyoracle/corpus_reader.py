# SPDX-License-Identifier: MIT OR Apache-2.0
"""corpus_reader -- extract structured data from corpus files via Python OOXML libs."""

import os
import pathlib
from typing import Any, Dict, List, Optional, Union

try:
    from docx import Document as DocxDocument
    from docx.opc.constants import RELATIONSHIP_TYPE as RT
    DOCX_AVAILABLE = True
except ImportError:
    DOCX_AVAILABLE = False

try:
    import openpyxl
    XLSX_AVAILABLE = True
except ImportError:
    XLSX_AVAILABLE = False

try:
    from pptx import Presentation as PptxPresentation
    PPTX_AVAILABLE = True
except ImportError:
    PPTX_AVAILABLE = False

_SUPPORTED = {".docx", ".xlsx", ".pptx"}


def _ext(path: Union[str, pathlib.Path]) -> str:
    return pathlib.Path(path).suffix.lower()


# ------------------------------------------------------------------ docx


def _docx_text(doc: "DocxDocument") -> List[str]:
    """Return the concatenated paragraph text lines."""
    lines: List[str] = []
    for para in doc.paragraphs:
        t = para.text
        if t:
            lines.append(t)
    return lines


def _docx_styles(doc: "DocxDocument") -> List[Dict[str, Any]]:
    """Extract named styles with their type and parent info."""
    styles: List[Dict[str, Any]] = []
    for s in doc.styles:
        base_name = None
        if hasattr(s, "base") and s.base is not None:
            base_name = s.base.name
        elif hasattr(s, "based_on") and s.based_on is not None:
            base_name = s.based_on.name
        styles.append({
            "name": s.name,
            "type": s.type.name if s.type else None,
            "base": base_name,
        })
    return styles


def _docx_hyperlinks(doc: "DocxDocument") -> List[Dict[str, Any]]:
    """Extract hyperlinks from the document relationship table."""
    links: List[Dict[str, Any]] = []
    if not hasattr(doc, "_part") or doc._part is None:
        return links
    for rel in doc._part.rels.values():
        if "Hyperlink" in rel.reltype:
            links.append({"target": rel.target_ref, "rel_id": rel.rId})
    return links


def _docx_images(doc: "DocxDocument") -> Dict[str, Any]:
    """Count images and return relationship info."""
    images: List[Dict[str, Any]] = []
    if not hasattr(doc, "_part") or doc._part is None:
        return {"count": 0, "relationships": images}
    for rel in doc._part.rels.values():
        if "image" in (rel.reltype or "").lower():
            images.append({"rel_id": rel.rId, "target": rel.target_ref})
    return {"count": len(images), "relationships": images}


def _docx_fields(doc: "DocxDocument") -> List[Dict[str, Any]]:
    """Extract field codes found in paragraph runs."""
    fields: List[Dict[str, Any]] = []
    for para in doc.paragraphs:
        for run in para.runs:
            if run._element is not None:
                for elem in run._element.iter():
                    tag = elem.tag
                    if "instrText" in tag or "fldSimple" in tag:
                        text = (elem.text or "").strip()
                        fields.append({"type": "field", "instruction": text})
    return fields


def _inspect_docx(path: Union[str, pathlib.Path]) -> Dict[str, Any]:
    if not DOCX_AVAILABLE:
        return {"error": "python-docx not available"}
    doc = DocxDocument(str(path))
    return {
        "text": _docx_text(doc),
        "styles": _docx_styles(doc),
        "hyperlinks": _docx_hyperlinks(doc),
        "images": _docx_images(doc),
        "fields": _docx_fields(doc),
            "page_setup": {
                "orientation": getattr(doc.sections[0], "page_orientation", None)
                or (doc.sections[0].orientation if hasattr(doc.sections[0], "orientation") else None),
                "page_width": float(doc.sections[0].page_width) / 914400 if doc.sections else None,
                "page_height": float(doc.sections[0].page_height) / 914400 if doc.sections else None,
            },
    }


# ----------------------------------------------------------------- xlsx


def _xlsx_cell_values(ws) -> List[Dict[str, Any]]:
    """Extract cell values with coordinates."""
    cells: List[Dict[str, Any]] = []
    for row in ws.iter_rows():
        for cell in row:
            if cell.value is not None:
                cells.append({
                    "coordinate": cell.coordinate,
                    "value": cell.value,
                    "cell_type": str(type(cell.value).__name__),
                })
    return cells


def _xlsx_cell_styles(ws) -> List[Dict[str, Any]]:
    """Extract per-cell style info (font/fill/border/alignment)."""
    styles: List[Dict[str, Any]] = []
    for row in ws.iter_rows():
        for cell in row:
            if cell.value is None:
                continue
            style: Dict[str, Any] = {"coordinate": cell.coordinate}
            if cell.font:
                style["font"] = {
                    "name": cell.font.name,
                    "size": cell.font.size,
                    "bold": cell.font.bold or False,
                    "italic": cell.font.italic or False,
                    "color": str(cell.font.color.rgb) if cell.font.color and cell.font.color.rgb else None,
                }
            if cell.fill:
                style["fill"] = {
                    "start_color": str(cell.fill.fgColor.rgb) if cell.fill.fgColor and cell.fill.fgColor.rgb else None,
                    "end_color": str(cell.fill.bgColor.rgb) if cell.fill.bgColor and cell.fill.bgColor.rgb else None,
                    "fill_type": cell.fill.fill_type,
                }
            if cell.border:
                style["border"] = {
                    k: str(v.style) for k, v in {
                        "left": cell.border.left,
                        "right": cell.border.right,
                        "top": cell.border.top,
                        "bottom": cell.border.bottom,
                    }.items() if v.style
                }
            if cell.alignment:
                style["alignment"] = {
                    "horizontal": cell.alignment.horizontal,
                    "vertical": cell.alignment.vertical,
                    "wrap_text": cell.alignment.wrap_text or False,
                }
            styles.append(style)
    return styles


def _xlsx_number_formats(ws) -> List[Dict[str, Any]]:
    """Extract cell number format info."""
    formats: List[Dict[str, Any]] = []
    for row in ws.iter_rows():
        for cell in row:
            if cell.number_format and cell.number_format != 'General':
                formats.append({
                    "coordinate": cell.coordinate,
                    "number_format": cell.number_format,
                })
    return formats


def _xlsx_freeze_panes(ws) -> Dict[str, Any]:
    return {
        "freeze_panes": getattr(ws, "freeze_panes", None),
    }


def _xlsx_sheet_views(ws) -> Dict[str, Any]:
    sv = getattr(ws, "sheet_view", None)
    return {
        "right_to_left": sv.rightToLeft if sv else False,
        "zoom_scale": sv.zoomScale if sv else None,
    }


def _xlsx_autofilter(ws) -> Dict[str, Any]:
    af = getattr(ws, "auto_filter", None)
    return {
        "autofilter": af.ref if af else None,
    }


def _xlsx_defined_names(wb) -> List[Dict[str, Any]]:
    names: List[Dict[str, Any]] = []
    for dn in wb.defined_names:
        names.append({
            "name": dn,
            "value": wb.defined_names[dn],
        })
    return names


def _xlsx_conditional_formats(ws) -> List[Dict[str, Any]]:
    cfs: List[Dict[str, Any]] = []
    cf_attr = getattr(ws, "conditional_formatting", None)
    if not cf_attr:
        return cfs
    for cf in cf_attr:
            cfs.append({
                "range": cf,
                "rules": [
                    {
                        "type": r.type,
                        "dxf": {
                            "fill": str(r.fill) if hasattr(r, "fill") and r.fill else None,
                            "font": str(r.font) if hasattr(r, "font") and r.font else None,
                        } if hasattr(r, "fill") or hasattr(r, "font") else None,
                    }
                    for r in cf._rules
                ] if hasattr(cf, "_rules") else [],
            })
    return cfs


def _xlsx_data_validations(ws) -> List[Dict[str, Any]]:
    dvs: List[Dict[str, Any]] = []
    dv_attr = getattr(ws, "data_validations", None)
    if dv_attr is None:
        return dvs
    for dv in dv_attr.dataValidation if hasattr(dv_attr, "dataValidation") else []:
        dvs.append({
            "type": dv.type,
            "operator": dv.operator,
            "formula1": dv.formula1,
            "formula2": dv.formula2,
            "sqref": dv.sqref,
        })
    return dvs


def _xlsx_tables(ws) -> List[Dict[str, Any]]:
    tables: List[Dict[str, Any]] = []
    if hasattr(ws, "tables"):
        for tbl in ws.tables:
            tables.append({
                "name": tbl.name,
                "ref": tbl.ref,
                "header_row": tbl.header_row,
                "style_name": tbl.style_name,
            })
    return tables


def _xlsx_protection(ws) -> Dict[str, Any]:
    prot = getattr(ws, "protection", None)
    if prot is None:
        return {"sheet_protection": {"sheet": None, "password": None}, "cells": {"locked": {}}}
    sheet_prot = getattr(prot, "sheet", None)
    pw = getattr(prot, "password", None)
    locked_map = {}
    try:
        for row in ws.iter_rows():
            for cell in row:
                cp = getattr(cell, "protection", None)
                if cp is not None and getattr(cp, "locked", None) is not None:
                    locked_map[cell.coordinate] = cp.locked
    except Exception:
        pass
    return {
        "sheet_protection": {
            "sheet": sheet_prot,
            "password": pw,
        },
        "cells": {
            "locked": locked_map,
        },
    }


def _xlsx_page_setup(ws) -> Dict[str, Any]:
    ps = getattr(ws, "page_setup", None)
    if ps is None:
        return {"page_setup": {}}
    return {
        "page_setup": {
            "orientation": getattr(ps, "orientation", None),
            "paper_width": getattr(ps, "paperWidth", None),
            "paper_height": getattr(ps, "paperHeight", None),
            "first_page_number": getattr(ps, "firstPageNumber", None),
            "fit_to_width": getattr(ps, "fitToWidth", None),
            "fit_to_height": getattr(ps, "fitToHeight", None),
        },
    }


def _xlsx_hyperlinks(ws) -> List[Dict[str, Any]]:
    links: List[Dict[str, Any]] = []
    if hasattr(ws, "hyperlinks"):
        for hl in ws.hyperlinks:
            if hasattr(hl, "ref"):
                links.append({"ref": hl.ref, "url": hl.url})
    return links


def _inspect_xlsx(path: Union[str, pathlib.Path]) -> Dict[str, Any]:
    if not XLSX_AVAILABLE:
        return {"error": "openpyxl not available"}
    wb = openpyxl.load_workbook(str(path), read_only=True, data_only=True)
    sheets: Dict[str, Any] = {}
    for ws in wb.worksheets:
        sheets[ws.title] = {
            "cell_values": _xlsx_cell_values(ws),
            "cell_styles": _xlsx_cell_styles(ws),
            "number_formats": _xlsx_number_formats(ws),
            "freeze_panes": _xlsx_freeze_panes(ws),
            "sheet_views": _xlsx_sheet_views(ws),
            "autofilter": _xlsx_autofilter(ws),
            "conditional_formats": _xlsx_conditional_formats(ws),
            "data_validations": _xlsx_data_validations(ws),
            "tables": _xlsx_tables(ws),
            "protection": _xlsx_protection(ws),
            "page_setup": _xlsx_page_setup(ws),
            "hyperlinks": _xlsx_hyperlinks(ws),
        }
    result = {
        "sheets": sheets,
        "defined_names": _xlsx_defined_names(wb),
        "sheet_count": len(wb.worksheets),
    }
    wb.close()
    return result


# ----------------------------------------------------------------- pptx


def _pptx_slide_title(slide) -> Optional[str]:
    """Try to extract a slide title from the first text-bearing shape."""
    if not hasattr(slide, "shapes"):
        return None
    for shape in slide.shapes:
        if hasattr(shape, "has_text_frame") and shape.has_text_frame:
            text = shape.text_frame.text.strip()
            if text:
                return text
    return None


def _pptx_shapes(slide) -> List[Dict[str, Any]]:
    """Extract shape info: text, position, fill, geometry."""
    shapes: List[Dict[str, Any]] = []
    from pptx.util import Emu

    for shape in slide.shapes:
        info: Dict[str, Any] = {
            "name": shape.name,
            "shape_type": str(shape.shape_type) if hasattr(shape, "shape_type") else None,
            "left": shape.left if shape.left is not None else 0,
            "top": shape.top if shape.top is not None else 0,
            "width": shape.width if shape.width is not None else 0,
            "height": shape.height if shape.height is not None else 0,
        }

        if shape.has_text_frame:
            info["text"] = shape.text_frame.text
            info["word_wrap"] = shape.text_frame.word_wrap
            runs_info = []
            for para in shape.text_frame.paragraphs:
                for run in para.runs:
                    try:
                        color_val = run.font.color.rgb if run.font.color else None
                    except (AttributeError, TypeError):
                        color_val = None
                    runs_info.append({
                        "text": run.text,
                        "bold": run.font.bold,
                        "italic": run.font.italic,
                        "size": int(run.font.size) if run.font.size else None,
                        "color": str(color_val) if color_val else None,
                        "name": run.font.name,
                    })
            info["runs"] = runs_info

            if shape.has_text_frame:
                for para in shape.text_frame.paragraphs:
                    for run in para.runs:
                        try:
                            if run.font.fill and hasattr(run.font.fill, "solid"):
                                sf = run.font.fill.solid()
                                if sf and hasattr(sf, "fill_color"):
                                    fill_color = sf.fill_color
                                    if hasattr(fill_color, "rgb"):
                                        info["fill"] = {
                                            "color": str(fill_color.rgb),
                                        }
                                        break
                        except (AttributeError, TypeError):
                            pass

        # Geometry
        if hasattr(shape, "shape_type") and shape.shape_type is not None:
            info["geometry"] = str(shape.shape_type)

        shapes.append(info)
    return shapes


def _pptx_tables(slide) -> List[Dict[str, Any]]:
    """Extract table data from shapes that are tables."""
    tables: List[Dict[str, Any]] = []
    for shape in slide.shapes:
        if shape.has_table:
            tbl = shape.table
            rows_data: List[List[str]] = []
            for row_idx in range(len(tbl.rows)):
                row_data: List[str] = []
                for col_idx in range(len(tbl.columns)):
                    row_data.append(tbl.cell(row_idx, col_idx).text)
                rows_data.append(row_data)
            tables.append({
                "name": shape.name,
                "rows": len(tbl.rows),
                "cols": len(tbl.columns),
                "data": rows_data,
            })
    return tables


def _pptx_placeholders(slide) -> List[Dict[str, Any]]:
    """Extract placeholder info from slides."""
    placeholders: List[Dict[str, Any]] = []
    from pptx.oxml.ns import qn
    for shape in slide.shapes:
        if not hasattr(shape, "is_placeholder") or not shape.is_placeholder:
            continue
        ph = getattr(shape, "placeholder_format", None)
        if ph is None:
            continue
        placeholders.append({
            "idx": ph.idx,
            "type": str(ph.type) if hasattr(ph, "type") else None,
            "name": shape.name,
        })
    return placeholders


def _pptx_notes(slide) -> Optional[str]:
    """Extract notes text from a slide."""
    if slide.has_notes_slide:
        notes_slide = slide.notes_slide
        if notes_slide and notes_slide.notes_text_frame:
            text = notes_slide.notes_text_frame.text.strip()
            if text:
                return text
    return None


def _pptx_theme_colors(prs) -> Dict[str, Any]:
    """Extract theme color info from presentation."""
    theme_info: Dict[str, Any] = {}
    try:
        from pptx.opc.constants import CONTENT_TYPE as CT
        from pptx.opc.part import Part

        # Try to get theme part
        if hasattr(prs, "_presentation"):
            parts = prs.part.rels
            for rel in parts.values():
                if "theme" in (rel.reltype or ""):
                    theme_part = rel.target_part
                    if hasattr(theme_part, "element"):
                        theme_info["theme_part"] = True
                        break
    except Exception:
        pass
    return theme_info


def _pptx_slide_size(prs) -> Dict[str, Any]:
    from pptx.util import Emu

    return {
        "width": int(prs.slide_width),
        "height": int(prs.slide_height),
    }


def _pptx_charts(slide) -> List[Dict[str, Any]]:
    """Extract chart series info from shapes that are charts."""
    charts: List[Dict[str, Any]] = []
    for shape in slide.shapes:
        if shape.has_chart:
            chart = shape.chart
            chart_info: Dict[str, Any] = {"chart_type": str(type(chart).__name__)}
            try:
                series_info: List[Dict[str, Any]] = []
                for si in range(len(chart.series)):
                    s = chart.series[si]
                    series_info.append({
                        "name": s.name,
                        "categories": list(s.x_values) if s.x_values else [],
                        "values": [float(v) if v is not None else None for v in s.y_values],
                    })
                chart_info["series"] = series_info
            except Exception:
                pass
            charts.append(chart_info)
    return charts


def _inspect_pptx(path: Union[str, pathlib.Path]) -> Dict[str, Any]:
    if not PPTX_AVAILABLE:
        return {"error": "python-pptx not available"}
    prs = PptxPresentation(str(path))
    slides: List[Dict[str, Any]] = []
    for slide in prs.slides:
        slides.append({
            "title": _pptx_slide_title(slide),
            "shapes": _pptx_shapes(slide),
            "tables": _pptx_tables(slide),
            "placeholders": _pptx_placeholders(slide),
            "notes": _pptx_notes(slide),
            "charts": _pptx_charts(slide),
        })

    theme_colors = _pptx_theme_colors(prs)
    slide_size = _pptx_slide_size(prs)

    return {
        "slides": slides,
        "slide_count": len(prs.slides),
        "theme_colors": theme_colors,
        "slide_size": slide_size,
    }


# ---------------------------------------------------------------- public API


def _detect_format(path: Union[str, pathlib.Path]) -> Optional[str]:
    ext = _ext(path)
    if ext == ".docx":
        return "docx"
    elif ext == ".xlsx":
        return "xlsx"
    elif ext == ".pptx":
        return "pptx"
    return None


def inspect_corpus_file(path: Union[str, pathlib.Path]) -> Dict[str, Any]:
    """Inspect a single corpus file and return a Python dict representation.

    The returned dict always contains at least:
      - "path": the input path
      - "format": "docx" | "xlsx" | "pptx" | None
      - "document_type": "document" | "spreadsheet" | "presentation" | None
      - Plus format-specific keys (text, sheets, slides, etc.)
    """
    path = str(path)
    fmt = _detect_format(path)
    if fmt is None:
        return {
            "path": path,
            "format": None,
            "error": f"Unsupported file format: {_ext(path)}",
        }

    if fmt == "docx":
        result = _inspect_docx(path)
        result.update({"path": path, "format": "docx", "document_type": "document"})
    elif fmt == "xlsx":
        result = _inspect_xlsx(path)
        result.update({"path": path, "format": "xlsx", "document_type": "spreadsheet"})
    elif fmt == "pptx":
        result = _inspect_pptx(path)
        result.update({"path": path, "format": "pptx", "document_type": "presentation"})
    else:
        result = {"path": path, "format": None, "error": f"Unknown format: {fmt}"}

    return result


def list_corpus_files(
    directory: Union[str, pathlib.Path],
    extensions: Optional[set] = None,
) -> List[str]:
    """List corpus files in a directory, optionally filtered by extension."""
    if extensions is None:
        extensions = _SUPPORTED
    dirpath = pathlib.Path(directory)
    files: List[str] = []
    if dirpath.is_dir():
        for p in sorted(dirpath.iterdir()):
            if p.is_file() and p.suffix.lower() in extensions:
                files.append(str(p))
    return files
