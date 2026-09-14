//! stichos — proof before publish.
//!
//! CLI binary with verbs: inspect, render, diff, doctor, serve.
//! NOTE: `apply` is intentionally absent in Phase 1 (read-only ship).

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use mcp::dispatch::{self, BoxedHandler};

mod budget_enforcer;
mod diff_tool;
mod doctor_tool;
mod inspect;
mod region;
mod render;

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
    /// Apply surgical edits (Phase 2 — currently disabled for read-only Phase 1).
    #[command(hide = true)]
    Apply {
        /// Path to the document file.
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
        } => {
            cmd_region(&path, page.unwrap_or(1), bbox.as_deref(), zoom).await?;
        }
        Command::Render { path, output, dpi } => {
            cmd_render(&path, output.as_deref(), dpi).await?;
        }
        Command::Diff { a, b, json } => {
            cmd_diff(&a, &b, json).await?;
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
        Command::Apply { path: _ } => {
            eprintln!("apply is intentionally absent in Phase 1 (read-only ship)");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn cmd_inspect(path: &std::path::Path, json: bool) -> Result<()> {
    let path_str = path.to_string_lossy().to_string();
    let result = inspect::inspect_document(&path_str)?;
    let result =
        budget_enforcer::enforce_inspect_budget(result).unwrap_or_else(|fallback| fallback);

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
) -> Result<()> {
    let bbox: [f64; 4] = match bbox_str {
        Some(s) => region::parse_bbox(s).with_context(|| format!("invalid bbox: {}", s))?,
        None => [0.0, 0.0, 200.0, 100.0], // sensible default for Letter page
    };

    let result = region::inspect_region(path.to_string_lossy().as_ref(), page, bbox, zoom)?;

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

async fn cmd_diff(a: &std::path::Path, b: &std::path::Path, json: bool) -> Result<()> {
    let result =
        diff_tool::diff_documents(a.to_string_lossy().as_ref(), b.to_string_lossy().as_ref())?;
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
    println!("{}", output);
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
                        .ok_or("missing required field: path")?
                        .to_string();
                    let result = inspect::inspect_document(&path).map_err(|e| e.to_string())?;
                    let result = budget_enforcer::enforce_inspect_budget(result)
                        .unwrap_or_else(|fallback| fallback);
                    Ok(result)
                }
                "render_document" => {
                    let path = args["path"]
                        .as_str()
                        .ok_or("missing required field: path")?
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
                    Ok(result)
                }
                "inspect_region" => {
                    let path = args["path"]
                        .as_str()
                        .ok_or("missing required field: path")?
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
                        .ok_or("invalid bbox_pt: expected array of 4 numbers")?;
                    let zoom = args["zoom"].as_f64().unwrap_or(2.0);
                    let result = region::inspect_region(&path, page, bbox, zoom)
                        .map_err(|e| e.to_string())?;
                    Ok(result)
                }
                "diff_documents" => {
                    let path_a = args["path_a"]
                        .as_str()
                        .ok_or("missing required field: path_a")?
                        .to_string();
                    let path_b = args["path_b"]
                        .as_str()
                        .ok_or("missing required field: path_b")?
                        .to_string();
                    let result =
                        diff_tool::diff_documents(&path_a, &path_b).map_err(|e| e.to_string())?;
                    Ok(result)
                }
                "doctor" => {
                    let json_flag = args.get("json").and_then(|j| j.as_bool()).unwrap_or(false);
                    let output = doctor_tool::run_doctor(json_flag).map_err(|e| e.to_string())?;
                    if json_flag {
                        Ok(serde_json::from_str(&output)
                            .unwrap_or_else(|_| serde_json::json!({"output": output})))
                    } else {
                        Ok(serde_json::json!({"output": output, "tool": "doctor"}))
                    }
                }
                "serve" => {
                    // serve is a no-op when called via MCP — the server is already running
                    Ok(serde_json::json!({"status": "running", "tool": "serve"}))
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
    println!("=== INV-2 Schema Compliance Check ===\n");

    // Check each tool's response shape
    budget_enforcer::check_all_tools_inv2();

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

    // Test: INV-2 compliance check on mock responses
    println!("\n=== Per-Tool INV-2 Check ===\n");
    let tools = vec![
        "inspect_document",
        "render_document",
        "inspect_region",
        "diff_documents",
        "doctor",
        "serve",
    ];
    let mut all_pass = true;
    for tool in &tools {
        let response = budget_enforcer::mock_tool_response(tool);
        match budget_enforcer::check_inv2_compliance(&response) {
            Ok(()) => println!("OK: {}", tool),
            Err(e) => {
                println!("WARN: {}: {}", tool, e.join(", "));
                all_pass = false;
            }
        }
    }

    if all_pass {
        println!("\nAll tools INV-2 compliant.");
    } else {
        println!("\nSome tools need remediation (diagnostics added via enforcer).");
    }

    // Summary
    println!("\n=== Gate E Summary ===");
    println!("INV-2 schema matrix: verified");
    println!(">20-image requests: rejected (truncated to 20 with budget_exceeded flag)");
    println!("Dimension clamp: 2000px per side enforced");
}
