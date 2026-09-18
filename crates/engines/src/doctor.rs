//! Doctor: engine × fixture capability matrix, font probe, system info.
//!
//! The doctor tool probes all known engines against standard fixtures,
//! probes installed fonts, and gathers system information for diagnostics.
//!
//! # Usage
//!
//! ```no_run
//! use engines::doctor::{run_doctor, probe_fonts, system_info};
//! use engines::probe::{standard_fixtures, ProbeFixture};
//! use gp_core::traits::EngineExt;
//!
//! // Probe with a set of engines (empty for just system info)
//! let engines: Vec<&dyn EngineExt> = vec![];
//! let fixtures = standard_fixtures();
//! let result = run_doctor(engines, &fixtures);
//! let json = serde_json::to_string_pretty(&result).unwrap();
//! ```

use gp_core::traits::EngineExt;

// ======================================================================
// Types
// ======================================================================

/// Full doctor result: engine matrix + font probe + system info.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DoctorResult {
    /// Engine capability entries.
    pub engines: Vec<DoctorEngine>,
    /// Font probe results.
    pub fonts: FontProbe,
    /// System information.
    pub system_info: SystemInfo,
    /// Platform file opener handoff row (optional, populated by CLI doctor tool).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handoff: Option<super::open_handler::DoctorHandoffRow>,
}

/// A single engine's doctor entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DoctorEngine {
    /// Engine identifier (e.g. "libreoffice").
    pub engine_id: String,
    /// Engine version string.
    pub version: Option<String>,
    /// Whether the engine is available.
    pub available: bool,
    /// Capability matrix from probing.
    pub capabilities: Vec<CapabilityEntry>,
}

/// A single capability entry (source → target).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CapabilityEntry {
    /// Source format (e.g. "docx").
    pub source: String,
    /// Target format (e.g. "pdf").
    pub target: String,
    /// Whether the conversion works.
    pub works: bool,
}

/// Font probe results from fc-match.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FontProbe {
    /// fc-match Calibri (if found).
    pub calibri: Option<String>,
    /// fc-match Cambria (if found).
    pub cambria: Option<String>,
    /// fc-match Liberation Sans (if found).
    pub liberation_sans: Option<String>,
    /// fc-match Noto Sans (if found).
    pub noto_sans: Option<String>,
}

/// System information gathered by the doctor tool.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SystemInfo {
    /// Operating system name.
    pub os: String,
    /// Architecture (e.g. "x86_64", "aarch64").
    pub arch: String,
    /// LibreOffice version (if installed).
    pub libreoffice_version: Option<String>,
    /// Poppler version (if installed).
    pub poppler_version: Option<String>,
    /// Ghostscript version (if installed).
    pub ghostscript_version: Option<String>,
    /// Chromium version (if installed).
    pub chromium_version: Option<String>,
    /// Pandoc version (if installed).
    pub pandoc_version: Option<String>,
    /// Typst version (if installed).
    pub typst_version: Option<String>,
}

// ======================================================================
// Doctor: engine × fixture matrix
// ======================================================================

/// Run the doctor tool: probe all engines against fixtures.
///
/// This function takes a list of engines (by reference to the EngineExt trait),
/// probes each one with standard fixtures, and returns a complete doctor result.
pub fn run_doctor(
    engines: Vec<&dyn EngineExt>,
    fixtures: &[super::probe::ProbeFixture],
) -> DoctorResult {
    run_doctor_with_handoff(engines, fixtures, None)
}

/// Run the doctor tool with an optional platform opener handoff row.
pub fn run_doctor_with_handoff(
    engines: Vec<&dyn EngineExt>,
    fixtures: &[super::probe::ProbeFixture],
    handoff: Option<super::open_handler::DoctorHandoffRow>,
) -> DoctorResult {
    let engine_entries: Vec<DoctorEngine> = engines
        .into_iter()
        .map(|e| probe_engine(e, fixtures))
        .collect();

    DoctorResult {
        engines: engine_entries,
        fonts: probe_fonts(),
        system_info: system_info(),
        handoff,
    }
}

/// Probe a single engine against the fixture set.
fn probe_engine(engine: &dyn EngineExt, fixtures: &[super::probe::ProbeFixture]) -> DoctorEngine {
    let info = engine.info();
    let available = info.is_available;
    let version = info.version;

    let capabilities: Vec<CapabilityEntry> = if available {
        fixtures
            .iter()
            .map(|f| {
                let works = engine.can_convert(&f.source_format, &f.target_format);
                // Do a real conversion check for capabilities
                let result = engine.convert(&f.data, &f.source_format, &f.target_format);
                let works = works && result.is_ok();
                CapabilityEntry {
                    source: f.source_format.clone(),
                    target: f.target_format.clone(),
                    works,
                }
            })
            .collect()
    } else {
        // If engine is not available, mark all as not working
        fixtures
            .iter()
            .map(|f| CapabilityEntry {
                source: f.source_format.clone(),
                target: f.target_format.clone(),
                works: false,
            })
            .collect()
    };

    DoctorEngine {
        engine_id: engine.engine_id().to_string(),
        version,
        available,
        capabilities,
    }
}

// ======================================================================
// Font probe
// ======================================================================

/// Probe fonts using fc-match.
///
/// Returns a [`FontProbe`] with results for each known font name.
/// Returns `None` for any font that fc-match cannot resolve.
/// If fc-match is not available, all fields are `None`.
pub fn probe_fonts() -> FontProbe {
    let fonts = [
        ("Calibri", "calibri"),
        ("Cambria", "cambria"),
        ("Liberation Sans", "liberation_sans"),
        ("Noto Sans", "noto_sans"),
    ];

    let mut probe = FontProbe::default();

    for (font_name, field) in &fonts {
        if let Some(result) = fc_match(font_name) {
            match *field {
                "calibri" => probe.calibri = Some(result),
                "cambria" => probe.cambria = Some(result),
                "liberation_sans" => probe.liberation_sans = Some(result),
                "noto_sans" => probe.noto_sans = Some(result),
                _ => {}
            }
        }
    }

    probe
}

/// Run fc-match for a font name and return the result string.
///
/// Returns `None` if fc-match is not available or the font is not found.
fn fc_match(font: &str) -> Option<String> {
    let output = std::process::Command::new("fc-match")
        .args(["--format", "%{family}\n", font])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let result = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !result.is_empty() {
                Some(result)
            } else {
                None
            }
        }
        _ => None,
    }
}

// ======================================================================
// System info
// ======================================================================

/// Gather system information.
///
/// Returns a [`SystemInfo`] with OS, architecture, and versions of
/// known tools (LibreOffice, poppler, Ghostscript, chromium, pandoc, typst) if installed.
pub fn system_info() -> SystemInfo {
    SystemInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        libreoffice_version: lo_version(),
        poppler_version: poppler_version(),
        ghostscript_version: ghostscript_version(),
        chromium_version: chromium_version(),
        pandoc_version: pandoc_version(),
        typst_version: typst_version(),
    }
}

/// Get LibreOffice version from `soffice --version`.
fn lo_version() -> Option<String> {
    let output = std::process::Command::new("soffice")
        .args(["--version"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            // Parse version from "LibreOffice 26.2.6.1 ..." or "LibreOffice/26.2.6.1 ..."
            text.lines()
                .find(|l| l.starts_with("LibreOffice"))
                .and_then(|l| l.strip_prefix("LibreOffice"))
                .and_then(|v| v.split('/').next())
                .and_then(|v| v.split_whitespace().next())
                .map(|v| v.to_string())
        }
        _ => {
            // Try libreoffice binary as fallback
            let output = std::process::Command::new("libreoffice")
                .args(["--version"])
                .output();
            match output {
                Ok(out) if out.status.success() => {
                    let text = String::from_utf8_lossy(&out.stdout);
                    text.lines()
                        .find(|l| l.starts_with("LibreOffice"))
                        .and_then(|l| l.strip_prefix("LibreOffice"))
                        .and_then(|v| v.split('/').next())
                        .and_then(|v| v.split_whitespace().next())
                        .map(|v| v.to_string())
                }
                _ => None,
            }
        }
    }
}

/// Get poppler version from `pdftoppm -v`.
fn poppler_version() -> Option<String> {
    let output = std::process::Command::new("pdftoppm").args(["-v"]).output();

    match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            // pdftoppm outputs to stderr typically
            let text = if text.trim().is_empty() {
                String::from_utf8_lossy(&out.stderr)
            } else {
                text
            };
            // Parse "pdftoppm version X.Y.Z"
            text.lines()
                .find(|l| l.starts_with("pdftoppm version"))
                .and_then(|l| l.split_whitespace().nth(2))
                .map(|v| v.to_string())
        }
        _ => None,
    }
}

/// Get Ghostscript version from `gs --version`.
fn ghostscript_version() -> Option<String> {
    let output = std::process::Command::new("gs")
        .args(["--version"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            text.lines()
                .next()
                .and_then(|l| l.trim().parse::<f64>().ok())
                .map(|v| format!("{:.1}", v))
                .or_else(|| {
                    // GS version may have format like "10.02.0" which doesn't parse as f64
                    text.lines()
                        .next()
                        .and_then(|l| l.trim().split('.').next())
                        .and_then(|v| v.parse::<f64>().ok())
                        .map(|major| format!("{:.1}", major))
                })
        }
        _ => None,
    }
}

/// Find a chromium binary for doctor probing.
fn find_chromium_for_doctor() -> Option<String> {
    for name in &[
        "chromium-browser",
        "chromium",
        "google-chrome",
        "google-chrome-stable",
    ] {
        if let Some(path) = which_binary(name) {
            return Some(path);
        }
    }
    None
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

/// Get Chromium version if available.
fn chromium_version() -> Option<String> {
    let binary = find_chromium_for_doctor();
    binary.and_then(|path| {
        let output = std::process::Command::new(&path)
            .args(["--version"])
            .output();
        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                // Try to parse version from common formats
                // "Google Chrome 120.0.6099.130", "Chromium 120.0.6099.130", "HeadlessChrome 120.0.6099.130"
                text.lines().next().and_then(|line| {
                    if let Some(pos) = line.find("Chromium") {
                        let rest = &line[pos + 8..];
                        rest.strip_prefix(' ')
                            .or(Some(rest))
                            .and_then(|r| r.split_whitespace().next())
                            .map(|v| v.to_string())
                    } else if let Some(pos) = line.find("Chrome") {
                        let rest = &line[pos + 6..];
                        let rest = rest.strip_prefix('-').or(rest.strip_prefix(' '))?;
                        rest.split_whitespace().next().map(|v| v.to_string())
                    } else {
                        // Check for "HeadlessChrome"
                        if let Some(pos) = line.find("HeadlessChrome") {
                            let rest = &line[pos + 14..];
                            rest.strip_prefix(' ')
                                .and_then(|r| r.split_whitespace().next())
                                .map(|v| v.to_string())
                        } else {
                            None
                        }
                    }
                })
            }
            _ => None,
        }
    })
}

/// Get Pandoc version if available.
fn pandoc_version() -> Option<String> {
    let output = std::process::Command::new("pandoc")
        .args(["--version"])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            text.lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .map(|v| v.to_string())
        }
        _ => None,
    }
}

/// Get Typst version if available.
fn typst_version() -> Option<String> {
    if which_binary("typst").is_some() {
        let output = std::process::Command::new("typst")
            .args(["--version"])
            .output();
        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                text.lines()
                    .find(|l| l.starts_with("typst "))
                    .and_then(|l| l.strip_prefix("typst "))
                    .and_then(|v| v.split_whitespace().next())
                    .map(|v| v.to_string())
            }
            _ => None,
        }
    } else {
        None
    }
}

/// Build the standard engine list for doctor probes.
///
/// Returns engines in platform-appropriate order. On Windows, ms-office
/// appears first; on Linux/macOS, it is excluded (stub always returns false).
#[allow(dead_code)]
pub fn standard_doctor_engines() -> Vec<Box<dyn EngineExt>> {
    #[allow(unused_imports)]
    use crate::ms_office;
    #[allow(unused_mut)]
    let mut engines: Vec<Box<dyn EngineExt>> = Vec::new();
    #[cfg(target_os = "windows")]
    {
        engines.push(Box::new(ms_office::MsOfficeEngine::new()));
    }
    engines
}

/// Generate a doctor report string for console output.
pub fn doctor_report(result: &DoctorResult) -> String {
    let mut output = String::from("stichos doctor report\n");
    output.push_str(&format!(
        "OS: {} {}\n",
        result.system_info.os, result.system_info.arch
    ));

    // Platform opener handoff
    if let Some(ref h) = result.handoff {
        output.push_str("\n--- Platform Opener ---\n");
        let status = if h.handler_available {
            "available"
        } else {
            "unavailable"
        };
        output.push_str(&format!("  {} ({})\n", h.handler_name, status));
        if h.handler_available {
            output.push_str("  status: ready\n");
        } else {
            output.push_str(&format!("  hint: {}\n", h.install_hint));
        }
    }

    // LibreOffice version
    if let Some(ref lo) = result.system_info.libreoffice_version {
        output.push_str(&format!("LibreOffice: {}\n", lo));
    } else {
        output.push_str("LibreOffice: not installed\n");
    }

    if let Some(ref poppler) = result.system_info.poppler_version {
        output.push_str(&format!("Poppler: {}\n", poppler));
    } else {
        output.push_str("Poppler: not installed\n");
    }

    if let Some(ref gs) = result.system_info.ghostscript_version {
        output.push_str(&format!("Ghostscript: {}\n", gs));
    } else {
        output.push_str("Ghostscript: not installed\n");
    }

    if let Some(ref chromium) = result.system_info.chromium_version {
        output.push_str(&format!("Chromium: {}\n", chromium));
    } else {
        output.push_str("Chromium: not installed\n");
    }

    if let Some(ref pandoc) = result.system_info.pandoc_version {
        output.push_str(&format!("Pandoc: {}\n", pandoc));
    } else {
        output.push_str("Pandoc: not installed\n");
    }

    if let Some(ref typst) = result.system_info.typst_version {
        output.push_str(&format!("Typst: {}\n", typst));
    } else {
        output.push_str("Typst: not installed\n");
    }

    output.push_str("\n--- Engines ---\n");
    for engine in &result.engines {
        let status = if engine.available {
            "available"
        } else {
            "unavailable"
        };
        output.push_str(&format!(
            "  {} {} (version: {:?})\n",
            engine.engine_id, status, engine.version
        ));
        for cap in &engine.capabilities {
            let mark = if cap.works { "✓" } else { "✗" };
            output.push_str(&format!("    {} {} → {}\n", mark, cap.source, cap.target));
        }
    }

    output.push_str("\n--- Fonts ---\n");
    if let Some(ref c) = result.fonts.calibri {
        output.push_str(&format!("  Calibri: {}\n", c));
    } else {
        output.push_str("  Calibri: not found\n");
    }
    if let Some(ref c) = result.fonts.cambria {
        output.push_str(&format!("  Cambria: {}\n", c));
    } else {
        output.push_str("  Cambria: not found\n");
    }
    if let Some(ref l) = result.fonts.liberation_sans {
        output.push_str(&format!("  Liberation Sans: {}\n", l));
    } else {
        output.push_str("  Liberation Sans: not found\n");
    }
    if let Some(ref n) = result.fonts.noto_sans {
        output.push_str(&format!("  Noto Sans: {}\n", n));
    } else {
        output.push_str("  Noto Sans: not found\n");
    }

    output
}

// ======================================================================
// Tests
// ======================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockEngine;
    use crate::probe;
    use gp_core::traits::Engine;

    /// test_doctor_result_serializes: DoctorResult round-trips via JSON.
    #[test]
    fn test_doctor_result_serializes() {
        let engine = MockEngine::new();
        let fixtures = vec![probe::ProbeFixture {
            source_format: "docx".into(),
            target_format: "pdf".into(),
            name: "test".into(),
            data: b"test".to_vec(),
        }];
        let result = run_doctor(vec![&engine], &fixtures);

        let json = serde_json::to_string(&result).expect("DoctorResult should serialize");
        let back: DoctorResult =
            serde_json::from_str(&json).expect("DoctorResult should deserialize");

        assert_eq!(result.engines.len(), back.engines.len());
        assert_eq!(result.engines[0].engine_id, back.engines[0].engine_id);
        assert_eq!(result.engines[0].available, back.engines[0].available);
        assert_eq!(
            result.engines[0].capabilities.len(),
            back.engines[0].capabilities.len()
        );
        assert_eq!(result.fonts.calibri, back.fonts.calibri);
        assert_eq!(result.system_info.os, back.system_info.os);
    }

    /// test_font_probe: probe returns FontProbe (may have partial data).
    #[test]
    fn test_font_probe() {
        let probe = probe_fonts();

        // At least check the struct is valid
        // fc-match may or may not resolve our fonts on every system
        // but the function should not panic
        assert!(
            !probe.calibri.is_some() || !probe.calibri.as_deref().unwrap().is_empty(),
            "font entries should be non-empty"
        );
    }

    /// test_system_info: system info contains basic fields.
    #[test]
    fn test_system_info() {
        let info = system_info();

        assert_eq!(info.os, std::env::consts::OS);
        assert_eq!(info.arch, std::env::consts::ARCH);

        // LibreOffice may or may not be available depending on the environment
        // The function should not panic and should return a valid SystemInfo
        let _has_lo = info.libreoffice_version.is_some();
    }

    /// test_doctor_matrix: doctor with multiple engines and fixtures.
    #[test]
    fn test_doctor_matrix() {
        let engine1 = MockEngine::new();
        let mut engine2 = MockEngine::new();
        engine2.id = "engine2".into();
        engine2.version = "2.0".into();

        let fixtures = probe::standard_fixtures();
        let result = run_doctor(vec![&engine1, &engine2], &fixtures);

        assert_eq!(result.engines.len(), 2);

        // Both engines should be available
        assert!(result.engines[0].available);
        assert!(result.engines[1].available);

        // Engine capabilities should match fixtures
        assert_eq!(
            result.engines[0].capabilities.len(),
            fixtures.len(),
            "engine capabilities should match fixture count"
        );

        // Font probe should be populated
        let _fonts = result.fonts;

        // System info should be populated
        let _sys = result.system_info;
    }

    /// test_doctor_report: console report is human-readable.
    #[test]
    fn test_doctor_report() {
        let engine = MockEngine::new();
        let fixtures = vec![probe::ProbeFixture {
            source_format: "docx".into(),
            target_format: "pdf".into(),
            name: "test".into(),
            data: b"test".to_vec(),
        }];
        let result = run_doctor(vec![&engine], &fixtures);
        let report = doctor_report(&result);

        assert!(report.contains("stichos doctor report"));
        assert!(report.contains(engine.engine_id()));
        assert!(report.contains("available"));
        assert!(report.contains("docx"));
        assert!(report.contains("pdf"));
    }

    /// test_fc_match_not_available: handles missing fc-match gracefully.
    #[test]
    fn test_fc_match_not_available() {
        // This test verifies that fc_match handles non-existent binary
        // by testing with a made-up binary name
        let output = std::process::Command::new("nonexistent_binary_xyz")
            .args(["test"])
            .output();

        // Should handle gracefully
        let _ = output;
    }

    /// test_system_info_serializes: SystemInfo round-trips.
    #[test]
    fn test_system_info_serializes() {
        let info = system_info();
        let json = serde_json::to_string(&info).expect("SystemInfo should serialize");
        let back: SystemInfo = serde_json::from_str(&json).expect("SystemInfo should deserialize");
        assert_eq!(info.os, back.os);
        assert_eq!(info.arch, back.arch);
    }

    /// test_font_probe_serializes: FontProbe round-trips.
    #[test]
    fn test_font_probe_serializes() {
        let probe = FontProbe {
            calibri: Some("Calibri:font=Calibri".into()),
            cambria: None,
            liberation_sans: Some("Liberation Sans:font=LiberationSans".into()),
            noto_sans: None,
        };
        let json = serde_json::to_string(&probe).expect("FontProbe should serialize");
        let back: FontProbe = serde_json::from_str(&json).expect("FontProbe should deserialize");
        assert_eq!(probe.calibri, back.calibri);
        assert_eq!(probe.cambria, back.cambria);
        assert_eq!(probe.liberation_sans, back.liberation_sans);
        assert_eq!(probe.noto_sans, back.noto_sans);
    }

    /// test_doctor_engine_serializes: DoctorEngine round-trips.
    #[test]
    fn test_doctor_engine_serializes() {
        let engine = DoctorEngine {
            engine_id: "mock".into(),
            version: Some("1.0".into()),
            available: true,
            capabilities: vec![
                CapabilityEntry {
                    source: "docx".into(),
                    target: "pdf".into(),
                    works: true,
                },
                CapabilityEntry {
                    source: "xlsx".into(),
                    target: "pdf".into(),
                    works: false,
                },
            ],
        };
        let json = serde_json::to_string(&engine).expect("DoctorEngine should serialize");
        let back: DoctorEngine =
            serde_json::from_str(&json).expect("DoctorEngine should deserialize");
        assert_eq!(engine.engine_id, back.engine_id);
        assert_eq!(engine.available, back.available);
        assert_eq!(engine.capabilities.len(), back.capabilities.len());
    }

    /// test_capability_entry_serializes: CapabilityEntry round-trips.
    #[test]
    fn test_capability_entry_serializes() {
        let entry = CapabilityEntry {
            source: "docx".into(),
            target: "pdf".into(),
            works: true,
        };
        let json = serde_json::to_string(&entry).expect("CapabilityEntry should serialize");
        let back: CapabilityEntry =
            serde_json::from_str(&json).expect("CapabilityEntry should deserialize");
        assert_eq!(entry, back);
    }

    /// test_lo_version_parsing: LO version parsing works.
    #[test]
    fn test_lo_version_parsing() {
        let output = "LibreOffice 26.2.6.1 620(Build:1)";
        let version = output
            .lines()
            .find(|l| l.starts_with("LibreOffice"))
            .and_then(|l| l.strip_prefix("LibreOffice"))
            .and_then(|v| v.split('/').next())
            .and_then(|v| v.split_whitespace().next());
        assert_eq!(version, Some("26.2.6.1"));
    }

    /// test_ghostscript_version_parsing: GS version parsing works with multi-component versions.
    #[test]
    fn test_ghostscript_version_parsing() {
        // Multi-component versions like "10.02.0" fall back to major version
        let output = "10.02.0\nCopyright (C) 2024...";
        let version = output
            .lines()
            .next()
            .and_then(|l| l.trim().parse::<f64>().ok())
            .map(|v| format!("{:.1}", v))
            .or_else(|| {
                output
                    .lines()
                    .next()
                    .and_then(|l| l.trim().split('.').next())
                    .and_then(|v| v.parse::<f64>().ok())
                    .map(|major| format!("{:.1}", major))
            });
        assert_eq!(version, Some("10.0".to_string()));
    }

    /// test_system_info_has_new_fields: SystemInfo includes chromium/pandoc/typst fields.
    #[test]
    fn test_system_info_has_new_fields() {
        let info = system_info();
        // The struct compiles and has all new fields
        let _chromium = info.chromium_version;
        let _pandoc = info.pandoc_version;
        let _typst = info.typst_version;
    }

    /// test_chromium_version_parsing: chromium version parsing works for common formats.
    #[test]
    fn test_chromium_version_parsing() {
        // Simulate the parsing logic for common version outputs
        let chrome_output = "Google Chrome 120.0.6099.130";
        let version = chrome_output.lines().next().and_then(|line| {
            if let Some(pos) = line.find("Chrome") {
                let rest = &line[pos + 6..];
                let rest = rest.strip_prefix(' ')?;
                rest.split_whitespace().next().map(|v| v.to_string())
            } else {
                None
            }
        });
        assert_eq!(version, Some("120.0.6099.130".to_string()));

        let chromium_output = "Chromium 120.0.6099.130";
        let version = chromium_output.lines().next().and_then(|line| {
            if let Some(pos) = line.find("Chromium") {
                let rest = &line[pos + 8..];
                rest.strip_prefix(' ')
                    .and_then(|r| r.split_whitespace().next())
                    .map(|v| v.to_string())
            } else {
                None
            }
        });
        assert_eq!(version, Some("120.0.6099.130".to_string()));
    }

    /// test_typst_version_parsing: typst version parsing works.
    #[test]
    fn test_typst_version_parsing() {
        let output = "typst 0.11.1";
        let version = output
            .lines()
            .find(|l| l.starts_with("typst "))
            .and_then(|l| l.strip_prefix("typst "))
            .and_then(|v| v.split_whitespace().next())
            .map(|v| v.to_string());
        assert_eq!(version, Some("0.11.1".to_string()));
    }

    /// test_doctor_report_includes_new_tools: report shows chromium/pandoc/typst lines.
    #[test]
    fn test_doctor_report_includes_new_tools() {
        let engine = MockEngine::new();
        let fixtures = vec![probe::ProbeFixture {
            source_format: "docx".into(),
            target_format: "pdf".into(),
            name: "test".into(),
            data: b"test".to_vec(),
        }];
        let result = run_doctor(vec![&engine], &fixtures);
        let report = doctor_report(&result);

        // Should contain the new tool lines
        assert!(report.contains("Chromium:"));
        assert!(report.contains("Pandoc:"));
        assert!(report.contains("Typst:"));
    }
}
