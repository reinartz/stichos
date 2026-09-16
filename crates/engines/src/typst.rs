// engines: Typst runner for markdown→PDF compilation.
//
// Spawns typst as a subprocess with `typst compile` for PDF rendering.
// Typst is a soft dependency; engines probe availability at runtime.
// ADR-0004: Typst engine = CLI shell-out only (never linked).

use std::io::Read;
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

const TYPST_TIMEOUT_SECS: u64 = 60;

/// Common typst binary paths to probe.
fn common_typst_paths() -> Vec<&'static str> {
    vec![
        "/usr/bin/typst",
        "/usr/local/bin/typst",
        "/opt/homebrew/bin/typst",
    ]
}

/// Find the first existing typst binary.
fn find_typst_binary() -> Option<String> {
    // Check environment variable first
    if let Ok(p) = std::env::var("TYPST_PATH") {
        if Path::new(&p).exists() {
            return Some(p);
        }
    }
    // Check PATH
    if let Ok(paths) = std::env::var("PATH") {
        for dir in paths.split(':') {
            let p = Path::new(dir).join("typst");
            if p.exists() {
                return Some(p.to_string_lossy().to_string());
            }
        }
    }
    // Check common paths
    common_typst_paths()
        .into_iter()
        .find(|p| Path::new(p).exists())
        .map(String::from)
}

/// Parse typst version from `--version` output.
///
/// E.g. `"typst 0.11.1"` → `"0.11.1"`
pub fn parse_typst_version(output: &str) -> Option<String> {
    output
        .lines()
        .find(|l| l.starts_with("typst "))
        .and_then(|l| {
            l.strip_prefix("typst ")
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

/// Check if typst is available via PATH or TYPST_PATH env var.
pub fn is_available() -> bool {
    find_typst_binary().is_some()
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
                format!("failed to spawn typst: {}", e),
            )]
        })?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let stdout_handle = read_all_async(stdout);
    let stderr_handle = read_all_async(stderr);

    let deadline = std::time::Instant::now() + Duration::from_secs(TYPST_TIMEOUT_SECS);
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
                        format!("typst timeout after {}ms", TYPST_TIMEOUT_SECS * 1000),
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
                    "failed to wait for typst",
                )]);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Typst engine — spawns typst subprocesses for PDF compilation.
#[derive(Debug)]
pub struct TypstEngine {
    path: String,
}

impl TypstEngine {
    /// Create a new TypstEngine for the given typst binary.
    pub fn new(path: &str) -> Result<Self, Vec<Diagnostic>> {
        if !Path::new(path).exists() {
            return Err(vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("typst not found: {}", path),
            )]);
        }
        Ok(Self {
            path: path.to_string(),
        })
    }

    /// Create a new engine by auto-detecting the typst binary.
    pub fn new_auto() -> Result<Self, Vec<Diagnostic>> {
        let path = find_typst_binary().ok_or_else(|| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                "typst not found in PATH or common paths — install from https://typst.world/",
            )]
        })?;
        Self::new(&path)
    }

    /// Get version by running `typst --version`.
    fn version_inner(&self) -> Option<String> {
        let mut cmd = Command::new(&self.path);
        cmd.arg("--version");
        match run_subprocess(&mut cmd) {
            Ok((exit_code, stdout, _stderr)) => {
                if exit_code == 0 {
                    let output = String::from_utf8_lossy(&stdout);
                    parse_typst_version(&output)
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    }

    /// Get the typst binary path.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Compile a typst file to PDF using this engine.
    pub fn compile(&self, input: &[u8], source_format: &str) -> Result<Vec<u8>, Vec<Diagnostic>> {
        if !Path::new(&self.path).exists() {
            return Err(vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("typst not found: {}", self.path),
            )]);
        }

        let tmp_dir = tempfile::tempdir().map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to create temp dir: {}", e),
            )]
        })?;

        let input_name = if source_format == "typ" {
            "input.typ"
        } else {
            "input.typst"
        };

        let input_path = tmp_dir.path().join(input_name);
        let output_path = tmp_dir.path().join("output.pdf");

        std::fs::write(&input_path, input).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to write typst input: {}", e),
            )]
        })?;

        let mut cmd = Command::new(&self.path);
        cmd.arg("compile")
            .arg(&input_path)
            .arg(&output_path)
            .current_dir(tmp_dir.path());

        let (exit_code, _stdout, stderr) = run_subprocess(&mut cmd)?;

        if exit_code != 0 {
            let msg = String::from_utf8_lossy(&stderr);
            return Err(vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("typst exited with code {}: {}", exit_code, msg.trim()),
            )]);
        }

        std::fs::read(&output_path).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to read typst output PDF: {}", e),
            )]
        })
    }
}

impl Engine for TypstEngine {
    fn engine_id(&self) -> &str {
        "typst"
    }

    fn version(&self) -> Option<&str> {
        None // version computed lazily via version_inner()
    }

    fn can_convert(&self, source: &str, target: &str) -> bool {
        let is_typst_source = source == "typst" || source == "typ";
        is_typst_source && target == "pdf"
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
                format!("typst not found: {}", self.path),
            )]);
        }

        if !(source == "typst" || source == "typ") || target != "pdf" {
            return Err(vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!(
                    "unsupported conversion: {} → {} (typst only supports typst/typ → pdf)",
                    source, target
                ),
            )]);
        }

        self.compile(input, source)
    }

    fn is_available(&self) -> bool {
        Path::new(&self.path).exists()
    }
}

impl EngineExt for TypstEngine {
    fn fontset_hash(&self) -> Option<String> {
        None // typst doesn't expose fontset information in our model
    }

    fn supported_conversions(&self) -> Vec<(String, String)> {
        vec![("typst".into(), "pdf".into()), ("typ".into(), "pdf".into())]
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
            engine_id: "typst".into(),
            version,
            fontset_hash: None,
            is_available: self.is_available(),
            capabilities,
        }
    }
}

impl Default for TypstEngine {
    fn default() -> Self {
        Self::new_auto().unwrap_or_else(|_| Self {
            path: "/usr/bin/typst".to_string(),
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
    fn test_typst_engine_invalid_path() {
        let result = TypstEngine::new("/nonexistent/typst");
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert_eq!(diags[0].code, "ENG-E-001");
    }

    #[test]
    fn test_parse_typst_version() {
        assert_eq!(parse_typst_version("typst 0.11.1"), Some("0.11.1".into()));
        assert_eq!(
            parse_typst_version("typst 0.12.0-beta.1"),
            Some("0.12.0-beta.1".into())
        );
        assert_eq!(parse_typst_version("some random output"), None);
        assert_eq!(parse_typst_version(""), None);
    }

    #[test]
    fn test_typst_can_convert() {
        let engine = TypstEngine {
            path: "/usr/bin/typst".to_string(),
        };
        assert!(engine.can_convert("typst", "pdf"));
        assert!(engine.can_convert("typ", "pdf"));
        assert!(!engine.can_convert("html", "pdf"));
        assert!(!engine.can_convert("docx", "pdf"));
    }

    #[test]
    fn test_typst_engine_id() {
        let engine = TypstEngine {
            path: "/usr/bin/typst".to_string(),
        };
        assert_eq!(engine.engine_id(), "typst");
    }

    #[test]
    fn test_typst_supported_conversions() {
        let engine = TypstEngine {
            path: "/usr/bin/typst".to_string(),
        };
        let conversions = engine.supported_conversions();
        assert!(conversions.contains(&("typst".into(), "pdf".into())));
        assert!(conversions.contains(&("typ".into(), "pdf".into())));
        assert_eq!(conversions.len(), 2);
    }

    #[test]
    fn test_typst_honest_absent() {
        // When typst is not available, is_available should return false
        let engine = TypstEngine {
            path: "/nonexistent/typst".to_string(),
        };
        assert!(!engine.is_available());
        let info = engine.info();
        assert!(!info.is_available);
        assert_eq!(info.engine_id, "typst");
    }
}
