// SPDX-License-Identifier: MIT OR Apache-2.0
//! engines: pandoc runner for office→markdown text extraction and JSON AST passthrough.
//!
//! Spawns pandoc as a subprocess for text extraction, format conversion,
//! and JSON AST export/import. Pandoc is a soft dependency probed at runtime.
//!
//! # Design decisions
//!
//! ## JSON AST surface API (additive)
//!
//! `PandocEngine::to_json()` and `PandocEngine::from_json()` expose raw
//! pandoc JSON as a "surface API" — consumers can round-trip any supported
//! format through pandoc's canonical AST. This is additive-only (C7 precedent)
//! and does not change existing public API signatures.
//!
//! ## IR bridge (`pandoc_to_ir`)
//!
//! The `pandoc_to_ir` module converts a deserialised pandoc AST into a
//! gp-core `IRDocument` for fidelity inspection. It is a separate module
//! that consumers can use independently of the JSON surface API.
//!
//! Mapping highlights:
//!
//! | Pandoc block       | IR element              |
//! |--------------------|-------------------------|
//! | `Header`           | `SectionItem::Heading`  |
//! | `Para`             | `SectionItem::Paragraph`|
//! | `BulletList`       | `SectionItem::List`     |
//! | `OrderedList`      | `SectionItem::List`     |
//! | `CodeBlock`        | `SectionItem::Paragraph` (style=CodeBlock) |
//! | `Table`            | `SectionItem::Table`    |
//! | `HorizontalRule`   | `SectionItem::Paragraph` (style=HorizontalRule) |
//! | `BlockQuote`       | `SectionItem::Paragraph` (style=BlockQuote) |
//!
//! Inline formatting is captured in `Run` fields (bold, italic, code).
//!
//! # Future work
//!
//! This bridge captures essential document structure. Future iterations may
//! add more sophisticated mappings (table headers, list nesting, images,
//! citations) as needed by downstream consumers.

use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use gp_core::diagnostics::Diagnostic;
use gp_core::diagnostics::Severity;
use gp_core::traits::CapabilityEntry;
use gp_core::traits::Engine;
use gp_core::traits::EngineExt;
use gp_core::traits::EngineInfo;

const PANDOC_TIMEOUT_SECS: u64 = 30;

/// Common pandoc binary paths to probe.
fn common_pandoc_paths() -> Vec<&'static str> {
    vec![
        "/usr/bin/pandoc",
        "/usr/local/bin/pandoc",
        "/opt/homebrew/bin/pandoc",
        "/usr/local/bin/pandoc",
    ]
}

/// Find the first existing pandoc binary.
fn find_pandoc_binary() -> Option<String> {
    // Check environment variable first
    if let Ok(p) = std::env::var("PANDOC_PATH") {
        if Path::new(&p).exists() {
            return Some(p);
        }
    }
    // Check PATH
    if let Ok(paths) = std::env::var("PATH") {
        for dir in paths.split(':') {
            let p = std::path::Path::new(dir).join("pandoc");
            if p.exists() {
                return Some(p.to_string_lossy().to_string());
            }
        }
    }
    // Check common paths
    common_pandoc_paths()
        .into_iter()
        .find(|p| Path::new(p).exists())
        .map(String::from)
}

/// Parse pandoc version from `--version` output.
///
/// E.g. `"pandoc 3.1.2"` → `"3.1.2"`
pub fn parse_pandoc_version(output: &str) -> Option<String> {
    output
        .lines()
        .find(|l| l.starts_with("pandoc "))
        .and_then(|l| {
            l.strip_prefix("pandoc ")
                .and_then(|v| v.split_whitespace().next())
                .map(|v| v.to_string())
        })
}

/// Read all bytes from a reader in a separate thread.
fn read_all_async<R: Read + Send + 'static>(mut reader: R) -> std::thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf);
        buf
    })
}

/// Check if pandoc is available via PATH or PANDOC_PATH env var.
pub fn is_available() -> bool {
    find_pandoc_binary().is_some()
}

/// Helper: check if a binary exists on PATH.
fn which_binary(name: &str) -> Option<String> {
    if let Ok(paths) = std::env::var("PATH") {
        for dir in paths.split(':') {
            let p = std::path::Path::new(dir).join(name);
            if p.exists() {
                return Some(p.to_string_lossy().to_string());
            }
        }
    }
    None
}

/// Check if any LaTeX engine suitable for pandoc PDF output is available.
pub fn is_latex_available() -> bool {
    which_binary("pdflatex").is_some()
        || which_binary("xelatex").is_some()
        || which_binary("lualatex").is_some()
}

/// Run a subprocess and capture stdout/stderr with hard timeout.
///
/// Returns (exit_code, stdout_bytes, stderr_bytes) on success.
/// Returns Err(diagnostics) on timeout or spawn failure.
fn run_subprocess(cmd: &mut Command) -> Result<(i32, Vec<u8>, Vec<u8>), Vec<Diagnostic>> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to spawn pandoc: {}", e),
            )]
        })?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let stdout_handle = read_all_async(stdout);
    let stderr_handle = read_all_async(stderr);

    let deadline = std::time::Instant::now() + Duration::from_secs(PANDOC_TIMEOUT_SECS);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let exit_code = status.code().unwrap_or(-1);
                let stdout_bytes = stdout_handle.join().unwrap_or_default();
                let stderr_bytes = stderr_handle.join().unwrap_or_default();
                return Ok((exit_code, stdout_bytes, stderr_bytes));
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_handle.join().unwrap_or_default();
                    let _ = stderr_handle.join().unwrap_or_default();
                    return Err(vec![Diagnostic::new(
                        "ENG-E-002",
                        Severity::Error,
                        format!("pandoc timeout after {}ms", PANDOC_TIMEOUT_SECS * 1000),
                    )]);
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = stdout_handle.join().unwrap_or_default();
                let _ = stderr_handle.join().unwrap_or_default();
                return Err(vec![Diagnostic::new(
                    "ENG-E-001",
                    Severity::Error,
                    "failed to wait for pandoc",
                )]);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Pandoc engine — spawns pandoc subprocesses for text extraction.
pub struct PandocEngine {
    path: String,
    version_cache: OnceLock<Option<String>>,
}

impl std::fmt::Debug for PandocEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PandocEngine")
            .field("path", &self.path)
            .field("version", &self.version())
            .finish()
    }
}

impl PandocEngine {
    /// Create a new PandocEngine for the given pandoc binary.
    pub fn new(path: &str) -> Result<Self, Vec<Diagnostic>> {
        if !Path::new(path).exists() {
            return Err(vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("pandoc not found: {}", path),
            )]);
        }
        Ok(Self {
            path: path.to_string(),
            version_cache: OnceLock::new(),
        })
    }

    /// Create a new engine by auto-detecting the pandoc binary.
    pub fn new_auto() -> Result<Self, Vec<Diagnostic>> {
        let path = find_pandoc_binary().ok_or_else(|| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                "pandoc not found in PATH or common paths",
            )]
        })?;
        Self::new(&path)
    }

    /// Get version by running `pandoc --version`.
    fn version_inner(&self) -> Option<String> {
        let mut cmd = Command::new(&self.path);
        cmd.arg("--version");
        match run_subprocess(&mut cmd) {
            Ok((exit_code, stdout, _stderr)) => {
                if exit_code == 0 {
                    let output = String::from_utf8_lossy(&stdout);
                    parse_pandoc_version(&output)
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    }

    /// Extract text from a document using pandoc.
    ///
    /// Runs: `pandoc --from=<source_format> --to=markdown-strict -`
    /// Reads document bytes from stdin, returns markdown text from stdout.
    pub fn extract_text(input: &[u8], source_format: &str) -> Result<String, Vec<Diagnostic>> {
        let pandoc_path = std::env::var("PANDOC_PATH")
            .ok()
            .filter(|p| Path::new(p).exists())
            .unwrap_or_else(|| find_pandoc_binary().unwrap_or_else(|| "pandoc".to_string()));

        let mut cmd = Command::new(&pandoc_path);
        cmd.arg("--from")
            .arg(source_format)
            .arg("--to")
            .arg("markdown")
            .arg("-");

        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                vec![Diagnostic::new(
                    "ENG-E-001",
                    Severity::Error,
                    format!("failed to spawn pandoc: {}", e),
                )]
            })?;

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // Write input in a separate thread (owns a clone of input)
        let input_bytes = input.to_vec();
        let stdin_handle = thread::spawn(move || {
            let _ = stdin.write_all(&input_bytes);
        });

        let stdout_handle = read_all_async(stdout);
        let stderr_handle = read_all_async(stderr);

        let deadline = std::time::Instant::now() + Duration::from_secs(PANDOC_TIMEOUT_SECS);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let exit_code = status.code().unwrap_or(-1);
                    let _ = stdin_handle.join();
                    let stdout_bytes = stdout_handle.join().unwrap_or_default();
                    let stderr_bytes = stderr_handle.join().unwrap_or_default();

                    if exit_code != 0 {
                        let msg = String::from_utf8_lossy(&stderr_bytes);
                        return Err(vec![Diagnostic::new(
                            "ENG-E-001",
                            Severity::Error,
                            format!("pandoc exited with code {}: {}", exit_code, msg.trim()),
                        )]);
                    }

                    return String::from_utf8(stdout_bytes).map_err(|_| {
                        vec![Diagnostic::new(
                            "ENG-E-001",
                            Severity::Error,
                            "pandoc output is not valid UTF-8",
                        )]
                    });
                }
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = stdin_handle.join();
                        let _ = stdout_handle.join().unwrap_or_default();
                        let _ = stderr_handle.join().unwrap_or_default();
                        return Err(vec![Diagnostic::new(
                            "ENG-E-002",
                            Severity::Error,
                            format!("pandoc timeout after {}ms", PANDOC_TIMEOUT_SECS * 1000),
                        )]);
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err(_) => {
                    let _ = child.kill();
                    let _ = stdin_handle.join();
                    let _ = stdout_handle.join().unwrap_or_default();
                    let _ = stderr_handle.join().unwrap_or_default();
                    return Err(vec![Diagnostic::new(
                        "ENG-E-001",
                        Severity::Error,
                        "failed to wait for pandoc",
                    )]);
                }
            }
        }
    }

    /// Get the pandoc binary path.
    pub fn path(&self) -> &str {
        &self.path
    }

    // ------------------------------------------------------------------
    // Creation path (P3-19: pandoc creation — write direction)
    // ------------------------------------------------------------------

    /// Convert content to a target format using pandoc (write-direction).
    ///
    /// Runs: `pandoc --from=<source_format> --to=<target_format> [--reference-doc=TMPFILE] -o OUTPUT -`
    /// Reads content from stdin (`-`), writes to a temp output file.
    ///
    /// # Arguments
    /// * `input` — raw input bytes (markdown, html, etc.)
    /// * `source_format` — pandoc source format (e.g. "markdown", "html", "typ")
    /// * `target_format` — pandoc target format (e.g. "docx", "odt", "pdf")
    /// * `reference_doc` — optional reference doc / branded template bytes
    ///
    /// # Returns
    /// The converted document bytes on success.
    ///
    /// # Errors
    /// Returns `Err(Vec<Diagnostic>)` on pandoc failure, timeout, or I/O error.
    pub fn convert_to(
        input: &[u8],
        source_format: &str,
        target_format: &str,
        reference_doc: Option<&[u8]>,
    ) -> Result<Vec<u8>, Vec<Diagnostic>> {
        let pandoc_path = std::env::var("PANDOC_PATH")
            .ok()
            .filter(|p| Path::new(p).exists())
            .unwrap_or_else(|| find_pandoc_binary().unwrap_or_else(|| "pandoc".to_string()));

        // Create a temporary working directory
        let tmp_dir = tempfile::tempdir().map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to create temp dir: {}", e),
            )]
        })?;

        // Build output path
        let output_path = tmp_dir.path().join("output").with_extension(target_format);

        // If a reference document is provided, write it to a temp file
        let mut ref_path: Option<std::path::PathBuf> = None;
        if let Some(ref_doc) = reference_doc {
            let ref_ext = match target_format {
                "docx" => "docx",
                "odt" => "odt",
                _ => "pdf",
            };
            let p = tmp_dir.path().join("reference").with_extension(ref_ext);
            std::fs::write(&p, ref_doc).map_err(|e| {
                vec![Diagnostic::new(
                    "ENG-E-001",
                    Severity::Error,
                    format!("failed to write reference doc: {}", e),
                )]
            })?;
            ref_path = Some(p);
        }

        // Build pandoc command: pandoc --from=SF --to=TF [--reference-doc=RP] -o OP -
        let mut cmd = Command::new(&pandoc_path);
        cmd.arg("--from")
            .arg(source_format)
            .arg("--to")
            .arg(target_format);

        if let Some(ref p) = ref_path {
            cmd.arg("--reference-doc").arg(p);
        }

        cmd.arg("-o").arg(&output_path).arg("-");

        Self::run_convert_subprocess(cmd, &pandoc_path, input, &output_path)
    }

    /// Internal helper: run the convert subprocess and read output.
    fn run_convert_subprocess(
        mut cmd: Command,
        pandoc_path: &str,
        input: &[u8],
        output_path: &std::path::Path,
    ) -> Result<Vec<u8>, Vec<Diagnostic>> {
        let _ = pandoc_path; // used at call site; kept for API consistency
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                vec![Diagnostic::new(
                    "ENG-E-001",
                    Severity::Error,
                    format!("failed to spawn pandoc: {}", e),
                )]
            })?;

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // Write input in a separate thread
        let input_bytes = input.to_vec();
        let stdin_handle = thread::spawn(move || {
            let _ = stdin.write_all(&input_bytes);
        });

        let stdout_handle = read_all_async(stdout);
        let stderr_handle = read_all_async(stderr);

        let deadline = std::time::Instant::now() + Duration::from_secs(PANDOC_TIMEOUT_SECS);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let exit_code = status.code().unwrap_or(-1);
                    let _ = stdin_handle.join();
                    let _ = stdout_handle.join().unwrap_or_default();
                    let stderr_bytes = stderr_handle.join().unwrap_or_default();

                    if exit_code != 0 {
                        let msg = String::from_utf8_lossy(&stderr_bytes);
                        return Err(vec![Diagnostic::new(
                            "ENG-E-001",
                            Severity::Error,
                            format!("pandoc exited with code {}: {}", exit_code, msg.trim()),
                        )]);
                    }

                    // Read output file
                    return std::fs::read(output_path).map_err(|e| {
                        vec![Diagnostic::new(
                            "ENG-E-001",
                            Severity::Error,
                            format!("failed to read pandoc output: {}", e),
                        )]
                    });
                }
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = stdin_handle.join();
                        let _ = stdout_handle.join().unwrap_or_default();
                        let _ = stderr_handle.join().unwrap_or_default();
                        return Err(vec![Diagnostic::new(
                            "ENG-E-002",
                            Severity::Error,
                            format!("pandoc timeout after {}ms", PANDOC_TIMEOUT_SECS * 1000),
                        )]);
                    }
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }
                Err(_) => {
                    let _ = child.kill();
                    let _ = stdin_handle.join();
                    let _ = stdout_handle.join().unwrap_or_default();
                    let _ = stderr_handle.join().unwrap_or_default();
                    return Err(vec![Diagnostic::new(
                        "ENG-E-001",
                        Severity::Error,
                        "failed to wait for pandoc",
                    )]);
                }
            }
        }
    }

    /// Convert any supported source to pandoc JSON AST.
    ///
    /// Runs: `pandoc --from=<source_format> --to=json -`
    /// Reads document bytes from stdin, returns JSON AST string from stdout.
    pub fn to_json(input: &[u8], source_format: &str) -> Result<String, Vec<Diagnostic>> {
        let pandoc_path = std::env::var("PANDOC_PATH")
            .ok()
            .filter(|p| Path::new(p).exists())
            .unwrap_or_else(|| find_pandoc_binary().unwrap_or_else(|| "pandoc".to_string()));

        let mut cmd = Command::new(&pandoc_path);
        cmd.arg("--from")
            .arg(source_format)
            .arg("--to")
            .arg("json")
            .arg("-");

        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                vec![Diagnostic::new(
                    "ENG-E-001",
                    Severity::Error,
                    format!("failed to spawn pandoc: {}", e),
                )]
            })?;

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // Write input in a separate thread
        let input_bytes = input.to_vec();
        let stdin_handle = thread::spawn(move || {
            let _ = stdin.write_all(&input_bytes);
        });

        let stdout_handle = read_all_async(stdout);
        let stderr_handle = read_all_async(stderr);

        let deadline = std::time::Instant::now() + Duration::from_secs(PANDOC_TIMEOUT_SECS);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let exit_code = status.code().unwrap_or(-1);
                    let _ = stdin_handle.join();
                    let stdout_bytes = stdout_handle.join().unwrap_or_default();
                    let stderr_bytes = stderr_handle.join().unwrap_or_default();

                    if exit_code != 0 {
                        let msg = String::from_utf8_lossy(&stderr_bytes);
                        return Err(vec![Diagnostic::new(
                            "ENG-E-001",
                            Severity::Error,
                            format!("pandoc exited with code {}: {}", exit_code, msg.trim()),
                        )]);
                    }

                    return String::from_utf8(stdout_bytes).map_err(|_| {
                        vec![Diagnostic::new(
                            "ENG-E-001",
                            Severity::Error,
                            "pandoc JSON output is not valid UTF-8",
                        )]
                    });
                }
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = stdin_handle.join();
                        let _ = stdout_handle.join().unwrap_or_default();
                        let _ = stderr_handle.join().unwrap_or_default();
                        return Err(vec![Diagnostic::new(
                            "ENG-E-002",
                            Severity::Error,
                            format!("pandoc timeout after {}ms", PANDOC_TIMEOUT_SECS * 1000),
                        )]);
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err(_) => {
                    let _ = child.kill();
                    let _ = stdin_handle.join();
                    let _ = stdout_handle.join().unwrap_or_default();
                    let _ = stderr_handle.join().unwrap_or_default();
                    return Err(vec![Diagnostic::new(
                        "ENG-E-001",
                        Severity::Error,
                        "failed to wait for pandoc",
                    )]);
                }
            }
        }
    }

    /// Convert pandoc JSON AST to a target format.
    ///
    /// Runs: `pandoc --from=json --to=<target_format> -o OUTPUT -`
    /// Reads JSON AST from stdin, writes to a temp output file.
    pub fn from_json(json_input: &str, target_format: &str) -> Result<Vec<u8>, Vec<Diagnostic>> {
        let pandoc_path = std::env::var("PANDOC_PATH")
            .ok()
            .filter(|p| Path::new(p).exists())
            .unwrap_or_else(|| find_pandoc_binary().unwrap_or_else(|| "pandoc".to_string()));

        // Create a temporary working directory (same pattern as convert_to)
        let tmp_dir = tempfile::tempdir().map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to create temp dir: {}", e),
            )]
        })?;

        // Build output path
        let output_path = tmp_dir.path().join("output").with_extension(target_format);

        // Build command: pandoc --from=json --to=TF -o OP -
        let mut cmd = Command::new(&pandoc_path);
        cmd.arg("--from")
            .arg("json")
            .arg("--to")
            .arg(target_format)
            .arg("-o")
            .arg(&output_path)
            .arg("-");

        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                vec![Diagnostic::new(
                    "ENG-E-001",
                    Severity::Error,
                    format!("failed to spawn pandoc: {}", e),
                )]
            })?;

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // Write JSON AST in a separate thread
        let json_bytes = json_input.as_bytes().to_vec();
        let stdin_handle = thread::spawn(move || {
            let _ = stdin.write_all(&json_bytes);
        });

        let stdout_handle = read_all_async(stdout);
        let stderr_handle = read_all_async(stderr);

        let deadline = std::time::Instant::now() + Duration::from_secs(PANDOC_TIMEOUT_SECS);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let exit_code = status.code().unwrap_or(-1);
                    let _ = stdin_handle.join();
                    let _ = stdout_handle.join().unwrap_or_default();
                    let stderr_bytes = stderr_handle.join().unwrap_or_default();

                    if exit_code != 0 {
                        let msg = String::from_utf8_lossy(&stderr_bytes);
                        return Err(vec![Diagnostic::new(
                            "ENG-E-001",
                            Severity::Error,
                            format!("pandoc exited with code {}: {}", exit_code, msg.trim()),
                        )]);
                    }

                    // Read output file
                    return std::fs::read(&output_path).map_err(|e| {
                        vec![Diagnostic::new(
                            "ENG-E-001",
                            Severity::Error,
                            format!("failed to read pandoc output: {}", e),
                        )]
                    });
                }
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = stdin_handle.join();
                        let _ = stdout_handle.join().unwrap_or_default();
                        let _ = stderr_handle.join().unwrap_or_default();
                        return Err(vec![Diagnostic::new(
                            "ENG-E-002",
                            Severity::Error,
                            format!("pandoc timeout after {}ms", PANDOC_TIMEOUT_SECS * 1000),
                        )]);
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err(_) => {
                    let _ = child.kill();
                    let _ = stdin_handle.join();
                    let _ = stdout_handle.join().unwrap_or_default();
                    let _ = stderr_handle.join().unwrap_or_default();
                    return Err(vec![Diagnostic::new(
                        "ENG-E-001",
                        Severity::Error,
                        "failed to wait for pandoc",
                    )]);
                }
            }
        }
    }

    /// Get pandoc version, returning `Some(version_string)` or `None` if unavailable.
    pub fn get_version() -> Option<String> {
        let pandoc_path = std::env::var("PANDOC_PATH")
            .ok()
            .filter(|p| Path::new(p).exists())
            .unwrap_or_else(|| find_pandoc_binary().unwrap_or_else(|| "pandoc".to_string()));

        let mut cmd = Command::new(&pandoc_path);
        cmd.arg("--version");
        match run_subprocess(&mut cmd) {
            Ok((exit_code, stdout, _stderr)) => {
                if exit_code == 0 {
                    let output = String::from_utf8_lossy(&stdout);
                    parse_pandoc_version(&output)
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    }
}

impl Engine for PandocEngine {
    fn engine_id(&self) -> &str {
        "pandoc"
    }

    fn version(&self) -> Option<&str> {
        self.version_cache
            .get_or_init(|| self.version_inner())
            .as_deref()
    }

    fn can_convert(&self, source: &str, target: &str) -> bool {
        let supported_sources = ["docx", "odt", "ods", "doc", "rtf", "html", "epub"];
        supported_sources.contains(&source) && (target == "markdown" || target == "md")
    }

    fn convert(
        &self,
        input: &[u8],
        source: &str,
        target: &str,
    ) -> Result<Vec<u8>, Vec<Diagnostic>> {
        if !Path::new(&self.path).exists() {
            return Err(vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("pandoc not found: {}", self.path),
            )]);
        }

        let _fmt = match target {
            "markdown" | "md" => "markdown",
            _ => target,
        };

        let result = Self::extract_text(input, source)?;
        Ok(result.into_bytes())
    }

    fn is_available(&self) -> bool {
        Path::new(&self.path).exists()
    }
}

impl EngineExt for PandocEngine {
    fn fontset_hash(&self) -> Option<String> {
        None // pandoc doesn't expose fontset information
    }

    fn supported_conversions(&self) -> Vec<(String, String)> {
        vec![
            ("docx".into(), "markdown".into()),
            ("odt".into(), "markdown".into()),
            ("ods".into(), "markdown".into()),
            ("doc".into(), "markdown".into()),
            ("rtf".into(), "markdown".into()),
            ("html".into(), "markdown".into()),
            ("epub".into(), "markdown".into()),
        ]
    }

    fn info(&self) -> EngineInfo {
        let version = self.version().map(|v| v.to_string());
        let capabilities: Vec<CapabilityEntry> = self
            .supported_conversions()
            .into_iter()
            .map(|(source, target)| CapabilityEntry {
                source,
                target,
                works: true,
            })
            .collect();
        EngineInfo {
            engine_id: "pandoc".into(),
            version,
            fontset_hash: None,
            is_available: self.is_available(),
            capabilities,
        }
    }
}

impl Default for PandocEngine {
    fn default() -> Self {
        Self::new_auto().unwrap_or_else(|_| Self {
            path: "/usr/bin/pandoc".to_string(),
            version_cache: OnceLock::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pandoc_is_available() {
        // This test checks the is_available() function directly.
        // It will pass if pandoc is installed on the system.
        let result = is_available();
        // We don't assert here since pandoc may not be installed
        let _ = result;
    }

    #[test]
    fn test_is_latex_available() {
        // Verify is_latex_available returns a bool and checks the expected binaries
        let available = is_latex_available();
        let has_pdflatex = which_binary("pdflatex").is_some();
        let has_xelatex = which_binary("xelatex").is_some();
        let has_lualatex = which_binary("lualatex").is_some();
        assert_eq!(available, has_pdflatex || has_xelatex || has_lualatex);
    }

    #[test]
    fn test_which_binary_detects_known() {
        // pandoc is available on this system (required by test suite)
        let result = which_binary("pandoc");
        assert!(result.is_some(), "pandoc should be on PATH for tests");
    }

    #[test]
    fn test_which_binary_miss() {
        // A clearly nonexistent binary
        assert!(which_binary("stichos_test_nonexistent_binary_xyz").is_none());
    }

    #[test]
    fn test_is_latex_available_engine_detection() {
        // Verify is_latex_available correctly detects engine presence
        let pdflatex = which_binary("pdflatex").is_some();
        let xelatex = which_binary("xelatex").is_some();
        let lualatex = which_binary("lualatex").is_some();
        let expected = pdflatex || xelatex || lualatex;
        let actual = is_latex_available();
        assert_eq!(
            actual, expected,
            "is_latex_available()={:?} should match pdflatex={:?} || xelatex={:?} || lualatex={:?}",
            actual, pdflatex, xelatex, lualatex
        );
    }

    #[test]
    fn test_pandoc_extract_text_from_markdown() {
        if !is_available() {
            eprintln!("SKIP: pandoc not available");
            return;
        }
        // Test pandoc can handle markdown input (pass-through)
        let input = b"# Hello\n\nThis is a test paragraph.\n\n- Bullet 1\n- Bullet 2";
        let result = PandocEngine::extract_text(input, "markdown");
        assert!(
            result.is_ok(),
            "pandoc should handle markdown: {:?}",
            result
        );
        let text = result.unwrap();
        assert!(text.contains("Hello"), "output should contain 'Hello'");
        assert!(text.contains("Bullet"), "output should contain bullets");
    }

    #[test]
    fn test_pandoc_extract_text_rtf() {
        if !is_available() {
            eprintln!("SKIP: pandoc not available");
            return;
        }
        // Test pandoc can handle RTF input
        let rtf = r#"\rtf1\ansi{\fonttbl\f0\fswiss Arial;}
{\colortbl;\red0\green0\blue0;}
\viewkind4\uc1\pard\f0\fs20 Hello from RTF.\par
}\0"#;
        let result = PandocEngine::extract_text(rtf.as_bytes(), "rtf");
        assert!(result.is_ok(), "pandoc should handle RTF: {:?}", result);
        let text = result.unwrap();
        assert!(text.contains("Hello"), "RTF output should contain 'Hello'");
    }

    #[test]
    fn test_pandoc_extract_text_html() {
        if !is_available() {
            eprintln!("SKIP: pandoc not available");
            return;
        }
        // Test pandoc can handle HTML input
        let html = b"<html><body><h1>Test</h1><p>Content here</p></body></html>";
        let result = PandocEngine::extract_text(html, "html");
        assert!(result.is_ok(), "pandoc should handle HTML: {:?}", result);
        let text = result.unwrap();
        assert!(text.contains("Test"), "HTML output should contain 'Test'");
    }

    #[test]
    fn test_pandoc_version_parsing() {
        let result = parse_pandoc_version("pandoc 3.1.2");
        assert_eq!(result, Some("3.1.2".to_string()));
    }

    #[test]
    fn test_pandoc_version_parsing_with_extra() {
        let output = "pandoc 3.1.2\nCompiled with pandoc-types 1.24,+lfs";
        let result = parse_pandoc_version(output);
        assert_eq!(result, Some("3.1.2".to_string()));
    }

    #[test]
    fn test_pandoc_version_parsing_empty() {
        let result = parse_pandoc_version("");
        assert!(result.is_none());
    }

    #[test]
    fn test_pandoc_version_parsing_no_match() {
        let result = parse_pandoc_version("some random output");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_pandoc_version_multiline() {
        let output = "pandoc 2.17.1.1\nCompiled with pandoc-types 1.22,\ntexmath 0.12.3.1";
        let result = parse_pandoc_version(output);
        assert_eq!(result, Some("2.17.1.1".to_string()));
    }

    #[test]
    fn test_engine_id() {
        let engine = PandocEngine::new("/usr/bin/pandoc").unwrap_or_else(|_| PandocEngine {
            path: "/usr/bin/pandoc".to_string(),
            version_cache: OnceLock::new(),
        });
        assert_eq!(engine.engine_id(), "pandoc");
    }

    #[test]
    fn test_can_convert() {
        let engine = PandocEngine::new("/usr/bin/pandoc").unwrap_or_else(|_| PandocEngine {
            path: "/usr/bin/pandoc".to_string(),
            version_cache: OnceLock::new(),
        });
        assert!(engine.can_convert("docx", "markdown"));
        assert!(engine.can_convert("odt", "markdown"));
        assert!(engine.can_convert("html", "markdown"));
        assert!(!engine.can_convert("docx", "pdf"));
        assert!(!engine.can_convert("unknown", "markdown"));
    }

    #[test]
    fn test_can_convert_no_xlsx() {
        let engine = PandocEngine::new("/usr/bin/pandoc").unwrap_or_else(|_| PandocEngine {
            path: "/usr/bin/pandoc".to_string(),
            version_cache: OnceLock::new(),
        });
        assert!(!engine.can_convert("xlsx", "markdown"));
    }

    #[test]
    fn test_can_convert_no_pptx() {
        let engine = PandocEngine::new("/usr/bin/pandoc").unwrap_or_else(|_| PandocEngine {
            path: "/usr/bin/pandoc".to_string(),
            version_cache: OnceLock::new(),
        });
        assert!(!engine.can_convert("pptx", "markdown"));
    }

    #[test]
    fn test_can_convert_no_odp() {
        let engine = PandocEngine::new("/usr/bin/pandoc").unwrap_or_else(|_| PandocEngine {
            path: "/usr/bin/pandoc".to_string(),
            version_cache: OnceLock::new(),
        });
        assert!(!engine.can_convert("odp", "markdown"));
    }

    #[test]
    fn test_can_convert_still_allows_ods() {
        let engine = PandocEngine::new("/usr/bin/pandoc").unwrap_or_else(|_| PandocEngine {
            path: "/usr/bin/pandoc".to_string(),
            version_cache: OnceLock::new(),
        });
        assert!(engine.can_convert("ods", "markdown"));
    }

    #[test]
    fn test_info_capabilities_works_true() {
        let engine = PandocEngine::new("/usr/bin/pandoc").unwrap_or_else(|_| PandocEngine {
            path: "/usr/bin/pandoc".to_string(),
            version_cache: OnceLock::new(),
        });
        let info = engine.info();
        assert!(
            info.capabilities.iter().all(|c| c.works),
            "all capabilities should report works: true"
        );
    }

    #[test]
    fn test_supported_conversions() {
        let engine = PandocEngine::new("/usr/bin/pandoc").unwrap_or_else(|_| PandocEngine {
            path: "/usr/bin/pandoc".to_string(),
            version_cache: OnceLock::new(),
        });
        let conversions = engine.supported_conversions();
        assert!(!conversions.is_empty());
        assert!(conversions.contains(&("docx".to_string(), "markdown".to_string())));
        assert!(conversions.contains(&("html".to_string(), "markdown".to_string())));
    }

    #[test]
    fn test_engine_info() {
        let engine = PandocEngine::new("/usr/bin/pandoc").unwrap_or_else(|_| PandocEngine {
            path: "/usr/bin/pandoc".to_string(),
            version_cache: OnceLock::new(),
        });
        let info = engine.info();
        assert_eq!(info.engine_id, "pandoc");
        assert!(!info.capabilities.is_empty());
        assert!(info.capabilities.iter().all(|c| c.target == "markdown"));
    }

    #[test]
    fn test_find_pandoc_binary() {
        let result = find_pandoc_binary();
        // May or may not find pandoc; just verify it returns Option
        let _ = result;
    }

    #[test]
    fn test_is_available_instance() {
        let engine = PandocEngine::new("/usr/bin/pandoc").unwrap_or_else(|_| PandocEngine {
            path: "/usr/bin/pandoc".to_string(),
            version_cache: OnceLock::new(),
        });
        // is_available checks if the binary exists on disk
        let _ = engine.is_available();
    }

    #[test]
    fn test_engine_not_found_error() {
        let result = PandocEngine::new("/nonexistent/pandoc");
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "ENG-E-001");
    }

    #[test]
    fn test_engine_info_serializes() {
        let engine = PandocEngine::new("/usr/bin/pandoc").unwrap_or_else(|_| PandocEngine {
            path: "/usr/bin/pandoc".to_string(),
            version_cache: OnceLock::new(),
        });
        let info = engine.info();
        let json = serde_json::to_string(&info).unwrap();
        let back: EngineInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info.engine_id, back.engine_id);
    }

    #[test]
    fn test_capability_entry_serializes() {
        let entry = CapabilityEntry {
            source: "docx".into(),
            target: "markdown".into(),
            works: true,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: CapabilityEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, back);
    }

    #[test]
    fn test_extract_text_invalid_utf8_source() {
        // Non-UTF8 input bytes should be passed through to pandoc as binary
        // Pandoc may reject or handle it; we just verify the API doesn't panic
        let input = b"\xff\xfe\x00\x01";
        let result = PandocEngine::extract_text(input, "docx");
        // May fail with a diagnostic from pandoc; should not panic
        let _ = result;
    }

    #[test]
    fn test_extract_text_empty_input() {
        let result = PandocEngine::extract_text(b"", "markdown");
        // Empty input should produce empty output
        if let Ok(text) = result {
            assert!(
                text.is_empty() || text.trim().is_empty(),
                "empty input should yield empty output"
            );
        }
    }

    #[test]
    fn test_version_caching() {
        let engine = PandocEngine::new("/usr/bin/pandoc").unwrap_or_else(|_| PandocEngine {
            path: "/usr/bin/pandoc".to_string(),
            version_cache: OnceLock::new(),
        });
        let v1 = engine.version();
        let v2 = engine.version();
        assert_eq!(
            v1, v2,
            "version() should return the same value on repeated calls"
        );
    }

    #[test]
    fn test_version_caching_with_absent_engine() {
        let engine = PandocEngine::new("/nonexistent/pandoc").unwrap_or_else(|_| PandocEngine {
            path: "/nonexistent/pandoc".to_string(),
            version_cache: OnceLock::new(),
        });
        // version_inner() would spawn a subprocess and fail; version() via cache
        // should return None for an absent engine
        assert!(
            engine.version().is_none(),
            "version() should return None for absent pandoc binary"
        );
    }

    #[test]
    fn test_engine_info_uses_cached_version() {
        let engine = PandocEngine::new("/usr/bin/pandoc").unwrap_or_else(|_| PandocEngine {
            path: "/usr/bin/pandoc".to_string(),
            version_cache: OnceLock::new(),
        });
        let info = engine.info();
        let info_version = info.version;
        // After calling info(), the version cache should be populated
        // and subsequent version() calls should return the same value
        let cached_version = engine.version();
        assert_eq!(
            info_version.as_deref(),
            cached_version,
            "info().version should match cached version()"
        );
    }

    #[test]
    fn test_to_json_markdown() {
        if !is_available() {
            return;
        }
        let input = b"# Hello World\n\nThis is a test paragraph.";
        let json = PandocEngine::to_json(input, "markdown");
        assert!(json.is_ok(), "to_json should succeed: {:?}", json);
        let json_str = json.unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&json_str).expect("to_json output should be valid JSON");
        // Verify pandoc-api-version field exists
        assert!(
            parsed["pandoc-api-version"].is_array(),
            "JSON should have pandoc-api-version array"
        );
        // Verify blocks contains a Header
        let blocks = &parsed["blocks"];
        assert!(blocks.is_array(), "JSON should have blocks array");
        assert!(
            !blocks.as_array().unwrap().is_empty(),
            "blocks should not be empty"
        );
        let first_block = &blocks[0];
        assert_eq!(first_block["t"], "Header", "first block should be a Header");
        // Header inline content is an array of elements (Str, Space, Str, ...)
        let inline = &first_block["c"][2];
        let full_text: String = inline
            .as_array()
            .unwrap()
            .iter()
            .filter(|el| el["t"] == "Str")
            .map(|el| el["c"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            full_text, "Hello World",
            "header content should be 'Hello World'"
        );
    }

    #[test]
    fn test_to_json_html() {
        if !is_available() {
            return;
        }
        let input = b"<h1>Test</h1><p>HTML content</p>";
        let json = PandocEngine::to_json(input, "html");
        assert!(json.is_ok(), "to_json should handle HTML: {:?}", json);
        let json_str = json.unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&json_str).expect("to_json HTML output should be valid JSON");
        let blocks = &parsed["blocks"];
        let first_block = &blocks[0];
        assert_eq!(first_block["t"], "Header", "first block should be a Header");
    }

    #[test]
    fn test_from_json_to_markdown() {
        if !is_available() {
            return;
        }
        let json_input = r#"{"pandoc-api-version":[1,23,1,1],"meta":{},"blocks":[{"t":"Header","c":[1,["test-heading",[],[]],[{"t":"Str","c":"Test Heading"}]]},{"t":"Para","c":[{"t":"Str","c":"Hello World"}]}]}"#;
        let result = PandocEngine::from_json(json_input, "markdown");
        assert!(result.is_ok(), "from_json should succeed: {:?}", result);
        let bytes = result.unwrap();
        let text = String::from_utf8(bytes).expect("markdown output should be valid UTF-8");
        assert!(
            text.contains("Test Heading"),
            "markdown output should contain 'Test Heading', got: {}",
            text
        );
        assert!(
            text.contains("Hello World"),
            "markdown output should contain 'Hello World', got: {}",
            text
        );
    }

    #[test]
    fn test_from_json_to_html() {
        if !is_available() {
            return;
        }
        let json_input = r#"{"pandoc-api-version":[1,23,1,1],"meta":{},"blocks":[{"t":"Para","c":[{"t":"Str","c":"Hello World"}]}]}"#;
        let result = PandocEngine::from_json(json_input, "html");
        assert!(
            result.is_ok(),
            "from_json HTML should succeed: {:?}",
            result
        );
        let bytes = result.unwrap();
        let text = String::from_utf8(bytes).expect("HTML output should be valid UTF-8");
        assert!(
            text.contains("Hello World"),
            "HTML output should contain 'Hello World', got: {}",
            text
        );
    }

    #[test]
    fn test_json_roundtrip() {
        if !is_available() {
            return;
        }
        let input = b"# Round Trip\n\nThis text survives JSON conversion.";
        // markdown -> JSON AST
        let json_ast = PandocEngine::to_json(input, "markdown").expect("to_json should succeed");
        // JSON AST -> markdown
        let output =
            PandocEngine::from_json(&json_ast, "markdown").expect("from_json should succeed");
        let text = String::from_utf8(output).expect("roundtrip output should be valid UTF-8");
        assert!(
            text.contains("Round Trip"),
            "roundtrip should preserve 'Round Trip', got: {}",
            text
        );
        assert!(
            text.contains("This text survives"),
            "roundtrip should preserve body text, got: {}",
            text
        );
    }
}
