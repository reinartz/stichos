// SPDX-License-Identifier: MIT OR Apache-2.0
//! MS Office engine — compile-only skeleton for COM interop.
//!
//! This module provides the `MsOfficeEngine` implementation for Windows COM automation.
//! On non-Windows platforms, or when the `ms-office` feature is disabled, all methods
//! return stub values — this is a **compile-only skeleton**.
//!
//! Runtime COM implementation requires:
//! - Windows host with MS Office installed
//! - The `windows` crate for COM interop (Phase 5)
//! - A separate worker process (see ADR-0008)
//!
//! # Feature flag
//! Enable with `--features ms-office` on Windows. Default is disabled.
//!
//! # Process model
//! Each ms-office engine instance spawns a separate `stichos-com-worker.exe` process
//! that initializes COM (`CoInitializeEx`), creates Word/Excel/PowerPoint application
//! objects via COM, performs conversion, and exits. This avoids in-process COM apartment
//! threading issues. See ADR-0008 for the full design.

use gp_core::diagnostics::Diagnostic;
use gp_core::traits::{CapabilityEntry, Engine, EngineExt, EngineInfo};

/// Engine identifier for the MS Office engine.
pub const MS_OFFICE_ENGINE_ID: &str = "ms-office";

/// MS Office engine — COM automation worker.
///
/// On non-Windows or without the `ms-office` feature, `available` is always false
/// and all conversion methods return stub/error results.
#[derive(Debug, Clone)]
pub struct MsOfficeEngine {
    /// Whether MS Office is available on this system.
    pub available: bool,
    /// Detected Word version string (Windows + feature only).
    #[cfg(all(feature = "ms-office", target_os = "windows"))]
    pub word_version: Option<String>,
    /// Detected Excel version string (Windows + feature only).
    #[cfg(all(feature = "ms-office", target_os = "windows"))]
    pub excel_version: Option<String>,
    /// Detected PowerPoint version string (Windows + feature only).
    #[cfg(all(feature = "ms-office", target_os = "windows"))]
    pub powerpoint_version: Option<String>,
}

impl MsOfficeEngine {
    /// Create a new MS Office engine.
    ///
    /// On non-Windows or without the `ms-office` feature, returns a stub engine
    /// with `available = false`. On Windows with the feature enabled, probes
    /// for installed Office applications (Phase 5).
    pub fn new() -> Self {
        MsOfficeEngine {
            available: false,
            #[cfg(all(feature = "ms-office", target_os = "windows"))]
            word_version: None,
            #[cfg(all(feature = "ms-office", target_os = "windows"))]
            excel_version: None,
            #[cfg(all(feature = "ms-office", target_os = "windows"))]
            powerpoint_version: None,
        }
    }

    /// Return the detected Word version, if available.
    #[allow(unreachable_code)]
    pub fn word_version(&self) -> Option<&str> {
        #[cfg(all(feature = "ms-office", target_os = "windows"))]
        {
            self.word_version.as_deref()
        }
        #[cfg(not(all(feature = "ms-office", target_os = "windows")))]
        None
    }

    /// Return the detected Excel version, if available.
    #[allow(unreachable_code)]
    pub fn excel_version(&self) -> Option<&str> {
        #[cfg(all(feature = "ms-office", target_os = "windows"))]
        {
            self.excel_version.as_deref()
        }
        #[cfg(not(all(feature = "ms-office", target_os = "windows")))]
        None
    }

    /// Return the detected PowerPoint version, if available.
    #[allow(unreachable_code)]
    pub fn powerpoint_version(&self) -> Option<&str> {
        #[cfg(all(feature = "ms-office", target_os = "windows"))]
        {
            self.powerpoint_version.as_deref()
        }
        #[cfg(not(all(feature = "ms-office", target_os = "windows")))]
        None
    }

    /// Probe for MS Office COM availability.
    ///
    /// On non-Windows or without the `ms-office` feature, returns all false.
    /// On Windows with the feature, checks registry/COM ProgID probes.
    pub fn probe() -> MsOfficeProbeResult {
        MsOfficeProbeResult::stub()
    }
}

impl Default for MsOfficeEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine for MsOfficeEngine {
    fn engine_id(&self) -> &str {
        MS_OFFICE_ENGINE_ID
    }

    fn version(&self) -> Option<&str> {
        #[cfg(all(feature = "ms-office", target_os = "windows"))]
        {
            self.word_version
                .as_deref()
                .or_else(|| self.excel_version.as_deref())
                .or_else(|| self.powerpoint_version.as_deref())
        }
        #[cfg(not(all(feature = "ms-office", target_os = "windows")))]
        None
    }

    fn can_convert(&self, source: &str, target: &str) -> bool {
        // Stub: the supported conversions would be:
        // docx->pdf, xlsx->pdf, pptx->pdf, pdf->png, png->pdf
        let supported_sources = ["docx", "xlsx", "pptx"];
        let supported_targets = ["pdf", "png"];
        let _ = supported_sources;
        let _ = supported_targets;
        // Always return false in stub mode
        #[cfg(not(all(feature = "ms-office", target_os = "windows")))]
        {
            let _ = source;
            let _ = target;
            false
        }
        #[cfg(all(feature = "ms-office", target_os = "windows"))]
        {
            self.available
                && supported_sources.contains(&source)
                && supported_targets.contains(&target)
                && source != target
        }
    }

    fn convert(
        &self,
        input: &[u8],
        source: &str,
        target: &str,
    ) -> Result<Vec<u8>, Vec<Diagnostic>> {
        let _ = input;
        let _ = source;
        let _ = target;
        Err(vec![Diagnostic::new(
            "ENG-E-001",
            gp_core::diagnostics::Severity::Error,
            "MS Office engine requires Windows + `ms-office` feature (compile-only skeleton)",
        )])
    }

    fn is_available(&self) -> bool {
        self.available
    }
}

impl EngineExt for MsOfficeEngine {
    fn info(&self) -> EngineInfo {
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
            engine_id: MS_OFFICE_ENGINE_ID.into(),
            version: self.version().map(String::from),
            fontset_hash: None,
            is_available: self.available,
            capabilities,
        }
    }

    fn fontset_hash(&self) -> Option<String> {
        None
    }

    fn supported_conversions(&self) -> Vec<(String, String)> {
        // MS Office supported conversion paths (stub order)
        vec![
            ("docx".into(), "pdf".into()),
            ("xlsx".into(), "pdf".into()),
            ("pptx".into(), "pdf".into()),
            ("pdf".into(), "png".into()),
            ("png".into(), "pdf".into()),
        ]
    }
}

/// Probe results for MS Office COM availability.
///
/// Each field indicates whether the corresponding Office application
/// is detectable via COM ProgID or registry on this system.
#[derive(Debug, Clone, Default)]
pub struct MsOfficeProbeResult {
    /// Word.Application detected.
    pub word_available: bool,
    /// Excel.Application detected.
    pub excel_available: bool,
    /// PowerPoint.Application detected.
    pub powerpoint_available: bool,
    /// Detected Word version string.
    pub word_version: Option<String>,
    /// Detected Excel version string.
    pub excel_version: Option<String>,
    /// Detected PowerPoint version string.
    pub powerpoint_version: Option<String>,
}

impl MsOfficeProbeResult {
    /// Create a stub probe result (all false, no versions).
    fn stub() -> Self {
        Self::default()
    }

    /// Whether any Office application is available.
    pub fn any_available(&self) -> bool {
        self.word_available || self.excel_available || self.powerpoint_available
    }
}

/// Check if MS Office is available on this system.
///
/// On non-Windows or without the `ms-office` feature, returns `false`.
/// On Windows with the feature, probes COM ProgIDs at runtime.
pub fn ms_office_available() -> bool {
    MsOfficeEngine::probe().any_available()
}

// ===========================================================================
// T4 COM operations: POTX finish & slide export
// ===========================================================================

/// Result of deleting the first N slides from a presentation.
#[derive(Debug, Clone)]
pub struct DeleteSlidesResult {
    /// Path to the saved PPTX (samples deleted).
    pub saved_path: String,
    /// Number of slides deleted.
    pub deleted_count: u32,
    /// Remaining slide count.
    pub remaining_slides: u32,
}

/// Result of exporting all slides as PNGs.
#[derive(Debug, Clone)]
pub struct ExportSlidePngsResult {
    /// Directory containing the exported PNG files.
    pub out_dir: String,
    /// Number of slides exported.
    pub exported_count: u32,
}

/// Result of validating a document via Word/PowerPoint COM open.
#[derive(Debug, Clone)]
pub struct ValidateOpenResult {
    /// Whether the document opened successfully.
    pub ok: bool,
    /// Which application opened it: "word", "excel", or "powerpoint".
    pub app: String,
    /// Application version string (if available).
    pub app_version: Option<String>,
    /// Detail message: for Word = paragraph/revision count, for PowerPoint = slide count, for Excel = sheet count.
    pub detail: String,
}

/// Delete the first N slides from a presentation via PowerPoint COM.
///
/// Workflow:
/// 1. Open the presentation **non-readonly** (PowerPoint won't let you save otherwise).
/// 2. Delete slides back-to-front (index N-1 down to 0) to preserve indices.
/// 3. `SaveAs(path, 24)` — 24 = ppFormatPPTX.
/// 4. `SaveAs(pdf_path, 32)` — 32 = ppFormatPDF (optional, caller-supplied path).
/// 5. Close **without** saving (PowerPoint already saved via SaveAs).
/// 6. `Quit()` always, even on failure (no orphan POWERPNT).
///
/// This **must** go through COM — stripping ZIP entries leaves orphan parts,
/// dangling relationships, and the zero-slide pitfall (T1 OOX-E-025).
///
/// # Arguments
/// * `path` — Path to the .pptx file.
/// * `n` — Number of leading slides to delete.
/// * `pdf_path` — Optional path for PDF export (pass `None` to skip).
///
/// # Feature gate
/// `ms-office` feature on Windows. On non-Windows or without the feature,
/// returns an error diagnostic.
#[allow(unreachable_code)]
pub fn delete_slides_first_n(
    path: &str,
    n: u32,
    pdf_path: Option<&str>,
) -> Result<DeleteSlidesResult, Vec<Diagnostic>> {
    #[cfg(all(feature = "ms-office", target_os = "windows"))]
    {
        let _ = path;
        let _ = n;
        let _ = pdf_path;
        // TODO (T4.2): COM implementation via Stickey/Windows crate.
        // This is a stub — the real implementation spawns a stichos-com-worker
        // process that uses PowerPoint COM APIs.
        Err(vec![Diagnostic::new(
            "ENG-E-010",
            gp_core::diagnostics::Severity::Error,
            "delete_slides_first_n: COM implementation requires Windows + `ms-office` feature (stub)",
        )])
    }
    #[cfg(not(all(feature = "ms-office", target_os = "windows")))]
    {
        let _ = path;
        let _ = n;
        let _ = pdf_path;
        Err(vec![Diagnostic::new(
            "ENG-E-010",
            gp_core::diagnostics::Severity::Error,
            "delete_slides_first_n: requires Windows + `ms-office` feature (stub)",
        )])
    }
}

/// Export all slides from a presentation as PNG images via PowerPoint COM.
///
/// Uses `Slide.Export()` (not `SaveCopyAs`) — this is the reliable per-slide
/// PNG export method. Each slide is rendered at the requested width × height.
///
/// # Arguments
/// * `path` — Path to the .pptx file.
/// * `out_dir` — Directory to write PNG files into (created if absent).
/// * `w` — Width in pixels.
/// * `h` — Height in pixels.
///
/// # Feature gate
/// `ms-office` feature on Windows. On non-Windows or without the feature,
/// returns an error diagnostic.
#[allow(unreachable_code)]
pub fn export_slide_pngs(
    path: &str,
    out_dir: &str,
    w: u32,
    h: u32,
) -> Result<ExportSlidePngsResult, Vec<Diagnostic>> {
    #[cfg(all(feature = "ms-office", target_os = "windows"))]
    {
        let _ = path;
        let _ = out_dir;
        let _ = w;
        let _ = h;
        // TODO (T4.2): COM implementation via Stickey/Windows crate.
        Err(vec![Diagnostic::new(
            "ENG-E-011",
            gp_core::diagnostics::Severity::Error,
            "export_slide_pngs: COM implementation requires Windows + `ms-office` feature (stub)",
        )])
    }
    #[cfg(not(all(feature = "ms-office", target_os = "windows")))]
    {
        let _ = path;
        let _ = out_dir;
        let _ = w;
        let _ = h;
        Err(vec![Diagnostic::new(
            "ENG-E-011",
            gp_core::diagnostics::Severity::Error,
            "export_slide_pngs: requires Windows + `ms-office` feature (stub)",
        )])
    }
}

/// Validate a Word/Excel/PowerPoint document by opening it read-only via COM,
/// reading metadata, then closing without saving.
///
/// This is the runtime authority complement to static OPC validation (T1).
/// Word/PowerPoint can detect issues that static ZIP/XML analysis misses:
/// - Corrupt internal relationships
/// - Missing required parts (e.g. master slides, theme parts)
/// - Invalid XML that only the app's parser catches
///
/// # Workflow
/// 1. Detect file format from extension (.docx, .xlsx, .pptx).
/// 2. Open the file **read-only** (never Save — avoid proofing/autocorrect side effects).
/// 3. Read counts: Word = paragraphs + revision count (grouped, report but don't equate to w:ins),
///    PowerPoint = slide count, Excel = worksheet count.
/// 4. Close the document **without saving**.
/// 5. `Quit()` + `ReleaseComObject` always, even on failure.
///
/// # Arguments
/// * `path` — Path to the document file (.docx, .xlsx, or .pptx).
///
/// # Returns
/// `Ok(ValidateOpenResult)` with validity info, or `Err(Vec<Diagnostic>)` on failure.
///
/// # Feature gate
/// `ms-office` feature on Windows. On non-Windows or without the feature,
/// returns an error diagnostic.
#[allow(unreachable_code)]
pub fn validate_open(path: &str) -> Result<ValidateOpenResult, Vec<Diagnostic>> {
    #[cfg(all(feature = "ms-office", target_os = "windows"))]
    {
        let _ = path;
        // TODO (T7.1): COM implementation via Stickey/Windows crate.
        // This is a stub — the real implementation spawns a stichos-com-worker
        // process that uses Word/Excel/PowerPoint COM APIs.
        // The stub mirrors the pattern used by delete_slides_first_n and export_slide_pngs.
        Err(vec![Diagnostic::new(
            "ENG-E-012",
            gp_core::diagnostics::Severity::Error,
            "validate_open: COM implementation requires Windows + `ms-office` feature (stub)",
        )])
    }
    #[cfg(not(all(feature = "ms-office", target_os = "windows")))]
    {
        let _ = path;
        Err(vec![Diagnostic::new(
            "ENG-E-012",
            gp_core::diagnostics::Severity::Error,
            "validate_open: requires Windows + `ms-office` feature (stub)",
        )])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_core::traits::Engine;

    /// test_delete_slides_stub_errors_non_windows: delete_slides_first_n returns error on non-Windows.
    #[test]
    fn test_delete_slides_stub_errors_non_windows() {
        let result = delete_slides_first_n("test.pptx", 1, None);
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "ENG-E-010");
    }

    /// test_export_pngs_stub_errors_non_windows: export_slide_pngs returns error on non-Windows.
    #[test]
    fn test_export_pngs_stub_errors_non_windows() {
        let result = export_slide_pngs("test.pptx", "/tmp/out", 1920, 1080);
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "ENG-E-011");
    }

    /// test_engine_id: engine identifier is "ms-office".
    #[test]
    fn test_engine_id() {
        let engine = MsOfficeEngine::new();
        assert_eq!(engine.engine_id(), "ms-office");
        assert_eq!(engine.engine_id(), MS_OFFICE_ENGINE_ID);
    }

    /// test_not_available_on_linux: engine is unavailable on non-Windows.
    #[test]
    #[allow(unused_variables)]
    fn test_not_available_on_linux() {
        let engine = MsOfficeEngine::new();
        #[cfg(not(target_os = "windows"))]
        assert!(!engine.is_available());
        #[cfg(not(target_os = "windows"))]
        assert!(!engine.can_convert("docx", "pdf"));
    }

    /// test_version_returns_none_stub: version is None in stub mode.
    #[test]
    fn test_version_returns_none_stub() {
        let engine = MsOfficeEngine::new();
        assert!(engine.version().is_none());
    }

    /// test_convert_returns_error: convert always errors in stub mode.
    #[test]
    fn test_convert_returns_error() {
        let engine = MsOfficeEngine::new();
        let result = engine.convert(b"test", "docx", "pdf");
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "ENG-E-001");
        assert!(diags[0].message.contains("compile-only skeleton"));
    }

    /// test_supported_conversions: lists the expected stub conversion paths.
    #[test]
    fn test_supported_conversions() {
        let engine = MsOfficeEngine::new();
        let conversions = engine.supported_conversions();
        assert_eq!(conversions.len(), 5);
        assert!(conversions.contains(&("docx".into(), "pdf".into())));
        assert!(conversions.contains(&("xlsx".into(), "pdf".into())));
        assert!(conversions.contains(&("pptx".into(), "pdf".into())));
    }

    /// test_probe_returns_stub: MsOfficeEngine::probe() returns all-false result.
    #[test]
    fn test_probe_returns_stub() {
        let result = MsOfficeEngine::probe();
        assert!(!result.word_available);
        assert!(!result.excel_available);
        assert!(!result.powerpoint_available);
        assert!(!result.any_available());
        assert!(result.word_version.is_none());
    }

    /// test_engine_info: EngineExt::info() produces valid EngineInfo.
    #[test]
    fn test_engine_info() {
        let engine = MsOfficeEngine::new();
        let info = engine.info();
        assert_eq!(info.engine_id, "ms-office");
        assert!(!info.is_available);
        assert!(!info.capabilities.is_empty());
        // All capabilities should have works=false in stub mode
        for cap in &info.capabilities {
            assert!(!cap.works);
        }
    }

    /// test_engine_is_send_sync: MsOfficeEngine implements Send + Sync.
    #[test]
    fn test_engine_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MsOfficeEngine>();
    }

    /// test_default_engine: Default trait returns stub engine.
    #[test]
    fn test_default_engine() {
        let engine = MsOfficeEngine::default();
        assert!(!engine.is_available());
        assert_eq!(engine.engine_id(), "ms-office");
    }

    /// test_clone_engine: MsOfficeEngine is cloneable.
    #[test]
    fn test_clone_engine() {
        let engine1 = MsOfficeEngine::new();
        let engine2 = engine1.clone();
        assert_eq!(engine1.engine_id(), engine2.engine_id());
        assert_eq!(engine1.is_available(), engine2.is_available());
    }

    /// test_debug_engine: MsOfficeEngine is debuggable.
    #[test]
    fn test_debug_engine() {
        let engine = MsOfficeEngine::new();
        let debug_str = format!("{:?}", engine);
        assert!(debug_str.contains("MsOfficeEngine"));
    }

    /// test_ms_office_available: ms_office_available() returns false in stub mode.
    #[test]
    fn test_ms_office_available() {
        assert!(!ms_office_available());
    }

    /// test_probe_result_any_available: stub result has any_available() == false.
    #[test]
    fn test_probe_result_any_available() {
        let result = MsOfficeProbeResult::default();
        assert!(!result.any_available());
    }

    /// test_probe_result_stub: MsOfficeProbeResult::stub() creates all-false result.
    #[test]
    fn test_probe_result_stub() {
        let result = MsOfficeProbeResult::stub();
        assert!(!result.any_available());
        assert!(result.word_version.is_none());
        assert!(result.excel_version.is_none());
        assert!(result.powerpoint_version.is_none());
    }

    /// test_validate_open_stub_errors_non_windows: validate_open returns error on non-Windows.
    #[test]
    fn test_validate_open_stub_errors_non_windows() {
        let result = validate_open("test.docx");
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "ENG-E-012");
    }
}
