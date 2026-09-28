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

#[cfg(test)]
mod tests {
    use super::*;
    use gp_core::traits::Engine;

    /// test_engine_id: engine identifier is "ms-office".
    #[test]
    fn test_engine_id() {
        let engine = MsOfficeEngine::new();
        assert_eq!(engine.engine_id(), "ms-office");
        assert_eq!(engine.engine_id(), MS_OFFICE_ENGINE_ID);
    }

    /// test_not_available_on_linux: engine is unavailable on non-Windows.
    #[test]
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
}
