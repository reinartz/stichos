// SPDX-License-Identifier: MIT OR Apache-2.0
//! doctor tool: probe system capabilities.
//!
//! Wraps `engines::doctor::run_doctor()` to produce CLI-friendly output
//! in both human-readable and JSON formats.

use anyhow::Result;
use engines::chromium::ChromiumEngine;
use engines::libreoffice::LibreOfficeEngine;
use engines::mock::MockEngine;
use engines::pandoc::PandocEngine;
use engines::probe::standard_fixtures;
use engines::typst::TypstEngine;
use engines::validate_open;
use gp_core::file_access::{check_file_locked, FileLockStatus};
use gp_core::traits::EngineExt;

/// Run the doctor command and return results.
pub fn run_doctor(json: bool, path: Option<&std::path::Path>, validate: bool) -> Result<String> {
    let fixtures = standard_fixtures();

    // Probe platform opener for handoff
    let handoff = engines::open_handler::probe_open_handler();

    // Probe real engines first, then mock as fallback
    let mut engine_refs: Vec<&dyn EngineExt> = Vec::new();

    // Probe LibreOffice
    if let Ok(lo) = LibreOfficeEngine::new_auto() {
        engine_refs.push(Box::leak(Box::new(lo)) as &dyn EngineExt);
    }

    // Probe Pandoc
    if let Ok(pandoc) = PandocEngine::new_auto() {
        engine_refs.push(Box::leak(Box::new(pandoc)) as &dyn EngineExt);
    }

    // Probe Chromium
    if let Ok(chromium) = ChromiumEngine::new_auto() {
        engine_refs.push(Box::leak(Box::new(chromium)) as &dyn EngineExt);
    }

    // Probe Typst
    if let Ok(typst) = TypstEngine::new_auto() {
        engine_refs.push(Box::leak(Box::new(typst)) as &dyn EngineExt);
    }

    // Always include mock engine
    let mock = MockEngine::new();
    engine_refs.push(Box::leak(Box::new(mock)) as &dyn EngineExt);

    let result = engines::doctor::run_doctor_with_handoff(engine_refs, &fixtures, Some(handoff));

    // File lock check for the given path
    let file_lock_status = path.map(|p| {
        let path_str = p.to_string_lossy().to_string();
        let status = check_file_locked(p);
        let status_str = match &status {
            FileLockStatus::Unlocked => "unlocked",
            FileLockStatus::Locked { .. } => "locked",
            FileLockStatus::Dehydrated { .. } => "dehydrated",
            FileLockStatus::Error { .. } => "error",
        };
        let detail = match &status {
            FileLockStatus::Unlocked => String::new(),
            FileLockStatus::Locked { path: locked_path } => {
                format!("file in use by another process: {}", locked_path.display())
            }
            FileLockStatus::Dehydrated { path: dpath, size } => {
                format!(
                    "dehydrated placeholder ({} bytes): {}",
                    size,
                    dpath.display()
                )
            }
            FileLockStatus::Error { path: epath, error } => {
                format!("error checking {}: {}", epath.display(), error)
            }
        };
        (path_str, status_str.to_string(), detail, status)
    });

    // COM validation (T7)
    let com_validation = if validate {
        match path {
            Some(p) => {
                let path_str = p.to_string_lossy().to_string();
                match validate_open(&path_str) {
                    Ok(result) => Some(serde_json::json!({
                        "path": path_str,
                        "ok": result.ok,
                        "app": result.app,
                        "app_version": result.app_version,
                        "detail": result.detail,
                    })),
                    Err(diags) => Some(serde_json::json!({
                        "path": path_str,
                        "ok": false,
                        "error": true,
                        "diagnostics": diags.iter().map(|d| {
                            serde_json::json!({
                                "code": d.code,
                                "severity": format!("{}", d.severity),
                                "message": d.message,
                            })
                        }).collect::<Vec<_>>(),
                    })),
                }
            }
            None => Some(serde_json::json!({
                "ok": false,
                "error": true,
                "diagnostics": [{
                    "code": "DOCTOR-E-001",
                    "severity": "Error",
                    "message": "--validate requires --path"
                }]
            })),
        }
    } else {
        None
    };

    // PDF/A revalidation status
    let (vera_available, qpdf_available): (bool, bool) = (
        which::which("verapdf").is_ok(),
        which::which("qpdf").is_ok(),
    );
    let pdfa_status = if vera_available || qpdf_available {
        "available".to_string()
    } else {
        "honest-absent: PDF/A revalidation skipped (no veraPDF or qpdf on this host; available in CI)".to_string()
    };

    let pdfa_json = serde_json::json!({
        "veraPDF_available": vera_available,
        "qpdf_available": qpdf_available,
        "status": pdfa_status,
    });

    if json {
        let output = serde_json::json!({
            "tool": "doctor",
            "engines": result.engines.iter().map(|e| {
                serde_json::json!({
                    "id": e.engine_id,
                    "version": e.version,
                    "available": e.available,
                    "capabilities": e.capabilities.iter().map(|c| {
                        serde_json::json!({
                            "source": c.source,
                            "target": c.target,
                            "works": c.works,
                        })
                    }).collect::<Vec<_>>(),
                })
            }).collect::<Vec<_>>(),
            "fonts": {
                "calibri": result.fonts.calibri,
                "cambria": result.fonts.cambria,
                "liberation_sans": result.fonts.liberation_sans,
                "noto_sans": result.fonts.noto_sans,
            },
            "system": {
                "os": result.system_info.os,
                "arch": result.system_info.arch,
                "libreoffice_version": result.system_info.libreoffice_version,
                "poppler_version": result.system_info.poppler_version,
                "ghostscript_version": result.system_info.ghostscript_version,
                "chromium_version": result.system_info.chromium_version,
                "pandoc_version": result.system_info.pandoc_version,
                "typst_version": result.system_info.typst_version,
            },
            "handoff": result.handoff.as_ref().map(|h| {
                serde_json::json!({
                    "platform": h.platform,
                    "handler_available": h.handler_available,
                    "handler_name": h.handler_name,
                    "install_hint": h.install_hint,
                })
            }),
            "pdfa_revalidation": pdfa_json,
            "file_lock": file_lock_status.as_ref().map(|(path_val, status_str, detail, _)| {
                serde_json::json!({
                    "path": path_val,
                    "status": status_str,
                    "detail": detail,
                })
            }),
            "com_validation": com_validation,
        });
        Ok(serde_json::to_string_pretty(&output)?)
    } else {
        let mut lines: Vec<String> = Vec::new();
        lines.push("=== stichos doctor ===".to_string());
        lines.push(format!(
            "OS: {}/{}",
            result.system_info.os, result.system_info.arch
        ));

        lines.push("Engines:".to_string());
        for e in &result.engines {
            let status = if e.available { "+" } else { "-" };
            lines.push(format!(
                "  {} {} {}",
                status,
                e.engine_id,
                e.version.as_deref().unwrap_or("unknown")
            ));
            for c in &e.capabilities {
                let ok = if c.works { "+" } else { "-" };
                lines.push(format!(
                    "    {} {} -> {}: {}",
                    ok,
                    c.source,
                    c.target,
                    if c.works { "works" } else { "fails" }
                ));
            }
        }

        lines.push("Fonts:".to_string());
        lines.push(format!(
            "  Calibri: {}",
            result.fonts.calibri.as_deref().unwrap_or("NOT FOUND")
        ));
        lines.push(format!(
            "  Cambria: {}",
            result.fonts.cambria.as_deref().unwrap_or("NOT FOUND")
        ));
        lines.push(format!(
            "  Liberation Sans: {}",
            result
                .fonts
                .liberation_sans
                .as_deref()
                .unwrap_or("NOT FOUND")
        ));
        lines.push(format!(
            "  Noto Sans: {}",
            result.fonts.noto_sans.as_deref().unwrap_or("NOT FOUND")
        ));

        // Platform opener handoff
        lines.push("Platform Opener:".to_string());
        if let Some(ref h) = result.handoff {
            let status = if h.handler_available { "+" } else { "-" };
            lines.push(format!("  {} {} ({})", status, h.handler_name, h.platform));
            if !h.handler_available && !h.install_hint.is_empty() {
                lines.push(format!("  hint: {}", h.install_hint));
            }
        } else {
            lines.push("  not probed".to_string());
        }

        // PDF/A revalidation
        lines.push("PDF/A Revalidation:".to_string());
        lines.push(format!("  {}", pdfa_status));

        // File lock status
        if let Some((path_val, _, _, ref status)) = file_lock_status {
            lines.push("File Lock:".to_string());
            lines.push(format!("  {}: {}", path_val, status));
        }

        // COM validation status
        if let Some(ref cv) = com_validation {
            lines.push("COM Validation:".to_string());
            if let Some(err_diag) = cv.get("diagnostics").and_then(|d| d.as_array()) {
                if !err_diag.is_empty() {
                    lines.push("  ERROR:".to_string());
                    for diag in err_diag {
                        if let Some(msg) = diag.get("message").and_then(|m| m.as_str()) {
                            lines.push(format!("    {}", msg));
                        }
                    }
                } else {
                    if let Some(ok) = cv.get("ok").and_then(|o| o.as_bool()) {
                        lines.push(format!(
                            "  {}: {}",
                            cv["path"],
                            if ok { "valid" } else { "invalid" }
                        ));
                    }
                    if let Some(app) = cv.get("app").and_then(|a| a.as_str()) {
                        lines.push(format!("  app: {}", app));
                    }
                    if let Some(detail) = cv.get("detail").and_then(|d| d.as_str()) {
                        lines.push(format!("  detail: {}", detail));
                    }
                }
            }
        }

        lines.push("".to_string());

        // Append JSON block for machine consumption
        let json_block = serde_json::json!({
            "tool": "doctor",
            "engines": result.engines.iter().map(|e| {
                serde_json::json!({
                    "id": e.engine_id,
                    "version": e.version,
                    "available": e.available,
                    "capabilities": e.capabilities.iter().map(|c| {
                        serde_json::json!({
                            "source": c.source,
                            "target": c.target,
                            "works": c.works,
                        })
                    }).collect::<Vec<_>>(),
                })
            }).collect::<Vec<_>>(),
            "fonts": {
                "calibri": result.fonts.calibri,
                "cambria": result.fonts.cambria,
                "liberation_sans": result.fonts.liberation_sans,
                "noto_sans": result.fonts.noto_sans,
            },
            "system": {
                "os": result.system_info.os,
                "arch": result.system_info.arch,
                "libreoffice_version": result.system_info.libreoffice_version,
                "poppler_version": result.system_info.poppler_version,
                "ghostscript_version": result.system_info.ghostscript_version,
                "chromium_version": result.system_info.chromium_version,
                "pandoc_version": result.system_info.pandoc_version,
                "typst_version": result.system_info.typst_version,
            },
            "handoff": result.handoff.as_ref().map(|h| {
                serde_json::json!({
                    "platform": h.platform,
                    "handler_available": h.handler_available,
                    "handler_name": h.handler_name,
                    "install_hint": h.install_hint,
                })
            }),
            "pdfa_revalidation": pdfa_json,
            "com_validation": com_validation,
        });
        lines.push(serde_json::to_string_pretty(&json_block)?);

        Ok(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// test_doctor_validate_without_path_errors: --validate without --path returns error in JSON.
    #[test]
    fn test_doctor_validate_without_path_errors() {
        let result = run_doctor(true, None, true).expect("run_doctor should not panic");
        let val: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");
        assert!(val.get("com_validation").is_some());
        let cv = val["com_validation"]
            .as_object()
            .expect("com_validation should be object");
        assert_eq!(cv["ok"], false);
        assert_eq!(cv["error"], true);
    }

    /// test_doctor_validate_with_path_returns_stub_error: COM validation on non-Windows returns stub error.
    #[test]
    fn test_doctor_validate_with_path_returns_stub_error() {
        let result = run_doctor(true, Some(std::path::Path::new("/tmp/test.docx")), true)
            .expect("run_doctor should not panic");
        let val: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");
        assert!(val.get("com_validation").is_some());
        let cv = val["com_validation"]
            .as_object()
            .expect("com_validation should be object");
        assert_eq!(cv["ok"], false);
        assert_eq!(cv["error"], true);
        assert_eq!(cv["path"], "/tmp/test.docx");
        let diags = cv["diagnostics"]
            .as_array()
            .expect("diagnostics should be array");
        assert!(!diags.is_empty());
        assert_eq!(diags[0]["code"], "ENG-E-012");
    }

    /// test_doctor_without_validate_excludes_com: --validate=false omits com_validation.
    #[test]
    fn test_doctor_without_validate_excludes_com() {
        let result = run_doctor(true, None, false).expect("run_doctor should not panic");
        let val: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");
        // com_validation should be null (absent) when --validate is not set
        let cv = val.get("com_validation");
        assert!(cv.is_none() || cv == Some(&serde_json::json!(null)));
    }

    /// test_doctor_validate_text_output: --validate in text mode includes COM Validation section.
    #[test]
    fn test_doctor_validate_text_output() {
        let result = run_doctor(false, Some(std::path::Path::new("/tmp/test.docx")), true)
            .expect("run_doctor should not panic");
        assert!(result.contains("COM Validation:"));
        assert!(result.contains("test.docx"));
    }
}
