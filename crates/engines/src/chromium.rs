// engines: Chromium runner with headless PDF export.
//
// Spawns chromium-browser as a subprocess with --headless=new --print-to-pdf
// for PDF rendering. Uses isolated --user-data-dir per call.

use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;
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

/// Chromium engine — spawns chromium-browser subprocesses for HTML→PDF.
#[derive(Debug)]
pub struct ChromiumEngine {
    pub path: String,
    pub temp_dir: String,
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
    fn version_inner(&self) -> Option<String> {
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
        None // version computed lazily via version_inner()
    }

    fn can_convert(&self, source: &str, target: &str) -> bool {
        source == "html" && target == "pdf"
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
        if source != "html" || target != "pdf" {
            return Err(vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!(
                    "chromium only supports html->pdf conversion (got {}->{})",
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

        // Build output PDF path
        let pdf_path = Path::new(&work_dir).join("output.pdf");
        let pdf_path_str = pdf_path.to_string_lossy().to_string();
        let html_path_str = html_path.to_string_lossy().to_string();

        // Build chromium command:
        // --headless=new      : new headless mode (CH_ mode)
        // --print-to-pdf=<path>: output PDF to specified path
        // --user-data-dir=<dir>: isolated profile per invocation
        // --no-sandbox         : required when running as root
        // --disable-gpu        : avoid GPU issues in headless mode
        // --disable-dev-shm-usage: avoid /dev/shm limits
        let mut cmd = Command::new(&self.path);
        cmd.arg("--headless=new")
            .arg("--print-to-pdf")
            .arg(&pdf_path_str)
            .arg("--no-sandbox")
            .arg("--disable-gpu")
            .arg("--disable-dev-shm-usage")
            .arg("--user-data-dir")
            .arg(&user_data_dir)
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
            .arg("--background")
            .arg(&html_path_str);

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

    fn is_available(&self) -> bool {
        Path::new(&self.path).exists()
    }
}

impl EngineExt for ChromiumEngine {
    fn fontset_hash(&self) -> Option<String> {
        None // chromium doesn't expose fontset information in the same way
    }

    fn supported_conversions(&self) -> Vec<(String, String)> {
        vec![("html".into(), "pdf".into())]
    }

    fn info(&self) -> EngineInfo {
        let version = self.version_inner();
        let capabilities: Vec<CapabilityEntry> = self
            .supported_conversions()
            .into_iter()
            .map(|(source, target)| CapabilityEntry {
                source,
                target,
                works: false,
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
            });
        assert!(engine.can_convert("html", "pdf"));
        assert!(!engine.can_convert("docx", "pdf"));
        assert!(!engine.can_convert("html", "png"));
        assert!(!engine.can_convert("xlsx", "pdf"));
        assert!(!engine.can_convert("html", "docx"));
        assert!(!engine.can_convert("unknown", "pdf"));
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
            });
        let conversions = engine.supported_conversions();
        assert_eq!(conversions.len(), 1);
        assert_eq!(conversions[0], ("html".into(), "pdf".into()));
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
            });
        let info = engine.info();
        assert_eq!(info.engine_id, "chromium");
        assert_eq!(info.capabilities.len(), 1);
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
}
