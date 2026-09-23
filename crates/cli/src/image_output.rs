// SPDX-License-Identifier: MIT OR Apache-2.0
//! CTX-03 image-output wiring (P6-03 part 2; contracts C14 §3.2/§3.3/§3.7).
//!
//! Shared helpers for `render_document` / `inspect_region` / `annotate_pages`:
//! `output=urls|base64|both` parsing (default **urls**), `max_pages` defaults
//! (render 3, annotate 5; region is a single crop, no `max_pages`), temp-file
//! publishing via [`gp_core::tempfiles::TempFileManager`], and envelope
//! stamping (`__budget` + `diagnostics` + `rendered_by`).
//!
//! URL-mode responses are greppably base64-free: the `"base64"` key (and the
//! word itself) appears only in `base64`/`both` modes.

use gp_core::envelope::{envelope_json, RenderOutput, ResponseBudget};
use gp_core::tempfiles::{StoredRender, TempFileManager};

/// Default `max_pages` for `render_document` (C14 §3.2).
pub const RENDER_DEFAULT_MAX_PAGES: usize = 3;
/// Default `max_pages` for `annotate_pages` (C14 §3.7).
pub const ANNOTATE_DEFAULT_MAX_PAGES: usize = 5;

/// Parse an `output` flag/arg: `None`/empty → [`RenderOutput::Urls`].
pub fn parse_output(raw: Option<&str>) -> Result<RenderOutput, String> {
    match raw.map(str::trim) {
        None | Some("") => Ok(RenderOutput::Urls),
        Some(s) => s.parse::<RenderOutput>(),
    }
}

/// Resolve `max_pages`: `None` or `0` → `default` (0 would render nothing,
/// which is never what a caller wants; unlimited arrives via large values).
pub fn parse_max_pages(raw: Option<usize>, default: usize) -> usize {
    match raw {
        None | Some(0) => default,
        Some(n) => n,
    }
}

/// Best-effort startup sweep: remove all `*.png` under the default renders
/// dir (C14 §3.2 server-restart semantics). Never fails the caller.
pub fn startup_sweep() -> usize {
    TempFileManager::open_default()
        .map(|mgr| mgr.sweep_all())
        .unwrap_or(0)
}

/// Open the default temp manager, mapped into `anyhow` for handlers.
pub fn open_default_manager() -> anyhow::Result<TempFileManager> {
    TempFileManager::open_default().map_err(|e| anyhow::anyhow!("temp store unavailable: {}", e))
}

/// Base64-encode image bytes for `base64`/`both` modes.
pub fn encode_base64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Build one page entry: always `page`/`width`/`height`/`fileSize`, plus
/// `url` + `expiresAt` in `urls`/`both` modes and `base64` in
/// `base64`/`both` modes. Callers pass `stored=None` only in `base64` mode.
pub fn page_entry(
    page: usize,
    width: usize,
    height: usize,
    png_bytes: &[u8],
    stored: Option<&StoredRender>,
    output: RenderOutput,
) -> serde_json::Value {
    let mut entry = serde_json::json!({
        "page": page,
        "width": width,
        "height": height,
        "fileSize": png_bytes.len(),
    });
    if matches!(output, RenderOutput::Urls | RenderOutput::Both) {
        if let Some(rec) = stored {
            entry["url"] = serde_json::Value::String(rec.url.clone());
            entry["expiresAt"] = serde_json::Value::String(rec.expires_at.clone());
        }
    }
    if matches!(output, RenderOutput::Base64 | RenderOutput::Both) {
        entry["base64"] = serde_json::Value::String(encode_base64(png_bytes));
    }
    entry
}

/// Stamp `__budget` (image count) + `diagnostics` + `rendered_by` onto an
/// image-tool response. Caller supplies the engine provenance string.
pub fn stamp_image_envelope(
    inner: serde_json::Value,
    image_count: usize,
    rendered_by: &str,
) -> serde_json::Value {
    // `inner` is always a JSON object at our call sites, so the gate error
    // path is unreachable; the clone only feeds the fallback.
    envelope_json(
        inner.clone(),
        &ResponseBudget::new(0, 0, image_count),
        None,
        rendered_by,
    )
    .unwrap_or(inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// test_parse_output_and_max_pages_defaults — urls default, rejects junk.
    #[test]
    fn test_parse_output_and_max_pages_defaults() {
        assert_eq!(parse_output(None).unwrap(), RenderOutput::Urls);
        assert_eq!(parse_output(Some("")).unwrap(), RenderOutput::Urls);
        assert_eq!(parse_output(Some("urls")).unwrap(), RenderOutput::Urls);
        assert_eq!(parse_output(Some("base64")).unwrap(), RenderOutput::Base64);
        assert_eq!(parse_output(Some("both")).unwrap(), RenderOutput::Both);
        assert!(parse_output(Some("inline")).is_err());
        assert_eq!(parse_max_pages(None, 3), 3);
        assert_eq!(parse_max_pages(Some(0), 3), 3);
        assert_eq!(parse_max_pages(Some(10), 3), 10);
    }
}
