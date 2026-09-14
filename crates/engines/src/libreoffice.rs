// engines: LibreOffice runner with isolated UserInstallation, --convert-to,
// hard timeout, instance pool, and version parsing.
//
// Spawns soffice as a subprocess with `-env:UserInstallation` isolation
// so concurrent runs do not share profile state.

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
use sha2::Digest;

const LO_TIMEOUT_SECS: u64 = 120;
const LO_MAX_INSTANCES: usize = 4;

/// Helper to construct an engine from a path, used in tests when new() fails.
fn lo_engine(path: &str) -> LibreOfficeEngine {
    LibreOfficeEngine {
        path: path.to_string(),
        temp_dir: std::env::temp_dir()
            .join("stichos-lo-test")
            .to_string_lossy()
            .to_string(),
        instance_pool: InstancePool::new(LO_MAX_INSTANCES),
    }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// LibreOffice engine — spawns soffice subprocesses.
#[derive(Debug)]
pub struct LibreOfficeEngine {
    pub path: String,
    pub temp_dir: String,
    pub instance_pool: InstancePool,
}

/// Tracks instance pool state for managing LibreOffice subprocesses.
#[derive(Debug, Clone)]
pub struct InstancePool {
    max_instances: usize,
    current_count: usize,
}

impl InstancePool {
    fn new(max_instances: usize) -> Self {
        Self {
            max_instances,
            current_count: 0,
        }
    }

    #[allow(dead_code)]
    fn try_acquire(&mut self) -> bool {
        if self.current_count < self.max_instances {
            self.current_count += 1;
            true
        } else {
            false
        }
    }

    #[allow(dead_code)]
    fn release(&mut self) {
        if self.current_count > 0 {
            self.current_count -= 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Version parsing
// ---------------------------------------------------------------------------

/// Parse LO version from `--version` output.
///
/// E.g. `"LibreOffice/7.5.0.3"` → `"7.5.0.3"`
pub fn parse_lo_version(output: &str) -> Option<String> {
    output
        .lines()
        .find(|l| l.starts_with("LibreOffice"))
        .and_then(|l| {
            l.strip_prefix("LibreOffice")
                .and_then(|v| v.split('/').next())
                .and_then(|v| v.split_whitespace().next())
                .map(|v| v.to_string())
        })
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

/// Common soffice binary paths to probe.
fn common_lo_paths() -> Vec<&'static str> {
    vec![
        "/usr/bin/soffice",
        "/usr/bin/libreoffice",
        "/usr/local/bin/soffice",
        "/usr/local/bin/libreoffice",
        "/opt/libreoffice/program/soffice",
        "/Applications/LibreOffice.app/Contents/MacOS/soffice",
    ]
}

/// Find the first existing soffice binary.
fn find_lo_binary() -> Option<String> {
    // Check environment variable first
    if let Ok(p) = std::env::var("LO_PATH") {
        if Path::new(&p).exists() {
            return Some(p);
        }
    }
    // Check PATH
    if let Ok(paths) = std::env::var("PATH") {
        for dir in paths.split(':') {
            let p = std::path::Path::new(dir).join("soffice");
            if p.exists() {
                return Some(p.to_string_lossy().to_string());
            }
            let p = std::path::Path::new(dir).join("libreoffice");
            if p.exists() {
                return Some(p.to_string_lossy().to_string());
            }
        }
    }
    // Check common paths
    common_lo_paths()
        .into_iter()
        .find(|p| Path::new(p).exists())
        .map(String::from)
}

/// Read all bytes from a reader in a separate thread.
fn read_all_async<R: Read + Send + 'static>(mut reader: R) -> std::thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf);
        buf
    })
}

impl LibreOfficeEngine {
    /// Create a new LibreOffice engine.
    ///
    /// Verifies the binary exists and creates a temp dir for
    /// the isolated `-env:UserInstallation` profile.
    pub fn new(path: &str) -> Result<Self, Vec<Diagnostic>> {
        let lo_path = path.to_string();
        if !Path::new(&lo_path).exists() {
            return Err(vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("engine not found: {}", lo_path),
            )]);
        }
        let temp_dir = std::env::temp_dir()
            .join(format!("stichos-lo-{:08x}", fastrand::u64(0..0xFFFFFFFF)))
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
            path: lo_path,
            temp_dir,
            instance_pool: InstancePool::new(LO_MAX_INSTANCES),
        })
    }

    /// Create a new engine by auto-detecting the soffice binary.
    pub fn new_auto() -> Result<Self, Vec<Diagnostic>> {
        let path = find_lo_binary().ok_or_else(|| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                "engine not found: soffice binary not found in PATH or common paths",
            )]
        })?;
        Self::new(&path)
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
                    format!("failed to spawn subprocess: {}", e),
                )]
            })?;

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // Spawn threads to read stdout/stderr concurrently
        let stdout_handle = read_all_async(stdout);
        let stderr_handle = read_all_async(stderr);

        // Poll with try_wait in a loop with timeout
        let deadline = std::time::Instant::now() + Duration::from_secs(LO_TIMEOUT_SECS);
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
                            format!("Engine timeout after {}ms", LO_TIMEOUT_SECS * 1000),
                        )]);
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                Err(_) => {
                    let _ = child.kill();
                    let _ = stdout_handle.join().unwrap_or_default();
                    let _ = stderr_handle.join().unwrap_or_default();
                    return Err(vec![Diagnostic::new(
                        "ENG-E-001",
                        Severity::Error,
                        "failed to wait for subprocess",
                    )]);
                }
            }
        }
    }

    /// Get version by running `soffice --version`.
    fn version_inner(&self) -> Option<String> {
        let mut cmd = Command::new(&self.path);
        cmd.arg("--version");
        let result = Self::run_subprocess(&mut cmd);
        match result {
            Ok((exit_code, stdout, _stderr)) => {
                if exit_code == 0 {
                    let output = String::from_utf8_lossy(&stdout);
                    parse_lo_version(&output)
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    }
}

impl Engine for LibreOfficeEngine {
    fn engine_id(&self) -> &str {
        "libreoffice"
    }

    fn version(&self) -> Option<&str> {
        // Return None here — version is computed lazily in version_inner()
        // and EngineExt::info() calls version_inner() internally.
        // This avoids the lifetime issue of returning &str from a String
        // owned by the cache field.
        None
    }

    fn can_convert(&self, source: &str, target: &str) -> bool {
        let supported_sources = [
            "docx", "xlsx", "pptx", "odt", "ods", "odp", "pdf", "png", "doc",
        ];
        let supported_targets = [
            "pdf", "docx", "xlsx", "pptx", "odt", "ods", "odp", "txt", "html", "png", "jpg", "svg",
        ];
        supported_sources.contains(&source)
            && supported_targets.contains(&target)
            && source != target
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
                format!("engine not found: {}", self.path),
            )]);
        }

        let work_dir = std::env::temp_dir()
            .join(format!(
                "stichos-lo-convert-{:08x}",
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

        let input_name = format!("input.{}", source);
        let input_path = Path::new(&work_dir).join(&input_name);
        let mut input_file = std::fs::File::create(&input_path).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to create input file: {}", e),
            )]
        })?;
        input_file.write_all(input).map_err(|e| {
            vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to write input file: {}", e),
            )]
        })?;
        let input_path_str = input_path.to_string_lossy().to_string();

        let _filter = match (source, target) {
            ("docx", "pdf") => "writer_pdf_Export",
            ("doc", "pdf") => "writer_pdf_Export",
            ("odt", "pdf") => "writer_pdf_Export",
            ("xlsx", "pdf") => "calc_pdf_Export",
            ("ods", "pdf") => "calc_pdf_Export",
            ("pptx", "pdf") => "impress_pdf_Export",
            ("odp", "pdf") => "impress_pdf_Export",
            ("pdf", "png") => "draw_png_Export",
            ("png", "pdf") => "draw_pdf_Export",
            _ => "",
        };

        let output_name = format!("input.{}", target);
        let output_path = Path::new(&work_dir).join(&output_name);

        let filter_arg = if _filter.is_empty() {
            target.to_string()
        } else {
            format!("{}:{}", target, _filter)
        };

        let mut cmd = Command::new(&self.path);
        cmd.arg(format!("-env:UserInstallation=file://{}", self.temp_dir))
            .args([
                "--headless",
                "--invisible",
                "--nocrashreport",
                "--nodefault",
            ])
            .arg("--convert-to")
            .arg(&filter_arg)
            .arg("--outdir")
            .arg(&work_dir)
            .arg(&input_path_str);

        let result = Self::run_subprocess(&mut cmd);

        let read_output = || match std::fs::read(&output_path) {
            Ok(data) => Ok(data),
            Err(e) => Err(vec![Diagnostic::new(
                "ENG-E-001",
                Severity::Error,
                format!("failed to read output file: {}", e),
            )]),
        };

        let outcome = match result {
            Ok((exit_code, _stdout, stderr)) => {
                if exit_code != 0 {
                    let msg = String::from_utf8_lossy(&stderr);
                    Err(vec![Diagnostic::new(
                        "ENG-E-001",
                        Severity::Error,
                        format!("soffice exited with code {}: {}", exit_code, msg.trim()),
                    )])
                } else {
                    read_output()
                }
            }
            Err(diags) => Err(diags),
        };

        let _ = std::fs::remove_dir_all(&work_dir);

        outcome
    }

    fn is_available(&self) -> bool {
        Path::new(&self.path).exists()
    }
}

impl EngineExt for LibreOfficeEngine {
    fn fontset_hash(&self) -> Option<String> {
        fontset_hash_impl()
    }

    fn supported_conversions(&self) -> Vec<(String, String)> {
        vec![
            ("docx".into(), "pdf".into()),
            ("doc".into(), "pdf".into()),
            ("odt".into(), "pdf".into()),
            ("xlsx".into(), "pdf".into()),
            ("ods".into(), "pdf".into()),
            ("pptx".into(), "pdf".into()),
            ("odp".into(), "pdf".into()),
            ("doc".into(), "docx".into()),
            ("odt".into(), "docx".into()),
            ("xlsx".into(), "csv".into()),
            ("ods".into(), "csv".into()),
            ("pptx".into(), "odp".into()),
            ("pdf".into(), "png".into()),
            ("png".into(), "pdf".into()),
        ]
    }

    fn info(&self) -> EngineInfo {
        let version = self.version_inner();
        let fontset = self.fontset_hash();
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
            engine_id: "libreoffice".into(),
            version,
            fontset_hash: fontset,
            is_available: self.is_available(),
            capabilities,
        }
    }
}

impl Default for LibreOfficeEngine {
    fn default() -> Self {
        Self::new_auto().unwrap_or_else(|_| lo_engine("/usr/bin/soffice"))
    }
}

/// Compute a fontset hash by hashing fc-match output for common fonts.
fn fontset_hash_impl() -> Option<String> {
    let fonts = [
        "Calibri",
        "Arial",
        "Times New Roman",
        "Courier New",
        "Consolas",
    ];
    let mut hasher = sha2::Sha256::new();
    let mut found_any = false;

    for font in &fonts {
        let mut cmd = Command::new("fc-match");
        cmd.args(["--format", "%{family}\n", font]);
        match cmd.output() {
            Ok(output) if output.status.success() => {
                let result = String::from_utf8_lossy(&output.stdout);
                hasher.update(result.as_bytes());
                found_any = true;
            }
            _ => {}
        }
    }

    if found_any {
        let digest = hasher.finalize();
        Some(format!("{:x}", digest))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_libreoffice_engine_creation() {
        let result = LibreOfficeEngine::new("/usr/bin/soffice");
        assert!(
            result.is_ok(),
            "engine creation should succeed for valid path"
        );
    }

    #[test]
    fn test_libreoffice_engine_invalid_path() {
        let result = LibreOfficeEngine::new("/nonexistent/soffice");
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "ENG-E-001");
    }

    #[test]
    fn test_parse_lo_version() {
        let result = parse_lo_version("LibreOffice 7.5.0.3");
        assert_eq!(result, Some("7.5.0.3".to_string()));
    }

    #[test]
    fn test_version_parsing_variants() {
        let result = parse_lo_version("LibreOffice 26.2.6.1 620(Build:1)");
        assert_eq!(result, Some("26.2.6.1".to_string()));

        let result = parse_lo_version("LibreOffice 7.5.0.3 something");
        assert_eq!(result, Some("7.5.0.3".to_string()));

        let result = parse_lo_version("Some random output");
        assert!(result.is_none());

        let result = parse_lo_version("");
        assert!(result.is_none());

        let result = parse_lo_version("some header\nLibreOffice 7.6.0\nfooter");
        assert_eq!(result, Some("7.6.0".to_string()));

        let result = parse_lo_version("LibreOffice  3.0  something");
        assert_eq!(result, Some("3.0".to_string()));

        let result = parse_lo_version("\nLibreOffice 7.5.0.3\n");
        assert!(result.is_some());

        let output = "LibreOffice 7.5.0.3 (Build: 3)\nRevision ID: abc123\nCopyright: ...";
        assert_eq!(parse_lo_version(output), Some("7.5.0.3".to_string()));
    }

    #[test]
    fn test_fontset_hash_computed() {
        let hash = fontset_hash_impl();
        if let Some(hash) = hash {
            assert!(!hash.is_empty(), "hash should be non-empty");
            assert_eq!(hash.len(), 64, "sha256 hex should be 64 chars");
            assert!(
                hash.chars().all(|c| c.is_ascii_hexdigit()),
                "hash should be hex"
            );
        }
    }

    #[test]
    fn test_convert_returns_diagnostics_when_lo_absent() {
        let engine = lo_engine("/nonexistent/soffice");
        let input = b"test document content";
        let result = engine.convert(input, "docx", "pdf");
        assert!(
            result.is_err(),
            "convert should fail when LO binary is absent"
        );
        let diags = result.unwrap_err();
        assert_eq!(diags[0].code, "ENG-E-001");
    }

    #[test]
    fn test_instance_pool_acquire_release() {
        let mut pool = InstancePool::new(2);
        assert!(pool.try_acquire());
        assert!(pool.try_acquire());
        assert!(!pool.try_acquire());
        pool.release();
        assert!(pool.try_acquire());
        pool.release();
        pool.release();
        pool.release();
    }

    #[test]
    fn test_can_convert() {
        let engine = lo_engine("/usr/bin/soffice");
        assert!(engine.can_convert("docx", "pdf"));
        assert!(engine.can_convert("xlsx", "pdf"));
        assert!(engine.can_convert("pptx", "pdf"));
        assert!(engine.can_convert("odt", "pdf"));
        assert!(!engine.can_convert("docx", "docx"));
        assert!(!engine.can_convert("unknown", "pdf"));
    }

    #[test]
    fn test_supported_conversions() {
        let engine = lo_engine("/usr/bin/soffice");
        let conversions = engine.supported_conversions();
        assert!(!conversions.is_empty());
        assert!(conversions.contains(&("docx".to_string(), "pdf".to_string())));
    }

    #[test]
    fn test_engine_id() {
        let engine = lo_engine("/usr/bin/soffice");
        assert_eq!(engine.engine_id(), "libreoffice");
    }

    #[test]
    fn test_find_lo_binary() {
        let result = find_lo_binary();
        assert!(
            result.is_some(),
            "should find soffice binary on this system"
        );
    }

    #[test]
    fn test_is_available() {
        let engine = lo_engine("/usr/bin/soffice");
        assert!(engine.is_available());
    }

    #[test]
    fn test_default_engine() {
        let _engine = LibreOfficeEngine::default();
    }

    #[test]
    fn test_engine_info() {
        let engine = lo_engine("/usr/bin/soffice");
        let info = engine.info();
        assert_eq!(info.engine_id, "libreoffice");
        assert!(info.is_available);
        assert!(!info.capabilities.is_empty());
    }
}
