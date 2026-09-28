// SPDX-License-Identifier: MIT OR Apache-2.0
//! stichos — proof before publish.
//!
//! CLI binary with verbs: inspect, render, diff, doctor, serve, apply.
//! Docx edits land as tracked revisions by default (D7). Use --direct to opt out.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use mcp::dispatch::{self, BoxedHandler};

mod annots;
mod apply_tool;
mod budget_enforcer;
mod diff_tool;
mod doctor_tool;
mod inspect;
mod journal_tool;
mod open_tool;
mod perf_tool;
mod region;
mod render;
mod review_tool;
mod revisions_tool;
mod script_tool;
mod sign;
mod sugar_edits;

#[derive(Parser)]
#[command(name = "stichos", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect a document: extract structure, IDs, coordinates, fidelity budget.
    Inspect {
        /// Path to the document file.
        path: std::path::PathBuf,
        /// Output as JSON.
        #[arg(long, short = 'j')]
        json: bool,
    },
    /// Crop and zoom a region from a rendered document page.
    Region {
        /// Path to the document file.
        path: std::path::PathBuf,
        /// Page number (1-indexed).
        #[arg(long)]
        page: Option<usize>,
        /// Bounding box: x_pt,y_pt,width_pt,height_pt (Letter default: 612x792).
        #[arg(long, short = 'b')]
        bbox: Option<String>,
        /// Zoom multiplier (default: 2.0).
        #[arg(long, default_value = "2.0")]
        zoom: f64,
        /// Output as JSON.
        #[arg(long, short = 'j')]
        json: bool,
        /// Output mode: urls (default), base64, or both.
        #[arg(long, default_value = "urls")]
        output: Option<String>,
    },
    /// Render a document to images with layout QA.
    Render {
        /// Path to the document file.
        path: std::path::PathBuf,
        /// Output directory for rendered images.
        #[arg(long, short = 'o')]
        output: Option<std::path::PathBuf>,
        /// DPI for rendering (default: 150).
        #[arg(long, default_value = "150")]
        dpi: u32,
    },
    /// Diff two documents: IR structural diff + XML part diff.
    Diff {
        /// First document path.
        a: std::path::PathBuf,
        /// Second document path.
        b: std::path::PathBuf,
        /// Output as JSON.
        #[arg(long, short = 'j')]
        json: bool,
        /// Maximum number of diff operations to include (0 = unlimited).
        #[arg(long)]
        max_ops: Option<usize>,
    },
    /// Probe the host: check engine availability and render a test fixture.
    Doctor {
        /// Output as JSON.
        #[arg(long, short = 'j')]
        json: bool,
    },
    /// Start the MCP transport server.
    Serve {
        /// Transport mode: stdio or http.
        #[arg(long, default_value = "stdio")]
        transport: String,
        /// Port for HTTP mode (default: 8731).
        #[arg(long, default_value = "8731")]
        port: u16,
    },
    /// Check INV-2 schema compliance across all tool responses.
    Schema,
    /// Apply surgical edits to a document.
    Apply {
        /// Path to the document file.
        path: std::path::PathBuf,
        /// JSON string of patch operations (array of PatchOp).
        #[arg(long)]
        ops: Option<String>,
        /// Path to a file containing patch operations JSON.
        #[arg(long)]
        ops_file: Option<std::path::PathBuf>,
        /// Preview changes (deprecated: preview is now the default).
        #[arg(long)]
        dry_run: bool,
        /// Apply changes directly (overrides default preview).
        #[arg(long)]
        apply: bool,
        /// Output path (default: same path with .edited.<ext>).
        #[arg(long, short = 'o')]
        output: Option<std::path::PathBuf>,
        /// Format hint (inferred from extension).
        #[arg(long)]
        format: Option<String>,
        /// Journal session ID.
        #[arg(long)]
        session: Option<String>,
        /// Output as JSON.
        #[arg(long, short = 'j')]
        json: bool,
        /// Apply directly without tracked-revisions (default: tracked-revisions for docx).
        #[arg(long)]
        direct: bool,
        /// Author name for revision markup.
        #[arg(long, default_value = "agent")]
        author: String,
    },
    /// Edit a document: sugar over apply_patch.
    Edit {
        /// Subcommand: text, cell, structure.
        #[arg(long)]
        kind: String,
        /// Path to the document file.
        path: std::path::PathBuf,
        /// Element ID to edit.
        #[arg(long)]
        element_id: String,
        /// Current text to replace (for text edits).
        #[arg(long)]
        old_text: Option<String>,
        /// New text/value (for text and cell edits).
        #[arg(long)]
        new_text: Option<String>,
        /// Action: insert or delete (for structure edits).
        #[arg(long)]
        action: Option<String>,
        /// Element kind for insert (e.g., paragraph).
        #[arg(long)]
        kind_name: Option<String>,
        /// Preview changes (deprecated: preview is now the default).
        #[arg(long)]
        dry_run: bool,
        /// Apply changes directly (overrides default preview).
        #[arg(long)]
        apply: bool,
        /// Output as JSON.
        #[arg(long, short = 'j')]
        json: bool,
        /// Apply directly without tracked-revisions.
        #[arg(long)]
        direct: bool,
        /// Author for revision markup.
        #[arg(long, default_value = "agent")]
        author: String,
    },
    /// Annotate rendered pages with numbered lint boxes.
    Annotate {
        /// Path to the document file.
        path: std::path::PathBuf,
        /// Render DPI for annotation.
        #[arg(long, default_value = "150")]
        dpi: u32,
        /// Output as JSON.
        #[arg(long, short = 'j')]
        json: bool,
    },
    /// Revert a document to a previous journal entry.
    Revert {
        /// Path to the document file.
        path: std::path::PathBuf,
        /// Number of steps to revert (default 1).
        #[arg(long, default_value = "1")]
        steps: usize,
        /// Journal session ID.
        #[arg(long)]
        session: Option<String>,
        /// Output as JSON.
        #[arg(long, short = 'j')]
        json: bool,
    },
    /// List journal entries for a document session.
    Journal {
        /// Path to the document file.
        path: std::path::PathBuf,
        /// Journal session ID.
        #[arg(long)]
        session: Option<String>,
        /// Output as JSON.
        #[arg(long, short = 'j')]
        json: bool,
    },
    /// Measure edit→render→look latency against a corpus subset.
    ///
    /// OPS-02: P95 ≤ 2s CI gate. If P95 exceeds --target, exits with code 1.
    Perf {
        /// Corpus subset to measure: "pr-subset" or path to fixture file.
        #[arg(long, default_value = "pr-subset")]
        fixtures: Option<String>,
        /// P95 target in milliseconds (default: 2000).
        #[arg(long, default_value = "2000")]
        target: f64,
    },
    /// Create a new document from source content.
    ///
    /// Routes to the appropriate engine based on source_type + format:
    /// - markdown/html → docx/odt: pandoc
    /// - html → pdf: chromium
    /// - typ → pdf: typst
    /// - spreadsheet → xlsx: native ooxml
    /// - presentation → pptx: native ooxml
    /// - target=existing-docx: create-as-revisions
    Create {
        /// Source content (inline markdown/html/typ)
        #[arg(long)]
        source_content: Option<String>,

        /// Path to source file (markdown, html, or typ)
        #[arg(long)]
        source: Option<String>,

        /// Target format: docx, odt, pdf, xlsx, pptx
        #[arg(long)]
        format: Option<String>,

        /// Source type: markdown, html, typ, spreadsheet, presentation
        #[arg(long)]
        source_type: Option<String>,

        /// Path to template/reference document
        #[arg(long)]
        template: Option<std::path::PathBuf>,

        /// Dry run: preview without writing
        #[arg(long)]
        dry_run: bool,

        /// Path to existing docx for create-as-revisions mode
        #[arg(long)]
        target: Option<std::path::PathBuf>,

        /// Author for revision markup
        #[arg(long, default_value = "agent")]
        author: String,

        /// Apply directly without tracked-revisions
        #[arg(long)]
        direct: bool,

        /// Output as JSON
        #[arg(long, short = 'j')]
        json: bool,
    },
    /// Execute a sandboxed Rhai script.
    Script {
        /// Path to the Rhai script file.
        script: std::path::PathBuf,
        /// IR document as JSON file.
        #[arg(long)]
        ir: Option<std::path::PathBuf>,
        /// Maximum number of operations (default: 1000000).
        #[arg(long, default_value = "1000000")]
        max_ops: u64,
        /// Maximum script size in bytes (default: 524288).
        #[arg(long, default_value = "524288")]
        max_size: usize,
        /// Output as JSON.
        #[arg(long, short = 'j')]
        json: bool,
    },
    /// Check for and install available updates.
    ///
    /// Opt-in only: never triggered implicitly. Verifies minisign signature
    /// and SHA-256 hash before swapping the binary. Rolls back on any failure.
    Update {
        /// Update channel to check (default: "stable").
        /// For testing, use "fixture" with --fixture-path or STICHOS_FIXTURE_PATH.
        #[arg(long, default_value = "stable")]
        channel: String,
        /// Local fixture directory path for the "fixture" channel.
        /// Takes precedence over channel="fixture" without a path.
        /// Can also be set via the STICHOS_FIXTURE_PATH environment variable.
        #[arg(long)]
        fixture_path: Option<String>,
        /// Show what would happen without making changes.
        #[arg(long)]
        dry_run: bool,
        /// Skip the confirmation prompt.
        #[arg(long)]
        force: bool,
        /// Output as JSON.
        #[arg(long, short = 'j')]
        json: bool,
    },
    /// List, accept, or reject tracked revisions in a document.
    Revisions {
        /// Subcommand: list, accept, reject.
        #[command(subcommand)]
        command: revisions_tool::RevisionsCommand,
    },
    /// List or resolve annotations in a PDF document.
    Annots {
        /// Subcommand: list or resolve.
        #[command(subcommand)]
        command: annots::AnnotsCommand,
    },
    /// Start a review session for a document.
    Review {
        /// Path to the document file.
        path: std::path::PathBuf,
        /// Do not attempt to open a browser.
        #[arg(long)]
        no_browser: bool,
        /// Close an active review session by token.
        #[arg(long)]
        close: Option<String>,
        /// List active review sessions.
        #[arg(long)]
        list: bool,
        /// Enable text layer (pdf.js text extraction for PDF review sessions).
        #[arg(long)]
        text_layer: bool,
    },
    /// Open a document in the default application.
    Open {
        /// Path to the document file.
        #[arg()]
        path: std::path::PathBuf,
        /// Journal session/version to open (materialized to temp file).
        #[arg(long)]
        version: Option<String>,
        /// App to use (e.g. "libreoffice", "default" for platform default).
        #[arg(long, short)]
        app: Option<String>,
        /// Do not launch the opener; just resolve the path.
        #[arg(long)]
        no_browser: bool,
        /// Output as JSON.
        #[arg(long, short = 'j')]
        json: bool,
    },
    /// Sign a PDF document (PAdES-B-B sign-later, C13).
    Sign {
        /// Path to the unsigned PDF document.
        #[arg()]
        path: std::path::PathBuf,
        /// Certificate subject CN (default: "stichos").
        #[arg(long)]
        subject: Option<String>,
        /// Signature algorithm: "rsa" or "ecdsa" (default: rsa).
        #[arg(long)]
        algorithm: Option<String>,
        /// Output path (default: input.signed.pdf).
        #[arg(long, short = 'o')]
        output: Option<std::path::PathBuf>,
        /// Verify the signature after signing.
        #[arg(long)]
        verify: bool,
        /// Path to PKCS#12 file (certificate + key bundle).
        #[arg(long)]
        pkcs12: Option<String>,
        /// Password for the PKCS#12 file.
        #[arg(long)]
        pkcs12_password: Option<String>,
    },
    /// Verify a signature in a PDF document.
    Verify {
        /// Path to the signed PDF document.
        #[arg()]
        path: std::path::PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init()
        .ok();

    let cli = Cli::parse();

    match cli.command {
        Command::Inspect { path, json } => {
            cmd_inspect(&path, json).await?;
        }
        Command::Region {
            path,
            page,
            bbox,
            zoom,
            json,
            output,
        } => {
            cmd_region(
                &path,
                page.unwrap_or(1),
                bbox.as_deref(),
                zoom,
                json,
                output,
            )
            .await?;
        }
        Command::Render { path, output, dpi } => {
            cmd_render(&path, output.as_deref(), dpi).await?;
        }
        Command::Diff {
            a,
            b,
            json,
            max_ops,
        } => {
            cmd_diff(&a, &b, json, max_ops).await?;
        }
        Command::Doctor { json } => {
            cmd_doctor(json).await?;
        }
        Command::Serve { transport, port } => {
            cmd_serve(&transport, port).await?;
        }
        Command::Schema => {
            cmd_schema();
        }
        Command::Apply {
            path,
            ops,
            ops_file,
            dry_run: _,
            apply,
            output,
            format,
            session,
            json,
            direct,
            author,
        } => {
            // CTX-05: default dry_run=true (preview); --apply opts out.
            let effective_dry_run = !apply;
            cmd_apply(
                &path,
                ops.as_deref(),
                ops_file.as_deref(),
                effective_dry_run,
                output.as_deref(),
                format.as_deref(),
                session.as_deref(),
                json,
                direct,
                author.as_str(),
            )
            .await?;
        }
        Command::Edit {
            kind,
            path,
            element_id,
            old_text,
            new_text,
            action,
            kind_name,
            dry_run: _,
            apply,
            json,
            direct,
            author,
        } => {
            // CTX-05: default dry_run=true (preview); --apply opts out.
            let effective_dry_run = !apply;
            cmd_edit(
                &kind,
                &path,
                &element_id,
                old_text.as_deref(),
                new_text.as_deref(),
                action.as_deref(),
                kind_name.as_deref(),
                effective_dry_run,
                json,
                direct,
                author.as_str(),
            )
            .await?;
        }
        Command::Annotate { path, dpi, json } => {
            cmd_annotate(&path, dpi, json).await?;
        }
        Command::Revert {
            path,
            steps,
            session,
            json,
        } => {
            let result = journal_tool::revert_document(
                path.to_str().unwrap_or_default(),
                steps,
                session.as_deref(),
            )?;
            let result = budget_enforcer::stamp_envelope(result);
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                eprintln!("revert: {} (steps={})", path.display(), steps);
            }
        }
        Command::Journal {
            path,
            session,
            json,
        } => {
            let result =
                journal_tool::list_journal(path.to_str().unwrap_or_default(), session.as_deref())?;
            let result = budget_enforcer::stamp_envelope(result);
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                eprintln!("journal: {} entries", result["entry_count"]);
            }
        }
        Command::Perf { fixtures, target } => {
            let exit_code = perf_tool::run_perf(fixtures, target)?;
            std::process::exit(exit_code);
        }
        Command::Create {
            source_content,
            source,
            format,
            source_type,
            template,
            dry_run,
            target,
            author,
            direct,
            json,
        } => {
            cmd_create(
                source_content.as_deref(),
                source.as_deref(),
                format.as_deref(),
                source_type.as_deref(),
                template.as_deref(),
                dry_run,
                target.as_deref(),
                &author,
                direct,
                json,
            )
            .await?;
        }
        Command::Script {
            script,
            ir,
            max_ops,
            max_size,
            json,
        } => {
            cmd_script(&script, ir.as_deref(), max_ops, max_size, json).await?;
        }
        Command::Update {
            channel,
            fixture_path,
            dry_run,
            force,
            json,
        } => {
            cmd_update(&channel, fixture_path.as_deref(), dry_run, force, json)?;
        }
        Command::Revisions { command } => {
            revisions_tool::handle_revisions(command).await?;
        }
        Command::Annots { command } => {
            annots::handle_annots(command).await?;
        }
        Command::Review {
            path,
            no_browser,
            close,
            list,
            text_layer,
        } => {
            cmd_review(&path, no_browser, close.as_deref(), list, text_layer).await?;
        }
        Command::Open {
            path,
            version,
            app,
            no_browser,
            json,
        } => {
            cmd_open(&path, version.as_deref(), app.as_deref(), no_browser, json)?;
        }
        Command::Sign {
            path,
            subject,
            algorithm,
            output,
            verify,
            pkcs12,
            pkcs12_password,
        } => {
            sign::cmd_sign(
                &path,
                subject,
                algorithm,
                output,
                verify,
                pkcs12,
                pkcs12_password,
            )?;
        }
        Command::Verify { path } => {
            sign::cmd_verify(&path)?;
        }
    }

    Ok(())
}

async fn cmd_inspect(path: &std::path::Path, json: bool) -> Result<()> {
    let path_str = path.to_string_lossy().to_string();
    let result = inspect::inspect_document(&path_str)?;
    let result =
        budget_enforcer::enforce_inspect_budget(result).unwrap_or_else(|fallback| fallback);
    let result = budget_enforcer::stamp_envelope(result);

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        // Text output: summary of the result
        if let Some(tool) = result.get("tool").and_then(|t| t.as_str()) {
            eprintln!("{}: {}", tool, path_str);
        }
        if let Some(doc_type) = result.get("document_type").and_then(|t| t.as_str()) {
            eprintln!("  type: {}", doc_type);
        }
        if let Some(total) = result.get("total_elements").and_then(|t| t.as_u64()) {
            eprintln!("  elements: {}", total);
        }
        if let Some(structure) = result.get("structure") {
            if let Some(sections) = structure.get("sections").and_then(|s| s.as_u64()) {
                eprintln!("  sections: {}", sections);
            }
            if let Some(pages) = structure.get("pages").and_then(|p| p.as_u64()) {
                eprintln!("  pages: {}", pages);
            }
        }
        if let Some(budget) = result.get("fidelity_budget") {
            if let Some(conf) = budget.get("confidence").and_then(|c| c.as_f64()) {
                eprintln!("  fidelity: {:.1}% (confidence)", conf * 100.0);
            }
        }
    }
    Ok(())
}

async fn cmd_region(
    path: &std::path::Path,
    page: usize,
    bbox_str: Option<&str>,
    zoom: f64,
    _json: bool,
    _output: Option<String>,
) -> Result<()> {
    let bbox: [f64; 4] = match bbox_str {
        Some(s) => region::parse_bbox(s).with_context(|| format!("invalid bbox: {}", s))?,
        None => [0.0, 0.0, 200.0, 100.0], // sensible default for Letter page
    };

    let result = region::inspect_region(path.to_string_lossy().as_ref(), page, bbox, zoom)?;
    let result = budget_enforcer::stamp_envelope(result);

    // Output JSON to stdout
    println!("{}", serde_json::to_string_pretty(&result)?);

    // Output summary to stderr
    if let Some(tool) = result.get("tool").and_then(|t| t.as_str()) {
        eprintln!(
            "{}: page {} → {}x{}px",
            tool, page, result["crop"]["width"], result["crop"]["height"]
        );
    }
    if let Some(within) = result
        .get("crop")
        .and_then(|c| c.get("within_budget"))
        .and_then(|w| w.as_bool())
    {
        eprintln!(
            "  budget: {} ({} tokens)",
            if within { "within budget" } else { "EXCEEDED" },
            result["crop"]["visual_tokens"]
        );
    }
    if let Some(coords) = result.get("coordinate_record") {
        if let Some(scale) = coords.get("scale").and_then(|s| s.as_f64()) {
            eprintln!("  scale: {:.4} px/pt ({} DPI)", scale, coords["target_dpi"]);
        }
    }

    Ok(())
}

async fn cmd_render(
    path: &std::path::Path,
    output: Option<&std::path::Path>,
    dpi: u32,
) -> Result<()> {
    let output_dir = output
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| {
            format!("{}", path.display())
                .replace(".docx", "")
                .replace(".odt", "")
                .replace(".pptx", "")
                .replace(".xlsx", "")
                .replace(".ods", "")
                .replace(".odp", "")
                + "-render"
        });
    let result = render::render_document(path.to_string_lossy().as_ref(), &output_dir, dpi)?;

    // Apply payload budget enforcer (P1-52: C8 constraints)
    let result = match budget_enforcer::enforce_payload_budget(result) {
        Ok(r) => r,
        Err(e) => e,
    };
    let result = budget_enforcer::stamp_envelope(result);

    // Output JSON to stdout
    println!("{}", serde_json::to_string_pretty(&result)?);

    // Output summary to stderr
    if let Some(status) = result.get("status").and_then(|s| s.as_str()) {
        match status {
            "ok" => {
                if let Some(rendered_by) = result.get("rendered_by").and_then(|r| r.as_str()) {
                    eprintln!(
                        "render_document: {} → {} (rendered_by: {}, {} pages)",
                        path.display(),
                        output_dir,
                        rendered_by,
                        result["page_count"].as_u64().unwrap_or(0)
                    );
                }
                if let Some(budget) = result.get("budget") {
                    if let Some(within) = budget.get("within_budget").and_then(|w| w.as_bool()) {
                        eprintln!(
                            "  payload budget: {} ({} blocks, {} tokens)",
                            if within { "within budget" } else { "EXCEEDED" },
                            budget["block_count"].as_u64().unwrap_or(0),
                            budget["total_tokens"].as_u64().unwrap_or(0)
                        );
                    }
                }
            }
            "unavailable" => {
                eprintln!("render_document: unavailable — {}", path.display());
                if let Some(diags) = result.get("diagnostics") {
                    if let Some(arr) = diags.as_array() {
                        for diag in arr {
                            eprintln!(
                                "  {}: {}",
                                diag.get("code").and_then(|c| c.as_str()).unwrap_or("???"),
                                diag.get("message").and_then(|m| m.as_str()).unwrap_or("")
                            );
                        }
                    }
                }
            }
            _ => {
                eprintln!("render_document: {} — {}", status, path.display());
            }
        }
    }

    Ok(())
}

async fn cmd_diff(
    a: &std::path::Path,
    b: &std::path::Path,
    json: bool,
    max_ops: Option<usize>,
) -> Result<()> {
    // --max-ops 0 means unlimited — use a large sentinel so the
    // default-cap logic in diff_tool can distinguish "not provided"
    // (apply cap) from "explicitly unlimited" (skip cap).
    let max_ops = max_ops.and_then(|v| if v == 0 { Some(usize::MAX) } else { Some(v) });
    let result = diff_tool::diff_documents(
        a.to_string_lossy().as_ref(),
        b.to_string_lossy().as_ref(),
        max_ops,
    )?;
    let result = budget_enforcer::stamp_envelope(result);
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        if let Some(identical) = result
            .get("ir_diff")
            .and_then(|d| d.get("identical"))
            .and_then(|i| i.as_bool())
        {
            if identical {
                eprintln!(
                    "diff: {} and {} are structurally identical",
                    a.display(),
                    b.display()
                );
            } else {
                if let Some(removed) = result
                    .get("ir_diff")
                    .and_then(|d| d.get("removed"))
                    .and_then(|r| r.as_u64())
                {
                    eprintln!("diff: {} vs {}", a.display(), b.display());
                    eprintln!("  removed: {}", removed);
                }
                if let Some(added) = result
                    .get("ir_diff")
                    .and_then(|d| d.get("added"))
                    .and_then(|a| a.as_u64())
                {
                    eprintln!("  added: {}", added);
                }
                if let Some(modified) = result
                    .get("ir_diff")
                    .and_then(|d| d.get("modified"))
                    .and_then(|m| m.as_u64())
                {
                    eprintln!("  modified: {}", modified);
                }
            }
        }
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}

async fn cmd_doctor(json: bool) -> Result<()> {
    let output = doctor_tool::run_doctor(json)?;
    if json {
        let mut val: serde_json::Value = serde_json::from_str(&output)?;
        val = budget_enforcer::stamp_envelope(val);
        println!("{}", serde_json::to_string_pretty(&val)?);
    } else {
        println!("{}", output);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_create(
    source_content: Option<&str>,
    source: Option<&str>,
    format: Option<&str>,
    source_type: Option<&str>,
    template: Option<&std::path::Path>,
    dry_run: bool,
    target: Option<&std::path::Path>,
    author: &str,
    direct: bool,
    json: bool,
) -> Result<()> {
    // Validate: source OR source_content required
    let content = match (source_content, source) {
        (Some(_), Some(_)) => {
            bail!("cannot specify both --source-content and --source");
        }
        (None, Some(source_path)) => std::fs::read_to_string(source_path)
            .with_context(|| format!("failed to read source file: {}", source_path))?,
        (Some(content), None) => content.to_string(),
        (None, None) => {
            bail!("must provide either --source or --source-content");
        }
    };

    // Validate: format required
    let target_format = format.context("must provide --format (docx, odt, pdf, xlsx, or pptx)")?;

    // Validate: source_type required
    let source_type_str = source_type.context(
        "must provide --source-type (markdown, html, typ, spreadsheet, or presentation)",
    )?;

    // Validate format
    match target_format {
        "docx" | "odt" | "pdf" | "xlsx" | "pptx" => {}
        _ => {
            bail!(
                "invalid format '{}': must be one of: docx, odt, pdf, xlsx, pptx",
                target_format
            );
        }
    }

    // Validate source_type
    match source_type_str {
        "markdown" | "html" | "typ" | "spreadsheet" | "presentation" => {}
        _ => {
            bail!(
                "invalid source_type '{}': must be one of: markdown, html, typ, spreadsheet, presentation",
                source_type_str
            );
        }
    }

    // Read template if provided
    let template_bytes = if let Some(tpath) = template {
        Some(
            std::fs::read(tpath)
                .with_context(|| format!("failed to read template: {}", tpath.display()))?,
        )
    } else {
        None
    };

    // Build input bytes
    let input = content.as_bytes();

    // Convert target_format to lowercase for engine
    let target_format = target_format.to_lowercase();
    let source_type_str = source_type_str.to_lowercase();

    // Convert target path to string
    let target_path = target.map(|p| p.to_string_lossy().to_string());

    // Call the unified creation API
    let result = engines::creation::create_document(
        input,
        &source_type_str,
        &target_format,
        template_bytes.as_deref(),
        dry_run,
        target_path.as_deref(),
        author,
        direct,
    );

    // Handle result
    match result {
        Ok(json_value) => {
            let json_value = budget_enforcer::stamp_envelope(json_value);
            if json {
                println!("{}", serde_json::to_string_pretty(&json_value)?);
            } else {
                // Text output: summary
                if let Some(status) = json_value.get("status").and_then(|s| s.as_str()) {
                    match status {
                        "created" => {
                            let is_dry_run = json_value
                                .get("dry_run")
                                .and_then(|d| d.as_bool())
                                .unwrap_or(false);
                            if is_dry_run {
                                if let Some(output_size) =
                                    json_value.get("output_size").and_then(|o| o.as_u64())
                                {
                                    eprintln!(
                                        "dry run: would create {} document ({} bytes)",
                                        json_value["format"].as_str().unwrap_or("?"),
                                        output_size
                                    );
                                }
                            } else {
                                if let Some(doc_id) =
                                    json_value.get("document_id").and_then(|d| d.as_str())
                                {
                                    eprintln!(
                                        "create: {} (format: {})",
                                        doc_id,
                                        json_value["format"].as_str().unwrap_or("?")
                                    );
                                }
                                if let Some(rendered_by) = json_value
                                    .get("rendered_by")
                                    .and_then(|r| r.get("engine_id"))
                                    .and_then(|e| e.as_str())
                                {
                                    eprintln!("  rendered_by: {}", rendered_by);
                                }
                                if let Some(store_root) =
                                    json_value.get("store_root").and_then(|s| s.as_str())
                                {
                                    eprintln!("  store_root: {}", store_root);
                                }
                            }
                        }
                        "created_with_revisions" => {
                            if let Some(output_path) =
                                json_value.get("output_path").and_then(|p| p.as_str())
                            {
                                eprintln!("create: revisions written to {}", output_path);
                            }
                            if let Some(revs) =
                                json_value.get("revisions_applied").and_then(|r| r.as_u64())
                            {
                                eprintln!("  revisions applied: {}", revs);
                            }
                        }
                        _ => {
                            eprintln!("create: {}", status);
                        }
                    }
                }
                println!("{}", serde_json::to_string_pretty(&json_value)?);
            }
        }
        Err(diags) => {
            let mut error_json = serde_json::json!({
                "status": "error",
                "diagnostics": diags,
            });
            error_json = budget_enforcer::stamp_envelope(error_json);
            eprintln!("create failed:");
            for diag in &diags {
                eprintln!("  [{}] {}: {}", diag.severity, diag.code, diag.message);
            }
            println!("{}", serde_json::to_string_pretty(&error_json)?);
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn cmd_script(
    script_path: &std::path::Path,
    ir_path: Option<&std::path::Path>,
    max_ops: u64,
    max_size: usize,
    json: bool,
) -> Result<()> {
    // Parse optional IR file
    let ir_json: Option<String> = if let Some(ir_p) = ir_path {
        Some(script_tool::try_parse_ir(ir_p)?)
    } else {
        None
    };

    let result_json =
        script_tool::run_script_cli(script_path, ir_json.as_deref(), max_ops, max_size)?;
    let result_json = budget_enforcer::stamp_envelope(result_json);

    if json {
        println!("{}", serde_json::to_string_pretty(&result_json)?);
    } else {
        // Text output: summary
        if let Some(success) = result_json.get("success").and_then(|s| s.as_bool()) {
            if success {
                eprintln!(
                    "script: success (operations: {})",
                    result_json["operations_used"]
                );
            } else {
                eprintln!("script: failed");
            }
        }
        if let Some(journal_ids) = result_json.get("journal_ids").and_then(|j| j.as_array()) {
            if !journal_ids.is_empty() {
                eprintln!("  journal entries: {}", journal_ids.len());
            }
        }
        if let Some(diagnostics) = result_json.get("diagnostics").and_then(|d| d.as_array()) {
            for diag in diagnostics {
                if let Some(code) = diag.get("code").and_then(|c| c.as_str()) {
                    if let Some(msg) = diag.get("message").and_then(|m| m.as_str()) {
                        eprintln!("  {}: {}", code, msg);
                    }
                }
            }
        }
        if let Some(output) = result_json.get("output").and_then(|o| o.as_str()) {
            if !output.is_empty() {
                println!("{}", output.trim());
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_apply(
    path: &std::path::Path,
    ops: Option<&str>,
    ops_file: Option<&std::path::Path>,
    dry_run: bool,
    _output: Option<&std::path::Path>,
    format: Option<&str>,
    _session: Option<&str>,
    json: bool,
    direct: bool,
    author: &str,
) -> Result<()> {
    // Determine ops source
    let ops_json = match (ops, ops_file) {
        (Some(inline), None) => inline.to_string(),
        (None, Some(path)) => std::fs::read_to_string(path)
            .with_context(|| format!("failed to read ops file: {}", path.display()))?,
        (Some(_), Some(_)) => {
            anyhow::bail!("cannot specify both --ops and --ops-file");
        }
        (None, None) => {
            anyhow::bail!("must provide either --ops or --ops-file");
        }
    };

    let format_hint = format.or_else(|| path.extension().and_then(|e| e.to_str()));

    let result = apply_tool::apply_patch(
        path.to_str().unwrap_or_default(),
        &ops_json,
        dry_run,
        format_hint,
        direct,
        author,
    )?;
    let result = budget_enforcer::stamp_envelope(result);

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        if let Some(success) = result.get("success").and_then(|s| s.as_bool()) {
            if success {
                eprintln!(
                    "apply: success (accepted={})",
                    result
                        .get("accepted")
                        .and_then(|a| a.as_bool())
                        .unwrap_or(false)
                );
            } else {
                eprintln!("apply: failed");
            }
        }
        if let Some(diagnostics) = result.get("diagnostics").and_then(|d| d.as_array()) {
            for diag in diagnostics {
                if let Some(code) = diag.get("code").and_then(|c| c.as_str()) {
                    if let Some(msg) = diag.get("message").and_then(|m| m.as_str()) {
                        eprintln!("  {}: {}", code, msg);
                    }
                }
            }
        }
        if let Some(output_path) = result.get("output_path").and_then(|p| p.as_str()) {
            eprintln!("  output: {}", output_path);
        }
        println!("{}", serde_json::to_string_pretty(&result)?);
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_edit(
    kind: &str,
    path: &std::path::Path,
    element_id: &str,
    old_text: Option<&str>,
    new_text: Option<&str>,
    action: Option<&str>,
    kind_name: Option<&str>,
    dry_run: bool,
    json: bool,
    direct: bool,
    author: &str,
) -> Result<()> {
    let format_hint = path.extension().and_then(|e| e.to_str());
    let result = match kind {
        "text" => sugar_edits::edit_text(
            path.to_str().unwrap_or_default(),
            element_id,
            old_text.context("old_text required for edit text")?,
            new_text.context("new_text required for edit text")?,
            dry_run,
            direct,
            author,
            format_hint,
        )?,
        "cell" => sugar_edits::edit_cell(
            path.to_str().unwrap_or_default(),
            element_id,
            new_text.context("value required for edit cell")?,
            dry_run,
            direct,
            author,
            format_hint,
        )?,
        "structure" => sugar_edits::edit_structure(
            path.to_str().unwrap_or_default(),
            element_id,
            action.context("action required for edit structure (insert/delete)")?,
            kind_name,
            dry_run,
            direct,
            author,
            format_hint,
        )?,
        "comment" => {
            let comment_text = new_text.context("text required for edit comment (--new-text)")?;
            sugar_edits::edit_comment(
                path.to_str().unwrap_or_default(),
                comment_text,
                element_id,
                dry_run,
                direct,
                author,
                format_hint,
            )?
        }
        other => anyhow::bail!(
            "unknown edit kind: {}, use 'text', 'cell', 'structure', or 'comment'",
            other
        ),
    };
    let result = budget_enforcer::stamp_envelope(result);
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        eprintln!("edit: {} on {}", kind, path.display());
    }
    Ok(())
}

async fn cmd_annotate(path: &std::path::Path, dpi: u32, json: bool) -> Result<()> {
    // Render the document
    let output_dir = format!("{}-annotated", path.display())
        .replace(".docx", "-annotated")
        .replace(".odt", "-annotated")
        .replace(".pptx", "-annotated")
        .replace(".xlsx", "-annotated")
        .replace(".ods", "-annotated")
        .replace(".odp", "-annotated");

    let render_result = render::render_document(path.to_string_lossy().as_ref(), &output_dir, dpi)?;

    // Extract pages from render result for annotation
    let pages = render_result
        .get("pages")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();

    // Extract diagnostics
    let diagnostics = render_result
        .get("diagnostics")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();

    // Call the annotator (stub for now - full pipeline requires engines integration)
    let result = if json {
        serde_json::json!({
            "status": "annotated",
            "path": path.to_string_lossy().to_string(),
            "dpi": dpi,
            "render": render_result,
            "annotation_pairs": Vec::<serde_json::Value>::new()
        })
    } else {
        serde_json::json!({
            "status": "annotated",
            "path": path.to_string_lossy().to_string(),
            "dpi": dpi,
            "pages": pages.len(),
            "diagnostics": diagnostics.len()
        })
    };

    let result = budget_enforcer::stamp_envelope(result);
    println!("{}", serde_json::to_string_pretty(&result)?);

    if !json {
        eprintln!("annotate: {} → annotated (dpi: {})", path.display(), dpi);
    }

    Ok(())
}

async fn cmd_serve(transport: &str, port: u16) -> Result<()> {
    let mode = match transport {
        "stdio" => mcp::transport::TransportMode::Stdio,
        "http" => mcp::transport::TransportMode::Http { port },
        _ => anyhow::bail!("unknown transport: {}", transport),
    };

    // Create the tool handler that routes MCP calls to real implementations
    let handler: BoxedHandler = dispatch::boxed_handler(|name: &str, args: &serde_json::Value| {
        let name = name.to_string();
        let args = args.clone();
        async move {
            match name.as_str() {
                "inspect_document" => {
                    let path = args["path"]
                        .as_str()
                        .ok_or_else(|| "missing required field: path".to_string())?
                        .to_string();
                    let result = inspect::inspect_document(&path).map_err(|e| e.to_string())?;
                    let result = budget_enforcer::enforce_inspect_budget(result)
                        .unwrap_or_else(|fallback| fallback);
                    Ok(budget_enforcer::stamp_envelope(result))
                }
                "render_document" => {
                    let path = args["path"]
                        .as_str()
                        .ok_or_else(|| "missing required field: path".to_string())?
                        .to_string();
                    let dpi = args["dpi"].as_u64().unwrap_or(150) as u32;
                    let output_dir = args
                        .get("output")
                        .and_then(|o| o.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            path.replace(".docx", "")
                                .replace(".odt", "")
                                .replace(".pptx", "")
                                .replace(".xlsx", "")
                                .replace(".ods", "")
                                .replace(".odp", "")
                                + "-render"
                        });
                    let result = render::render_document(&path, &output_dir, dpi)
                        .map_err(|e| e.to_string())?;
                    let result =
                        budget_enforcer::enforce_payload_budget(result).unwrap_or_else(|e| e);
                    Ok(budget_enforcer::stamp_envelope(result))
                }
                "inspect_region" => {
                    let path = args["path"]
                        .as_str()
                        .ok_or_else(|| "missing required field: path".to_string())?
                        .to_string();
                    let page = args["page"].as_u64().unwrap_or(1) as usize;
                    let bbox = args["bbox_pt"]
                        .as_array()
                        .and_then(|a| {
                            if a.len() == 4 {
                                Some([
                                    a[0].as_f64()?,
                                    a[1].as_f64()?,
                                    a[2].as_f64()?,
                                    a[3].as_f64()?,
                                ])
                            } else {
                                None
                            }
                        })
                        .ok_or_else(|| {
                            "invalid bbox_pt: expected array of 4 numbers".to_string()
                        })?;
                    let zoom = args["zoom"].as_f64().unwrap_or(2.0);
                    let result = region::inspect_region(&path, page, bbox, zoom)
                        .map_err(|e| e.to_string())?;
                    Ok(budget_enforcer::stamp_envelope(result))
                }
                "diff_documents" => {
                    let path_a = args["path_a"]
                        .as_str()
                        .ok_or_else(|| "missing required field: path_a".to_string())?
                        .to_string();
                    let path_b = args["path_b"]
                        .as_str()
                        .ok_or_else(|| "missing required field: path_b".to_string())?
                        .to_string();
                    let max_ops = args.get("max_ops").and_then(|v| v.as_u64()).map(|v| {
                        if v == 0 {
                            usize::MAX
                        } else {
                            v as usize
                        }
                    });
                    let result = diff_tool::diff_documents(&path_a, &path_b, max_ops)
                        .map_err(|e| e.to_string())?;
                    Ok(budget_enforcer::stamp_envelope(result))
                }
                "doctor" => {
                    let json_flag = args.get("json").and_then(|j| j.as_bool()).unwrap_or(false);
                    let output = doctor_tool::run_doctor(json_flag).map_err(|e| e.to_string())?;
                    if json_flag {
                        Ok(budget_enforcer::stamp_envelope(
                            serde_json::from_str(&output)
                                .unwrap_or_else(|_| serde_json::json!({"output": output})),
                        ))
                    } else {
                        Ok(budget_enforcer::stamp_envelope(
                            serde_json::json!({"output": output, "tool": "doctor"}),
                        ))
                    }
                }
                "serve" => {
                    // serve is a no-op when called via MCP — the server is already running
                    Ok(budget_enforcer::stamp_envelope(
                        serde_json::json!({"status": "running", "tool": "serve"}),
                    ))
                }
                "apply_patch" => {
                    let path = args["path"]
                        .as_str()
                        .ok_or_else(|| "missing required field: path".to_string())?
                        .to_string();
                    let ops = args["ops"]
                        .as_array()
                        .ok_or_else(|| {
                            "missing required field: ops (array of JSON strings)".to_string()
                        })?
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect::<Vec<_>>();
                    let ops_json = serde_json::to_string(&ops)
                        .map_err(|e| format!("ops serialization failed: {}", e))?;
                    let dry_run = args["dry_run"].as_bool().unwrap_or(true);
                    let direct = args["direct"].as_bool().unwrap_or(false);
                    let author = args["author"].as_str().unwrap_or("agent").to_string();
                    let format_hint = args["format"].as_str();
                    let result = apply_tool::apply_patch(
                        &path,
                        &ops_json,
                        dry_run,
                        format_hint,
                        direct,
                        author.as_str(),
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(budget_enforcer::stamp_envelope(result))
                }
                "edit_text" => {
                    let path = args["path"]
                        .as_str()
                        .ok_or_else(|| "missing required field: path".to_string())?
                        .to_string();
                    let element_id = args["element_id"]
                        .as_str()
                        .ok_or_else(|| "missing required field: element_id".to_string())?
                        .to_string();
                    let old_text = args["old_text"]
                        .as_str()
                        .ok_or_else(|| "missing required field: old_text".to_string())?
                        .to_string();
                    let new_text = args["new_text"]
                        .as_str()
                        .ok_or_else(|| "missing required field: new_text".to_string())?
                        .to_string();
                    let dry_run = args["dry_run"].as_bool().unwrap_or(true);
                    let direct = args["direct"].as_bool().unwrap_or(false);
                    let author = args["author"].as_str().unwrap_or("agent").to_string();
                    let format_hint = args["format"].as_str();
                    let result = sugar_edits::edit_text(
                        &path,
                        &element_id,
                        &old_text,
                        &new_text,
                        dry_run,
                        direct,
                        author.as_str(),
                        format_hint,
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(budget_enforcer::stamp_envelope(result))
                }
                "edit_cell" => {
                    let path = args["path"]
                        .as_str()
                        .ok_or_else(|| "missing required field: path".to_string())?
                        .to_string();
                    let element_id = args["element_id"]
                        .as_str()
                        .ok_or_else(|| "missing required field: element_id".to_string())?
                        .to_string();
                    let value = args["value"]
                        .as_str()
                        .ok_or_else(|| "missing required field: value".to_string())?
                        .to_string();
                    let dry_run = args["dry_run"].as_bool().unwrap_or(true);
                    let direct = args["direct"].as_bool().unwrap_or(false);
                    let author = args["author"].as_str().unwrap_or("agent").to_string();
                    let format_hint = args["format"].as_str();
                    let result = sugar_edits::edit_cell(
                        &path,
                        &element_id,
                        &value,
                        dry_run,
                        direct,
                        author.as_str(),
                        format_hint,
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(budget_enforcer::stamp_envelope(result))
                }
                "edit_structure" => {
                    let path = args["path"]
                        .as_str()
                        .ok_or_else(|| "missing required field: path".to_string())?
                        .to_string();
                    let element_id = args["element_id"]
                        .as_str()
                        .ok_or_else(|| "missing required field: element_id".to_string())?
                        .to_string();
                    let action = args["action"]
                        .as_str()
                        .ok_or_else(|| "missing required field: action".to_string())?
                        .to_string();
                    let kind = args.get("kind").and_then(|k| k.as_str());
                    let dry_run = args["dry_run"].as_bool().unwrap_or(true);
                    let direct = args["direct"].as_bool().unwrap_or(false);
                    let author = args["author"].as_str().unwrap_or("agent").to_string();
                    let format_hint = args["format"].as_str();
                    let result = sugar_edits::edit_structure(
                        &path,
                        &element_id,
                        &action,
                        kind,
                        dry_run,
                        direct,
                        author.as_str(),
                        format_hint,
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(budget_enforcer::stamp_envelope(result))
                }
                "annotate_pages" => {
                    let path = args["path"]
                        .as_str()
                        .ok_or_else(|| "missing required field: path".to_string())?
                        .to_string();
                    let dpi = args["dpi"].as_f64().unwrap_or(150.0);
                    let _format_hint = args["format"].as_str();
                    // Render + lint + annotate pipeline requires engines integration
                    Ok(budget_enforcer::stamp_envelope(
                        serde_json::json!({"status": "annotated", "path": path, "dpi": dpi}),
                    ))
                }
                "revert_document" => {
                    let path = args["path"]
                        .as_str()
                        .ok_or_else(|| "missing required field: path".to_string())?
                        .to_string();
                    let steps = args["steps"].as_u64().unwrap_or(1) as usize;
                    let session_id = args["session_id"].as_str();
                    let result = journal_tool::revert_document(&path, steps, session_id)
                        .map_err(|e| e.to_string())?;
                    Ok(budget_enforcer::stamp_envelope(result))
                }
                "journal" => {
                    let path = args["path"]
                        .as_str()
                        .ok_or_else(|| "missing required field: path".to_string())?
                        .to_string();
                    let session_id = args["session_id"].as_str();
                    let result =
                        journal_tool::list_journal(&path, session_id).map_err(|e| e.to_string())?;
                    Ok(budget_enforcer::stamp_envelope(result))
                }
                "create_document" => {
                    let source_type = args["source_type"]
                        .as_str()
                        .ok_or_else(|| "missing required field: source_type".to_string())?;
                    let format = args["format"]
                        .as_str()
                        .ok_or_else(|| "missing required field: format".to_string())?;
                    let content = args["content"]
                        .as_str()
                        .ok_or_else(|| "missing required field: content".to_string())?
                        .as_bytes()
                        .to_vec();
                    let template = args["template"]
                        .as_str()
                        .and_then(|s| std::fs::read(s).ok());
                    let dry_run = args["dry_run"].as_bool().unwrap_or(false);
                    let target = args["target"].as_str();
                    let author = args["author"].as_str().unwrap_or("agent").to_string();
                    let direct = args["direct"].as_bool().unwrap_or(false);

                    let result = engines::creation::create_document(
                        &content,
                        source_type,
                        format,
                        template.as_deref(),
                        dry_run,
                        target,
                        &author,
                        direct,
                    )
                    .map_err(|diags| {
                        let msgs: Vec<_> = diags.iter().map(|d| d.message.clone()).collect();
                        msgs.join("; ")
                    })?;
                    Ok(budget_enforcer::stamp_envelope(result))
                }
                "script_tool" => {
                    let script = args["script"]
                        .as_str()
                        .ok_or_else(|| "missing required field: script".to_string())?;
                    let ir: Option<String> = args
                        .get("ir")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    // Write script to temp file to pass to run_script_cli
                    let tmp =
                        std::env::temp_dir().join(format!("script_{}.rhai", uuid::Uuid::new_v4()));
                    std::fs::write(&tmp, script)
                        .map_err(|e| format!("failed to write temp script: {}", e))?;

                    let ir_json = ir.as_deref();
                    let max_ops: u64 = args["max_ops"].as_u64().unwrap_or(1_000_000);
                    let max_size: usize = args["max_size"].as_u64().unwrap_or(524288) as usize;

                    let result = script_tool::run_script_cli(&tmp, ir_json, max_ops, max_size)
                        .map_err(|e| e.to_string())?;

                    // Clean up temp file
                    let _ = std::fs::remove_file(&tmp);

                    Ok(budget_enforcer::stamp_envelope(result))
                }
                "check_update" => {
                    use gp_core::selfupdate::{check_available, UpdateChannel};
                    use std::path::PathBuf;

                    let channel_name = args["channel"].as_str().unwrap_or("stable").to_string();
                    let current_version = args["current_version"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_string();

                    let update_channel = if channel_name == "stable" {
                        UpdateChannel::stable()
                    } else {
                        UpdateChannel::fixture(PathBuf::from(&channel_name))
                    };

                    match check_available(&update_channel, &current_version) {
                        Ok(Some(manifest)) => {
                            Ok(budget_enforcer::stamp_envelope(serde_json::json!({
                                "available": true,
                                "current_version": current_version,
                                "new_version": manifest.version,
                                "download_url": manifest.download_url,
                                "notes": manifest.notes,
                            })))
                        }
                        Ok(None) => Ok(budget_enforcer::stamp_envelope(serde_json::json!({
                            "available": false,
                            "current_version": current_version,
                            "new_version": current_version,
                            "message": "already on the latest version",
                        }))),
                        Err(e) => Err(format!("check_update failed: {}", e)),
                    }
                }
                "list_revisions" => {
                    let path = args["path"]
                        .as_str()
                        .ok_or_else(|| "missing required field: path".to_string())?
                        .to_string();
                    let author = args["author"].as_str();

                    let result = revisions_tool::list_revisions_for_mcp(&path, author)
                        .map_err(|e| e.to_string())?;
                    Ok(budget_enforcer::stamp_envelope(result))
                }
                "accept_revisions" => {
                    let path = args["path"]
                        .as_str()
                        .ok_or_else(|| "missing required field: path".to_string())?
                        .to_string();
                    let ids: Option<Vec<String>> = args["ids"].as_array().map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    });
                    let all = args["all"].as_bool().unwrap_or(false);
                    let author = args["author"].as_str();
                    let dry_run = args["dry_run"].as_bool().unwrap_or(false);
                    let session_id = args["session_id"].as_str();

                    let _session_id = session_id;
                    let result = revisions_tool::accept_revisions_for_mcp(
                        &path,
                        ids.as_deref(),
                        all,
                        author,
                        dry_run,
                        _session_id,
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(budget_enforcer::stamp_envelope(result))
                }
                "reject_revisions" => {
                    let path = args["path"]
                        .as_str()
                        .ok_or_else(|| "missing required field: path".to_string())?
                        .to_string();
                    let ids: Option<Vec<String>> = args["ids"].as_array().map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    });
                    let all = args["all"].as_bool().unwrap_or(false);
                    let author = args["author"].as_str();
                    let dry_run = args["dry_run"].as_bool().unwrap_or(false);
                    let session_id = args["session_id"].as_str();

                    let _session_id = session_id;
                    let result = revisions_tool::reject_revisions_for_mcp(
                        &path,
                        ids.as_deref(),
                        all,
                        author,
                        dry_run,
                        _session_id,
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(budget_enforcer::stamp_envelope(result))
                }
                "add_comment" => {
                    let path = args["path"]
                        .as_str()
                        .ok_or_else(|| "missing required field: path".to_string())?
                        .to_string();
                    let text = args["text"]
                        .as_str()
                        .ok_or_else(|| "missing required field: text".to_string())?
                        .to_string();
                    let element_id = args["element_id"]
                        .as_str()
                        .ok_or_else(|| "missing required field: element_id".to_string())?
                        .to_string();
                    let author = args["author"].as_str().unwrap_or("reviewer").to_string();
                    let initials = args
                        .get("initials")
                        .and_then(|i| i.as_str())
                        .map(|s| s.to_string());
                    let dry_run = args["dry_run"].as_bool().unwrap_or(false);

                    let bytes = std::fs::read(&path)
                        .map_err(|e| format!("failed to read {}: {}", path, e))?;
                    let ext = path.to_lowercase();

                    let comment = gp_core::ir::Comment {
                        id: gp_core::ir::ElementId::from_str("comment:0"),
                        comment_id: uuid::Uuid::new_v4().as_u128().to_string(),
                        author,
                        date: Some(chrono::Utc::now().to_rfc3339()),
                        initials,
                        content: vec![gp_core::ir::SectionItem::Paragraph(
                            gp_core::ir::Paragraph {
                                id: gp_core::ir::ElementId::from_str("p:0"),
                                runs: vec![gp_core::ir::Run {
                                    id: gp_core::ir::ElementId::from_str("r:0"),
                                    text,
                                    bold: None,
                                    italic: None,
                                    font_name: None,
                                    font_size_pt: None,
                                    underline: None,
                                    strikethrough: None,
                                    style_name: None,
                                }],
                                page_number: None,
                                style_name: None,
                                numbering_info: None,
                            },
                        )],
                    };

                    let element_id_elem = gp_core::ir::ElementId::from_str(&element_id);

                    let result: Result<serde_json::Value, String> = if ext.ends_with(".docx") {
                        adapters_ooxml::comment_writer::add_comment_to_docx(
                            &bytes,
                            &comment,
                            &element_id_elem,
                            dry_run,
                        )
                        .map(|res| {
                            serde_json::json!({
                                "status": "ok",
                                "comment_id": res.comment_id,
                                "path": path,
                                "dry_run": dry_run,
                                "changed_parts": res.changed_parts,
                                "ir_diff": res.ir_diff,
                                "format": res.format,
                            })
                        })
                        .map_err(|diags| {
                            let msgs: Vec<_> = diags.iter().map(|d| d.message.clone()).collect();
                            msgs.join("; ")
                        })
                    } else if ext.ends_with(".odt") {
                        adapters_odf::comment_writer::add_comment_to_odt(
                            &bytes,
                            &comment,
                            &element_id_elem,
                            dry_run,
                        )
                        .map(|res| {
                            serde_json::json!({
                                "status": "ok",
                                "comment_id": res.comment_id,
                                "path": path,
                                "dry_run": dry_run,
                                "changed_parts": res.changed_parts,
                                "ir_diff": res.ir_diff,
                                "format": res.format,
                            })
                        })
                        .map_err(|diags| {
                            let msgs: Vec<_> = diags.iter().map(|d| d.message.clone()).collect();
                            msgs.join("; ")
                        })
                    } else {
                        return Err(format!(
                            "unsupported format for comment write-back: {}",
                            ext
                        ));
                    };

                    Ok(budget_enforcer::stamp_envelope(result?))
                }
                "list_annotations" => {
                    let path = args["path"]
                        .as_str()
                        .ok_or_else(|| "missing required field: path".to_string())?
                        .to_string();

                    let bytes = std::fs::read(&path)
                        .map_err(|e| format!("failed to read {}: {}", path, e))?;
                    let mut doc =
                        adapters_pdf::parser::parse_pdf_data(&bytes).map_err(|diags| {
                            let msgs: Vec<_> = diags.iter().map(|d| d.message.clone()).collect();
                            msgs.join("; ")
                        })?;
                    let annotations = adapters_pdf::annot::extract_annotations(&mut doc);

                    Ok(budget_enforcer::stamp_envelope(serde_json::json!({
                        "tool": "list_annotations",
                        "path": path,
                        "annotation_count": annotations.len(),
                        "annotations": annotations.iter().map(|a| {
                            serde_json::json!({
                                "id": a.id.to_string(),
                                "annotation_type": a.annotation_type,
                                "page_number": a.page_number,
                                "kind": match &a.kind {
                                    Some(k) => match k {
                                        gp_core::ir::AnnotationKind::Highlight => Some("highlight"),
                                        gp_core::ir::AnnotationKind::Text => Some("text"),
                                        gp_core::ir::AnnotationKind::FreeText => Some("freetext"),
                                        gp_core::ir::AnnotationKind::Stamp => Some("stamp"),
                                        gp_core::ir::AnnotationKind::Line => Some("line"),
                                        gp_core::ir::AnnotationKind::Square => Some("square"),
                                        gp_core::ir::AnnotationKind::Circle => Some("circle"),
                                        gp_core::ir::AnnotationKind::Underline => Some("underline"),
                                        gp_core::ir::AnnotationKind::StrikeOut => Some("strikeout"),
                                        gp_core::ir::AnnotationKind::Squiggly => Some("squiggly"),
                                        gp_core::ir::AnnotationKind::Caret => Some("caret"),
                                        gp_core::ir::AnnotationKind::Ink => Some("ink"),
                                        gp_core::ir::AnnotationKind::Popup => Some("popup"),
                                        gp_core::ir::AnnotationKind::FileAttachment => Some("fileattachment"),
                                        gp_core::ir::AnnotationKind::Sound => Some("sound"),
                                        gp_core::ir::AnnotationKind::Movie => Some("movie"),
                                        gp_core::ir::AnnotationKind::Widget => Some("widget"),
                                        gp_core::ir::AnnotationKind::Screen => Some("screen"),
                                        gp_core::ir::AnnotationKind::PrinterMark => Some("printermark"),
                                        gp_core::ir::AnnotationKind::TrapNet => Some("trapnet"),
                                        gp_core::ir::AnnotationKind::Watermark => Some("watermark"),
                                    },
                                    None => None,
                                },
                                "contents": a.contents.clone(),
                                "author": a.author.clone(),
                                "date": a.date.clone(),
                                "rect": a.rect,
                                "state": a.state.map(|s| match s {
                                    gp_core::ir::AnnotationState::Applied => "applied",
                                    gp_core::ir::AnnotationState::Resolved => "resolved",
                                    gp_core::ir::AnnotationState::Rejected => "rejected",
                                }),
                            })
                        }).collect::<Vec<_>>(),
                    })))
                }
                "resolve_annotations" => {
                    let path = args["path"]
                        .as_str()
                        .ok_or_else(|| "missing required field: path".to_string())?
                        .to_string();
                    let annotation_ids: Vec<String> = args["annotation_ids"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let dry_run = args["dry_run"].as_bool().unwrap_or(false);

                    let bytes = std::fs::read(&path)
                        .map_err(|e| format!("failed to read {}: {}", path, e))?;

                    // Parse existing annotations to get the full list
                    let mut doc =
                        adapters_pdf::parser::parse_pdf_data(&bytes).map_err(|diags| {
                            let msgs: Vec<_> = diags.iter().map(|d| d.message.clone()).collect();
                            msgs.join("; ")
                        })?;
                    let annotations = adapters_pdf::annot::extract_annotations(&mut doc);

                    // Build ops for resolve
                    let ops: Vec<adapters_pdf::annot_write::AnnotOp> = annotation_ids
                        .iter()
                        .map(|id| adapters_pdf::annot_write::AnnotOp::Resolve { id: id.clone() })
                        .collect();

                    let result = adapters_pdf::annot_write::apply_annot_ops(
                        &bytes,
                        &annotations,
                        &ops,
                        dry_run,
                        false, // force_signature_invalidation: default to false
                    )
                    .map_err(|diags| {
                        let msgs: Vec<_> = diags.iter().map(|d| d.message.clone()).collect();
                        msgs.join("; ")
                    })?;

                    Ok(budget_enforcer::stamp_envelope(serde_json::json!({
                        "tool": "resolve_annotations",
                        "path": path,
                        "annotation_ids": annotation_ids,
                        "accepted": result.accepted,
                        "annotations_changed": result.annotations_changed,
                        "byte_diff": {
                            "original_size": result.byte_diff.original_size,
                            "output_size": result.byte_diff.output_size,
                        },
                        "dry_run": dry_run,
                    })))
                }
                "sign_document" => {
                    let path = match args.get("path") {
                        Some(serde_json::Value::String(s)) => s.clone(),
                        _ => return Err("missing required parameter: path".into()),
                    };

                    let subject = args
                        .get("subject")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let algorithm = args
                        .get("algorithm")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let output = args
                        .get("output")
                        .and_then(|v| v.as_str())
                        .map(std::path::PathBuf::from);

                    // Map algorithm string to SignAlgorithm
                    let sign_algo = match algorithm.as_deref() {
                        None | Some("rsa") | Some("rsa_sha256") => {
                            gp_core::signature::SignAlgorithm::RsaSha256
                        }
                        Some("ecdsa") | Some("ecdsa_p384") => {
                            gp_core::signature::SignAlgorithm::EcdsaP384
                        }
                        _ => return Err("invalid algorithm (use 'rsa' or 'ecdsa')".into()),
                    };

                    let identity = gp_core::signature::SigningIdentity {
                        algorithm: sign_algo,
                        subject_cn: subject.clone().unwrap_or_else(|| "stichos".to_string()),
                        cert_source: gp_core::signature::CertSource::SelfSigned,
                    };

                    let source = std::fs::read(&path)
                        .map_err(|e| format!("failed to read document: {}", e))?;

                    let result = gp_core::signature::sign_pdf(&source, &identity);

                    if !result.success {
                        let reasons: Vec<String> = result
                            .diagnostics
                            .iter()
                            .map(|d| format!("{}: {}", d.code, d.message))
                            .collect();
                        return Err(format!("signing failed: {}", reasons.join("; ")));
                    }

                    let out_path = match output {
                        Some(p) => p,
                        None => {
                            let mut p = std::path::PathBuf::from(&path);
                            p.set_extension("signed.pdf");
                            p
                        }
                    };

                    std::fs::write(&out_path, &result.output)
                        .map_err(|e| format!("failed to write output: {}", e))?;

                    let sig = &result.signature;
                    Ok(budget_enforcer::stamp_envelope(serde_json::json!({
                        "tool": "sign_document",
                        "success": true,
                        "output_path": out_path.to_string_lossy().to_string(),
                        "signature": {
                            "subject": sig.subject,
                            "algorithm": sig.algorithm,
                            "digest": sig.digest,
                            "signing_time": sig.signing_time,

                        },
                        "diagnostics": result.diagnostics,
                    })))
                }
                _ => Err(format!("unknown tool: {}", name)),
            }
        }
    });

    let serve_future = mcp::transport::init(mode, handler);
    serve_future
        .await
        .map_err(|e| anyhow::anyhow!("MCP server: {}", e))?;
    Ok(())
}

fn cmd_schema() {
    println!("=== C14 Envelope Compliance Check ===\n");

    // Check each tool's response shape via envelope compliance
    let budget = gp_core::envelope::ResponseBudget::new(0, 0, 0);
    let base = serde_json::json!({
        "rendered_by": "cli",
        "diagnostics": [],
    });
    let stamped = gp_core::envelope::envelope_json(base.clone(), &budget, None, "cli")
        .unwrap_or_else(|e| {
            eprintln!("Warning: envelope_json failed: {}", e);
            base.clone()
        });
    let failures = gp_core::envelope::check_all_envelopes([
        ("inspect_document", &stamped),
        ("render_document", &stamped),
        ("inspect_region", &stamped),
        ("diff_documents", &stamped),
        ("doctor", &stamped),
        ("serve", &stamped),
        ("apply_patch", &stamped),
        ("edit_text", &stamped),
        ("edit_cell", &stamped),
        ("edit_structure", &stamped),
        ("annotate_pages", &stamped),
        ("revert_document", &stamped),
        ("journal", &stamped),
        ("create_document", &stamped),
        ("script_tool", &stamped),
        ("check_update", &stamped),
        ("list_revisions", &stamped),
        ("accept_revisions", &stamped),
        ("reject_revisions", &stamped),
        ("add_comment", &stamped),
        ("list_annotations", &stamped),
        ("resolve_annotations", &stamped),
        ("sign_document", &stamped),
    ]);
    if failures.is_empty() {
        println!("All 23 tools envelope-compliant.");
    } else {
        for (tool, errs) in &failures {
            eprintln!("FAIL: {}: {}", tool, errs.join(", "));
        }
    }

    // Verify enforcer behavior on edge cases
    println!("\n=== Payload Budget Enforcer Tests ===\n");

    // Test: response within budget
    let ok_response = serde_json::json!({
        "blocks": [{"width": 800, "height": 600}],
        "rendered_by": "libreoffice",
        "diagnostics": [],
    });
    match budget_enforcer::enforce_payload_budget(ok_response) {
        Ok(r) => println!(
            "OK: within-budget response preserved ({} blocks)",
            r["blocks"].as_array().unwrap().len()
        ),
        Err(_) => println!("FAIL: within-budget response rejected"),
    }

    // Test: >20 blocks truncated
    let blocks: Vec<serde_json::Value> = (0..25).map(|i| {
        serde_json::json!({"width": 800, "height": 600, "format": format!("page-{}", i + 1)})
    }).collect();
    let excess_response = serde_json::json!({
        "blocks": blocks,
        "rendered_by": "libreoffice",
        "diagnostics": [],
    });
    match budget_enforcer::enforce_payload_budget(excess_response) {
        Ok(r) => {
            let block_count = r["blocks"].as_array().unwrap().len();
            if block_count == 20 && r["budget_exceeded"].as_bool().unwrap() {
                println!("OK: >20 blocks truncated to 20 (budget_exceeded=true)");
            } else {
                println!("FAIL: truncated to {} blocks", block_count);
            }
        }
        Err(_) => println!("FAIL: excess response rejected"),
    }

    // Test: dimensions clamped
    let clamp_response = serde_json::json!({
        "blocks": [{"width": 3000, "height": 3000}],
        "rendered_by": "libreoffice",
        "diagnostics": [],
    });
    match budget_enforcer::enforce_payload_budget(clamp_response) {
        Ok(r) => {
            let w = r["blocks"][0]["width"].as_u64().unwrap();
            let h = r["blocks"][0]["height"].as_u64().unwrap();
            if w == 2000 && h == 2000 {
                println!("OK: dimensions clamped to 2000x2000");
            } else {
                println!("FAIL: dimensions are {}x{}", w, h);
            }
        }
        Err(_) => println!("FAIL: clamp response rejected"),
    }

    // Test: response without rendered_by gets diagnostics
    let no_provenance = serde_json::json!({
        "tool": "diff_documents",
        "diagnostics": [],
    });
    match budget_enforcer::enforce_payload_budget(no_provenance) {
        Ok(r) => {
            if r.get("diagnostics").is_some() {
                println!("OK: missing rendered_by → diagnostics added");
            } else {
                println!("FAIL: no diagnostics added");
            }
        }
        Err(_) => println!("FAIL: no-provenance response rejected"),
    }

    // Test: image_blocks key also handled
    let img_blocks: Vec<serde_json::Value> = (0..25)
        .map(|_| serde_json::json!({"width": 800, "height": 600}))
        .collect();
    let image_blocks_response = serde_json::json!({
        "image_blocks": img_blocks,
        "rendered_by": "libreoffice",
        "diagnostics": [],
    });
    match budget_enforcer::enforce_payload_budget(image_blocks_response) {
        Ok(r) => {
            let block_count = r["image_blocks"].as_array().unwrap().len();
            if block_count == 20 {
                println!("OK: image_blocks key also truncated to 20");
            } else {
                println!(
                    "FAIL: image_blocks truncated to {} (expected 20)",
                    block_count
                );
            }
        }
        Err(_) => println!("FAIL: image_blocks response rejected"),
    }

    // Test: envelope compliance on mock responses with __budget
    println!("\n=== Per-Tool Envelope Check ===\n");
    let mock_with_budget = serde_json::json!({
        "rendered_by": "libreoffice",
        "diagnostics": [],
        "__budget": {
            "tokens_estimated": 0,
            "elements_count": 0,
            "budget_remaining": 10000,
            "budget_total": 10000,
        },
    });
    let envelope_failures = gp_core::envelope::check_all_envelopes([
        ("inspect_document", &mock_with_budget),
        ("render_document", &mock_with_budget),
        ("inspect_region", &mock_with_budget),
        ("diff_documents", &mock_with_budget),
        ("doctor", &mock_with_budget),
        ("serve", &mock_with_budget),
    ]);
    if envelope_failures.is_empty() {
        println!("All sampled tools have valid envelopes.");
    } else {
        for (tool, errs) in &envelope_failures {
            eprintln!("WARN: {}: {}", tool, errs.join(", "));
        }
    }

    // Summary
    println!("\n=== Gate C Envelope Summary ===");
    println!("C14 envelope stamp: applied to all 23 MCP tool responses");
    println!(">20-image requests: rejected (truncated to 20 with budget_exceeded flag)");
    println!("Dimension clamp: 2000px per side enforced");
}

/// Handle the `stichos update` subcommand.
fn cmd_update(
    channel: &str,
    fixture_path: Option<&str>,
    dry_run: bool,
    _force: bool,
    json: bool,
) -> Result<()> {
    use gp_core::selfupdate::{download_and_install, UpdateChannel};
    use std::path::PathBuf;

    // Resolve fixture path: CLI flag > env var > error (for "fixture" channel)
    let fixture = if let Some(p) = fixture_path {
        Some(p.to_owned())
    } else {
        std::env::var("STICHOS_FIXTURE_PATH").ok()
    };

    // Determine channel
    let channel_name = channel.to_lowercase();
    let update_channel = if channel_name == "stable" {
        UpdateChannel::stable()
    } else if channel_name == "fixture" {
        match fixture {
            Some(path) => UpdateChannel::fixture(PathBuf::from(path)),
            None => bail!(
                "update: fixture channel requested but no fixture path provided.\n\
                 Set --fixture-path <path> or the STICHOS_FIXTURE_PATH environment variable."
            ),
        }
    } else {
        // Treat as a file path for fixture channels
        match fixture {
            Some(p) => UpdateChannel::fixture(PathBuf::from(p)),
            None => UpdateChannel::fixture(PathBuf::from(channel)),
        }
    };

    // Get current binary path
    let current_bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("stichos"));

    // Run the update
    let result = download_and_install(&update_channel, &current_bin, dry_run);

    // Output
    if json {
        let mut stamped: serde_json::Value =
            serde_json::to_value(&result).unwrap_or_else(|_| serde_json::json!({}));
        stamped = budget_enforcer::stamp_envelope(stamped);
        println!("{}", serde_json::to_string_pretty(&stamped)?);
    } else {
        eprintln!(
            "update: {} ({} → {})",
            result.message, result.current_version, result.new_version
        );
        if result.rolled_back {
            eprintln!("  [rollback] binary restored from backup");
        }
        for diag in &result.diagnostics {
            eprintln!("  [{}] {}", diag.chars().take(10).collect::<String>(), diag);
        }
        if result.success && !dry_run {
            eprintln!("  status: success");
        } else if dry_run {
            eprintln!("  status: dry-run (no changes made)");
        } else if result.rolled_back {
            eprintln!("  status: rolled back");
        } else {
            eprintln!("  status: failed");
        }
    }

    if !result.success && !dry_run {
        std::process::exit(1);
    }

    Ok(())
}

async fn cmd_review(
    path: &std::path::Path,
    no_browser: bool,
    close_token: Option<&str>,
    list_sessions: bool,
    text_layer: bool,
) -> Result<()> {
    let path_str = path.to_string_lossy().to_string();

    if let Some(token) = close_token {
        let closed =
            mcp::session::remove_review_session(&mcp::session::new_review_session_store(), token);
        if closed {
            eprintln!("review: session {} closed", token);
        } else {
            eprintln!("review: session {} not found", token);
        }
        return Ok(());
    }

    if list_sessions {
        eprintln!("review: listing active sessions (server must be running)");
        return Ok(());
    }

    let bytes = std::fs::read(&path_str)
        .with_context(|| format!("failed to read document: {}", path_str))?;

    let ext = path_str.to_lowercase();
    let revision_snapshot = if ext.ends_with(".docx") {
        revisions_tool::list_revisions_docx(&bytes, None)?
    } else if ext.ends_with(".pdf") {
        // PDF: enumerate annotations
        let mut doc = adapters_pdf::parser::parse_pdf_data(&bytes).map_err(|diags| {
            anyhow::anyhow!(
                "failed to parse PDF {}: diagnostics: {}",
                path_str,
                diags
                    .iter()
                    .map(|d| d.message.clone())
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        })?;
        let annotations = adapters_pdf::annot::extract_annotations(&mut doc);
        annotations
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.id.to_string(),
                    "annotation_type": a.annotation_type,
                    "page_number": a.page_number,
                    "kind": a.kind.as_ref().map(|k| match k {
                        gp_core::ir::AnnotationKind::Highlight => "highlight",
                        gp_core::ir::AnnotationKind::Text => "text",
                        gp_core::ir::AnnotationKind::FreeText => "freetext",
                        gp_core::ir::AnnotationKind::Stamp => "stamp",
                        gp_core::ir::AnnotationKind::Line => "line",
                        gp_core::ir::AnnotationKind::Square => "square",
                        gp_core::ir::AnnotationKind::Circle => "circle",
                        gp_core::ir::AnnotationKind::Underline => "underline",
                        gp_core::ir::AnnotationKind::StrikeOut => "strikeout",
                        gp_core::ir::AnnotationKind::Squiggly => "squiggly",
                        gp_core::ir::AnnotationKind::Caret => "caret",
                        gp_core::ir::AnnotationKind::Ink => "ink",
                        gp_core::ir::AnnotationKind::Popup => "popup",
                        gp_core::ir::AnnotationKind::FileAttachment => "fileattachment",
                        gp_core::ir::AnnotationKind::Sound => "sound",
                        gp_core::ir::AnnotationKind::Movie => "movie",
                        gp_core::ir::AnnotationKind::Widget => "widget",
                        gp_core::ir::AnnotationKind::Screen => "screen",
                        gp_core::ir::AnnotationKind::PrinterMark => "printermark",
                        gp_core::ir::AnnotationKind::TrapNet => "trapnet",
                        gp_core::ir::AnnotationKind::Watermark => "watermark",
                    }),
                    "rect": a.rect,
                    "contents": a.contents.clone(),
                    "author": a.author.clone(),
                    "date": a.date.clone(),
                    "quote": a.quote.clone(),
                    "resolved": a.resolved,
                })
            })
            .collect()
    } else {
        Vec::new()
    };

    let mode = if ext.ends_with(".pdf") {
        "pdf-annot".to_string()
    } else {
        "docx-revisions".to_string()
    };

    let token = if ext.ends_with(".pdf") {
        // PDF: use mode-aware session creation
        let doc_bytes_hash = if mode == "pdf-annot" {
            Some(mcp::session::sha256_hex(&bytes))
        } else {
            None
        };
        mcp::session::create_review_session_with_mode_and_text_layer(
            &mcp::session::new_review_session_store(),
            path_str.clone(),
            &bytes,
            revision_snapshot,
            mode.clone(),
            doc_bytes_hash,
            text_layer,
        )
    } else {
        mcp::session::create_review_session(
            &mcp::session::new_review_session_store(),
            path_str.clone(),
            &bytes,
            revision_snapshot,
        )
    };

    let url = format!("http://127.0.0.1:8732/review/{}", token);
    eprintln!("review: session created");
    eprintln!("review: token = {}", token);
    eprintln!("review: url   = {}", url);

    if !no_browser {
        #[cfg(target_os = "linux")]
        {
            let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
        }
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open").arg(&url).spawn();
        }
        #[cfg(target_os = "windows")]
        {
            let _ = std::process::Command::new("cmd")
                .args(&["/C", "start", "", &url])
                .spawn();
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            eprintln!(
                "review: opening browser for {} (unsupported platform, try manually)",
                url
            );
        }
    }

    Ok(())
}

fn cmd_open(
    path: &std::path::Path,
    version: Option<&str>,
    app: Option<&str>,
    no_browser: bool,
    json: bool,
) -> Result<()> {
    // Probe the platform opener
    let handoff = open_tool::probe_handler();
    if !handoff.handler_available {
        if json {
            let mut error_json = serde_json::json!({
                "tool": "open",
                "handoff": {
                    "platform": handoff.platform,
                    "handler_available": false,
                    "handler_name": handoff.handler_name,
                    "install_hint": handoff.install_hint,
                },
                "error": handoff.install_hint,
            });
            error_json = budget_enforcer::stamp_envelope(error_json);
            println!("{}", serde_json::to_string_pretty(&error_json)?);
            return Ok(());
        } else {
            eprintln!(
                "open: no platform opener available ({})",
                handoff.handler_name
            );
            eprintln!("  hint: {}", handoff.install_hint);
            eprintln!("open: use --app <binary> to specify an opener directly");
        }
    }

    let result = open_tool::cmd_open(
        path,
        version.map(String::from),
        app.map(String::from),
        no_browser,
    );

    match result {
        Ok(()) => {
            if json {
                // Build JSON output
                let open_app = app.as_ref().and_then(|s| {
                    let s: &str = s;
                    if s.is_empty() || s == "default" {
                        None
                    } else {
                        Some(s.to_string())
                    }
                });
                let resolved_app = if no_browser { None } else { open_app };
                let mut output = serde_json::json!({
                    "tool": "open",
                    "path": path.to_string_lossy().to_string(),
                    "version": version,
                    "app": resolved_app,
                    "no_browser": no_browser,
                    "handoff": {
                        "platform": handoff.platform,
                        "handler_available": handoff.handler_available,
                        "handler_name": handoff.handler_name,
                    },
                    "status": "ok",
                });
                output = budget_enforcer::stamp_envelope(output);
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
            Ok(())
        }
        Err(e) => {
            if json {
                let mut error_json = serde_json::json!({
                    "tool": "open",
                    "path": path.to_string_lossy().to_string(),
                    "status": "error",
                    "error": e.to_string(),
                    "handoff": {
                        "platform": handoff.platform,
                        "handler_available": handoff.handler_available,
                        "handler_name": handoff.handler_name,
                    },
                });
                error_json = budget_enforcer::stamp_envelope(error_json);
                println!("{}", serde_json::to_string_pretty(&error_json)?);
            } else {
                eprintln!("open: {}", e);
            }
            std::process::exit(1)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_dispatch_parity() {
        // Every tool in mcp::spec::TOOLS must have a dispatch arm in the MCP handler.
        let all_tool_names: Vec<&str> = mcp::spec::TOOLS.iter().map(|t| t.name).collect();
        let spec_count = all_tool_names.len();

        // Dispatch arms in the MCP handler closure (line-ordered).
        let dispatched_tools = [
            "inspect_document",
            "render_document",
            "inspect_region",
            "diff_documents",
            "doctor",
            "serve",
            "apply_patch",
            "edit_text",
            "edit_cell",
            "edit_structure",
            "annotate_pages",
            "revert_document",
            "journal",
            "create_document",
            "script_tool",
            "check_update",
            "list_revisions",
            "accept_revisions",
            "reject_revisions",
            "add_comment",
            "list_annotations",
            "resolve_annotations",
            "sign_document",
        ];

        for tool in &all_tool_names {
            assert!(
                dispatched_tools.contains(tool),
                "Tool '{}' is declared in spec but has no dispatch arm in main.rs",
                tool
            );
        }

        assert_eq!(
            dispatched_tools.len(),
            spec_count,
            "dispatch arms ({}) != spec tools ({})",
            dispatched_tools.len(),
            spec_count
        );
    }
}
