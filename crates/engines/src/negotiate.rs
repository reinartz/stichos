// SPDX-License-Identifier: MIT OR Apache-2.0
//! Renderer negotiation: per-platform order, preference, session cache, honest feedback.
//!
//! This module provides renderer selection logic that chooses the best available
//! rendering engine based on platform constraints, user preference, and runtime
//! availability checks.
//!
//! # Architecture
//!
//! - [`RendererPreference`] — user intent (default, LO, MS Office, chromium, typst, custom).
//! - [`negotiate_renderer`] — selects the best renderer given preference + platform.
//! - [`negotiate_for_conversion`] — engine selection for a specific source→target path.
//! - [`authoring_order`] — preferred engine for each conversion path.
//! - [`platform_renderer_order`] — platform-specific engine ordering.
//! - [`unavailable_response`] — honest visual_feedback message when nothing works.
//! - [`SessionCache`] — per-session renderer cache keyed by engine_id.

use gp_core::diagnostics::Diagnostic;
use gp_core::diagnostics::Severity;
use gp_core::traits::Engine;

use crate::ms_office;

// ======================================================================
// Types
// ======================================================================

/// Renderer preference from environment or config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererPreference {
    /// Auto-detect best available renderer.
    Default,
    /// Prefer LibreOffice renderer.
    LibreOffice,
    /// Prefer Microsoft Office renderer.
    MSOffice,
    /// Prefer Chromium renderer (html→pdf).
    Chromium,
    /// Prefer Typst renderer (typst/typ→pdf).
    Typst,
    /// Custom renderer identifier (e.g. "my-engine-v2").
    Custom(String),
}

/// A renderer choice produced by negotiation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererChoice {
    /// The selected engine identifier.
    pub engine_id: String,
    /// Whether the selected renderer is available on this system.
    pub available: bool,
    /// Human-readable reason for the choice.
    pub reason: String,
}

/// Per-session cache of negotiated renderers.
#[derive(Debug, Clone, Default)]
pub struct SessionCache {
    entries: std::collections::HashMap<String, RendererChoice>,
}

// ======================================================================
// Platform-specific renderer order
// ======================================================================

/// Return the platform-specific renderer priority order.
///
/// On Linux: LibreOffice first (only practical option).
/// On macOS: LibreOffice first (MS Office is rare on Mac).
/// On Windows: MS Office first, LibreOffice second.
#[allow(unreachable_code)]
pub fn platform_renderer_order() -> Vec<&'static str> {
    #[cfg(target_os = "windows")]
    return vec![
        MS_OFFICE_ENGINE_ID,
        LIBREOFFICE_ENGINE_ID,
        PANDOC_ENGINE_ID,
        CHROMIUM_ENGINE_ID,
    ];
    #[cfg(target_os = "macos")]
    return vec![
        LIBREOFFICE_ENGINE_ID,
        CHROMIUM_ENGINE_ID,
        PANDOC_ENGINE_ID,
        TYPST_ENGINE_ID,
    ];
    // Linux (and everything else)
    vec![
        LIBREOFFICE_ENGINE_ID,
        CHROMIUM_ENGINE_ID,
        PANDOC_ENGINE_ID,
        TYPST_ENGINE_ID,
    ]
}

/// Engine identifiers for platform negotiation.
pub const MS_OFFICE_ENGINE_ID: &str = "ms-office";
pub const LIBREOFFICE_ENGINE_ID: &str = "libreoffice";
pub const PANDOC_ENGINE_ID: &str = "pandoc";
pub const CHROMIUM_ENGINE_ID: &str = "chromium";
pub const TYPST_ENGINE_ID: &str = "typst";

/// Check if a renderer engine is available on the system.
pub fn check_engine_available(engine_id: &str) -> bool {
    match engine_id {
        LIBREOFFICE_ENGINE_ID => check_libreoffice_available(),
        MS_OFFICE_ENGINE_ID => check_ms_office_available(),
        PANDOC_ENGINE_ID => check_pandoc_available(),
        CHROMIUM_ENGINE_ID => check_chromium_available(),
        TYPST_ENGINE_ID => check_typst_available(),
        _ => false,
    }
}

/// Check if LibreOffice is available (soffice binary on PATH).
fn check_libreoffice_available() -> bool {
    which_binary("soffice").is_some() || which_binary("libreoffice").is_some()
}

/// Check if Microsoft Office is available (heuristic: check common paths).
///
/// Delegates to `ms_office::ms_office_available()` for the authoritative check.
fn check_ms_office_available() -> bool {
    ms_office::ms_office_available()
}

/// Check if pandoc is available (binary on PATH).
fn check_pandoc_available() -> bool {
    which_binary("pandoc").is_some()
}

/// Check if chromium is available (binary on PATH, common names).
fn check_chromium_available() -> bool {
    let names = [
        "chromium-browser",
        "chromium",
        "google-chrome",
        "google-chrome-stable",
    ];
    names.iter().any(|n| which_binary(n).is_some())
}

/// Check if typst is available (binary on PATH).
fn check_typst_available() -> bool {
    which_binary("typst").is_some()
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

// ======================================================================
// Negotiation
// ======================================================================

/// Choose the best renderer based on platform and preferences.
///
/// This function implements the renderer negotiation algorithm:
/// 1. If preference is `Default`, use platform-specific order.
/// 2. If preference specifies an engine, try it first, then fall back.
/// 3. Return the first available renderer, or an unavailable choice with explanation.
pub fn negotiate_renderer(preference: &RendererPreference) -> RendererChoice {
    let candidates = resolve_candidates(preference);

    // Try each candidate in order
    for (engine_id, preferred) in &candidates {
        let available = check_engine_available(engine_id);
        if available {
            return RendererChoice {
                engine_id: engine_id.to_string(),
                available: true,
                reason: if *preferred {
                    format!("{} selected by preference", engine_id)
                } else {
                    format!("{} selected (platform order, available)", engine_id)
                },
            };
        }
    }

    // Nothing available — return honest unavailable response
    unavailable_response()
}

/// Resolve candidate engines based on preference and platform order.
fn resolve_candidates(preference: &RendererPreference) -> Vec<(String, bool)> {
    let platform_order = platform_renderer_order();

    match preference {
        RendererPreference::Default => {
            // Platform order is the candidate list; all marked as non-preferred
            platform_order
                .into_iter()
                .map(|s| (s.to_string(), false))
                .collect()
        }
        RendererPreference::LibreOffice => {
            let mut candidates = vec![(LIBREOFFICE_ENGINE_ID.to_string(), true)];
            for id in &platform_order {
                if *id != LIBREOFFICE_ENGINE_ID {
                    candidates.push((id.to_string(), false));
                }
            }
            candidates
        }
        RendererPreference::MSOffice => {
            let mut candidates = vec![(MS_OFFICE_ENGINE_ID.to_string(), true)];
            for id in &platform_order {
                if *id != MS_OFFICE_ENGINE_ID {
                    candidates.push((id.to_string(), false));
                }
            }
            candidates
        }
        RendererPreference::Chromium => {
            let mut candidates = vec![(CHROMIUM_ENGINE_ID.to_string(), true)];
            for id in &platform_order {
                if *id != CHROMIUM_ENGINE_ID {
                    candidates.push((id.to_string(), false));
                }
            }
            candidates
        }
        RendererPreference::Typst => {
            let mut candidates = vec![(TYPST_ENGINE_ID.to_string(), true)];
            for id in &platform_order {
                if *id != TYPST_ENGINE_ID {
                    candidates.push((id.to_string(), false));
                }
            }
            candidates
        }
        RendererPreference::Custom(ref custom) => {
            let mut candidates = vec![(custom.clone(), true)];
            for id in &platform_order {
                if *id != custom.as_str() {
                    candidates.push((id.to_string(), false));
                }
            }
            candidates
        }
    }
}

/// Produce an honest unavailable response.
///
/// Returns a choice indicating no renderer is available, with
/// the message "visual_feedback: unavailable".
pub fn unavailable_response() -> RendererChoice {
    RendererChoice {
        engine_id: "none".into(),
        available: false,
        reason: "visual_feedback: unavailable".to_string(),
    }
}

/// Return the unavailable diagnostic list for a requested engine.
pub fn unavailable_diagnostics(engine_id: &str) -> Vec<Diagnostic> {
    vec![Diagnostic::new(
        "ENG-E-001",
        Severity::Error,
        format!(
            "engine not found: {} — visual_feedback: unavailable",
            engine_id
        ),
    )]
}

/// Authoring order: preferred engine for each conversion path.
///
/// html→pdf: chromium→LO (chromium gives better WYSIWYG HTML→PDF)
/// md→docx: pandoc→native-later (pandoc converts markdown→docx)
/// Default: platform order
pub fn authoring_order(source: &str, target: &str) -> Vec<&'static str> {
    match (source, target) {
        ("html", "pdf") => vec![CHROMIUM_ENGINE_ID, LIBREOFFICE_ENGINE_ID],
        ("md" | "markdown", "docx") => vec![PANDOC_ENGINE_ID],
        _ => platform_renderer_order(),
    }
}

/// Negotiate an engine for a specific source→target conversion.
///
/// Uses authoring_order() to select the best engine for the conversion,
/// falling back to platform_renderer_order() for unspecified paths.
pub fn negotiate_for_conversion(source: &str, target: &str) -> RendererChoice {
    let order = authoring_order(source, target);
    // Try each engine in authoring order
    for engine_id in &order {
        if check_engine_available(engine_id) {
            return RendererChoice {
                engine_id: engine_id.to_string(),
                available: true,
                reason: format!("{} selected for {}→{}", engine_id, source, target),
            };
        }
    }
    // Fall back to platform order if authoring order didn't match
    let platform = platform_renderer_order();
    for engine_id in &platform {
        if check_engine_available(engine_id) {
            return RendererChoice {
                engine_id: engine_id.to_string(),
                available: true,
                reason: format!(
                    "{} selected (platform order fallback for {}→{})",
                    engine_id, source, target
                ),
            };
        }
    }
    unavailable_response()
}

/// Session cache: store a negotiated choice for reuse within a session.
impl SessionCache {
    /// Create a new empty session cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a cached renderer choice for the given engine_id.
    pub fn get(&self, engine_id: &str) -> Option<&RendererChoice> {
        self.entries.get(engine_id)
    }

    /// Store a negotiated choice in the cache.
    pub fn put(&mut self, engine_id: &str, choice: RendererChoice) {
        self.entries.insert(engine_id.to_string(), choice);
    }

    /// Negotiate with cache: try cache first, then negotiate fresh.
    pub fn negotiate(&mut self, preference: &RendererPreference) -> RendererChoice {
        // Try to find any cached entry that is available
        if let Some(cached) = self.entries.values().find(|c| c.available) {
            return cached.clone();
        }

        // Cache miss — negotiate fresh
        let choice = negotiate_renderer(preference);
        if !choice.engine_id.is_empty() {
            self.put(&choice.engine_id, choice.clone());
        }
        choice
    }

    /// Return true if the cache has any available entries.
    pub fn has_available(&self) -> bool {
        self.entries.values().any(|c| c.available)
    }
}

/// Negotiate for a specific engine using the Engine trait.
///
/// This is a convenience function that checks if an engine is available
/// and returns either a valid choice or an unavailable response.
pub fn negotiate_for_engine<E: Engine>(engine: &E) -> RendererChoice {
    let available = engine.is_available();
    let version = engine.version().unwrap_or("unknown").to_string();

    if available {
        RendererChoice {
            engine_id: engine.engine_id().to_string(),
            available: true,
            reason: format!("{} available (version: {})", engine.engine_id(), version),
        }
    } else {
        RendererChoice {
            engine_id: engine.engine_id().to_string(),
            available: false,
            reason: format!(
                "{} not available (version: {})",
                engine.engine_id(),
                version
            ),
        }
    }
}

// ======================================================================
// Tests
// ======================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// test_negotiate_renderer_default: default preference selects platform-appropriate renderer.
    #[test]
    fn test_negotiate_renderer_default() {
        let choice = negotiate_renderer(&RendererPreference::Default);

        // On Linux, should pick libreoffice; on Windows, ms-office; on macOS, libreoffice
        let expected_platform = platform_renderer_order();
        assert!(
            expected_platform.contains(&choice.engine_id.as_str()),
            "default should select platform renderer (got {})",
            choice.engine_id
        );
    }

    /// test_negotiate_renderer_lo: LibreOffice preference selects LO when available.
    #[test]
    fn test_negotiate_renderer_lo() {
        let choice = negotiate_renderer(&RendererPreference::LibreOffice);

        if check_libreoffice_available() {
            assert_eq!(choice.engine_id, LIBREOFFICE_ENGINE_ID);
            assert!(choice.available);
        } else {
            // LO not available — should fall through platform order
            assert!(
                platform_renderer_order().contains(&choice.engine_id.as_str()) || !choice.available,
                "should have platform-appropriate engine or be unavailable"
            );
        }
    }

    /// test_negotiate_renderer_ms_office: MS Office preference selects MSO on Windows.
    #[test]
    fn test_negotiate_renderer_ms_office() {
        let choice = negotiate_renderer(&RendererPreference::MSOffice);

        if cfg!(target_os = "windows") && check_ms_office_available() {
            assert_eq!(choice.engine_id, MS_OFFICE_ENGINE_ID);
            assert!(choice.available);
        } else {
            // On non-Windows or when MS Office unavailable, falls through
            assert!(
                platform_renderer_order().contains(&choice.engine_id.as_str()) || !choice.available,
                "MS Office not available on this platform"
            );
        }
    }

    /// test_negotiate_renderer_unavailable: when nothing is available, returns honest response.
    #[test]
    fn test_negotiate_renderer_unavailable() {
        let choice = unavailable_response();

        assert_eq!(choice.engine_id, "none");
        assert!(!choice.available);
        assert_eq!(choice.reason, "visual_feedback: unavailable");
    }

    /// test_negotiate_renderer_custom: custom preference is tried first.
    #[test]
    fn test_negotiate_renderer_custom() {
        let choice = negotiate_renderer(&RendererPreference::Custom("my-engine".into()));

        // Should try "my-engine" first, then fall back to platform order
        // If my-engine doesn't exist, should pick from platform order
        let platform = platform_renderer_order();
        if !check_engine_available("my-engine") {
            assert!(
                platform.contains(&choice.engine_id.as_str()) || !choice.available,
                "custom engine unavailable, should fall back"
            );
        }
    }

    /// test_platform_renderer_order_linux: on Linux, LO is first.
    #[test]
    fn test_platform_renderer_order_linux() {
        // This test runs on Linux in CI
        #[cfg(target_os = "linux")]
        {
            let order = platform_renderer_order();
            assert!(
                order.contains(&LIBREOFFICE_ENGINE_ID),
                "LO should be in Linux platform order"
            );
            assert!(
                !order.contains(&MS_OFFICE_ENGINE_ID),
                "MS Office should not be in Linux platform order"
            );
        }
    }

    /// test_platform_renderer_order_windows: on Windows, MS Office is first.
    #[test]
    #[cfg(target_os = "windows")]
    fn test_platform_renderer_order_windows() {
        let order = platform_renderer_order();
        assert!(
            order.contains(&MS_OFFICE_ENGINE_ID),
            "MS Office should be in Windows platform order"
        );
        assert!(
            order.contains(&LIBREOFFICE_ENGINE_ID),
            "LO should be in Windows platform order"
        );
        assert_eq!(order[0], MS_OFFICE_ENGINE_ID);
    }

    /// test_session_cache_get_put: basic cache operations.
    #[test]
    fn test_session_cache_get_put() {
        let mut cache = SessionCache::new();
        assert!(!cache.has_available());

        let choice = RendererChoice {
            engine_id: "libreoffice".into(),
            available: true,
            reason: "test".into(),
        };
        cache.put("libreoffice", choice.clone());

        assert_eq!(cache.get("libreoffice"), Some(&choice));
        assert_eq!(cache.get("pandoc"), None);
        assert!(cache.has_available());
    }

    /// test_session_cache_negotiate: cache negotiates when empty.
    #[test]
    fn test_session_cache_negotiate() {
        let mut cache = SessionCache::new();
        let choice = cache.negotiate(&RendererPreference::Default);

        // Should negotiate fresh and cache the result
        assert!(
            cache.get(&choice.engine_id).is_some(),
            "negotiated choice should be cached"
        );
    }

    /// test_session_cache_reuse: cached available renderer is reused.
    #[test]
    fn test_session_cache_reuse() {
        let mut cache = SessionCache::new();
        cache.put(
            "libreoffice",
            RendererChoice {
                engine_id: "libreoffice".into(),
                available: true,
                reason: "cached".into(),
            },
        );

        let choice1 = cache.negotiate(&RendererPreference::Default);
        let choice2 = cache.negotiate(&RendererPreference::Default);

        assert_eq!(choice1.engine_id, choice2.engine_id);
    }

    /// test_negotiate_for_engine: negotiates for a specific engine.
    #[test]
    fn test_negotiate_for_engine_mock() {
        use crate::mock::MockEngine;
        let engine = MockEngine::new();
        let choice = negotiate_for_engine(&engine);

        assert_eq!(choice.engine_id, "mock");
        assert!(choice.available);
        assert!(choice.reason.contains("mock"));
    }

    /// test_negotiate_for_engine_unavailable: unavailable engine returns unavailable choice.
    #[test]
    fn test_negotiate_for_engine_unavailable() {
        use crate::mock::MockEngine;
        let mut engine = MockEngine::new();
        engine.available = false;
        let choice = negotiate_for_engine(&engine);

        assert_eq!(choice.engine_id, "mock");
        assert!(!choice.available);
        assert!(choice.reason.contains("not available"));
    }

    /// test_unavailable_diagnostics: returns proper diagnostic.
    #[test]
    fn test_unavailable_diagnostics() {
        let diags = unavailable_diagnostics("libreoffice");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "ENG-E-001");
        assert_eq!(diags[0].severity, Severity::Error);
        assert!(diags[0].message.contains("visual_feedback: unavailable"));
    }

    /// test_resolve_candidates_default: default returns platform order.
    #[test]
    fn test_resolve_candidates_default() {
        let candidates = resolve_candidates(&RendererPreference::Default);
        let platform = platform_renderer_order();

        assert_eq!(candidates.len(), platform.len());
        for (i, (id, preferred)) in candidates.iter().enumerate() {
            assert_eq!(id.as_str(), platform[i]);
            assert!(!*preferred);
        }
    }

    /// test_resolve_candidates_preferred: preferred engine is first.
    #[test]
    fn test_resolve_candidates_preferred() {
        let candidates = resolve_candidates(&RendererPreference::LibreOffice);
        assert_eq!(candidates[0].0, LIBREOFFICE_ENGINE_ID);
        assert!(candidates[0].1);
    }

    /// test_engine_available_checks: individual engine checks work.
    #[test]
    fn test_engine_available_checks() {
        // LibreOffice is available on this test system
        let lo = check_libreoffice_available();
        assert!(lo, "LibreOffice should be available on this system");

        // Pandoc may or may not be installed
        let _pandoc = check_pandoc_available();
        // Don't assert — pandoc may or may not be installed

        // MS Office is not available on Linux
        #[cfg(target_os = "linux")]
        {
            let mso = check_ms_office_available();
            assert!(!mso, "MS Office should not be available on Linux");
        }

        // Chromium may or may not be installed
        let _chromium = check_chromium_available();
        // Don't assert — chromium may or may not be installed
    }

    /// test_check_chromium_available: chromium check works.
    #[test]
    fn test_check_chromium_available() {
        let result = check_chromium_available();
        // May or may not find chromium; just verify it returns bool
        let _ = result;
    }

    /// test_chromium_engine_id: CHROMIUM_ENGINE_ID is defined.
    #[test]
    fn test_chromium_engine_id() {
        assert_eq!(CHROMIUM_ENGINE_ID, "chromium");
    }

    /// test_authoring_order_html_to_pdf: html→pdf prefers chromium.
    #[test]
    fn test_authoring_order_html_to_pdf() {
        let order = authoring_order("html", "pdf");
        assert_eq!(order, vec![CHROMIUM_ENGINE_ID, LIBREOFFICE_ENGINE_ID]);
    }

    /// test_authoring_order_md_to_docx: md→docx prefers pandoc.
    #[test]
    fn test_authoring_order_md_to_docx() {
        let order = authoring_order("md", "docx");
        assert_eq!(order, vec![PANDOC_ENGINE_ID]);
    }

    /// test_authoring_order_markdown_to_docx: markdown→docx prefers pandoc.
    #[test]
    fn test_authoring_order_markdown_to_docx() {
        let order = authoring_order("markdown", "docx");
        assert_eq!(order, vec![PANDOC_ENGINE_ID]);
    }

    /// test_authoring_order_default: unspecified paths use platform order.
    #[test]
    fn test_authoring_order_default() {
        let order = authoring_order("docx", "pdf");
        let platform = platform_renderer_order();
        assert_eq!(order, platform);
    }

    /// test_negotiate_for_conversion_html_to_pdf: prefers chromium for html→pdf.
    #[test]
    fn test_negotiate_for_conversion_html_to_pdf() {
        let choice = negotiate_for_conversion("html", "pdf");
        assert!(
            choice.engine_id == CHROMIUM_ENGINE_ID
                || choice.engine_id == LIBREOFFICE_ENGINE_ID
                || !choice.available,
            "should prefer chromium or LO for html→pdf (got {})",
            choice.engine_id
        );
    }

    /// test_negotiate_for_conversion_md_to_docx: prefers pandoc for md→docx.
    #[test]
    fn test_negotiate_for_conversion_md_to_docx() {
        let choice = negotiate_for_conversion("md", "docx");
        assert!(
            choice.engine_id == PANDOC_ENGINE_ID || !choice.available,
            "should prefer pandoc for md→docx (got {})",
            choice.engine_id
        );
    }

    /// test_negotiate_for_conversion_unknown_path: falls back to platform order.
    #[test]
    fn test_negotiate_for_conversion_fallback() {
        let choice = negotiate_for_conversion("docx", "pdf");
        let platform = platform_renderer_order();
        assert!(
            platform.contains(&choice.engine_id.as_str()) || !choice.available,
            "should fall back to platform order for unspecified path (got {})",
            choice.engine_id
        );
    }

    /// test_renderer_preference_chromium: Chromium preference selects chromium when available.
    #[test]
    fn test_renderer_preference_chromium() {
        let choice = negotiate_renderer(&RendererPreference::Chromium);
        if check_chromium_available() {
            assert_eq!(choice.engine_id, CHROMIUM_ENGINE_ID);
            assert!(choice.available);
        }
    }

    /// test_renderer_preference_typst: Typst preference selects typst when available.
    #[test]
    fn test_renderer_preference_typst() {
        let choice = negotiate_renderer(&RendererPreference::Typst);
        if check_typst_available() {
            assert_eq!(choice.engine_id, TYPST_ENGINE_ID);
            assert!(choice.available);
        }
    }

    /// test_resolve_candidates_chromium: chromium preference puts chromium first.
    #[test]
    fn test_resolve_candidates_chromium() {
        let candidates = resolve_candidates(&RendererPreference::Chromium);
        assert_eq!(candidates[0].0, CHROMIUM_ENGINE_ID);
        assert!(candidates[0].1);
    }

    /// test_resolve_candidates_typst: typst preference puts typst first.
    #[test]
    fn test_resolve_candidates_typst() {
        let candidates = resolve_candidates(&RendererPreference::Typst);
        assert_eq!(candidates[0].0, TYPST_ENGINE_ID);
        assert!(candidates[0].1);
    }
}
