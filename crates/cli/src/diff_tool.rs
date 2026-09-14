//! diff_documents tool: compare two documents structurally.
//!
//! Parses both documents into gp-core IR, runs `lint::diff::diff_documents`,
//! and returns a JSON result with the structural diff.

use anyhow::{anyhow, Context, Result};
use gp_core::ir::Document;
use gp_core::ir::PdfDocument;
use lint::diff;
use lint::diff::IrDiff;

/// Compare two documents and return structural + XML differences.
pub fn diff_documents(a_path: &str, b_path: &str) -> Result<serde_json::Value> {
    let a_bytes = std::fs::read(a_path).with_context(|| format!("read {}", a_path))?;
    let b_bytes = std::fs::read(b_path).with_context(|| format!("read {}", b_path))?;

    let (a_ir, a_type) = parse_document(&a_bytes, a_path)?;
    let (b_ir, b_type) = parse_document(&b_bytes, b_path)?;

    // Both must be the same type for comparison
    if a_type != b_type {
        anyhow::bail!("incompatible document types: {} vs {}", a_type, b_type);
    }

    let ir_diff = match a_type.as_str() {
        "docx" | "odt" => {
            let doc_a: Document =
                serde_json::from_slice(&a_ir).context("deserialize document A")?;
            let doc_b: Document =
                serde_json::from_slice(&b_ir).context("deserialize document B")?;
            lint::diff::diff_documents(&doc_a, &doc_b)
        }
        "pdf" => {
            let pdf_a: PdfDocument = serde_json::from_slice(&a_ir).context("deserialize PDF A")?;
            let pdf_b: PdfDocument = serde_json::from_slice(&b_ir).context("deserialize PDF B")?;
            diff_documents_pdf(&pdf_a, &pdf_b)
        }
        other => anyhow::bail!("unsupported document type for diff: {}", other),
    };

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

    Ok(serde_json::json!({
        "tool": "diff_documents",
        "path_a": a_path,
        "path_b": b_path,
        "document_type": a_type,
        "ir_diff": {
            "total_ops": ir_diff.total_ops,
            "removed": ir_diff.removed,
            "added": ir_diff.added,
            "modified": ir_diff.modified,
            "identical": ir_diff.identical,
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
