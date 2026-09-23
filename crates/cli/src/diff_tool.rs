// SPDX-License-Identifier: MIT OR Apache-2.0
//! diff_documents tool: compare two documents structurally.
//!
//! Parses both documents into gp-core IR, runs `lint::diff::diff_documents`,
//! and returns a JSON result with the structural diff.

use anyhow::{anyhow, Context, Result};
use gp_core::ir::{Document, PdfDocument, Presentation, Spreadsheet};
use lint::diff;
use lint::diff::IrDiff;

/// Compare two documents and return structural + XML differences.
///
/// If `max_ops` is set, the ops array is truncated to that many entries
/// and the `ir_diff.truncated` flag is set to `true`.
pub fn diff_documents(
    a_path: &str,
    b_path: &str,
    max_ops: Option<usize>,
) -> Result<serde_json::Value> {
    let a_bytes = std::fs::read(a_path).with_context(|| format!("read {}", a_path))?;
    let b_bytes = std::fs::read(b_path).with_context(|| format!("read {}", b_path))?;

    let (a_ir, a_type) = parse_document(&a_bytes, a_path)?;
    let (b_ir, b_type) = parse_document(&b_bytes, b_path)?;

    // Both must be compatible document types for comparison
    let compatible = (a_type == b_type)
        || (matches!(a_type.as_str(), "xlsx" | "ods") && matches!(b_type.as_str(), "xlsx" | "ods"))
        || (matches!(a_type.as_str(), "pptx" | "odp") && matches!(b_type.as_str(), "pptx" | "odp"))
        || (matches!(a_type.as_str(), "docx" | "odt") && matches!(b_type.as_str(), "docx" | "odt"));
    if !compatible {
        anyhow::bail!("incompatible document types: {} vs {}", a_type, b_type);
    }

    // Normalize the type name for the response
    let response_type = match (a_type.as_str(), b_type.as_str()) {
        ("xlsx" | "ods", _) => "spreadsheet".to_string(),
        ("pptx" | "odp", _) => "presentation".to_string(),
        ("docx" | "odt", _) => "document".to_string(),
        ("pdf", _) => "pdf".to_string(),
        _ => a_type.clone(),
    };

    let ir_diff = match (a_type.as_str(), b_type.as_str()) {
        ("docx" | "odt", _) => {
            let doc_a: Document =
                serde_json::from_slice(&a_ir).context("deserialize document A")?;
            let doc_b: Document =
                serde_json::from_slice(&b_ir).context("deserialize document B")?;
            lint::diff::diff_documents(&doc_a, &doc_b)
        }
        ("pdf", _) => {
            let pdf_a: PdfDocument = serde_json::from_slice(&a_ir).context("deserialize PDF A")?;
            let pdf_b: PdfDocument = serde_json::from_slice(&b_ir).context("deserialize PDF B")?;
            diff_documents_pdf(&pdf_a, &pdf_b)
        }
        ("xlsx" | "ods", _) => {
            let ss_a: Spreadsheet =
                serde_json::from_slice(&a_ir).context("deserialize spreadsheet A")?;
            let ss_b: Spreadsheet =
                serde_json::from_slice(&b_ir).context("deserialize spreadsheet B")?;
            diff_documents_spreadsheet(&ss_a, &ss_b)
        }
        ("pptx" | "odp", _) => {
            let pres_a: Presentation =
                serde_json::from_slice(&a_ir).context("deserialize presentation A")?;
            let pres_b: Presentation =
                serde_json::from_slice(&b_ir).context("deserialize presentation B")?;
            diff_documents_presentation(&pres_a, &pres_b)
        }
        (other, _) => anyhow::bail!("unsupported document type for diff: {}", other),
    };

    let total_ops = ir_diff.total_ops;
    let truncated = max_ops.is_some_and(|m| total_ops > m);

    let ops_json: Vec<serde_json::Value> = ir_diff
        .ops
        .iter()
        .map(|op| match op {
            diff::DiffOp::Removed { id, kind } => serde_json::json!({
                "op": "removed", "id": id, "kind": kind
            }),
            diff::DiffOp::Added { id, kind } => serde_json::json!({
                "op": "added", "id": id, "kind": kind
            }),
            diff::DiffOp::Modified { id, kind } => serde_json::json!({
                "op": "modified", "id": id, "kind": kind
            }),
        })
        .collect();

    let ops_json: Vec<serde_json::Value> = match max_ops {
        Some(limit) if total_ops > limit => ops_json.into_iter().take(limit).collect(),
        _ => ops_json,
    };

    Ok(serde_json::json!({
        "tool": "diff_documents",
        "path_a": a_path,
        "path_b": b_path,
        "document_type": response_type,
        "ir_diff": {
            "total_ops": total_ops,
            "removed": ir_diff.removed,
            "added": ir_diff.added,
            "modified": ir_diff.modified,
            "identical": ir_diff.identical,
            "truncated": truncated,
            "ops": ops_json,
        },
        "xml_diffs": Vec::<serde_json::Value>::new(),
    }))
}

/// Compare two PDF documents by serializing their IR to JSON and diffing.
fn diff_documents_pdf(a: &PdfDocument, b: &PdfDocument) -> IrDiff {
    let json_a = serde_json::to_string(a).unwrap_or_default();
    let json_b = serde_json::to_string(b).unwrap_or_default();

    if json_a == json_b {
        return diff::IrDiff::empty();
    }

    let mut ops = Vec::new();

    // Compare pages
    let a_pages = a.pages.len();
    let b_pages = b.pages.len();

    if a_pages != b_pages {
        ops.push(diff::DiffOp::Modified {
            id: "pages".to_string(),
            kind: format!("page_count:{}→{}", a_pages, b_pages),
        });
    }

    // Compare annotations
    let a_annot = a.annotations.len();
    let b_annot = b.annotations.len();
    if a_annot != b_annot {
        ops.push(diff::DiffOp::Modified {
            id: "annotations".to_string(),
            kind: format!("count:{}→{}", a_annot, b_annot),
        });
    }

    // Compare outlines
    let a_outline = outline_count(&a.outlines);
    let b_outline = outline_count(&b.outlines);
    if a_outline != b_outline {
        ops.push(diff::DiffOp::Modified {
            id: "outlines".to_string(),
            kind: format!("count:{}→{}", a_outline, b_outline),
        });
    }

    let removed = ops
        .iter()
        .filter(|op| matches!(op, diff::DiffOp::Removed { .. }))
        .count();
    let added = ops
        .iter()
        .filter(|op| matches!(op, diff::DiffOp::Added { .. }))
        .count();
    let modified = ops
        .iter()
        .filter(|op| matches!(op, diff::DiffOp::Modified { .. }))
        .count();
    let identical = ops.is_empty();

    diff::IrDiff {
        ops,
        total_ops: removed + added + modified,
        removed,
        added,
        modified,
        identical,
    }
}

fn outline_count(outlines: &[gp_core::ir::PdfOutlineItem]) -> usize {
    outlines
        .iter()
        .map(|o| 1 + outline_count(&o.children))
        .sum()
}

/// Parse document bytes into IR bytes and return the document type.
fn parse_document(bytes: &[u8], path: &str) -> Result<(Vec<u8>, String)> {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();

    match ext.as_str() {
        "docx" => parse_docx(bytes).map(|ir| (ir, "docx".to_string())),
        "odt" => parse_odt(bytes).map(|ir| (ir, "odt".to_string())),
        "pdf" => parse_pdf(bytes).map(|ir| (ir, "pdf".to_string())),
        "xlsx" => parse_xlsx(bytes).map(|ir| (ir, "xlsx".to_string())),
        "pptx" => parse_pptx(bytes).map(|ir| (ir, "pptx".to_string())),
        "ods" => parse_ods(bytes).map(|ir| (ir, "ods".to_string())),
        "odp" => parse_odp(bytes).map(|ir| (ir, "odp".to_string())),
        _ => anyhow::bail!("unsupported format: {}", path),
    }
}

/// Parse a DOCX file into Document IR bytes.
fn parse_docx(bytes: &[u8]) -> Result<Vec<u8>> {
    let package = adapters_ooxml::opc::parse_opc(bytes).map_err(|diags| {
        anyhow!(
            "parse OPC: {}",
            diags
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    let result = adapters_ooxml::docx::read_docx(&package).map_err(|diags| {
        anyhow!(
            "read DOCX: {}",
            diags
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    Ok(result.ir)
}

/// Parse an ODT file into Document IR bytes.
fn parse_odt(bytes: &[u8]) -> Result<Vec<u8>> {
    let package = adapters_odf::manifest::parse_odf(bytes).map_err(|diags| {
        anyhow!(
            "parse ODF: {}",
            diags
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    let result = adapters_odf::odt::read_odt(&package).map_err(|diags| {
        anyhow!(
            "read ODT: {}",
            diags
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    Ok(result.ir)
}

/// Parse a PDF file into PdfDocument IR bytes.
fn parse_pdf(bytes: &[u8]) -> Result<Vec<u8>> {
    let pdf = adapters_pdf::parser::parse_pdf(bytes).map_err(|diags| {
        anyhow!(
            "parse PDF: {}",
            diags
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    serde_json::to_vec(&pdf).map_err(|e| anyhow!("serialize PDF IR: {}", e))
}

/// Parse an XLSX file into Spreadsheet IR bytes.
fn parse_xlsx(bytes: &[u8]) -> Result<Vec<u8>> {
    let package = adapters_ooxml::opc::parse_opc(bytes).map_err(|diags| {
        anyhow!(
            "parse OPC: {}",
            diags
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    let result = adapters_ooxml::xlsx::read_xlsx(&package).map_err(|diags| {
        anyhow!(
            "read XLSX: {}",
            diags
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    Ok(result.ir)
}

/// Parse a PPTX file into Presentation IR bytes.
fn parse_pptx(bytes: &[u8]) -> Result<Vec<u8>> {
    let package = adapters_ooxml::opc::parse_opc(bytes).map_err(|diags| {
        anyhow!(
            "parse OPC: {}",
            diags
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    let result = adapters_ooxml::pptx::read_pptx(&package).map_err(|diags| {
        anyhow!(
            "read PPTX: {}",
            diags
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    Ok(result.ir)
}

/// Parse an ODS file into Spreadsheet IR bytes.
fn parse_ods(bytes: &[u8]) -> Result<Vec<u8>> {
    let package = adapters_odf::manifest::parse_odf(bytes).map_err(|diags| {
        anyhow!(
            "parse ODF: {}",
            diags
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    let result = adapters_odf::ods::read_ods(&package).map_err(|diags| {
        anyhow!(
            "read ODS: {}",
            diags
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    Ok(result.ir)
}

/// Parse an ODP file into Presentation IR bytes.
fn parse_odp(bytes: &[u8]) -> Result<Vec<u8>> {
    let package = adapters_odf::manifest::parse_odf(bytes).map_err(|diags| {
        anyhow!(
            "parse ODF: {}",
            diags
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    let result = adapters_odf::odp::read_odp(&package).map_err(|diags| {
        anyhow!(
            "read ODP: {}",
            diags
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    Ok(result.ir)
}

/// Compare two spreadsheet IRs and return a structural diff.
fn diff_documents_spreadsheet(a: &Spreadsheet, b: &Spreadsheet) -> IrDiff {
    let mut ops = Vec::new();

    let a_sheet_count = a.sheets.len();
    let b_sheet_count = b.sheets.len();

    if a_sheet_count != b_sheet_count {
        ops.push(diff::DiffOp::Modified {
            id: "sheets".to_string(),
            kind: format!("count:{}→{}", a_sheet_count, b_sheet_count),
        });
    }

    // Build name→index maps for matching
    let a_sheet_names: std::collections::HashMap<&str, usize> = a
        .sheets
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name.as_str(), i))
        .collect();
    let b_sheet_names: std::collections::HashMap<&str, usize> = b
        .sheets
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name.as_str(), i))
        .collect();

    // Sheets added in B (not in A)
    for name in b_sheet_names.keys() {
        if !a_sheet_names.contains_key(name) {
            ops.push(diff::DiffOp::Added {
                id: format!("sheet:{}", name),
                kind: "sheet".to_string(),
            });
        }
    }

    // Sheets removed in B (in A but not B)
    for name in a_sheet_names.keys() {
        if !b_sheet_names.contains_key(name) {
            ops.push(diff::DiffOp::Removed {
                id: format!("sheet:{}", name),
                kind: "sheet".to_string(),
            });
        }
    }

    // For matching sheet names, compare row counts and cells
    let common_sheets: Vec<&str> = a_sheet_names
        .keys()
        .filter(|n| b_sheet_names.contains_key(**n))
        .copied()
        .collect();

    for sheet_name in &common_sheets {
        let a_si = a_sheet_names[sheet_name];
        let b_si = b_sheet_names[sheet_name];
        let a_sheet = &a.sheets[a_si];
        let b_sheet = &b.sheets[b_si];

        let a_row_count = a_sheet.rows.len();
        let b_row_count = b_sheet.rows.len();

        if a_row_count != b_row_count {
            ops.push(diff::DiffOp::Modified {
                id: format!("sheet:{}:rows", sheet_name),
                kind: format!("count:{}→{}", a_row_count, b_row_count),
            });
        }

        let max_rows = a_row_count.max(b_row_count);
        for ri in 0..max_rows {
            let row_id = format!("sheet:{}:row:{}", sheet_name, ri);
            match (a_sheet.rows.get(ri), b_sheet.rows.get(ri)) {
                (None, Some(_)) => {
                    ops.push(diff::DiffOp::Added {
                        id: row_id,
                        kind: "sheet_row".to_string(),
                    });
                }
                (Some(_), None) => {
                    ops.push(diff::DiffOp::Removed {
                        id: row_id,
                        kind: "sheet_row".to_string(),
                    });
                }
                (Some(a_row), Some(b_row)) => {
                    let a_cell_count = a_row.cells.len();
                    let b_cell_count = b_row.cells.len();

                    if a_cell_count != b_cell_count {
                        ops.push(diff::DiffOp::Modified {
                            id: format!("{}:cells", row_id),
                            kind: format!("count:{}→{}", a_cell_count, b_cell_count),
                        });
                    }

                    let max_cells = a_cell_count.max(b_cell_count);
                    for ci in 0..max_cells {
                        let cell_id = format!("{}:cell:{}", row_id, ci);
                        match (a_row.cells.get(ci), b_row.cells.get(ci)) {
                            (None, Some(_)) => {
                                ops.push(diff::DiffOp::Added {
                                    id: cell_id,
                                    kind: "sheet_cell".to_string(),
                                });
                            }
                            (Some(_), None) => {
                                ops.push(diff::DiffOp::Removed {
                                    id: cell_id,
                                    kind: "sheet_cell".to_string(),
                                });
                            }
                            (Some(a_cell), Some(b_cell)) => {
                                let a_val = a_cell.value.as_deref().unwrap_or_default();
                                let b_val = b_cell.value.as_deref().unwrap_or_default();
                                let a_formula = a_cell.formula.as_deref().unwrap_or_default();
                                let b_formula = b_cell.formula.as_deref().unwrap_or_default();

                                let mut changes = Vec::new();
                                if a_val != b_val {
                                    changes.push("value");
                                }
                                if a_formula != b_formula {
                                    changes.push("formula");
                                }

                                if !changes.is_empty() {
                                    ops.push(diff::DiffOp::Modified {
                                        id: cell_id,
                                        kind: changes.join(","),
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let removed = ops
        .iter()
        .filter(|op| matches!(op, diff::DiffOp::Removed { .. }))
        .count();
    let added = ops
        .iter()
        .filter(|op| matches!(op, diff::DiffOp::Added { .. }))
        .count();
    let modified = ops
        .iter()
        .filter(|op| matches!(op, diff::DiffOp::Modified { .. }))
        .count();
    let identical = ops.is_empty();

    diff::IrDiff {
        ops,
        total_ops: removed + added + modified,
        removed,
        added,
        modified,
        identical,
    }
}

/// Compare two presentation IRs and return a structural diff.
fn diff_documents_presentation(a: &Presentation, b: &Presentation) -> IrDiff {
    let mut ops = Vec::new();

    let a_slide_count = a.slides.len();
    let b_slide_count = b.slides.len();

    if a_slide_count != b_slide_count {
        ops.push(diff::DiffOp::Modified {
            id: "slides".to_string(),
            kind: format!("count:{}→{}", a_slide_count, b_slide_count),
        });
    }

    // Layouts comparison
    let a_layout_count = a.layouts.len();
    let b_layout_count = b.layouts.len();
    if a_layout_count != b_layout_count {
        ops.push(diff::DiffOp::Modified {
            id: "layouts".to_string(),
            kind: format!("count:{}→{}", a_layout_count, b_layout_count),
        });
    }

    // Masters comparison
    let a_master_count = a.masters.len();
    let b_master_count = b.masters.len();
    if a_master_count != b_master_count {
        ops.push(diff::DiffOp::Modified {
            id: "masters".to_string(),
            kind: format!("count:{}→{}", a_master_count, b_master_count),
        });
    }

    // Per-slide comparison
    let max_slides = a_slide_count.max(b_slide_count);
    for si in 0..max_slides {
        let slide_id = format!("slide:{}", si);
        match (a.slides.get(si), b.slides.get(si)) {
            (None, Some(_)) => {
                ops.push(diff::DiffOp::Added {
                    id: slide_id,
                    kind: "slide".to_string(),
                });
            }
            (Some(_), None) => {
                ops.push(diff::DiffOp::Removed {
                    id: slide_id,
                    kind: "slide".to_string(),
                });
            }
            (Some(a_slide), Some(b_slide)) => {
                let a_el_count = a_slide.elements.len();
                let b_el_count = b_slide.elements.len();

                if a_el_count != b_el_count {
                    ops.push(diff::DiffOp::Modified {
                        id: format!("{}:elements", slide_id),
                        kind: format!("count:{}→{}", a_el_count, b_el_count),
                    });
                }

                let max_elements = a_el_count.max(b_el_count);
                for ei in 0..max_elements {
                    let el_id = format!("{}:element:{}", slide_id, ei);
                    match (a_slide.elements.get(ei), b_slide.elements.get(ei)) {
                        (None, Some(_)) => {
                            ops.push(diff::DiffOp::Added {
                                id: el_id,
                                kind: "shape".to_string(),
                            });
                        }
                        (Some(_), None) => {
                            ops.push(diff::DiffOp::Removed {
                                id: el_id,
                                kind: "shape".to_string(),
                            });
                        }
                        (Some(a_el), Some(b_el)) if a_el.kind != b_el.kind => {
                            ops.push(diff::DiffOp::Modified {
                                id: el_id,
                                kind: format!("kind:{}→{}", a_el.kind, b_el.kind),
                            });
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let removed = ops
        .iter()
        .filter(|op| matches!(op, diff::DiffOp::Removed { .. }))
        .count();
    let added = ops
        .iter()
        .filter(|op| matches!(op, diff::DiffOp::Added { .. }))
        .count();
    let modified = ops
        .iter()
        .filter(|op| matches!(op, diff::DiffOp::Modified { .. }))
        .count();
    let identical = ops.is_empty();

    diff::IrDiff {
        ops,
        total_ops: removed + added + modified,
        removed,
        added,
        modified,
        identical,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_truncation_logic() {
        let ops = vec![
            serde_json::json!({"op": "added", "id": "id-001", "kind": "Paragraph"}),
            serde_json::json!({"op": "added", "id": "id-002", "kind": "Run"}),
            serde_json::json!({"op": "removed", "id": "id-003", "kind": "Paragraph"}),
            serde_json::json!({"op": "modified", "id": "id-004", "kind": "Run"}),
            serde_json::json!({"op": "added", "id": "id-005", "kind": "Table"}),
        ];
        let total_ops = 5;

        // No limit: keep all
        let result = match None::<usize> {
            Some(limit) if total_ops > limit => ops.iter().take(limit).cloned().collect(),
            _ => ops.clone(),
        };
        assert_eq!(result.len(), 5);

        // Limit 3: truncate to 3, set truncated flag
        let truncated = total_ops > 3;
        let result = match Some(3usize) {
            Some(limit) if total_ops > limit => ops.iter().take(limit).cloned().collect(),
            _ => ops.clone(),
        };
        assert_eq!(result.len(), 3);
        assert!(truncated);

        // Limit 10: no truncation needed
        let truncated = total_ops > 10;
        let result = match Some(10usize) {
            Some(limit) if total_ops > limit => ops.iter().take(limit).cloned().collect(),
            _ => ops.clone(),
        };
        assert_eq!(result.len(), 5);
        assert!(!truncated);
    }

    #[test]
    fn test_truncated_flag_logic() {
        assert!(!max_ops_truncates(None, 0));
        assert!(!max_ops_truncates(None, 5));
        assert!(!max_ops_truncates(Some(10), 5));
        assert!(max_ops_truncates(Some(3), 5));
        assert!(max_ops_truncates(Some(5), 6));
        assert!(!max_ops_truncates(Some(5), 5));
    }

    fn max_ops_truncates(max_ops: Option<usize>, total_ops: usize) -> bool {
        max_ops.is_some_and(|m| total_ops > m)
    }
}
