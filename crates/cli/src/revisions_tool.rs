// SPDX-License-Identifier: MIT OR Apache-2.0
//! `stichos revisions` subcommand — list/accept/reject tracked revisions.
//!
//! Subcommands:
//! - `list` — list tracked revisions in a document
//! - `accept` — accept tracked revisions
//! - `reject` — reject tracked revisions
//!
//! Options: `--id`, `--author`, `--all`, `--dry-run`, `--json`

use clap::Subcommand;
use gp_core::ir::*;
use gp_core::policy::EditPolicy;
use serde_json::json;

#[derive(Subcommand)]
pub enum RevisionsCommand {
    /// List tracked revisions in a document.
    List {
        /// Path to the document file.
        #[arg(long)]
        path: String,
        /// Filter by author name.
        #[arg(long)]
        author: Option<String>,
    },
    /// Accept tracked revisions in a document.
    Accept {
        /// Path to the document file.
        #[arg(long)]
        path: String,
        /// Revision IDs to accept (accepts all if omitted).
        #[arg(long)]
        ids: Option<Vec<String>>,
        /// Accept all revisions.
        #[arg(long)]
        all: bool,
        /// Accept only revisions by this author.
        #[arg(long)]
        author: Option<String>,
        /// Preview changes without applying.
        #[arg(long)]
        dry_run: bool,
    },
    /// Reject tracked revisions in a document.
    Reject {
        /// Path to the document file.
        #[arg(long)]
        path: String,
        /// Revision IDs to reject (rejects all if omitted).
        #[arg(long)]
        ids: Option<Vec<String>>,
        /// Reject all revisions.
        #[arg(long)]
        all: bool,
        /// Reject only revisions by this author.
        #[arg(long)]
        author: Option<String>,
        /// Preview changes without applying.
        #[arg(long)]
        dry_run: bool,
    },
}

/// Handle the `stichos revisions` subcommand.
pub async fn handle_revisions(cmd: RevisionsCommand) -> Result<(), anyhow::Error> {
    match cmd {
        RevisionsCommand::List { path, author } => {
            cmd_revisions_list(&path, author.as_deref())?;
        }
        RevisionsCommand::Accept {
            path,
            ids,
            all,
            author,
            dry_run,
        } => {
            cmd_revisions_accept(&path, ids.as_deref(), all, author.as_deref(), dry_run)?;
        }
        RevisionsCommand::Reject {
            path,
            ids,
            all,
            author,
            dry_run,
        } => {
            cmd_revisions_reject(&path, ids.as_deref(), all, author.as_deref(), dry_run)?;
        }
    }
    Ok(())
}

/// Return the ODF file extension from a path.
fn odf_ext(path: &str) -> &'static str {
    let p = path.to_lowercase();
    if p.ends_with(".odt") {
        "odt"
    } else if p.ends_with(".ods") {
        "ods"
    } else {
        "odp"
    }
}

fn cmd_revisions_list(path: &str, author_filter: Option<&str>) -> Result<(), anyhow::Error> {
    let result = list_revisions_for_mcp(path, author_filter)?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    eprintln!(
        "list_revisions: {} revisions in {}",
        result["revision_count"].as_u64().unwrap_or(0),
        path
    );
    Ok(())
}

fn cmd_revisions_accept(
    path: &str,
    ids: Option<&[String]>,
    all: bool,
    author: Option<&str>,
    dry_run: bool,
) -> Result<(), anyhow::Error> {
    let result = accept_revisions_for_mcp(path, ids, all, author, dry_run, None)?;
    if dry_run {
        eprintln!(
            "accept_revisions: dry_run — {} revision(s) would be accepted",
            result["operations_succeeded"].as_u64().unwrap_or(0)
        );
    } else {
        eprintln!(
            "accept_revisions: {} revision(s) accepted in {}",
            result["operations_succeeded"].as_u64().unwrap_or(0),
            path
        );
    }
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn cmd_revisions_reject(
    path: &str,
    ids: Option<&[String]>,
    all: bool,
    author: Option<&str>,
    dry_run: bool,
) -> Result<(), anyhow::Error> {
    let result = reject_revisions_for_mcp(path, ids, all, author, dry_run, None)?;
    if dry_run {
        eprintln!(
            "reject_revisions: dry_run — {} revision(s) would be rejected",
            result["operations_succeeded"].as_u64().unwrap_or(0)
        );
    } else {
        eprintln!(
            "reject_revisions: {} revision(s) rejected in {}",
            result["operations_succeeded"].as_u64().unwrap_or(0),
            path
        );
    }
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

/// List revisions from a docx file.
pub fn list_revisions_docx(
    bytes: &[u8],
    author_filter: Option<&str>,
) -> Result<Vec<serde_json::Value>, anyhow::Error> {
    use adapters_ooxml::opc::parse_opc;
    use adapters_ooxml::revision_reader::enumerate_revisions;
    use adapters_ooxml::xml::parse_xml;

    let pkg = parse_opc(bytes).map_err(|e| anyhow::anyhow!("failed to parse OPC: {:?}", e))?;

    let doc_part = pkg
        .parts
        .iter()
        .find(|p| p.uri == "word/document.xml")
        .ok_or_else(|| anyhow::anyhow!("missing word/document.xml in OPC package"))?;

    let xml_doc = parse_xml(&doc_part.data)
        .map_err(|e| anyhow::anyhow!("failed to parse document XML: {:?}", e))?;

    let mut diags = Vec::new();
    let _ = diags;
    let revisions = enumerate_revisions(&xml_doc, &mut diags);

    // Apply author filter
    let revisions: Vec<_> = if let Some(author) = author_filter {
        revisions
            .iter()
            .filter(|r| r.author.as_deref() == Some(author))
            .cloned()
            .collect()
    } else {
        revisions
    };

    let result: Vec<_> = revisions
        .iter()
        .map(|r| {
            json!({
                "id": r.id.to_string(),
                "kind": match r.kind {
                    RevisionKind::Insert => "insert",
                    RevisionKind::Delete => "delete",
                    RevisionKind::FormatChange => "format_change",
                },
                "author": r.author.clone().unwrap_or_else(|| "<unknown>".to_string()),
                "date": r.date.clone().unwrap_or_else(|| "<unknown>".to_string()),
                "anchor_element_id": r.anchor_element_id.to_string(),
                "page_hint": r.page_hint,
                "content": r.content.clone(),
            })
        })
        .collect();

    Ok(result)
}

/// List revisions from an ODF document (ODT/ODS/ODP).
pub fn list_revisions_odf(
    bytes: &[u8],
    author_filter: Option<&str>,
) -> Result<Vec<serde_json::Value>, anyhow::Error> {
    use adapters_odf::manifest::parse_odf;
    use adapters_odf::revision_reader;
    use adapters_odf::xml::parse_xml;

    let pkg =
        parse_odf(bytes).map_err(|e| anyhow::anyhow!("failed to parse ODF package: {:?}", e))?;

    let content_part = pkg
        .parts
        .iter()
        .find(|p| p.path == "content.xml")
        .ok_or_else(|| anyhow::anyhow!("missing content.xml in ODF package"))?;

    let xml_doc = parse_xml(&content_part.data)
        .map_err(|e| anyhow::anyhow!("failed to parse content.xml: {:?}", e))?;

    let mut diags = Vec::new();
    let revisions = revision_reader::enumerate_revisions(&xml_doc, &mut diags);

    // Apply author filter
    let revisions: Vec<_> = if let Some(author) = author_filter {
        revisions
            .iter()
            .filter(|r| r.author.as_deref() == Some(author))
            .cloned()
            .collect()
    } else {
        revisions
    };

    let result: Vec<_> = revisions
        .iter()
        .map(|r| {
            json!({
                "id": r.id.to_string(),
                "kind": match r.kind {
                    RevisionKind::Insert => "insert",
                    RevisionKind::Delete => "delete",
                    RevisionKind::FormatChange => "format_change",
                },
                "author": r.author.clone().unwrap_or_else(|| "<unknown>".to_string()),
                "date": r.date.clone().unwrap_or_else(|| "<unknown>".to_string()),
                "anchor_element_id": r.anchor_element_id.to_string(),
                "page_hint": r.page_hint,
                "content": r.content.clone(),
            })
        })
        .collect();

    Ok(result)
}

/// MCP handler: list revisions (used by MCP tool dispatch).
pub fn list_revisions_for_mcp(
    path: &str,
    author_filter: Option<&str>,
) -> Result<serde_json::Value, anyhow::Error> {
    let bytes = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("failed to read document: {}: {}", path, e))?;

    let ext = path.to_lowercase();

    let revisions = if ext.ends_with(".docx") {
        list_revisions_docx(&bytes, author_filter)?
    } else if ext.ends_with(".odt") || ext.ends_with(".ods") || ext.ends_with(".odp") {
        list_revisions_odf(&bytes, author_filter)?
    } else if ext.ends_with(".pdf") {
        return Ok(json!({
            "tool": "list_revisions",
            "error": "GP-I-006",
            "message": "PDF documents do not support revision tracking"
        }));
    } else if ext.ends_with(".xlsx") {
        return Ok(json!({
            "tool": "list_revisions",
            "error": "GP-I-007",
            "message": "xlsx documents do not support revision tracking (use cell-level edits instead)"
        }));
    } else if ext.ends_with(".pptx") {
        return Ok(json!({
            "tool": "list_revisions",
            "error": "GP-I-008",
            "message": "pptx documents do not support revision tracking"
        }));
    } else {
        return Ok(json!({
            "tool": "list_revisions",
            "error": "GP-I-009",
            "message": format!("Revision tracking is not supported for format: {}", ext)
        }));
    };

    Ok(json!({
        "tool": "list_revisions",
        "path": path,
        "revision_count": revisions.len(),
        "revisions": revisions,
    }))
}

/// MCP handler: accept revisions (used by MCP tool dispatch).
pub fn accept_revisions_for_mcp(
    path: &str,
    ids: Option<&[String]>,
    all: bool,
    author: Option<&str>,
    dry_run: bool,
    _session_id: Option<&str>,
) -> Result<serde_json::Value, anyhow::Error> {
    let bytes = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("failed to read document: {}: {}", path, e))?;

    let ext = path.to_lowercase();

    if ext.ends_with(".docx") {
        handle_revision_accept_docx(path, &bytes, ids, all, author, dry_run)
    } else if ext.ends_with(".odt") || ext.ends_with(".ods") || ext.ends_with(".odp") {
        handle_revision_accept_odf(path, &bytes, ids, all, author, dry_run)
    } else if ext.ends_with(".pdf") {
        Ok(json!({
            "tool": "accept_revisions",
            "error": "GP-I-006",
            "message": "PDF documents do not support revision tracking"
        }))
    } else if ext.ends_with(".xlsx") {
        Ok(json!({
            "tool": "accept_revisions",
            "error": "GP-I-007",
            "message": "xlsx documents do not support revision tracking (use cell-level edits instead)"
        }))
    } else if ext.ends_with(".pptx") {
        Ok(json!({
            "tool": "accept_revisions",
            "error": "GP-I-008",
            "message": "pptx documents do not support revision tracking"
        }))
    } else {
        Ok(json!({
            "tool": "accept_revisions",
            "error": "GP-I-009",
            "message": format!("Revision tracking is not supported for format: {}", ext)
        }))
    }
}

/// MCP handler: reject revisions (used by MCP tool dispatch).
pub fn reject_revisions_for_mcp(
    path: &str,
    ids: Option<&[String]>,
    all: bool,
    author: Option<&str>,
    dry_run: bool,
    _session_id: Option<&str>,
) -> Result<serde_json::Value, anyhow::Error> {
    let bytes = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("failed to read document: {}: {}", path, e))?;

    let ext = path.to_lowercase();

    if ext.ends_with(".docx") {
        handle_revision_reject_docx(path, &bytes, ids, all, author, dry_run)
    } else if ext.ends_with(".odt") || ext.ends_with(".ods") || ext.ends_with(".odp") {
        handle_revision_reject_odf(path, &bytes, ids, all, author, dry_run)
    } else if ext.ends_with(".pdf") {
        Ok(json!({
            "tool": "reject_revisions",
            "error": "GP-I-006",
            "message": "PDF documents do not support revision tracking"
        }))
    } else if ext.ends_with(".xlsx") {
        Ok(json!({
            "tool": "reject_revisions",
            "error": "GP-I-007",
            "message": "xlsx documents do not support revision tracking (use cell-level edits instead)"
        }))
    } else if ext.ends_with(".pptx") {
        Ok(json!({
            "tool": "reject_revisions",
            "error": "GP-I-008",
            "message": "pptx documents do not support revision tracking"
        }))
    } else {
        Ok(json!({
            "tool": "reject_revisions",
            "error": "GP-I-009",
            "message": format!("Revision tracking is not supported for format: {}", ext)
        }))
    }
}

/// Handle accept revisions for docx.
fn handle_revision_accept_docx(
    path: &str,
    bytes: &[u8],
    ids: Option<&[String]>,
    all: bool,
    author: Option<&str>,
    dry_run: bool,
) -> Result<serde_json::Value, anyhow::Error> {
    use adapters_ooxml::opc::parse_opc;
    use adapters_ooxml::revision_ops::{accept_filtered, RevisionFilter, RevisionWriterMeta};
    use adapters_ooxml::revision_reader::enumerate_revisions;
    use adapters_ooxml::xml::parse_xml;

    let pkg = parse_opc(bytes).map_err(|e| anyhow::anyhow!("failed to parse OPC: {:?}", e))?;

    let doc_part = pkg
        .parts
        .iter()
        .find(|p| p.uri == "word/document.xml")
        .ok_or_else(|| anyhow::anyhow!("missing word/document.xml in OPC package"))?;

    let xml_doc = parse_xml(&doc_part.data)
        .map_err(|e| anyhow::anyhow!("failed to parse document XML: {:?}", e))?;

    let mut diags = Vec::new();
    let revisions = enumerate_revisions(&xml_doc, &mut diags);

    // Build filter
    let mut revision_ids = Vec::new();
    if let Some(id_list) = ids {
        for id_str in id_list {
            revision_ids.push(ElementId::from_str(id_str));
        }
    }

    let filter = RevisionFilter {
        author: author.map(|a| a.to_string()),
        date_before: None,
        date_after: None,
        kind: None,
        ids: revision_ids,
        all,
    };

    let policy = EditPolicy::default();
    let meta = RevisionWriterMeta::new("agent", "2026-01-01T00:00:00Z");

    match accept_filtered(&pkg, &revisions, &filter, &policy, &meta, dry_run) {
        Ok(result) => {
            let output_path = if dry_run {
                None
            } else {
                let out = format!("{}.revised.docx", path.trim_end_matches(".docx"));
                std::fs::write(&out, &result.output)
                    .map_err(|e| anyhow::anyhow!("failed to write output: {}", e))?;
                Some(out)
            };

            let diagnostic_list: Vec<_> = result
                .diagnostics
                .iter()
                .map(|d| {
                    json!({
                        "code": d.code,
                        "severity": match d.severity {
                            gp_core::diagnostics::Severity::Error => "error",
                            gp_core::diagnostics::Severity::Warning => "warning",
                            gp_core::diagnostics::Severity::Info => "info",
                            gp_core::diagnostics::Severity::Hint => "hint",
                            gp_core::diagnostics::Severity::Fatal => "fatal",
                        },
                        "message": d.message,
                    })
                })
                .collect();

            Ok(json!({
                "tool": "accept_revisions",
                "path": path,
                "status": if dry_run { "dry_run" } else { "accepted" },
                "operations_performed": result.operations_performed,
                "operations_succeeded": result.operations_succeeded,
                "operations_failed": result.operations_failed,
                "journal_ids": result.journal_ids,
                "output_path": output_path,
                "dry_run": dry_run,
                "diagnostics": diagnostic_list,
            }))
        }
        Err(err_diags) => {
            let diagnostic_list: Vec<_> = err_diags
                .iter()
                .map(|d| {
                    json!({
                        "code": d.code,
                        "severity": match d.severity {
                            gp_core::diagnostics::Severity::Error => "error",
                            gp_core::diagnostics::Severity::Warning => "warning",
                            gp_core::diagnostics::Severity::Info => "info",
                            gp_core::diagnostics::Severity::Hint => "hint",
                            gp_core::diagnostics::Severity::Fatal => "fatal",
                        },
                        "message": d.message,
                    })
                })
                .collect();
            Ok(json!({
                "tool": "accept_revisions",
                "path": path,
                "status": "error",
                "diagnostics": diagnostic_list,
            }))
        }
    }
}

/// Handle accept revisions for ODF (ODT/ODS/ODP).
fn handle_revision_accept_odf(
    path: &str,
    bytes: &[u8],
    _ids: Option<&[String]>,
    all: bool,
    _author: Option<&str>,
    dry_run: bool,
) -> Result<serde_json::Value, anyhow::Error> {
    use adapters_odf::manifest::parse_odf;
    use adapters_odf::revision_executor;
    use adapters_odf::revision_reader;
    use adapters_odf::xml::parse_xml;

    let pkg =
        parse_odf(bytes).map_err(|e| anyhow::anyhow!("failed to parse ODF package: {:?}", e))?;

    let content_part = pkg
        .parts
        .iter()
        .find(|p| p.path == "content.xml")
        .ok_or_else(|| anyhow::anyhow!("missing content.xml in ODF package"))?;

    let xml_doc = parse_xml(&content_part.data)
        .map_err(|e| anyhow::anyhow!("failed to parse content.xml: {:?}", e))?;

    let mut diags = Vec::new();
    let revisions = revision_reader::enumerate_revisions(&xml_doc, &mut diags);

    // If specific IDs requested, filter
    let revisions: Vec<_> = if let Some(id_list) = _ids {
        let id_set: std::collections::HashSet<String> = id_list.iter().cloned().collect();
        revisions
            .into_iter()
            .filter(|r| id_set.contains(&r.id.to_string()))
            .collect()
    } else if all {
        revisions
    } else {
        revisions
    };

    if revisions.is_empty() {
        let out_path = if dry_run {
            None
        } else {
            let ext = odf_ext(path);
            let base = path.trim_end_matches(&format!(".{}", ext));
            Some(format!("{}.revised.{}", base, ext))
        };
        return Ok(json!({
            "tool": "accept_revisions",
            "path": path,
            "status": if dry_run { "dry_run" } else { "accepted" },
            "operations_performed": 0,
            "operations_succeeded": 0,
            "operations_failed": 0,
            "journal_ids": [],
            "output_path": out_path,
            "dry_run": dry_run,
            "diagnostics": [],
        }));
    }

    let result = if all {
        revision_executor::accept_all_revisions(bytes, &revisions)
    } else {
        // Accept each revision sequentially
        let mut current_bytes = bytes.to_vec();
        let mut journal_ids: Vec<String> = Vec::new();
        let mut success_count = 0u64;
        let mut fail_count = 0u64;
        let mut all_diagnostics: Vec<gp_core::diagnostics::Diagnostic> = Vec::new();

        for rev in &revisions {
            match revision_executor::accept_revision(&current_bytes, rev) {
                Ok(action) => {
                    journal_ids.extend(action.journal_ids);
                    current_bytes = action.output;
                    success_count += 1;
                    all_diagnostics.extend(action.diagnostics);
                }
                Err(d) => {
                    all_diagnostics.extend(d);
                    fail_count += 1;
                }
            }
        }

        Ok(revision_executor::RevisionActionResult {
            output: current_bytes,
            journal_ids,
            accepted: true,
            changed_element_ids: revisions.iter().map(|r| r.id.to_string()).collect(),
            diagnostics: all_diagnostics,
            operations_performed: success_count + fail_count,
            operations_succeeded: success_count,
            operations_failed: fail_count,
        })
    };

    match result {
        Ok(action) => {
            let output_path = if dry_run {
                None
            } else {
                let ext = if path.to_lowercase().ends_with(".odt") {
                    "odt"
                } else if path.to_lowercase().ends_with(".ods") {
                    "ods"
                } else {
                    "odp"
                };
                let out = format!(
                    "{}.revised.{}",
                    path.trim_end_matches(&format!(".{}", ext)),
                    ext
                );
                std::fs::write(&out, &action.output)
                    .map_err(|e| anyhow::anyhow!("failed to write output: {}", e))?;
                Some(out)
            };

            let diagnostic_list: Vec<_> = action
                .diagnostics
                .iter()
                .map(|d| {
                    json!({
                        "code": d.code,
                        "severity": match d.severity {
                            gp_core::diagnostics::Severity::Error => "error",
                            gp_core::diagnostics::Severity::Warning => "warning",
                            gp_core::diagnostics::Severity::Info => "info",
                            gp_core::diagnostics::Severity::Hint => "hint",
                            gp_core::diagnostics::Severity::Fatal => "fatal",
                        },
                        "message": d.message,
                    })
                })
                .collect();

            Ok(json!({
                "tool": "accept_revisions",
                "path": path,
                "status": if dry_run { "dry_run" } else { "accepted" },
                "operations_performed": action.operations_succeeded + action.operations_failed,
                "operations_succeeded": action.operations_succeeded,
                "operations_failed": action.operations_failed,
                "journal_ids": action.journal_ids,
                "output_path": output_path,
                "dry_run": dry_run,
                "diagnostics": diagnostic_list,
            }))
        }
        Err(err_diags) => {
            let diagnostic_list: Vec<_> = err_diags
                .iter()
                .map(|d| {
                    json!({
                        "code": d.code,
                        "severity": match d.severity {
                            gp_core::diagnostics::Severity::Error => "error",
                            gp_core::diagnostics::Severity::Warning => "warning",
                            gp_core::diagnostics::Severity::Info => "info",
                            gp_core::diagnostics::Severity::Hint => "hint",
                            gp_core::diagnostics::Severity::Fatal => "fatal",
                        },
                        "message": d.message,
                    })
                })
                .collect();
            Ok(json!({
                "tool": "accept_revisions",
                "path": path,
                "status": "error",
                "diagnostics": diagnostic_list,
            }))
        }
    }
}

/// Handle reject revisions for docx.
fn handle_revision_reject_docx(
    path: &str,
    bytes: &[u8],
    ids: Option<&[String]>,
    all: bool,
    author: Option<&str>,
    dry_run: bool,
) -> Result<serde_json::Value, anyhow::Error> {
    use adapters_ooxml::opc::parse_opc;
    use adapters_ooxml::revision_ops::{reject_filtered, RevisionFilter, RevisionWriterMeta};
    use adapters_ooxml::revision_reader::enumerate_revisions;
    use adapters_ooxml::xml::parse_xml;

    let pkg = parse_opc(bytes).map_err(|e| anyhow::anyhow!("failed to parse OPC: {:?}", e))?;

    let doc_part = pkg
        .parts
        .iter()
        .find(|p| p.uri == "word/document.xml")
        .ok_or_else(|| anyhow::anyhow!("missing word/document.xml in OPC package"))?;

    let xml_doc = parse_xml(&doc_part.data)
        .map_err(|e| anyhow::anyhow!("failed to parse document XML: {:?}", e))?;

    let mut diags = Vec::new();
    let _ = diags;
    let revisions = enumerate_revisions(&xml_doc, &mut diags);

    // Build filter
    let mut revision_ids = Vec::new();
    if let Some(id_list) = ids {
        for id_str in id_list {
            revision_ids.push(ElementId::from_str(id_str));
        }
    }

    let filter = RevisionFilter {
        author: author.map(|a| a.to_string()),
        date_before: None,
        date_after: None,
        kind: None,
        ids: revision_ids,
        all,
    };

    let policy = EditPolicy::default();
    let meta = RevisionWriterMeta::new("agent", "2026-01-01T00:00:00Z");

    match reject_filtered(&pkg, &revisions, &filter, &policy, &meta, dry_run) {
        Ok(result) => {
            let output_path = if dry_run {
                None
            } else {
                let out = format!("{}.revised.docx", path.trim_end_matches(".docx"));
                std::fs::write(&out, &result.output)
                    .map_err(|e| anyhow::anyhow!("failed to write output: {}", e))?;
                Some(out)
            };

            let diagnostic_list: Vec<_> = result
                .diagnostics
                .iter()
                .map(|d| {
                    json!({
                        "code": d.code,
                        "severity": match d.severity {
                            gp_core::diagnostics::Severity::Error => "error",
                            gp_core::diagnostics::Severity::Warning => "warning",
                            gp_core::diagnostics::Severity::Info => "info",
                            gp_core::diagnostics::Severity::Hint => "hint",
                            gp_core::diagnostics::Severity::Fatal => "fatal",
                        },
                        "message": d.message,
                    })
                })
                .collect();

            Ok(json!({
                "tool": "reject_revisions",
                "path": path,
                "status": if dry_run { "dry_run" } else { "rejected" },
                "operations_performed": result.operations_performed,
                "operations_succeeded": result.operations_succeeded,
                "operations_failed": result.operations_failed,
                "journal_ids": result.journal_ids,
                "output_path": output_path,
                "dry_run": dry_run,
                "diagnostics": diagnostic_list,
            }))
        }
        Err(err_diags) => {
            let diagnostic_list: Vec<_> = err_diags
                .iter()
                .map(|d| {
                    json!({
                        "code": d.code,
                        "severity": match d.severity {
                            gp_core::diagnostics::Severity::Error => "error",
                            gp_core::diagnostics::Severity::Warning => "warning",
                            gp_core::diagnostics::Severity::Info => "info",
                            gp_core::diagnostics::Severity::Hint => "hint",
                            gp_core::diagnostics::Severity::Fatal => "fatal",
                        },
                        "message": d.message,
                    })
                })
                .collect();
            Ok(json!({
                "tool": "reject_revisions",
                "path": path,
                "status": "error",
                "diagnostics": diagnostic_list,
            }))
        }
    }
}

/// Handle reject revisions for ODF (ODT/ODS/ODP).
fn handle_revision_reject_odf(
    path: &str,
    bytes: &[u8],
    _ids: Option<&[String]>,
    all: bool,
    _author: Option<&str>,
    dry_run: bool,
) -> Result<serde_json::Value, anyhow::Error> {
    use adapters_odf::manifest::parse_odf;
    use adapters_odf::revision_executor;
    use adapters_odf::revision_reader;
    use adapters_odf::xml::parse_xml;

    let pkg =
        parse_odf(bytes).map_err(|e| anyhow::anyhow!("failed to parse ODF package: {:?}", e))?;

    let content_part = pkg
        .parts
        .iter()
        .find(|p| p.path == "content.xml")
        .ok_or_else(|| anyhow::anyhow!("missing content.xml in ODF package"))?;

    let xml_doc = parse_xml(&content_part.data)
        .map_err(|e| anyhow::anyhow!("failed to parse content.xml: {:?}", e))?;

    let mut diags = Vec::new();
    let revisions = revision_reader::enumerate_revisions(&xml_doc, &mut diags);

    // If specific IDs requested, filter
    let revisions: Vec<_> = if let Some(id_list) = _ids {
        let id_set: std::collections::HashSet<String> = id_list.iter().cloned().collect();
        revisions
            .into_iter()
            .filter(|r| id_set.contains(&r.id.to_string()))
            .collect()
    } else if all {
        revisions
    } else {
        revisions
    };

    if revisions.is_empty() {
        let out_path = if dry_run {
            None
        } else {
            let ext = odf_ext(path);
            let base = path.trim_end_matches(&format!(".{}", ext));
            Some(format!("{}.revised.{}", base, ext))
        };
        return Ok(json!({
            "tool": "reject_revisions",
            "path": path,
            "status": if dry_run { "dry_run" } else { "rejected" },
            "operations_performed": 0,
            "operations_succeeded": 0,
            "operations_failed": 0,
            "journal_ids": [],
            "output_path": out_path,
            "dry_run": dry_run,
            "diagnostics": [],
        }));
    }

    let result = if all {
        revision_executor::reject_all_revisions(bytes, &revisions)
    } else {
        // Reject each revision sequentially
        let mut current_bytes = bytes.to_vec();
        let mut journal_ids: Vec<String> = Vec::new();
        let mut success_count = 0u64;
        let mut fail_count = 0u64;
        let mut all_diagnostics: Vec<gp_core::diagnostics::Diagnostic> = Vec::new();

        for rev in &revisions {
            match revision_executor::reject_revision(&current_bytes, rev) {
                Ok(action) => {
                    journal_ids.extend(action.journal_ids);
                    current_bytes = action.output;
                    success_count += 1;
                    all_diagnostics.extend(action.diagnostics);
                }
                Err(d) => {
                    all_diagnostics.extend(d);
                    fail_count += 1;
                }
            }
        }

        Ok(revision_executor::RevisionActionResult {
            output: current_bytes,
            journal_ids,
            accepted: false,
            changed_element_ids: revisions.iter().map(|r| r.id.to_string()).collect(),
            diagnostics: all_diagnostics,
            operations_performed: success_count + fail_count,
            operations_succeeded: success_count,
            operations_failed: fail_count,
        })
    };

    match result {
        Ok(action) => {
            let output_path = if dry_run {
                None
            } else {
                let ext = if path.to_lowercase().ends_with(".odt") {
                    "odt"
                } else if path.to_lowercase().ends_with(".ods") {
                    "ods"
                } else {
                    "odp"
                };
                let out = format!(
                    "{}.revised.{}",
                    path.trim_end_matches(&format!(".{}", ext)),
                    ext
                );
                std::fs::write(&out, &action.output)
                    .map_err(|e| anyhow::anyhow!("failed to write output: {}", e))?;
                Some(out)
            };

            let diagnostic_list: Vec<_> = action
                .diagnostics
                .iter()
                .map(|d| {
                    json!({
                        "code": d.code,
                        "severity": match d.severity {
                            gp_core::diagnostics::Severity::Error => "error",
                            gp_core::diagnostics::Severity::Warning => "warning",
                            gp_core::diagnostics::Severity::Info => "info",
                            gp_core::diagnostics::Severity::Hint => "hint",
                            gp_core::diagnostics::Severity::Fatal => "fatal",
                        },
                        "message": d.message,
                    })
                })
                .collect();

            Ok(json!({
                "tool": "reject_revisions",
                "path": path,
                "status": if dry_run { "dry_run" } else { "rejected" },
                "operations_performed": action.operations_succeeded + action.operations_failed,
                "operations_succeeded": action.operations_succeeded,
                "operations_failed": action.operations_failed,
                "journal_ids": action.journal_ids,
                "output_path": output_path,
                "dry_run": dry_run,
                "diagnostics": diagnostic_list,
            }))
        }
        Err(err_diags) => {
            let diagnostic_list: Vec<_> = err_diags
                .iter()
                .map(|d| {
                    json!({
                        "code": d.code,
                        "severity": match d.severity {
                            gp_core::diagnostics::Severity::Error => "error",
                            gp_core::diagnostics::Severity::Warning => "warning",
                            gp_core::diagnostics::Severity::Info => "info",
                            gp_core::diagnostics::Severity::Hint => "hint",
                            gp_core::diagnostics::Severity::Fatal => "fatal",
                        },
                        "message": d.message,
                    })
                })
                .collect();
            Ok(json!({
                "tool": "reject_revisions",
                "path": path,
                "status": "error",
                "diagnostics": diagnostic_list,
            }))
        }
    }
}
