// SPDX-License-Identifier: MIT OR Apache-2.0
//! Platform handler for opening files with system default application.
//!
//! Provides platform-specific file opening (xdg-open on Linux, open on macOS)
//! and probe/doctor handoff for the `stichos open` command.

/// Doctor handoff row for the open handler.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DoctorHandoffRow {
    /// Platform identifier (e.g. "linux", "macos", "windows").
    pub platform: String,
    /// Whether a handler is available.
    pub handler_available: bool,
    /// Human-readable handler name.
    pub handler_name: String,
    /// Install hint if handler is not available.
    pub install_hint: String,
}

impl DoctorHandoffRow {
    /// Create a row indicating the handler was not found.
    pub fn absent(platform: &str, handler_name: &str, install_hint: &str) -> Self {
        Self {
            platform: platform.to_string(),
            handler_available: false,
            handler_name: handler_name.to_string(),
            install_hint: install_hint.to_string(),
        }
    }

    /// Create a row indicating the handler is available.
    pub fn available(platform: &str, handler_name: &str) -> Self {
        Self {
            platform: platform.to_string(),
            handler_available: true,
            handler_name: handler_name.to_string(),
            install_hint: String::new(),
        }
    }
}

/// Probe for platform handler availability.
pub fn probe_open_handler() -> DoctorHandoffRow {
    let platform = std::env::consts::OS.to_string();

    #[cfg(target_os = "linux")]
    {
        if which_binary("xdg-open").is_some() {
            return DoctorHandoffRow::available(&platform, "xdg-open");
        }
        DoctorHandoffRow::absent(
            &platform,
            "xdg-open",
            "Install xdg-utils: sudo apt install xdg-utils",
        )
    }

    #[cfg(target_os = "macos")]
    {
        // `open` is part of macOS — always available.
        return DoctorHandoffRow::available(&platform, "open");
    }

    #[cfg(target_os = "windows")]
    {
        // `cmd /C start` is part of Windows — always available.
        return DoctorHandoffRow::available(&platform, "cmd /C start");
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        return DoctorHandoffRow::absent(
            &platform,
            "platform default",
            &format!(
                "No default opener for platform '{}'. Install a file opener or use --app to specify one.",
                platform
            ),
        );
    }
}

/// Check if a binary exists on PATH.
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

/// Open a file path using the platform's default handler or a specified app.
///
/// Returns the path that was opened (or the app path if --app was used).
/// Never blocks — spawns the process and returns immediately.
pub fn open_path(path: &std::path::Path, app: Option<&str>) -> OpenResult {
    let app = app.map(|a| a.to_string());

    match app.as_deref() {
        Some("default") | None => open_platform(path),
        Some(app_name) => open_with_app(path, app_name),
    }
}

/// Open using the platform default opener.
fn open_platform(path: &std::path::Path) -> OpenResult {
    let path_str = path.to_string_lossy().to_string();

    #[cfg(target_os = "linux")]
    {
        if which_binary("xdg-open").is_none() {
            return OpenResult::absent(
                "linux",
                "xdg-open",
                "Install xdg-utils: sudo apt install xdg-utils",
                &path_str,
            );
        }
        match std::process::Command::new("xdg-open")
            .arg(&path_str)
            .spawn()
        {
            Ok(child) => OpenResult::launched(&path_str, Some(child.id())),
            Err(e) => {
                OpenResult::error(&path_str, Some(format!("xdg-open failed to launch: {}", e)))
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        match std::process::Command::new("open").arg(&path_str).spawn() {
            Ok(child) => OpenResult::launched(&path_str, Some(child.id())),
            Err(e) => OpenResult::error(&path_str, Some(format!("open failed to launch: {}", e))),
        }
    }

    #[cfg(target_os = "windows")]
    {
        match std::process::Command::new("cmd")
            .args(&["/C", "start", "", &path_str])
            .spawn()
        {
            Ok(child) => OpenResult::launched(&path_str, Some(child.id())),
            Err(e) => OpenResult::error(
                &path_str,
                Some(format!("cmd start failed to launch: {}", e)),
            ),
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        OpenResult::absent(
            &std::env::consts::OS.to_string(),
            "platform default",
            &format!(
                "No platform opener for '{}'. Try --app to specify a handler.",
                std::env::consts::OS
            ),
            &path_str,
        )
    }
}

/// Open using a specific application (e.g. "libreoffice").
fn open_with_app(path: &std::path::Path, app_name: &str) -> OpenResult {
    let path_str = path.to_string_lossy().to_string();

    match std::process::Command::new(app_name).arg(&path_str).spawn() {
        Ok(child) => OpenResult::launched(&format!("{} {}", app_name, path_str), Some(child.id())),
        Err(e) => {
            let hint = if e.kind() == std::io::ErrorKind::NotFound {
                format!("{} not found — install it or use --app default", app_name)
            } else {
                format!("{} failed to launch: {}", app_name, e)
            };
            OpenResult::error(&path_str, Some(hint))
        }
    }
}

/// Result of an open_path call.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OpenResult {
    /// Whether the opener was launched.
    pub launched: bool,
    /// The command string that was (or would be) executed.
    pub command: String,
    /// Process ID if spawned, None if not launched.
    pub pid: Option<u32>,
    /// Hint for the user when something went wrong.
    pub error_hint: Option<String>,
}

impl OpenResult {
    pub fn launched(command: &str, pid: Option<u32>) -> Self {
        Self {
            launched: true,
            command: command.to_string(),
            pid,
            error_hint: None,
        }
    }

    pub fn absent(platform: &str, handler_name: &str, install_hint: &str, path: &str) -> Self {
        Self {
            launched: false,
            command: path.to_string(),
            pid: None,
            error_hint: Some(format!(
                "No opener on {} ({}). {}",
                platform, handler_name, install_hint
            )),
        }
    }

    pub fn error(path: &str, hint: Option<String>) -> Self {
        Self {
            launched: false,
            command: path.to_string(),
            pid: None,
            error_hint: hint,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// test_probe_returns_platform: probe returns the current OS.
    #[test]
    fn test_probe_returns_platform() {
        let row = probe_open_handler();
        assert_eq!(row.platform, std::env::consts::OS);
        // The struct must be valid regardless of platform
        let _json = serde_json::to_string(&row).unwrap();
    }

    /// test_doctor_handoff_serializes: DoctorHandoffRow round-trips via JSON.
    #[test]
    fn test_doctor_handoff_serializes() {
        let row = DoctorHandoffRow::available("linux", "xdg-open");
        let json = serde_json::to_string(&row).unwrap();
        let back: DoctorHandoffRow = serde_json::from_str(&json).unwrap();
        assert_eq!(row.platform, back.platform);
        assert_eq!(row.handler_available, back.handler_available);
        assert_eq!(row.handler_name, back.handler_name);
    }

    /// test_open_result_serializes: OpenResult round-trips via JSON.
    #[test]
    fn test_open_result_serializes() {
        let result = OpenResult::launched("xdg-open /tmp/test.docx", Some(1234));
        let json = serde_json::to_string(&result).unwrap();
        let back: OpenResult = serde_json::from_str(&json).unwrap();
        assert!(back.launched);
        assert_eq!(back.pid, Some(1234));
    }

    /// test_open_absent: OpenResult::absent produces correct structure.
    #[test]
    fn test_open_absent() {
        let result = OpenResult::absent("linux", "xdg-open", "install hint", "/tmp/test");
        assert!(!result.launched);
        assert_eq!(
            result.error_hint.as_deref(),
            Some("No opener on linux (xdg-open). install hint")
        );
        assert!(result.pid.is_none());
    }

    /// test_open_with_app_error: opening with missing app returns error.
    #[test]
    fn test_open_with_app_error() {
        let result = open_path(
            std::path::Path::new("/tmp/nonexistent.docx"),
            Some("nonexistent_app_xyz"),
        );
        assert!(!result.launched);
        assert!(result.error_hint.is_some());
        assert!(result.error_hint.as_deref().unwrap().contains("not found"));
    }

    /// test_probe_handler_struct: probe returns valid DoctorHandoffRow.
    #[test]
    fn test_probe_handler_struct() {
        let row = probe_open_handler();
        assert!(!row.platform.is_empty());
        assert!(!row.handler_name.is_empty());
        // On headless systems, handler may or may not be available
        // but the struct should always be valid
        let _json = serde_json::to_string(&row).unwrap();
    }
}
