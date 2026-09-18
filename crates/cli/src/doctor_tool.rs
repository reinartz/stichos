//! doctor tool: probe system capabilities.
//!
//! Wraps `engines::doctor::run_doctor()` to produce CLI-friendly output
//! in both human-readable and JSON formats.

use anyhow::Result;
use engines::libreoffice::LibreOfficeEngine;
use engines::mock::MockEngine;
use engines::pandoc::PandocEngine;
use engines::probe::standard_fixtures;
use gp_core::traits::EngineExt;

/// Run the doctor command and return results.
pub fn run_doctor(json: bool) -> Result<String> {
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

    // Always include mock engine
    let mock = MockEngine::new();
    engine_refs.push(Box::leak(Box::new(mock)) as &dyn EngineExt);

    let result = engines::doctor::run_doctor_with_handoff(engine_refs, &fixtures, Some(handoff));

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
            },
            "handoff": result.handoff.as_ref().map(|h| {
                serde_json::json!({
                    "platform": h.platform,
                    "handler_available": h.handler_available,
                    "handler_name": h.handler_name,
                    "install_hint": h.install_hint,
                })
            }),
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
            },
            "handoff": result.handoff.as_ref().map(|h| {
                serde_json::json!({
                    "platform": h.platform,
                    "handler_available": h.handler_available,
                    "handler_name": h.handler_name,
                    "install_hint": h.install_hint,
                })
            }),
        });
        lines.push(serde_json::to_string_pretty(&json_block)?);

        Ok(lines.join("\n"))
    }
}
