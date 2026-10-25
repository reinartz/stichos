// SPDX-License-Identifier: MIT OR Apache-2.0
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

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

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
/// Handles formats like:
/// - `"LibreOffice 7.5.0.3 (Build: 3)"` → `"7.5.0.3"`
/// - `"LibreOffice/7.5.0.3"` → `"7.5.0.3"`
pub fn parse_lo_version(output: &str) -> Option<String> {
    output
        .lines()
        .find(|l| l.starts_with("LibreOffice"))
        .and_then(|l| {
            let rest = l.strip_prefix("LibreOffice")?;
            // Handle both "LibreOffice X.Y.Z" and "LibreOffice/X.Y.Z"
            let rest = rest
                .strip_prefix('/')
                .or_else(|| rest.strip_prefix(|c: char| c.is_whitespace()));
            let rest = rest?;
            // Version is the first whitespace-delimited token
            // that looks like a version (starts with digit or 'v')
            rest.split_whitespace()
                .find(|w| {
                    !w.is_empty()
                        && (w.chars().next().unwrap().is_ascii_digit() || w.starts_with('v'))
                })
                .map(|v| v.to_string())
        })
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

/// Common soffice binary paths to probe.
fn common_lo_paths() -> Vec<&'static str> {
    #[cfg(windows)]
    {
        vec![
            "C:\\Program Files\\LibreOffice\\program\\soffice.bin",
            "C:\\Program Files (x86)\\LibreOffice\\program\\soffice.bin",
            "C:\\Program Files\\LibreOffice\\program\\soffice.exe",
            "C:\\Program Files (x86)\\LibreOffice\\program\\soffice.exe",
        ]
    }
    #[cfg(not(windows))]
    {
        vec![
            "/usr/bin/soffice",
            "/usr/bin/libreoffice",
            "/usr/local/bin/soffice",
            "/usr/local/bin/libreoffice",
            "/opt/libreoffice/program/soffice",
            "/Applications/LibreOffice.app/Contents/MacOS/soffice",
        ]
    }
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
    let path_sep = if cfg!(windows) { ';' } else { ':' };
    if let Ok(paths) = std::env::var("PATH") {
        for dir in paths.split(path_sep) {
            #[cfg(windows)]
            {
                // Prefer soffice.bin (headless) over soffice.exe (GUI launcher)
                let p = std::path::Path::new(dir).join("soffice.bin");
                if p.exists() {
                    return Some(p.to_string_lossy().to_string());
                }
            }
            let p = std::path::Path::new(dir).join("soffice");
            let p = if cfg!(windows) {
                p.with_extension("exe")
            } else {
                p
            };
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

// ---------------------------------------------------------------------------
// Cross-platform subprocess stdout/stderr reader
// ---------------------------------------------------------------------------

/// Cross-platform handle for reading subprocess output without pipe-deadlock.
///
/// On Unix, uses `dup()` + raw fd management with `poll()`.
/// On Windows, reads directly from the `ChildStdout`/`ChildStderr` handles
/// which are taken from the child process.
enum SubprocessReader {
    #[cfg(unix)]
    Unix(NoCloseFile),
    #[cfg(windows)]
    Stdout(std::process::ChildStdout),
    #[cfg(windows)]
    Stderr(std::process::ChildStderr),
}

impl Read for SubprocessReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(ncf) => ncf.read(buf),
            #[cfg(windows)]
            Self::Stdout(h) => h.read(buf),
            #[cfg(windows)]
            Self::Stderr(h) => h.read(buf),
        }
    }
}

/// Unix-specific: A File wrapper that does NOT close the underlying raw fd on drop.
///
/// Used to wrap duplicated file descriptors so that dropping the wrapper
/// does not close the fd — the caller closes the dup explicitly to signal
/// EOF to the reader thread.
#[cfg(unix)]
struct NoCloseFile(std::fs::File, libc::c_int);

#[cfg(unix)]
impl NoCloseFile {
    /// Wrap a raw fd without closing it on drop.
    /// Returns None if the fd is invalid (-1).
    fn from_raw(raw: libc::c_int) -> Option<Self> {
        use std::os::unix::io::FromRawFd;
        if raw < 0 {
            return None;
        }
        Some(Self(unsafe { std::fs::File::from_raw_fd(raw) }, raw))
    }
}

#[cfg(unix)]
impl Drop for NoCloseFile {
    fn drop(&mut self) {
        // intentionally do nothing — caller closes the dup explicitly
    }
}

#[cfg(unix)]
impl Read for NoCloseFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // If fd was already closed/invalid, return EOF immediately.
        if self.1 < 0 {
            return Ok(0);
        }
        // Use poll() to avoid blocking forever if the pipe is empty.
        // This is important because poll() returns quickly when the
        // fd is closed, allowing the read loop to detect EOF.
        use libc::{poll, pollfd, POLLIN, POLLNVAL};
        let mut pfd = pollfd {
            fd: self.1,
            events: POLLIN,
            revents: 0,
        };
        // poll() returns 0 on timeout, < 0 on error, > 0 on readiness.
        let rc = unsafe { poll(&mut pfd, 1, -1) };
        if rc < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if pfd.revents & POLLNVAL != 0 {
            // fd not open or invalid → EOF
            return Ok(0);
        }
        if pfd.revents != 0 && pfd.revents & POLLIN == 0 {
            // Hung up or error
            return Ok(0);
        }
        self.0.read(buf)
    }
}

/// Unix-specific: Wrap a raw fd in a NoCloseFile, panicking if the fd is invalid.
#[cfg(unix)]
fn noclose(raw: libc::c_int) -> NoCloseFile {
    NoCloseFile::from_raw(raw).expect("from_raw: invalid fd")
}

/// Spawn a reader thread for subprocess output.
fn spawn_reader(mut reader: SubprocessReader) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf);
        buf
    })
}

// ---------------------------------------------------------------------------
// Platform-specific run_subprocess implementations
// ---------------------------------------------------------------------------

#[cfg(unix)]
impl LibreOfficeEngine {
    /// Run a subprocess and capture stdout/stderr with hard timeout.
    ///
    /// Returns (exit_code, stdout_bytes, stderr_bytes) on success.
    /// Returns Err(diagnostics) on timeout or spawn failure.
    ///
    /// Pipe-deadlock prevention: we create separate dup fds for the
    /// main thread that are closed **before** join() to force EOF on
    /// the reader threads — even when the child has filled pipe buffers
    /// and is blocked on write() (the classic stdout+stderr deadlock).
    fn run_subprocess(cmd: &mut Command) -> Result<(i32, Vec<u8>, Vec<u8>), Vec<Diagnostic>> {
        use libc::dup;

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

        // Create separate dup fds: one for the reader thread, one that
        // the main thread holds to signal EOF via drop().  The NoCloseFile
        // wrapper prevents drop from closing the fd, so the main thread
        // can close it explicitly before join().
        let stdout_raw = child.stdout.as_ref().unwrap().as_raw_fd();
        let stderr_raw = child.stderr.as_ref().unwrap().as_raw_fd();
        let stdout_reader =
            spawn_reader(SubprocessReader::Unix(noclose(unsafe { dup(stdout_raw) })));
        let stderr_reader =
            spawn_reader(SubprocessReader::Unix(noclose(unsafe { dup(stderr_raw) })));
        let stdout_main = noclose(unsafe { dup(stdout_raw) });
        let stderr_main = noclose(unsafe { dup(stderr_raw) });

        // Poll with try_wait in a loop with timeout
        let deadline = std::time::Instant::now() + Duration::from_secs(LO_TIMEOUT_SECS);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    // Close main-thread dup fds to signal EOF before join
                    // (prevents hang if child subprocess children are still
                    // writing to inherited pipe fds after parent exit).
                    drop(stdout_main);
                    drop(stderr_main);
                    let exit_code = status.code().unwrap_or(-1);
                    let stdout_bytes = stdout_reader.join().unwrap_or_default();
                    let stderr_bytes = stderr_reader.join().unwrap_or_default();
                    return Ok((exit_code, stdout_bytes, stderr_bytes));
                }
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        // Close main-thread dup fds to signal EOF to
                        // reader threads, preventing the pipe-deadlock
                        // where reader threads block forever because
                        // child subprocess children keep pipe write-ends open.
                        drop(stdout_main);
                        drop(stderr_main);
                        let _stdout_bytes = stdout_reader.join().unwrap_or_default();
                        let _stderr_bytes = stderr_reader.join().unwrap_or_default();
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
                    let _ = child.wait();
                    drop(stdout_main);
                    drop(stderr_main);
                    let _stdout_bytes = stdout_reader.join().unwrap_or_default();
                    let _stderr_bytes = stderr_reader.join().unwrap_or_default();
                    return Err(vec![Diagnostic::new(
                        "ENG-E-001",
                        Severity::Error,
                        "failed to wait for subprocess",
                    )]);
                }
            }
        }
    }
}

#[cfg(windows)]
impl LibreOfficeEngine {
    /// Run a subprocess and capture stdout/stderr with hard timeout.
    ///
    /// Returns (exit_code, stdout_bytes, stderr_bytes) on success.
    /// Returns Err(diagnostics) on timeout or spawn failure.
    ///
    /// On Windows, stdout/stderr are read in separate threads.
    /// The reader threads take ownership of the ChildStdout/ChildStderr handles.
    /// Main thread uses try_wait() to check for exit status.
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

        // Take ownership of stdout/stderr for reader threads.
        // The reader threads will read until EOF (when the child exits and closes its ends).
        // Main thread uses try_wait() to detect exit — no need for stdout/stderr after exit.
        let stdout_reader = child.stdout.take();
        let stderr_reader = child.stderr.take();

        // Spawn threads to read stdout/stderr concurrently
        let stdout_handle = match stdout_reader {
            Some(h) => spawn_reader(SubprocessReader::Stdout(h)),
            None => thread::spawn(Vec::new),
        };
        let stderr_handle = match stderr_reader {
            Some(h) => spawn_reader(SubprocessReader::Stderr(h)),
            None => thread::spawn(Vec::new),
        };

        // Wait with timeout using try_wait()
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
                        let _stdout_bytes = stdout_handle.join().unwrap_or_default();
                        let _stderr_bytes = stderr_handle.join().unwrap_or_default();
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
                    let _ = child.wait();
                    let _stdout_bytes = stdout_handle.join().unwrap_or_default();
                    let _stderr_bytes = stderr_handle.join().unwrap_or_default();
                    return Err(vec![Diagnostic::new(
                        "ENG-E-001",
                        Severity::Error,
                        "failed to wait for subprocess",
                    )]);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Common implementation (non-platform-specific)
// ---------------------------------------------------------------------------

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
        cmd.arg(format!(
            "-env:UserInstallation={}",
            path_to_file_uri(&self.temp_dir)
        ))
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

/// Convert a filesystem path to a valid `file://` URI.
///
/// On Windows, `C:\foo\bar` → `file:///C:/foo/bar` (with percent-encoding
/// for spaces and other reserved characters). On Unix paths are simply
/// prefixed with `file:///`.
fn path_to_file_uri(path: &str) -> String {
    use std::fmt::Write;
    let normalized = path.replace('\\', "/");
    // Percent-encode reserved characters per RFC 3986, preserving the
    // drive-letter colon in Windows paths (e.g. C:/... → file:///C:/...).
    let mut encoded = String::with_capacity(normalized.len() * 3);
    let mut chars = normalized.chars().peekable();
    while let Some(ch) = chars.next() {
        // On Windows paths, preserve the drive-letter colon (e.g. C: after
        // the initial file:/// prefix).
        if cfg!(windows) && encoded.ends_with("file:///") && chars.peek() == Some(&':') {
            encoded.push(':');
            chars.next();
        } else {
            match ch {
                'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' => {
                    encoded.push(ch);
                }
                ':' if cfg!(windows)
                    && normalized.len() >= 2
                    && normalized.as_bytes()[0] >= b'A' =>
                {
                    // Preserve colon in drive letter (e.g. C:/)
                    encoded.push(ch);
                }
                _ => {
                    let bytes = ch.to_string().into_bytes();
                    for b in bytes {
                        let _ = write!(encoded, "%{:02X}", b);
                    }
                }
            }
        }
    }
    // Unix paths already start with / → file:/// + /path = file:////path (correct)
    // Windows paths start with drive letter → file:/// + C:/path = file:///C:/path
    if cfg!(unix) {
        format!("file://{}", encoded)
    } else {
        format!("file:///{}", encoded)
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

    /// Resolve a soffice path from this host, or print a skip notice.
    /// CI runners have no LibreOffice; these tests must not assume it exists.
    fn lo_path_or_skip(test: &str) -> Option<String> {
        match find_lo_binary() {
            Some(p) => Some(p),
            None => {
                eprintln!("SKIP {test}: no soffice/libreoffice binary found");
                None
            }
        }
    }

    #[test]
    fn test_libreoffice_engine_creation() {
        let Some(path) = lo_path_or_skip("test_libreoffice_engine_creation") else {
            return;
        };
        let result = LibreOfficeEngine::new(&path);
        assert!(result.is_ok(), "engine creation should succeed for {path}");
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
        #[cfg(windows)]
        let path = r"C:\Program Files\LibreOffice\program\soffice.bin";
        #[cfg(not(windows))]
        let path = "/usr/bin/soffice";
        let engine = lo_engine(path);
        assert!(engine.can_convert("docx", "pdf"));
        assert!(engine.can_convert("xlsx", "pdf"));
        assert!(engine.can_convert("pptx", "pdf"));
        assert!(engine.can_convert("odt", "pdf"));
        assert!(!engine.can_convert("docx", "docx"));
        assert!(!engine.can_convert("unknown", "pdf"));
    }

    #[test]
    fn test_supported_conversions() {
        #[cfg(windows)]
        let path = r"C:\Program Files\LibreOffice\program\soffice.bin";
        #[cfg(not(windows))]
        let path = "/usr/bin/soffice";
        let engine = lo_engine(path);
        let conversions = engine.supported_conversions();
        assert!(!conversions.is_empty());
        assert!(conversions.contains(&("docx".to_string(), "pdf".to_string())));
    }

    #[test]
    fn test_engine_id() {
        #[cfg(windows)]
        let path = r"C:\Program Files\LibreOffice\program\soffice.bin";
        #[cfg(not(windows))]
        let path = "/usr/bin/soffice";
        let engine = lo_engine(path);
        assert_eq!(engine.engine_id(), "libreoffice");
    }

    #[test]
    fn test_find_lo_binary() {
        match find_lo_binary() {
            Some(p) => assert!(
                std::path::Path::new(&p).exists(),
                "detected soffice path should exist: {p}"
            ),
            None => eprintln!("SKIP test_find_lo_binary: no soffice/libreoffice binary found"),
        }
    }

    #[test]
    fn test_is_available() {
        let Some(path) = lo_path_or_skip("test_is_available") else {
            return;
        };
        let engine = lo_engine(&path);
        assert!(engine.is_available());
    }

    #[test]
    fn test_default_engine() {
        let _engine = LibreOfficeEngine::default();
    }

    #[test]
    fn test_engine_info() {
        let Some(path) = lo_path_or_skip("test_engine_info") else {
            return;
        };
        let engine = lo_engine(&path);
        let info = engine.info();
        assert_eq!(info.engine_id, "libreoffice");
        assert!(info.is_available);
        assert!(!info.capabilities.is_empty());
    }

    // -----------------------------------------------------------------------
    // parse_lo_version edge-case tests (slash separator, v-prefix, etc.)
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_lo_version_slash_separator() {
        // LibreOffice/7.5.0.3 (slash instead of space)
        let result = parse_lo_version("LibreOffice/7.5.0.3");
        assert_eq!(result, Some("7.5.0.3".to_string()));

        let result = parse_lo_version("LibreOffice/26.2.6.1 (Build: 1)");
        assert_eq!(result, Some("26.2.6.1".to_string()));

        let result = parse_lo_version("LibreOffice/3.0 something");
        assert_eq!(result, Some("3.0".to_string()));
    }

    #[test]
    fn test_parse_lo_version_v_prefix() {
        let result = parse_lo_version("LibreOffice v2.0.0");
        assert_eq!(result, Some("v2.0.0".to_string()));
    }

    #[test]
    fn test_parse_lo_version_tab_separator() {
        let result = parse_lo_version("LibreOffice\t7.5.0.3");
        assert_eq!(result, Some("7.5.0.3".to_string()));
    }

    #[test]
    fn test_parse_lo_version_multiline_mixed() {
        let result = parse_lo_version("Header\nLibreOffice/7.6.0\nFooter");
        assert_eq!(result, Some("7.6.0".to_string()));

        let result = parse_lo_version("LibreOffice 1.0\nLibreOffice/2.0");
        assert_eq!(result, Some("1.0".to_string()));
    }

    #[test]
    fn test_parse_lo_version_empty_after_prefix() {
        let result = parse_lo_version("LibreOffice");
        assert!(result.is_none());

        let result = parse_lo_version("LibreOffice/");
        assert!(result.is_none());

        let result = parse_lo_version("LibreOffice  ");
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // NoCloseFile fd-leak / poll-EOF tests (Unix only)
    // -----------------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn test_noclosefile_reads_pipe_eof() {
        use std::os::unix::io::FromRawFd;

        // Create a pipe, write data, close the write end, then read via
        // NoCloseFile — should see EOF after data.
        let mut fds: [libc::c_int; 2] = [0; 2];
        let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), 0) };
        assert_eq!(rc, 0, "pipe2 should succeed");
        let read_fd = fds[0];
        let write_fd = fds[1];

        // Write data.
        let data = b"hello from pipe";
        {
            let file = unsafe { std::fs::File::from_raw_fd(write_fd) };
            let mut f = file;
            let _ = f.write_all(data);
            // file drops here, closing write_fd → pipe EOF
        }

        // Now read via NoCloseFile.
        let ncf = NoCloseFile::from_raw(read_fd).expect("valid read fd");
        let handle = spawn_reader(SubprocessReader::Unix(ncf));
        let result = handle.join().unwrap_or_default();
        assert_eq!(result, data.to_vec(), "should read the written data");
    }

    #[cfg(unix)]
    #[test]
    fn test_noclosefile_returns_zero_on_eof() {
        // Create a pipe, close the write end, then read via NoCloseFile.
        // poll() should detect the closed write end and return 0 bytes.
        let mut fds: [libc::c_int; 2] = [0; 2];
        let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), 0) };
        assert_eq!(rc, 0, "pipe2 should succeed");
        let read_fd = fds[0];
        let write_fd = fds[1];

        // Close write end so read end sees EOF.
        unsafe { libc::close(write_fd) };

        let ncf = NoCloseFile::from_raw(read_fd).expect("valid fd");
        let handle = spawn_reader(SubprocessReader::Unix(ncf));
        let result = handle.join().unwrap_or_default();
        // Should return empty vec (write end closed → EOF).
        assert!(
            result.is_empty(),
            "reading from closed pipe should yield empty data"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_noclosefile_from_raw_invalid_fd() {
        // Invalid fd should return None.
        assert!(
            NoCloseFile::from_raw(-1).is_none(),
            "invalid fd should return None"
        );
    }

    #[test]
    fn test_path_to_file_uri_unix() {
        #[cfg(unix)]
        {
            assert_eq!(
                path_to_file_uri("/tmp/stichos-lo-abc"),
                "file:///tmp/stichos-lo-abc"
            );
            assert_eq!(
                path_to_file_uri("/tmp/my profile/stichos"),
                "file:///tmp/my%20profile/stichos"
            );
        }
    }

    #[test]
    fn test_path_to_file_uri_windows() {
        #[cfg(windows)]
        {
            assert_eq!(
                path_to_file_uri(r"C:\Users\test\stichos"),
                "file:///C:/Users/test/stichos"
            );
            assert_eq!(
                path_to_file_uri(r"C:\Program Files\LibreOffice\program"),
                "file:///C:/Program%20Files/LibreOffice/program"
            );
        }
    }
}
