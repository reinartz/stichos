// engines: pandoc runner for office→markdown text extraction.
//
// Spawns pandoc as a subprocess for text extraction (read-only).
// Pandoc is a soft dependency; engines probe availability at runtime.

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
#[derive(Debug)]
pub struct PandocEngine {
    path: String,
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
}

impl Engine for PandocEngine {
    fn engine_id(&self) -> &str {
        "pandoc"
    }

    fn version(&self) -> Option<&str> {
        None // version computed lazily via version_inner()
    }

    fn can_convert(&self, source: &str, target: &str) -> bool {
        let supported_sources = [
            "docx", "xlsx", "pptx", "odt", "ods", "odp", "doc", "xls", "ppt", "rtf", "html", "epub",
        ];
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
            ("xlsx".into(), "markdown".into()),
            ("pptx".into(), "markdown".into()),
            ("odt".into(), "markdown".into()),
            ("ods".into(), "markdown".into()),
            ("odp".into(), "markdown".into()),
            ("doc".into(), "markdown".into()),
            ("rtf".into(), "markdown".into()),
            ("html".into(), "markdown".into()),
            ("epub".into(), "markdown".into()),
        ]
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
    fn test_pandoc_extract_text_from_markdown() {
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
        });
        assert_eq!(engine.engine_id(), "pandoc");
    }

    #[test]
    fn test_can_convert() {
        let engine = PandocEngine::new("/usr/bin/pandoc").unwrap_or_else(|_| PandocEngine {
            path: "/usr/bin/pandoc".to_string(),
        });
        assert!(engine.can_convert("docx", "markdown"));
        assert!(engine.can_convert("xlsx", "markdown"));
        assert!(engine.can_convert("pptx", "markdown"));
        assert!(engine.can_convert("odt", "markdown"));
        assert!(engine.can_convert("html", "markdown"));
        assert!(!engine.can_convert("docx", "pdf"));
        assert!(!engine.can_convert("unknown", "markdown"));
    }

    #[test]
    fn test_supported_conversions() {
        let engine = PandocEngine::new("/usr/bin/pandoc").unwrap_or_else(|_| PandocEngine {
            path: "/usr/bin/pandoc".to_string(),
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
            works: false,
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
}
