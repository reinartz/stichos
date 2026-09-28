// SPDX-License-Identifier: MIT OR Apache-2.0
// engines: Typst runner for markdown→PDF compilation.
//
// Spawns typst as a subprocess with `typst compile` for PDF rendering.
// Typst is a soft dependency; engines probe availability at runtime.
// ADR-0004: Typst engine = CLI shell-out only (never linked).

use std::io::Read;
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
pub struct TypstEngine {
    path: String,
    version_cache: OnceLock<Option<String>>,
}

impl std::fmt::Debug for TypstEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypstEngine")
            .field("path", &self.path)
            .field("version", &self.version())
            .finish()
    }
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
            version_cache: OnceLock::new(),
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
    pub fn version_inner(&self) -> Option<String> {
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
    pub fn compile(&self, input: &[u8], _source_format: &str) -> Result<Vec<u8>, Vec<Diagnostic>> {
        self.create_pdf_with_template(input, None)
    }

    /// Compile a typst file to PDF with an optional offline template.
    ///
    /// If `template` is provided, it is written to a temp directory alongside the input,
    /// making it available via typst's local package resolution. No network access is used.
    pub fn create_pdf_with_template(
        &self,
        input: &[u8],
        template: Option<&[u8]>,
    ) -> Result<Vec<u8>, Vec<Diagnostic>> {
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

        // Build input content: inject template rules directly into the document
        // when a template is provided. The template is written to a sibling directory
        // for documentation purposes (sibling-import contract), but the actual
        // rule application uses direct concatenation since Typst's import scoping
        // does not propagate `#set` rules to the importing document.
        let input_content = if let Some(template_bytes) = template {
            let template_dir = tmp_dir.path().join("template");
            std::fs::create_dir(&template_dir).map_err(|e| {
                vec![Diagnostic::new(
                    "ENG-E-001",
                    Severity::Error,
                    format!("failed to create template dir: {}", e),
                )]
            })?;
            std::fs::write(template_dir.join("template.typ"), template_bytes).map_err(|e| {
                vec![Diagnostic::new(
                    "ENG-E-001",
                    Severity::Error,
                    format!("failed to write template: {}", e),
                )]
            })?;
            format!(
                "{}\n\n{}",
                String::from_utf8_lossy(template_bytes),
                String::from_utf8_lossy(input)
            )
        } else {
            String::from_utf8_lossy(input).to_string()
        };

        // Write the main typst input (with optional preamble)
        let input_path = tmp_dir.path().join("input.typ");
        std::fs::write(&input_path, &input_content).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to write typst input: {}", e),
            )]
        })?;

        let output_path = tmp_dir.path().join("output.pdf");

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
        self.version_cache
            .get_or_init(|| self.version_inner())
            .as_deref()
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
            version_cache: OnceLock::new(),
        })
    }
}

impl TypstEngine {
    /// Auto-detect typst binary and compile input to PDF.
    ///
    /// Convenience static method — equivalent to creating `TypstEngine::new_auto()`
    /// then calling `compile()`.
    pub fn convert_to(
        input: &[u8],
        _source_format: &str,
        reference_doc: Option<&[u8]>,
    ) -> Result<Vec<u8>, Vec<Diagnostic>> {
        let engine = Self::new_auto().map_err(|diags| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!(
                    "typst not available — install from https://typst.world/ (P3-21): {}",
                    diags[0].message
                ),
            )]
        })?;
        engine.create_pdf_with_template(input, reference_doc)
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
            version_cache: OnceLock::new(),
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
            version_cache: OnceLock::new(),
        };
        assert_eq!(engine.engine_id(), "typst");
    }

    #[test]
    fn test_typst_supported_conversions() {
        let engine = TypstEngine {
            path: "/usr/bin/typst".to_string(),
            version_cache: OnceLock::new(),
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
            version_cache: OnceLock::new(),
        };
        assert!(!engine.is_available());
        let info = engine.info();
        assert!(!info.is_available);
        assert_eq!(info.engine_id, "typst");
    }

    #[test]
    fn test_info_capabilities_works_true() {
        let engine = TypstEngine {
            path: "/usr/bin/typst".to_string(),
            version_cache: OnceLock::new(),
        };
        let info = engine.info();
        assert!(
            info.capabilities.iter().all(|c| c.works),
            "all typst capabilities should report works: true"
        );
    }

    #[test]
    fn test_compile_minimal_typst_input() {
        // Check if typst is available; skip otherwise
        if !is_available() {
            eprintln!("SKIP: typst not available");
            return;
        }

        let engine = TypstEngine::new_auto().unwrap();
        let input = b"= Hello World\n\nThis is a minimal typst document.";
        let result = engine.compile(input, "typ");
        assert!(result.is_ok(), "compile should succeed: {:?}", result);
        let bytes = result.unwrap();
        assert!(!bytes.is_empty(), "output PDF should not be empty");
        // PDF files start with %PDF
        assert!(
            bytes.len() >= 4 && &bytes[0..4] == b"%PDF",
            "output should be a PDF file"
        );
    }

    #[test]
    fn test_create_pdf_with_template_no_template() {
        if !is_available() {
            eprintln!("SKIP: typst not available");
            return;
        }

        let engine = TypstEngine::new_auto().unwrap();
        let input = b"= Title\n\nBody text without template.";
        let result = engine.create_pdf_with_template(input, None);
        assert!(
            result.is_ok(),
            "compile without template should succeed: {:?}",
            result
        );
        let bytes = result.unwrap();
        assert!(!bytes.is_empty());
        assert!(bytes.len() >= 4 && &bytes[0..4] == b"%PDF");
    }

    #[test]
    fn test_create_pdf_with_template_with_template() {
        if !is_available() {
            eprintln!("SKIP: typst not available");
            return;
        }

        let engine = TypstEngine::new_auto().unwrap();
        let input = b"= Title\n\nBody text with template.";
        // The template sets page margins and adds a red background, which must
        // change the PDF output compared to a default white page.
        let template = b"#set page(margin: 1cm)\n#set page(background: rect(fill: rgb(1, 0, 0)))\n";
        let result = engine.create_pdf_with_template(input, Some(template));
        // The template is imported via preamble injection, so its set/show rules
        // apply to the document. The PDF should be valid and non-empty.
        assert!(
            result.is_ok(),
            "compile with template should succeed: {:?}",
            result
        );
        let bytes = result.unwrap();
        assert!(
            !bytes.is_empty(),
            "output PDF with template should not be empty"
        );
        assert!(
            bytes.len() >= 4 && &bytes[0..4] == b"%PDF",
            "output should be a PDF file"
        );
    }

    #[test]
    fn test_template_affects_output() {
        if !is_available() {
            eprintln!("SKIP: typst not available");
            return;
        }

        let engine = TypstEngine::new_auto().unwrap();
        let input = b"= Title\n\nBody text content for comparison.";
        // Same document compiled with a template that sets page background to red,
        // which must produce a different PDF than the default white page.
        let template = b"#set page(background: rect(fill: rgb(1, 0, 0)))\n";
        let result_with_template = engine.create_pdf_with_template(input, Some(template));
        assert!(
            result_with_template.is_ok(),
            "compile with template: {:?}",
            result_with_template
        );
        // Same document compiled without any template
        let result_without_template = engine.create_pdf_with_template(input, None);
        assert!(
            result_without_template.is_ok(),
            "compile without template: {:?}",
            result_without_template
        );
        // The two PDFs must differ, proving the template content affected the output
        let pdf_with = result_with_template.unwrap();
        let pdf_without = result_without_template.unwrap();
        assert_ne!(
            pdf_with, pdf_without,
            "PDFs with and without template should differ; template has no effect"
        );
    }

    #[test]
    fn test_invalid_typst_input_returns_error_not_panic() {
        if !is_available() {
            eprintln!("SKIP: typst not available");
            return;
        }

        let engine = TypstEngine::new_auto().unwrap();
        // Invalid typst that will cause a compile error
        // Typst is very permissive; `#let x = ` without expression is a reliable parse error
        let input = b"#let x = ";
        let result = engine.compile(input, "typ");
        assert!(
            result.is_err(),
            "invalid typst input should return error, not panic"
        );
        let diags = result.unwrap_err();
        assert_eq!(diags[0].code, "ENG-E-001");
        // Error message should contain typst diagnostic info
        assert!(
            diags[0].message.contains("typst") || diags[0].message.contains("exit"),
            "error message should mention typst or exit code: {}",
            diags[0].message
        );
    }

    #[test]
    fn test_typst_convert_to_absent_returns_eng_e_001() {
        // Simulate absent typst by pointing to nonexistent binary
        // We can't easily remove typst from PATH in tests, so we test the
        // error path by checking the error code is ENG-E-001
        // When typst IS available, this will succeed; when absent, ENG-E-001
        let input = b"= Test Document\n\nContent.";
        let result = TypstEngine::convert_to(input, "typ", None);

        match result {
            Ok(bytes) => {
                // typst available — verify output
                assert!(!bytes.is_empty());
                assert!(bytes.len() >= 4 && &bytes[0..4] == b"%PDF");
            }
            Err(diags) => {
                // typst absent — verify honest error path (P3-21)
                assert_eq!(diags[0].code, "ENG-E-001");
                assert!(
                    diags[0].message.contains("typst")
                        || diags[0].message.contains("install")
                        || diags[0].message.contains("not available"),
                    "error should mention typst/install: {}",
                    diags[0].message
                );
            }
        }
    }

    #[test]
    fn test_typst_compile_with_typ_extension() {
        if !is_available() {
            eprintln!("SKIP: typst not available");
            return;
        }

        let engine = TypstEngine::new_auto().unwrap();
        let input = b"= Document\n\nContent here.";
        let result = engine.compile(input, "typst");
        assert!(result.is_ok(), "typst extension should work: {:?}", result);
    }

    #[test]
    fn test_typst_compile_positive() {
        if !is_available() {
            eprintln!("SKIP: typst not available");
            return;
        }

        let engine = TypstEngine::new_auto().unwrap();
        let result = engine.compile(b"= Test\n\nContent.", "typst");
        assert!(result.is_ok(), "compile should succeed: {:?}", result);
        let bytes = result.unwrap();
        assert!(!bytes.is_empty(), "output PDF should not be empty");
        assert!(
            bytes.len() >= 4 && &bytes[0..4] == b"%PDF",
            "output should start with %PDF magic bytes, got first 4 bytes: {:?}",
            &bytes[..4.min(bytes.len())]
        );
    }

    #[test]
    fn test_version_caching() {
        let engine = TypstEngine::new("/usr/bin/typst").unwrap_or_else(|_| TypstEngine {
            path: "/usr/bin/typst".to_string(),
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
        let engine = TypstEngine::new("/nonexistent/typst").unwrap_or_else(|_| TypstEngine {
            path: "/nonexistent/typst".to_string(),
            version_cache: OnceLock::new(),
        });
        assert!(
            engine.version().is_none(),
            "version() should return None for absent typst binary"
        );
    }

    #[test]
    fn test_engine_info_uses_cached_version() {
        let engine = TypstEngine::new("/usr/bin/typst").unwrap_or_else(|_| TypstEngine {
            path: "/usr/bin/typst".to_string(),
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
