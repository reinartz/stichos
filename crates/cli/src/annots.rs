// SPDX-License-Identifier: MIT OR Apache-2.0
/// `stichos annots` subcommand — list/resolve PDF annotations.
///
/// Subcommands:
/// - `list` — list annotations in a PDF document
/// - `resolve` — resolve or reject annotations in a PDF document
///
/// Options: `--id`, `--author`, `--all`, `--dry-run`, `--json`
use anyhow::Result;
use clap::Subcommand;
use gp_core::ir::{AnnotationKind, AnnotationState};
use serde_json::json;

#[derive(Subcommand)]
pub enum AnnotsCommand {
    /// List annotations in a PDF document.
    List {
        /// Path to the PDF document.
        #[arg(long)]
        path: String,
        /// Show all annotations including unsupported kinds.
        #[arg(long)]
        all: bool,
        /// Filter by author name.
        #[arg(long)]
        author: Option<String>,
    },
    /// Resolve or reject annotations in a PDF document.
    Resolve {
        /// Path to the PDF document.
        #[arg(long)]
        path: String,
        /// Annotation IDs to resolve/reject.
        #[arg(long)]
        ids: Option<Vec<String>>,
        /// Operate on all annotations.
        #[arg(long)]
        all: bool,
        /// Filter by author name.
        #[arg(long)]
        author: Option<String>,
        /// Action: resolve or reject.
        #[arg(long)]
        state: String,
        /// Preview changes without applying.
        #[arg(long)]
        dry_run: bool,
        /// Journal session ID.
        #[arg(long)]
        session: Option<String>,
    },
}

/// Handle the `stichos annots` subcommand.
pub async fn handle_annots(cmd: AnnotsCommand) -> Result<()> {
    match cmd {
        AnnotsCommand::List { path, all, author } => {
            let result = list_annotations(&path, all, author.as_deref())?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        AnnotsCommand::Resolve {
            path,
            ids,
            all,
            author,
            state,
            dry_run,
            session,
        } => {
            let result = resolve_annotations(
                &path,
                ids.as_deref(),
                all,
                author.as_deref(),
                &state,
                dry_run,
                session.as_deref(),
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }
    Ok(())
}

/// List annotations from a PDF document.
///
/// Returns JSON with annotations array, counts, and document version hash.
pub fn list_annotations(path: &str, all: bool, author: Option<&str>) -> Result<serde_json::Value> {
    let bytes = std::fs::read(path).with_context(|| format!("failed to read {}", path))?;

    let mut doc = adapters_pdf::parser::parse_pdf_data(&bytes).map_err(|diags| {
        anyhow::anyhow!(
            "failed to parse PDF {}: diagnostics: {}",
            path,
            diags
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;

    let annotations = adapters_pdf::annot::extract_annotations(&mut doc);

    let version_hash = sha2::Sha256::digest(&bytes);
    let doc_version = format!("{:x}", version_hash);

    // Filter annotations
    let filtered: Vec<serde_json::Value> = annotations
        .iter()
        .filter(|a| {
            if !all && !a.is_supported_kind() {
                return false;
            }
            match author {
                Some(auth) => match &a.author {
                    Some(a) => a == auth,
                    None => false,
                },
                None => true,
            }
        })
        .map(annotation_to_json)
        .collect();

    let supported: usize = filtered
        .iter()
        .filter(|a| a.get("kind").and_then(|k| k.as_str()).is_some())
        .count();
    let unsupported: usize = filtered.len() - supported;

    // Count by state
    let mut state_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for a in &filtered {
        let state = a
            .get("state")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown")
            .to_string();
        *state_counts.entry(state).or_default() += 1;
    }

    // Count by page
    let mut page_counts: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for a in &filtered {
        if let Some(page) = a.get("page_number").and_then(|p| p.as_u64()) {
            *page_counts.entry(page as usize).or_default() += 1;
        }
    }

    Ok(json!({
        "tool": "list_annotations",
        "path": path,
        "doc_version": doc_version,
        "annotation_count": filtered.len(),
        "annotations": filtered,
        "counts": {
            "total": filtered.len(),
            "supported": supported,
            "unsupported": unsupported,
            "by_state": state_counts,
            "by_page": page_counts
        }
    }))
}

/// Resolve or reject annotations in a PDF document.
///
/// Returns JSON with acceptance status, changed annotations, byte diff, and journal ID.
#[allow(clippy::too_many_arguments)]
pub fn resolve_annotations(
    path: &str,
    ids: Option<&[String]>,
    all: bool,
    author: Option<&str>,
    state: &str,
    dry_run: bool,
    session: Option<&str>,
) -> Result<serde_json::Value> {
    let bytes = std::fs::read(path).with_context(|| format!("failed to read {}", path))?;

    let mut doc = adapters_pdf::parser::parse_pdf_data(&bytes).map_err(|diags| {
        anyhow::anyhow!(
            "failed to parse PDF {}: diagnostics: {}",
            path,
            diags
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;

    let annotations = adapters_pdf::annot::extract_annotations(&mut doc);

    // Determine which annotations to operate on
    let target_ids: Vec<String> = if all {
        annotations
            .iter()
            .filter(|a| match author {
                Some(auth) => match &a.author {
                    Some(an) => an == auth,
                    None => false,
                },
                None => true,
            })
            .map(|a| a.id.to_string())
            .collect()
    } else if let Some(id_list) = ids {
        id_list.to_vec()
    } else if let Some(auth) = author {
        annotations
            .iter()
            .filter(|a| match &a.author {
                Some(an) => an == auth,
                None => false,
            })
            .map(|a| a.id.to_string())
            .collect()
    } else {
        Vec::new()
    };

    // Map state string to AnnotOp variant
    let state_lower = state.to_lowercase();
    let ops: Vec<adapters_pdf::annot_write::AnnotOp> = target_ids
        .iter()
        .map(|id| {
            if state_lower == "rejected" || state_lower == "reject" {
                adapters_pdf::annot_write::AnnotOp::Reject { id: id.clone() }
            } else {
                adapters_pdf::annot_write::AnnotOp::Resolve { id: id.clone() }
            }
        })
        .collect();

    let result =
        adapters_pdf::annot_write::apply_annot_ops(&bytes, &annotations, &ops, dry_run, false)
            .map_err(|diags| {
                anyhow::anyhow!(
                    "apply_annot_ops failed: {}",
                    diags
                        .iter()
                        .map(|d| d.message.clone())
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            })?;

    let journal_id = session
        .map(String::from)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    Ok(json!({
        "tool": "resolve_annotations",
        "path": path,
        "accepted": result.accepted,
        "annotations_changed": result.annotations_changed,
        "annotations_added": result.annotations_added,
        "byte_diff": {
            "original_size": result.byte_diff.original_size,
            "output_size": result.byte_diff.output_size,
            "byte_delta": result.byte_diff.byte_delta()
        },
        "dry_run": dry_run,
        "journal_id": journal_id,
        "session": session.map(String::from),
        "diagnostics": result.diagnostics.iter().map(|d| {
            json!({
                "code": d.code,
                "severity": format!("{:?}", d.severity),
                "message": d.message
            })
        }).collect::<Vec<_>>()
    }))
}

/// Convert a PdfAnnotation to a JSON value for list output.
fn annotation_to_json(a: &gp_core::ir::PdfAnnotation) -> serde_json::Value {
    let kind_str = a.kind.as_ref().map(|k| kind_to_str(k));
    let state_str = a.state.map(|s| match s {
        AnnotationState::Applied => "applied",
        AnnotationState::Resolved => "resolved",
        AnnotationState::Rejected => "rejected",
    });
    json!({
        "id": a.id.to_string(),
        "annotation_type": a.annotation_type,
        "page_number": a.page_number,
        "kind": kind_str,
        "contents": a.contents.clone(),
        "author": a.author.clone(),
        "date": a.date.clone(),
        "rect": a.rect,
        "state": state_str,
        "resolved": a.resolved,
        "quote": a.quote.clone()
    })
}

/// Convert an AnnotationKind enum to a string.
fn kind_to_str(k: &AnnotationKind) -> &'static str {
    match k {
        AnnotationKind::Highlight => "highlight",
        AnnotationKind::Text => "text",
        AnnotationKind::FreeText => "freetext",
        AnnotationKind::Stamp => "stamp",
        AnnotationKind::Line => "line",
        AnnotationKind::Square => "square",
        AnnotationKind::Circle => "circle",
        AnnotationKind::Underline => "underline",
        AnnotationKind::StrikeOut => "strikeout",
        AnnotationKind::Squiggly => "squiggly",
        AnnotationKind::Caret => "caret",
        AnnotationKind::Ink => "ink",
        AnnotationKind::Popup => "popup",
        AnnotationKind::FileAttachment => "fileattachment",
        AnnotationKind::Sound => "sound",
        AnnotationKind::Movie => "movie",
        AnnotationKind::Widget => "widget",
        AnnotationKind::Screen => "screen",
        AnnotationKind::PrinterMark => "printermark",
        AnnotationKind::TrapNet => "trapnet",
        AnnotationKind::Watermark => "watermark",
    }
}

// Re-export for use in main.rs
use anyhow::Context;
use sha2::Digest;
