// SPDX-License-Identifier: MIT OR Apache-2.0
// engines: Chromium runner with headless PDF export and PNG screenshot.
//
// Spawns chromium-browser as a subprocess with --headless=new --print-to-pdf
// for PDF rendering and --headless=new --screenshot for PNG screenshots.
// Uses isolated --user-data-dir per call.

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

const CHROMIUM_TIMEOUT_SECS: u64 = 120;

// ---------------------------------------------------------------------------
// Binary discovery
// ---------------------------------------------------------------------------

/// Common Chromium/Chrome binary names to probe, in discovery order.
fn common_chromium_paths() -> Vec<&'static str> {
    vec![
        "chromium-browser",
        "chromium",
        "google-chrome",
        "google-chrome-stable",
    ]
}

/// Find the first existing chromium binary.
fn find_chromium_binary() -> Option<String> {
    // Check environment variable first
    if let Ok(p) = std::env::var("CHROMIUM_PATH") {
        if Path::new(&p).exists() {
            return Some(p);
        }
    }
    // Check PATH for each binary name
    if let Ok(paths) = std::env::var("PATH") {
        for dir in paths.split(':') {
            for bin in &common_chromium_paths() {
                let p = std::path::Path::new(dir).join(bin);
                if p.exists() {
                    return Some(p.to_string_lossy().to_string());
                }
            }
        }
    }
    // Check common paths
    common_chromium_paths()
        .into_iter()
        .find(|b| Path::new(b).exists())
        .map(String::from)
}

// ---------------------------------------------------------------------------
// Version parsing
// ---------------------------------------------------------------------------

/// Parse chromium version from `--version` output.
///
/// Handles formats like:
/// - `"Google Chrome 120.0.6099.130"` → `"120.0.6099.130"`
/// - `"Chromium 120.0.6099.130"` → `"120.0.6099.130"`
/// - `"HeadlessChrome 120.0.6099.130"` → `"120.0.6099.130"`
pub fn parse_chromium_version(output: &str) -> Option<String> {
    output.lines().next().and_then(|line| {
        // Check "Chromium" before "Chrome" since "Chromium" contains "Chrome"
        if let Some(pos) = line.find("Chromium") {
            let rest = &line[pos + 8..];
            let rest = rest.strip_prefix(' ')?;
            rest.split_whitespace().next().map(|v| v.to_string())
        } else if let Some(pos) = line.find("Chrome") {
            let rest = &line[pos + 6..];
            let rest = rest.strip_prefix('-').or(rest.strip_prefix(' '))?;
            rest.split_whitespace().next().map(|v| v.to_string())
        } else {
            None
        }
    })
}

// ---------------------------------------------------------------------------
// Subprocess runner
// ---------------------------------------------------------------------------

/// Read all bytes from a reader in a separate thread.
fn read_all_async<R: Read + Send + 'static>(mut reader: R) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf);
        buf
    })
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
                format!("failed to spawn chromium: {}", e),
            )]
        })?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let stdout_handle = read_all_async(stdout);
    let stderr_handle = read_all_async(stderr);

    let deadline = std::time::Instant::now() + Duration::from_secs(CHROMIUM_TIMEOUT_SECS);
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
                        format!("chromium timeout after {}s", CHROMIUM_TIMEOUT_SECS),
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
                    "failed to wait for chromium",
                )]);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Page margins in inches for chromium --print-to-pdf.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageMargins {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl Default for PageMargins {
    fn default() -> Self {
        Self {
            top: 0.5,
            right: 0.5,
            bottom: 0.5,
            left: 0.5,
        }
    }
}

/// Chromium engine — spawns chromium-browser subprocesses for HTML→PDF.
pub struct ChromiumEngine {
    pub path: String,
    pub temp_dir: String,
    version_cache: OnceLock<Option<String>>,
}

impl std::fmt::Debug for ChromiumEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChromiumEngine")
            .field("path", &self.path)
            .field("temp_dir", &self.temp_dir)
            .field("version", &self.version())
            .finish()
    }
}

impl ChromiumEngine {
    /// Create a new Chromium engine with the given binary path.
    ///
    /// Verifies the binary exists and creates a temp dir for
    /// the isolated `--user-data-dir`.
    pub fn new(path: &str) -> Result<Self, Vec<Diagnostic>> {
        if !Path::new(path).exists() {
            return Err(vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("chromium not found: {}", path),
            )]);
        }
        let temp_dir = std::env::temp_dir()
            .join(format!(
                "stichos-chromium-{:08x}",
                fastrand::u64(0..0xFFFFFFFF)
            ))
            .to_string_lossy()
            .to_string();
        std::fs::create_dir_all(&temp_dir).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to create temp dir: {}", e),
            )]
        })?;
        Ok(Self {
            path: path.to_string(),
            temp_dir,
            version_cache: OnceLock::new(),
        })
    }

    /// Create a new engine by auto-detecting the chromium binary.
    pub fn new_auto() -> Result<Self, Vec<Diagnostic>> {
        let path = find_chromium_binary().ok_or_else(|| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                "chromium not found in PATH or common paths",
            )]
        })?;
        Self::new(&path)
    }

    /// Get version by running `<chromium> --version`.
    pub(crate) fn version_inner(&self) -> Option<String> {
        let mut cmd = Command::new(&self.path);
        cmd.arg("--version");
        match run_subprocess(&mut cmd) {
            Ok((exit_code, stdout, _stderr)) => {
                if exit_code == 0 {
                    let output = String::from_utf8_lossy(&stdout);
                    parse_chromium_version(&output)
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    }

    /// Get the chromium binary path.
    pub fn path_ref(&self) -> &str {
        &self.path
    }
}

impl Engine for ChromiumEngine {
    fn engine_id(&self) -> &str {
        "chromium"
    }

    fn version(&self) -> Option<&str> {
        self.version_cache
            .get_or_init(|| self.version_inner())
            .as_deref()
    }

    fn can_convert(&self, source: &str, target: &str) -> bool {
        (source == "html" && target == "pdf") || (source == "html" && target == "png")
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
                format!("chromium not found: {}", self.path),
            )]);
        }

        // Validate source/target
        if source != "html" || (target != "pdf" && target != "png") {
            return Err(vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!(
                    "chromium only supports html->pdf and html->png conversions (got {}->{})",
                    source, target
                ),
            )]);
        }

        // Create isolated work directory
        let work_dir = std::env::temp_dir()
            .join(format!(
                "stichos-chromium-convert-{:08x}",
                fastrand::u64(0..0xFFFFFFFF)
            ))
            .to_string_lossy()
            .to_string();
        std::fs::create_dir_all(&work_dir).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to create work dir: {}", e),
            )]
        })?;

        // Write HTML input to temp file
        let html_path = Path::new(&work_dir).join("input.html");
        let mut html_file = std::fs::File::create(&html_path).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to create HTML file: {}", e),
            )]
        })?;
        html_file.write_all(input).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to write HTML file: {}", e),
            )]
        })?;

        // Create isolated user-data-dir
        let user_data_dir = work_dir.clone() + "-userData";
        std::fs::create_dir_all(&user_data_dir).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to create user-data-dir: {}", e),
            )]
        })?;

        // Build chromium command based on target format
        let html_path_str = html_path.to_string_lossy().to_string();
        let mut cmd = Command::new(&self.path);
        cmd.arg("--headless=new")
            .arg("--no-sandbox")
            .arg("--disable-gpu")
            .arg("--disable-dev-shm-usage")
            .arg(format!("--user-data-dir={}", user_data_dir))
            .arg(&html_path_str);

        let outcome = if target == "pdf" {
            let pdf_path = Path::new(&work_dir).join("output.pdf");
            let pdf_path_str = pdf_path.to_string_lossy().to_string();

            // --print-to-pdf=<path> outputs PDF to specified path
            cmd.arg(format!("--print-to-pdf={}", pdf_path_str))
                // Paper size and margins (reasonable defaults)
                .arg("--paper-size")
                .arg("letter")
                .arg("--margin-top")
                .arg("0.5")
                .arg("--margin-bottom")
                .arg("0.5")
                .arg("--margin-left")
                .arg("0.5")
                .arg("--margin-right")
                .arg("0.5")
                .arg("--background");

            let result = run_subprocess(&mut cmd);

            let read_output = || match std::fs::read(&pdf_path) {
                Ok(data) => Ok(data),
                Err(e) => Err(vec![Diagnostic::new(
                    "ENG-E-001",
                    Severity::Error,
                    format!("failed to read PDF output: {}", e),
                )]),
            };

            match result {
                Ok((exit_code, _stdout, stderr)) => {
                    if exit_code != 0 {
                        let msg = String::from_utf8_lossy(&stderr);
                        Err(vec![Diagnostic::new(
                            "ENG-E-001",
                            Severity::Error,
                            format!("chromium exited with code {}: {}", exit_code, msg.trim()),
                        )])
                    } else {
                        read_output()
                    }
                }
                Err(diags) => Err(diags),
            }
        } else {
            // target == "png"
            let png_path = Path::new(&work_dir).join("output.png");
            let png_path_str = png_path.to_string_lossy().to_string();

            // --screenshot=<path> outputs PNG to specified file path
            cmd.arg(format!("--screenshot={}", png_path_str));

            let result = run_subprocess(&mut cmd);

            let read_output = || match std::fs::read(&png_path) {
                Ok(data) => Ok(data),
                Err(e) => Err(vec![Diagnostic::new(
                    "ENG-E-001",
                    Severity::Error,
                    format!("failed to read PNG output: {}", e),
                )]),
            };

            match result {
                Ok((exit_code, _stdout, stderr)) => {
                    if exit_code != 0 {
                        let msg = String::from_utf8_lossy(&stderr);
                        Err(vec![Diagnostic::new(
                            "ENG-E-001",
                            Severity::Error,
                            format!("chromium exited with code {}: {}", exit_code, msg.trim()),
                        )])
                    } else {
                        read_output()
                    }
                }
                Err(diags) => Err(diags),
            }
        };

        // Clean up work directory
        let _ = std::fs::remove_dir_all(&work_dir);

        outcome
    }

    fn is_available(&self) -> bool {
        Path::new(&self.path).exists()
    }
}

/// Convert HTML to PDF with configurable paper size, margins, and background.
///
/// # Arguments
/// * `input` — HTML bytes to render
/// * `paper_size` — paper size name (e.g. "letter", "a4") or None for default
/// * `margins` — page margins in inches or None for defaults (0.5in each side)
/// * `background` — whether to render CSS backgrounds
///
/// # Returns
/// PDF bytes on success, or diagnostics on failure.
impl ChromiumEngine {
    pub fn create_pdf(
        &self,
        input: &[u8],
        paper_size: Option<&str>,
        margins: Option<PageMargins>,
        background: bool,
    ) -> Result<Vec<u8>, Vec<Diagnostic>> {
        if !Path::new(&self.path).exists() {
            return Err(vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("chromium not found: {}", self.path),
            )]);
        }

        // Create isolated work directory
        let work_dir = std::env::temp_dir()
            .join(format!(
                "stichos-chromium-create-{:08x}",
                fastrand::u64(0..0xFFFFFFFF)
            ))
            .to_string_lossy()
            .to_string();
        std::fs::create_dir_all(&work_dir).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to create work dir: {}", e),
            )]
        })?;

        // Write HTML input to temp file
        let html_path = Path::new(&work_dir).join("input.html");
        let mut html_file = std::fs::File::create(&html_path).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to create HTML file: {}", e),
            )]
        })?;
        html_file.write_all(input).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to write HTML file: {}", e),
            )]
        })?;

        // Create isolated user-data-dir
        let user_data_dir = work_dir.clone() + "-userData";
        std::fs::create_dir_all(&user_data_dir).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to create user-data-dir: {}", e),
            )]
        })?;

        // Build output PDF path
        let pdf_path = Path::new(&work_dir).join("output.pdf");
        let pdf_path_str = pdf_path.to_string_lossy().to_string();
        let html_path_str = html_path.to_string_lossy().to_string();

        // Build chromium command
        let mut cmd = Command::new(&self.path);
        cmd.arg("--headless=new")
            .arg(format!("--print-to-pdf={}", pdf_path_str))
            .arg("--no-sandbox")
            .arg("--disable-gpu")
            .arg("--disable-dev-shm-usage")
            .arg(format!("--user-data-dir={}", user_data_dir));

        // Paper size
        let ps = match paper_size {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => "letter".to_string(),
        };
        cmd.arg(format!("--paper-size={}", ps));

        // Margins
        let m = margins.unwrap_or_default();
        cmd.arg(format!("--margin-top={}", m.top))
            .arg(format!("--margin-bottom={}", m.bottom))
            .arg(format!("--margin-left={}", m.left))
            .arg(format!("--margin-right={}", m.right));

        // Background
        if background {
            cmd.arg("--background");
        }

        // Input file
        cmd.arg(&html_path_str);

        let result = run_subprocess(&mut cmd);

        let read_output = || match std::fs::read(&pdf_path) {
            Ok(data) => Ok(data),
            Err(e) => Err(vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to read PDF output: {}", e),
            )]),
        };

        let outcome = match result {
            Ok((exit_code, _stdout, stderr)) => {
                if exit_code != 0 {
                    let msg = String::from_utf8_lossy(&stderr);
                    Err(vec![Diagnostic::new(
                        "ENG-E-001",
                        Severity::Error,
                        format!("chromium exited with code {}: {}", exit_code, msg.trim()),
                    )])
                } else {
                    read_output()
                }
            }
            Err(diags) => Err(diags),
        };

        // Clean up work directory
        let _ = std::fs::remove_dir_all(&work_dir);

        outcome
    }

    /// Render HTML to a PNG screenshot.
    ///
    /// # Arguments
    /// * `input` — HTML bytes to render as a screenshot
    ///
    /// # Returns
    /// PNG bytes on success, or diagnostics on failure.
    pub fn screenshot(&self, input: &[u8]) -> Result<Vec<u8>, Vec<Diagnostic>> {
        if !Path::new(&self.path).exists() {
            return Err(vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("chromium not found: {}", self.path),
            )]);
        }

        // Create isolated work directory
        let work_dir = std::env::temp_dir()
            .join(format!(
                "stichos-chromium-screenshot-{:08x}",
                fastrand::u64(0..0xFFFFFFFF)
            ))
            .to_string_lossy()
            .to_string();
        std::fs::create_dir_all(&work_dir).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to create work dir: {}", e),
            )]
        })?;

        // Write HTML input to temp file
        let html_path = Path::new(&work_dir).join("input.html");
        let mut html_file = std::fs::File::create(&html_path).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to create HTML file: {}", e),
            )]
        })?;
        html_file.write_all(input).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to write HTML file: {}", e),
            )]
        })?;

        // Create isolated user-data-dir
        let user_data_dir = work_dir.clone() + "-userData";
        std::fs::create_dir_all(&user_data_dir).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to create user-data-dir: {}", e),
            )]
        })?;

        // Build output PNG path
        let png_path = Path::new(&work_dir).join("output.png");
        let png_path_str = png_path.to_string_lossy().to_string();
        let html_path_str = html_path.to_string_lossy().to_string();

        // Build chromium command:
        // --headless=new      : new headless mode (CH_ mode)
        // --screenshot=<path> : output PNG to specified file path
        // --user-data-dir=<dir>: isolated profile per invocation
        // --no-sandbox         : required when running as root
        // --disable-gpu        : avoid GPU issues in headless mode
        // --disable-dev-shm-usage: avoid /dev/shm limits
        let mut cmd = Command::new(&self.path);
        cmd.arg("--headless=new")
            .arg(format!("--screenshot={}", png_path_str))
            .arg("--no-sandbox")
            .arg("--disable-gpu")
            .arg("--disable-dev-shm-usage")
            .arg(format!("--user-data-dir={}", user_data_dir))
            .arg(&html_path_str);

        let result = run_subprocess(&mut cmd);

        let read_output = || match std::fs::read(&png_path) {
            Ok(data) => Ok(data),
            Err(e) => Err(vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to read PNG output: {}", e),
            )]),
        };

        let outcome = match result {
            Ok((exit_code, _stdout, stderr)) => {
                if exit_code != 0 {
                    let msg = String::from_utf8_lossy(&stderr);
                    Err(vec![Diagnostic::new(
                        "ENG-E-001",
                        Severity::Error,
                        format!("chromium exited with code {}: {}", exit_code, msg.trim()),
                    )])
                } else {
                    read_output()
                }
            }
            Err(diags) => Err(diags),
        };

        // Clean up work directory
        let _ = std::fs::remove_dir_all(&work_dir);

        outcome
    }
}

impl EngineExt for ChromiumEngine {
    fn fontset_hash(&self) -> Option<String> {
        None // chromium doesn't expose fontset information in the same way
    }

    fn supported_conversions(&self) -> Vec<(String, String)> {
        vec![("html".into(), "pdf".into()), ("html".into(), "png".into())]
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
            engine_id: "chromium".into(),
            version,
            fontset_hash: None,
            is_available: self.is_available(),
            capabilities,
        }
    }
}

impl Default for ChromiumEngine {
    fn default() -> Self {
        Self::new_auto().unwrap_or_else(|_| Self {
            path: "/usr/bin/chromium-browser".to_string(),
            temp_dir: std::env::temp_dir()
                .join("stichos-chromium-default")
                .to_string_lossy()
                .to_string(),
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
    fn test_chromium_engine_creation() {
        let result = ChromiumEngine::new("/usr/bin/chromium-browser");
        assert!(
            result.is_ok(),
            "engine creation should succeed for valid path"
        );
    }

    #[test]
    fn test_chromium_engine_invalid_path() {
        let result = ChromiumEngine::new("/nonexistent/chromium");
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "ENG-E-001");
    }

    #[test]
    fn test_parse_chromium_version_chrome() {
        let result = parse_chromium_version("Google Chrome 120.0.6099.130");
        assert_eq!(result, Some("120.0.6099.130".to_string()));
    }

    #[test]
    fn test_parse_chromium_version_chromium() {
        let result = parse_chromium_version("Chromium 120.0.6099.130");
        assert_eq!(result, Some("120.0.6099.130".to_string()));
    }

    #[test]
    fn test_parse_chromium_version_headless() {
        let result = parse_chromium_version("HeadlessChrome 120.0.6099.130");
        assert_eq!(result, Some("120.0.6099.130".to_string()));
    }

    #[test]
    fn test_parse_chromium_version_empty() {
        let result = parse_chromium_version("");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_chromium_version_no_match() {
        let result = parse_chromium_version("some random output");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_chromium_version_extra_info() {
        let result = parse_chromium_version("Google Chrome 120.0.6099.130 (Build: 3)");
        assert_eq!(result, Some("120.0.6099.130".to_string()));
    }

    #[test]
    fn test_parse_chromium_version_double_space() {
        let result = parse_chromium_version("Google Chrome  120.0.6099.130");
        assert_eq!(result, Some("120.0.6099.130".to_string()));
    }

    #[test]
    fn test_parse_chromium_version_name_only() {
        let result = parse_chromium_version("Google Chrome");
        assert!(result.is_none());
    }

    #[test]
    fn test_can_convert() {
        let engine =
            ChromiumEngine::new("/usr/bin/chromium-browser").unwrap_or_else(|_| ChromiumEngine {
                path: "/usr/bin/chromium-browser".to_string(),
                temp_dir: std::env::temp_dir()
                    .join("stichos-chromium-test")
                    .to_string_lossy()
                    .to_string(),
                version_cache: OnceLock::new(),
            });
        assert!(engine.can_convert("html", "pdf"));
        assert!(engine.can_convert("html", "png"));
        assert!(!engine.can_convert("docx", "pdf"));
        assert!(!engine.can_convert("xlsx", "pdf"));
        assert!(!engine.can_convert("html", "docx"));
        assert!(!engine.can_convert("unknown", "pdf"));
    }

    #[test]
    fn test_can_convert_html_to_png() {
        let engine =
            ChromiumEngine::new("/usr/bin/chromium-browser").unwrap_or_else(|_| ChromiumEngine {
                path: "/usr/bin/chromium-browser".to_string(),
                temp_dir: std::env::temp_dir()
                    .join("stichos-chromium-test")
                    .to_string_lossy()
                    .to_string(),
                version_cache: OnceLock::new(),
            });
        assert!(engine.can_convert("html", "png"));
    }

    #[test]
    fn test_supported_conversions() {
        let engine =
            ChromiumEngine::new("/usr/bin/chromium-browser").unwrap_or_else(|_| ChromiumEngine {
                path: "/usr/bin/chromium-browser".to_string(),
                temp_dir: std::env::temp_dir()
                    .join("stichos-chromium-test")
                    .to_string_lossy()
                    .to_string(),
                version_cache: OnceLock::new(),
            });
        let conversions = engine.supported_conversions();
        assert_eq!(conversions.len(), 2);
        assert!(conversions.contains(&("html".to_string(), "pdf".to_string())));
        assert!(conversions.contains(&("html".to_string(), "png".to_string())));
    }

    #[test]
    fn test_supported_conversions_includes_png() {
        let engine =
            ChromiumEngine::new("/usr/bin/chromium-browser").unwrap_or_else(|_| ChromiumEngine {
                path: "/usr/bin/chromium-browser".to_string(),
                temp_dir: std::env::temp_dir()
                    .join("stichos-chromium-test")
                    .to_string_lossy()
                    .to_string(),
                version_cache: OnceLock::new(),
            });
        let conversions = engine.supported_conversions();
        assert!(
            conversions.iter().any(|(_, t)| t == "png"),
            "supported_conversions should include html->png"
        );
    }

    #[test]
    fn test_engine_id() {
        let engine =
            ChromiumEngine::new("/usr/bin/chromium-browser").unwrap_or_else(|_| ChromiumEngine {
                path: "/usr/bin/chromium-browser".to_string(),
                temp_dir: std::env::temp_dir()
                    .join("stichos-chromium-test")
                    .to_string_lossy()
                    .to_string(),
                version_cache: OnceLock::new(),
            });
        assert_eq!(engine.engine_id(), "chromium");
    }

    #[test]
    fn test_engine_info() {
        let engine =
            ChromiumEngine::new("/usr/bin/chromium-browser").unwrap_or_else(|_| ChromiumEngine {
                path: "/usr/bin/chromium-browser".to_string(),
                temp_dir: std::env::temp_dir()
                    .join("stichos-chromium-test")
                    .to_string_lossy()
                    .to_string(),
                version_cache: OnceLock::new(),
            });
        let info = engine.info();
        assert_eq!(info.engine_id, "chromium");
        assert_eq!(info.capabilities.len(), 2);
    }

    #[test]
    fn test_info_capabilities_works_true() {
        let engine =
            ChromiumEngine::new("/usr/bin/chromium-browser").unwrap_or_else(|_| ChromiumEngine {
                path: "/usr/bin/chromium-browser".to_string(),
                temp_dir: std::env::temp_dir()
                    .join("stichos-chromium-test")
                    .to_string_lossy()
                    .to_string(),
                version_cache: OnceLock::new(),
            });
        let info = engine.info();
        assert!(
            info.capabilities.iter().all(|c| c.works),
            "all chromium capabilities should report works: true"
        );
    }

    #[test]
    fn test_find_chromium_binary() {
        let result = find_chromium_binary();
        // May or may not find chromium; just verify it returns Option
        let _ = result;
    }

    #[test]
    fn test_convert_returns_diagnostics_when_chromium_absent() {
        let engine = ChromiumEngine {
            path: "/nonexistent/chromium".to_string(),
            temp_dir: std::env::temp_dir()
                .join("stichos-chromium-test")
                .to_string_lossy()
                .to_string(),
            version_cache: OnceLock::new(),
        };
        let input = b"<html><body>test</body></html>";
        let result = engine.convert(input, "html", "pdf");
        assert!(
            result.is_err(),
            "convert should fail when chromium is absent"
        );
        let diags = result.unwrap_err();
        assert_eq!(diags[0].code, "ENG-E-001");
    }

    #[test]
    fn test_default_engine() {
        let _engine = ChromiumEngine::default();
    }

    #[test]
    fn test_page_margins_defaults() {
        let margins = PageMargins::default();
        assert_eq!(margins.top, 0.5);
        assert_eq!(margins.right, 0.5);
        assert_eq!(margins.bottom, 0.5);
        assert_eq!(margins.left, 0.5);
    }

    #[test]
    fn test_page_margins_custom() {
        let margins = PageMargins {
            top: 1.0,
            right: 0.75,
            bottom: 1.0,
            left: 0.75,
        };
        assert_eq!(margins.top, 1.0);
        assert_eq!(margins.right, 0.75);
        assert_eq!(margins.bottom, 1.0);
        assert_eq!(margins.left, 0.75);
    }

    #[test]
    fn test_create_pdf_returns_diagnostics_when_chromium_absent() {
        let engine = ChromiumEngine {
            path: "/nonexistent/chromium".to_string(),
            temp_dir: std::env::temp_dir()
                .join("stichos-chromium-test")
                .to_string_lossy()
                .to_string(),
            version_cache: OnceLock::new(),
        };
        let input = b"<html><body><h1>Test</h1></body></html>";
        let result = engine.create_pdf(input, None, None, false);
        assert!(
            result.is_err(),
            "create_pdf should fail when chromium is absent"
        );
        let diags = result.unwrap_err();
        assert_eq!(diags[0].code, "ENG-E-001");
    }

    #[test]
    fn test_create_pdf_with_custom_paper_size() {
        let engine = ChromiumEngine {
            path: "/nonexistent/chromium".to_string(),
            temp_dir: std::env::temp_dir()
                .join("stichos-chromium-test")
                .to_string_lossy()
                .to_string(),
            version_cache: OnceLock::new(),
        };
        let input = b"<html><body><h1>A4 Test</h1></body></html>";
        // Should fail with ENG-E-001 (chromium not found) regardless of params
        let result = engine.create_pdf(input, Some("a4"), None, false);
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert_eq!(diags[0].code, "ENG-E-001");
    }

    #[test]
    fn test_create_pdf_with_custom_margins() {
        let engine = ChromiumEngine {
            path: "/nonexistent/chromium".to_string(),
            temp_dir: std::env::temp_dir()
                .join("stichos-chromium-test")
                .to_string_lossy()
                .to_string(),
            version_cache: OnceLock::new(),
        };
        let input = b"<html><body><h1>Margins Test</h1></body></html>";
        let margins = PageMargins {
            top: 1.0,
            right: 1.0,
            bottom: 1.0,
            left: 1.0,
        };
        let result = engine.create_pdf(input, None, Some(margins), false);
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert_eq!(diags[0].code, "ENG-E-001");
    }

    #[test]
    fn test_create_pdf_with_background() {
        let engine = ChromiumEngine {
            path: "/nonexistent/chromium".to_string(),
            temp_dir: std::env::temp_dir()
                .join("stichos-chromium-test")
                .to_string_lossy()
                .to_string(),
            version_cache: OnceLock::new(),
        };
        let input = b"<html><body><h1>Background Test</h1></body></html>";
        let result = engine.create_pdf(input, None, None, true);
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert_eq!(diags[0].code, "ENG-E-001");
    }

    #[test]
    fn test_screenshot_fails_when_chromium_absent() {
        let engine = ChromiumEngine {
            path: "/nonexistent/chromium".to_string(),
            temp_dir: std::env::temp_dir()
                .join("stichos-chromium-test")
                .to_string_lossy()
                .to_string(),
            version_cache: OnceLock::new(),
        };
        let input = b"<html><body><h1>Screenshot Test</h1></body></html>";
        let result = engine.screenshot(input);
        assert!(
            result.is_err(),
            "screenshot should fail when chromium is absent"
        );
        let diags = result.unwrap_err();
        assert_eq!(diags[0].code, "ENG-E-001");
    }

    #[test]
    fn test_chromium_screenshot_positive() {
        if ChromiumEngine::new_auto().is_err() {
            eprintln!("SKIP: chromium not available");
            return;
        }

        let engine = ChromiumEngine::new_auto().unwrap();
        let input = b"<html><body><h1>Screenshot Test</h1></body></html>";
        let result = engine.screenshot(input);
        assert!(result.is_ok(), "screenshot should succeed: {:?}", result);

        let bytes = result.unwrap();
        assert!(!bytes.is_empty(), "screenshot should not be empty");
        assert!(
            bytes.len() >= 4 && &bytes[0..4] == b"\x89PNG",
            "output should be PNG (magic: {:02x} {:02x} {:02x} {:02x})",
            bytes[0],
            bytes[1],
            bytes[2],
            bytes[3]
        );
    }

    #[test]
    fn test_version_caching() {
        let engine =
            ChromiumEngine::new("/usr/bin/chromium-browser").unwrap_or_else(|_| ChromiumEngine {
                path: "/usr/bin/chromium-browser".to_string(),
                temp_dir: std::env::temp_dir()
                    .join("stichos-chromium-test")
                    .to_string_lossy()
                    .to_string(),
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
        let engine =
            ChromiumEngine::new("/nonexistent/chromium").unwrap_or_else(|_| ChromiumEngine {
                path: "/nonexistent/chromium".to_string(),
                temp_dir: std::env::temp_dir()
                    .join("stichos-chromium-test")
                    .to_string_lossy()
                    .to_string(),
                version_cache: OnceLock::new(),
            });
        assert!(
            engine.version().is_none(),
            "version() should return None for absent chromium binary"
        );
    }

    #[test]
    fn test_engine_info_uses_cached_version() {
        let engine =
            ChromiumEngine::new("/usr/bin/chromium-browser").unwrap_or_else(|_| ChromiumEngine {
                path: "/usr/bin/chromium-browser".to_string(),
                temp_dir: std::env::temp_dir()
                    .join("stichos-chromium-test")
                    .to_string_lossy()
                    .to_string(),
                version_cache: OnceLock::new(),
            });
        let info = engine.info();
        let info_version = info.version;
        let cached_version = engine.version();
        assert_eq!(
            info_version.as_deref(),
            cached_version,
            "info().version should match cached version()"
        );
    }
}
