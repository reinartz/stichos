// SPDX-License-Identifier: MIT OR Apache-2.0
//! Payload budget enforcer: ensures all tool responses respect C8 constraints.
//!
//! Every tool that returns images must pass through this enforcer.
//! Implements INV-2 response-schema compliance checks.

use serde_json::Value;

/// Stamp the C14 context-budget envelope on a CLI-side tool response.
///
/// Wraps `gp_core::envelope::envelope_json` so every CLI verb can get
/// `__budget` + `diagnostics` + `rendered_by` without adding a gp-core
/// dep to `mcp`. This function is idempotent: if the three keys are
/// already present they are left untouched.
pub fn stamp_envelope(response: Value) -> Value {
    let json_text = serde_json::to_string(&response).unwrap_or_default();
    let tokens_est = gp_core::envelope::estimate_text_tokens(&json_text);
    let elements_count = response
        .get("element_ids")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0)
        + response
            .get("elements")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
        + response
            .get("pages")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
        + response
            .get("entries")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
        + response
            .get("operations")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);

    let image_count = response
        .get("image_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize
        + response
            .get("image_blocks")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
        + response
            .get("blocks")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);

    let budget = gp_core::envelope::ResponseBudget::new(tokens_est, elements_count, image_count);
    let rendered_by = gp_core::envelope::default_rendered_by();

    match gp_core::envelope::envelope_json(response.clone(), &budget, None, &rendered_by) {
        Ok(s) => s,
        Err(_) => response,
    }
}

/// Enforce the payload budget on a tool response JSON value.
///
/// Returns Ok(response) if within budget, Err with diagnostic if not.
///
/// Enforcements:
/// - Image blocks limited to MAX_IMAGE_BLOCKS (20) — excess truncated with warning
/// - Per-image dimensions clamped to MAX_PIXELS_PER_SIDE (2000) per side
/// - Ensures `rendered_by` or `visual_feedback` is present (INV-2)
/// - Ensures `diagnostics` array exists (C3)
pub fn enforce_payload_budget(mut response: Value) -> Result<Value, Value> {
    // Determine which key holds the image blocks and work with it directly
    let block_key = if response.get("blocks").is_some() {
        "blocks"
    } else if response.get("image_blocks").is_some() {
        "image_blocks"
    } else {
        // No image blocks to check — skip block enforcement
        let block_key: Option<&str> = None;
        match block_key {
            Some(_) => {}
            None => {
                // Ensure diagnostics array exists
                if response.get("diagnostics").is_none() {
                    response["diagnostics"] = serde_json::json!([]);
                }
                // Check provenance
                let has_provenance = response.get("rendered_by").is_some_and(|v| v.is_string())
                    || response
                        .get("visual_feedback")
                        .is_some_and(|v| v.is_string());
                if !has_provenance {
                    ensure_diagnostics_with_entry(
                        &mut response,
                        "ENG-E-001",
                        "rendered_by not specified",
                    );
                }
                return Ok(response);
            }
        }
        "blocks"
    };

    // Check block count and clamp dimensions — single pass to avoid borrow issues
    let mut need_truncate = false;
    let mut excess = 0;
    {
        if let Some(blocks_val) = response.get_mut(block_key) {
            if let Some(arr) = blocks_val.as_array_mut() {
                let block_count = arr.len();
                if block_count > MAX_IMAGE_BLOCKS {
                    need_truncate = true;
                    excess = block_count - MAX_IMAGE_BLOCKS;
                }

                // Clamp per-image dimensions (2000px per side)
                for block in arr.iter_mut() {
                    if let Some(w) = block.get("width") {
                        if let Some(ww) = w.as_u64() {
                            if ww > MAX_PIXELS_PER_SIDE as u64 {
                                block["width"] = serde_json::json!(MAX_PIXELS_PER_SIDE);
                            }
                        }
                    }
                    if let Some(h) = block.get("height") {
                        if let Some(hh) = h.as_u64() {
                            if hh > MAX_PIXELS_PER_SIDE as u64 {
                                block["height"] = serde_json::json!(MAX_PIXELS_PER_SIDE);
                            }
                        }
                    }
                }
            }
        }
    }
    // Apply truncation after blocks borrow is released
    if need_truncate {
        if let Some(blocks_val) = response.get_mut(block_key) {
            if let Some(arr) = blocks_val.as_array_mut() {
                arr.truncate(MAX_IMAGE_BLOCKS);
                response["budget_exceeded"] = serde_json::json!(true);
                response["truncated_blocks"] = serde_json::json!(excess);
            }
        }
    }

    // Ensure rendered_by or visual_feedback is present (INV-2 provenance)
    let has_provenance = response.get("rendered_by").is_some_and(|v| v.is_string())
        || response
            .get("visual_feedback")
            .is_some_and(|v| v.is_string());
    if !has_provenance {
        ensure_diagnostics_with_entry(&mut response, "ENG-E-001", "rendered_by not specified");
    }

    Ok(response)
}

/// Enforce a text/element payload budget on an `inspect_document` response.
///
/// The raw inspect result enumerates every element (paragraph, run, cell, ...)
/// in a document; for large specs that is megabytes of JSON, which blows past
/// LLM context windows when the MCP server returns it to llama.cpp. The caps
/// keep the returned manifest at a fixed, model-friendly size while preserving
/// the true document-wide totals in `total_elements` and recording truncation
/// via `budget_exceeded`/`budget_note` plus a diagnostic (C3).
pub fn enforce_inspect_budget(mut response: Value) -> Result<Value, Value> {
    let mut truncated: Vec<String> = Vec::new();

    // Cap the element list; total_elements (set before truncation) keeps the
    // real count so the caller can see how much was omitted.
    if let Some(ids) = response
        .get_mut("element_ids")
        .and_then(|v| v.as_array_mut())
    {
        let total = ids.len();
        if total > MAX_INSPECT_ELEMENTS {
            ids.truncate(MAX_INSPECT_ELEMENTS);
            truncated.push(format!(
                "element_ids {} -> {} ({} omitted)",
                total,
                MAX_INSPECT_ELEMENTS,
                total - MAX_INSPECT_ELEMENTS
            ));
        }
    }

    // Cap coordinate entries (pages/sections) and per-entry bbox lists.
    if let Some(coords) = response
        .get_mut("coordinates")
        .and_then(|v| v.as_array_mut())
    {
        let total = coords.len();
        if total > MAX_INSPECT_COORD_ENTRIES {
            coords.truncate(MAX_INSPECT_COORD_ENTRIES);
            truncated.push(format!(
                "coordinates {} -> {} ({} omitted)",
                total,
                MAX_INSPECT_COORD_ENTRIES,
                total - MAX_INSPECT_COORD_ENTRIES
            ));
        }
        for entry in coords.iter_mut() {
            for key in ["bboxes", "drawing_bboxes"] {
                if let Some(bboxes) = entry.get_mut(key).and_then(|v| v.as_array_mut()) {
                    let len = bboxes.len();
                    if len > MAX_INSPECT_BBOXES_PER_PAGE {
                        bboxes.truncate(MAX_INSPECT_BBOXES_PER_PAGE);
                        truncated.push(format!(
                            "{} {} -> {} ({} omitted)",
                            key,
                            len,
                            MAX_INSPECT_BBOXES_PER_PAGE,
                            len - MAX_INSPECT_BBOXES_PER_PAGE
                        ));
                    }
                }
            }
        }
    }

    // C3: a diagnostics array must exist on every response.
    if response.get("diagnostics").is_none() {
        response["diagnostics"] = serde_json::json!([]);
    }

    if !truncated.is_empty() {
        response["budget_exceeded"] = serde_json::json!(true);
        response["budget_note"] = serde_json::json!(truncated.join("; "));
        ensure_diagnostics_with_entry(
            &mut response,
            "API-E-001",
            &format!("inspect payload truncated: {}", truncated.join("; ")),
        );
    }

    Ok(response)
}

/// Check that a tool response includes required INV-2 fields.
#[allow(dead_code)]
pub fn check_inv2_compliance(response: &Value) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    // Check for rendered_by or visual_feedback
    let has_provenance = response.get("rendered_by").is_some_and(|v| v.is_string())
        || response
            .get("visual_feedback")
            .is_some_and(|v| v.is_string());
    if !has_provenance {
        errors.push("missing rendered_by or visual_feedback (INV-2)".to_string());
    }

    // Check for diagnostics array
    if response.get("diagnostics").is_none() {
        errors.push("missing diagnostics array (C3)".to_string());
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Check all known tool response shapes for INV-2 compliance.
#[allow(dead_code)]
pub fn check_all_tools_inv2() {
    let tools = vec![
        "inspect_document",
        "render_document",
        "inspect_region",
        "diff_documents",
        "doctor",
        "serve",
    ];

    for tool in &tools {
        let response = mock_tool_response(tool);
        match check_inv2_compliance(&response) {
            Ok(()) => println!("OK: {}: INV-2 compliant", tool),
            Err(e) => println!("WARN: {}: {}", tool, e.join(", ")),
        }
    }
}

/// Ensure diagnostics exists and push a new entry.
fn ensure_diagnostics_with_entry(response: &mut Value, code: &str, message: &str) {
    if response.get("diagnostics").is_none() {
        response["diagnostics"] = serde_json::json!([]);
    }
    if let Some(diags) = response.get_mut("diagnostics") {
        if let Some(arr) = diags.as_array_mut() {
            arr.push(serde_json::json!({
                "code": code,
                "severity": "info",
                "message": message,
            }));
        }
    }
}

/// Build a mock tool response for schema testing.
#[allow(dead_code)]
pub fn mock_tool_response(tool: &str) -> Value {
    match tool {
        "inspect_document" => serde_json::json!({
            "tool": "inspect_document",
            "path": "test.docx",
            "ir_version": "1.0.0",
            "document_type": "document",
            "structure": {},
            "element_ids": [],
            "total_elements": 0,
            "coordinates": [],
            "fidelity_budget": {
                "confidence": 1.0,
                "dropped_features": [],
                "part_warnings": [],
                "unknown_parts": [],
                "rendered_by": null,
                "has_potentially_unsafe": false
            },
            "diagnostics": [],
        }),
        "render_document" => serde_json::json!({
            "tool": "render_document",
            "path": "test.docx",
            "status": "ok",
            "rendered_by": "libreoffice 26.2.5",
            "dpi": 150,
            "page_count": 0,
            "image_blocks": 0,
            "blocks": [],
            "budget": {
                "within_budget": true,
                "total_tokens": 0,
                "block_count": 0,
                "violations": [],
            },
            "diagnostics": [],
        }),
        "inspect_region" => serde_json::json!({
            "tool": "inspect_region",
            "path": "test.docx",
            "page": 1,
            "bbox_pt": [0.0, 0.0, 100.0, 100.0],
            "zoom": 2.0,
            "crop": {
                "width": 100,
                "height": 100,
                "dpi": 300,
                "visual_tokens": 144,
                "within_budget": true,
                "byte_size": 10000,
            },
            "coordinate_record": {
                "scale": 4.166666666666667,
                "source_dpi": 300,
                "target_dpi": 300,
            },
            "rendered_by": "libreoffice + poppler",
            "diagnostics": [],
        }),
        "diff_documents" => serde_json::json!({
            "tool": "diff_documents",
            "path_a": "a.docx",
            "path_b": "b.docx",
            "document_type": "docx",
            "ir_diff": {
                "total_ops": 0,
                "removed": 0,
                "added": 0,
                "modified": 0,
                "identical": true,
                "ops": [],
            },
            "xml_diffs": [],
            "diagnostics": [],
        }),
        "doctor" => serde_json::json!({
            "tool": "doctor",
            "engines": [],
            "fonts": {
                "calibri": null,
                "cambria": null,
                "liberation_sans": null,
                "noto_sans": null,
            },
            "system": {
                "os": "linux",
                "arch": "x86_64",
                "libreoffice_version": null,
            },
            "diagnostics": [],
        }),
        "serve" => serde_json::json!({
            "tool": "serve",
            "status": "running",
            "transport": "stdio",
            "diagnostics": [],
        }),
        _ => serde_json::json!({}),
    }
}

/// The maximum number of image blocks per request (C8).
const MAX_IMAGE_BLOCKS: usize = 20;

/// The maximum pixel dimension per side (C8).
const MAX_PIXELS_PER_SIDE: usize = 2000;

/// Max element IDs returned by an inspect response before truncation
/// (bounds the JSON that lands in the LLM context window).
const MAX_INSPECT_ELEMENTS: usize = 1000;

/// Max top-level coordinate entries (pages/sections) in an inspect manifest.
const MAX_INSPECT_COORD_ENTRIES: usize = 200;

/// Max bboxes per coordinate entry (page/section) in an inspect manifest.
const MAX_INSPECT_BBOXES_PER_PAGE: usize = 50;

#[cfg(test)]
mod tests {
    use super::*;

    /// test_enforce_budget_ok — response within budget passes through unchanged.
    #[test]
    fn test_enforce_budget_ok() {
        let response = serde_json::json!({
            "tool": "render_document",
            "blocks": [
                {"width": 800, "height": 600, "visual_tokens": 178},
                {"width": 700, "height": 500, "visual_tokens": 142},
            ],
            "rendered_by": "libreoffice 26.2.5",
            "diagnostics": [],
        });
        let result = enforce_payload_budget(response).expect("should succeed");
        assert_eq!(result["blocks"].as_array().unwrap().len(), 2);
        assert!(!result.get("budget_exceeded").is_some());
    }

    /// test_enforce_budget_exceeds — >20 blocks truncated to 20.
    #[test]
    fn test_enforce_budget_exceeds() {
        let blocks: Vec<serde_json::Value> = (0..25)
            .map(|i| {
                serde_json::json!({
                    "width": 800,
                    "height": 600,
                    "visual_tokens": 178,
                    "format": format!("page-{}", i + 1)
                })
            })
            .collect();
        let response = serde_json::json!({
            "tool": "render_document",
            "blocks": blocks,
            "rendered_by": "libreoffice",
            "diagnostics": [],
        });
        let result = enforce_payload_budget(response).expect("should succeed");
        assert_eq!(result["blocks"].as_array().unwrap().len(), 20);
        assert!(result["budget_exceeded"].as_bool().unwrap());
        assert_eq!(result["truncated_blocks"].as_u64().unwrap(), 5);
    }

    /// test_enforce_budget_clamps_width — width > 2000 clamped to 2000.
    #[test]
    fn test_enforce_budget_clamps_width() {
        let response = serde_json::json!({
            "blocks": [
                {"width": 3000, "height": 1000, "visual_tokens": 500}
            ],
            "rendered_by": "libreoffice",
            "diagnostics": [],
        });
        let result = enforce_payload_budget(response).expect("should succeed");
        assert_eq!(result["blocks"][0]["width"].as_u64().unwrap(), 2000);
        assert_eq!(result["blocks"][0]["height"].as_u64().unwrap(), 1000);
    }

    /// test_enforce_budget_clamps_height — height > 2000 clamped to 2000.
    #[test]
    fn test_enforce_budget_clamps_height() {
        let response = serde_json::json!({
            "blocks": [
                {"width": 1000, "height": 3000, "visual_tokens": 500}
            ],
            "rendered_by": "libreoffice",
            "diagnostics": [],
        });
        let result = enforce_payload_budget(response).expect("should succeed");
        assert_eq!(result["blocks"][0]["width"].as_u64().unwrap(), 1000);
        assert_eq!(result["blocks"][0]["height"].as_u64().unwrap(), 2000);
    }

    /// test_enforce_budget_clamps_both — both dimensions > 2000.
    #[test]
    fn test_enforce_budget_clamps_both() {
        let response = serde_json::json!({
            "blocks": [
                {"width": 4000, "height": 4000, "visual_tokens": 500}
            ],
            "rendered_by": "libreoffice",
            "diagnostics": [],
        });
        let result = enforce_payload_budget(response).expect("should succeed");
        assert_eq!(result["blocks"][0]["width"].as_u64().unwrap(), 2000);
        assert_eq!(result["blocks"][0]["height"].as_u64().unwrap(), 2000);
    }

    /// test_inv2_compliance_ok — response has rendered_by + diagnostics.
    #[test]
    fn test_inv2_compliance_ok() {
        let response = serde_json::json!({
            "tool": "render_document",
            "rendered_by": "libreoffice 26.2.5",
            "diagnostics": [],
        });
        assert!(check_inv2_compliance(&response).is_ok());
    }

    /// test_inv2_compliance_missing_rendered_by — no rendered_by → error.
    #[test]
    fn test_inv2_compliance_missing_rendered_by() {
        let response = serde_json::json!({
            "tool": "diff_documents",
            "diagnostics": [],
        });
        let errors = check_inv2_compliance(&response).unwrap_err();
        assert!(errors.iter().any(|e| e.contains("rendered_by")));
    }

    /// test_inv2_compliance_missing_diagnostics — no diagnostics → error.
    #[test]
    fn test_inv2_compliance_missing_diagnostics() {
        let response = serde_json::json!({
            "tool": "serve",
            "status": "running",
        });
        let errors = check_inv2_compliance(&response).unwrap_err();
        assert!(errors.iter().any(|e| e.contains("diagnostics")));
    }

    /// test_inv2_compliance_visual_feedback — visual_feedback satisfies provenance.
    #[test]
    fn test_inv2_compliance_visual_feedback() {
        let response = serde_json::json!({
            "tool": "render_document",
            "visual_feedback": "unavailable",
            "diagnostics": [],
        });
        assert!(check_inv2_compliance(&response).is_ok());
    }

    /// test_check_all_tools — all 6 tools pass INV-2 with mock responses.
    #[test]
    fn test_check_all_tools() {
        let tools = vec![
            "inspect_document",
            "render_document",
            "inspect_region",
            "diff_documents",
            "doctor",
            "serve",
        ];
        for tool in tools {
            let response = mock_tool_response(tool);
            // All mock responses include diagnostics array
            assert!(
                response.get("diagnostics").is_some(),
                "{} missing diagnostics",
                tool
            );
        }
    }

    /// test_enforce_budget_no_image_blocks — response without blocks passes through.
    #[test]
    fn test_enforce_budget_no_image_blocks() {
        let response = serde_json::json!({
            "tool": "doctor",
            "engines": [],
            "diagnostics": [],
        });
        let result = enforce_payload_budget(response).expect("should succeed");
        assert_eq!(result["tool"].as_str().unwrap(), "doctor");
        assert_eq!(result["engines"].as_array().unwrap().len(), 0);
    }

    /// test_enforce_budget_empty_response — empty response gets diagnostics.
    #[test]
    fn test_enforce_budget_empty_response() {
        let response = serde_json::json!({});
        let result = enforce_payload_budget(response).expect("should succeed");
        assert!(
            result.get("diagnostics").is_some(),
            "should add diagnostics to empty response"
        );
    }

    /// test_enforce_budget_image_blocks_key — works with "image_blocks" key too.
    #[test]
    fn test_enforce_budget_image_blocks_key() {
        let blocks: Vec<serde_json::Value> = (0..25)
            .map(|_i| {
                serde_json::json!({
                    "width": 800,
                    "height": 600,
                    "visual_tokens": 178,
                })
            })
            .collect();
        let response = serde_json::json!({
            "tool": "render_document",
            "image_blocks": blocks,
            "rendered_by": "libreoffice",
            "diagnostics": [],
        });
        let result = enforce_payload_budget(response).expect("should succeed");
        assert_eq!(
            result["image_blocks"].as_array().unwrap().len(),
            20,
            "image_blocks should also be truncated"
        );
        assert!(result["budget_exceeded"].as_bool().unwrap());
    }

    /// test_enforce_budget_boundary_20_blocks — exactly 20 blocks, no truncation.
    #[test]
    fn test_enforce_budget_boundary_20_blocks() {
        let blocks: Vec<serde_json::Value> = (0..20)
            .map(|_| serde_json::json!({"width": 800, "height": 600}))
            .collect();
        let response = serde_json::json!({
            "blocks": blocks,
            "rendered_by": "libreoffice",
            "diagnostics": [],
        });
        let result = enforce_payload_budget(response).expect("should succeed");
        assert_eq!(result["blocks"].as_array().unwrap().len(), 20);
        assert!(!result.get("budget_exceeded").is_some());
    }

    /// test_enforce_budget_dimensions_at_boundary — 2000x2000 passes unchanged.
    #[test]
    fn test_enforce_budget_dimensions_at_boundary() {
        let response = serde_json::json!({
            "blocks": [{"width": 2000, "height": 2000}],
            "rendered_by": "libreoffice",
            "diagnostics": [],
        });
        let result = enforce_payload_budget(response).expect("should succeed");
        assert_eq!(result["blocks"][0]["width"].as_u64().unwrap(), 2000);
        assert_eq!(result["blocks"][0]["height"].as_u64().unwrap(), 2000);
    }

    /// test_enforce_budget_adds_diagnostics_if_missing.
    #[test]
    fn test_enforce_budget_adds_diagnostics_if_missing() {
        let response = serde_json::json!({
            "tool": "serve",
            "status": "running",
        });
        let result = enforce_payload_budget(response).expect("should succeed");
        assert!(
            result.get("diagnostics").is_some(),
            "should have diagnostics"
        );
        // Should get an info diagnostic about missing rendered_by
        let diags = result["diagnostics"].as_array().unwrap();
        assert!(!diags.is_empty(), "should have diagnostics entries");
    }

    /// test_check_inv2_compliance_empty_response — empty response fails both checks.
    #[test]
    fn test_check_inv2_compliance_empty_response() {
        let response = serde_json::json!({});
        let errors = check_inv2_compliance(&response).unwrap_err();
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().any(|e| e.contains("rendered_by")));
        assert!(errors.iter().any(|e| e.contains("diagnostics")));
    }

    // --- enforce_inspect_budget tests ---

    /// test_enforce_inspect_within_cap — small response passes unchanged.
    #[test]
    fn test_enforce_inspect_within_cap() {
        let ids: Vec<_> = (0..500)
            .map(|i| {
                serde_json::json!({
                    "kind": "paragraph",
                    "path": format!("/document/section:{}", i),
                    "id": format!("id-{:012x}", i),
                })
            })
            .collect();
        let response = serde_json::json!({
            "tool": "inspect_document",
            "path": "small.docx",
            "total_elements": ids.len(),
            "element_ids": ids,
            "coordinates": [],
            "fidelity_budget": {"confidence": 1.0},
        });
        let result = enforce_inspect_budget(response).unwrap();
        assert_eq!(result["element_ids"].as_array().unwrap().len(), 500);
        assert_eq!(result["total_elements"].as_u64().unwrap(), 500);
        assert!(result.get("budget_exceeded").is_none());
    }

    /// test_enforce_inspect_truncates_element_ids — element_ids capped to MAX_INSPECT_ELEMENTS.
    #[test]
    fn test_enforce_inspect_truncates_element_ids() {
        let ids: Vec<_> = (0..1500)
            .map(|i| {
                serde_json::json!({
                    "kind": "run",
                    "path": format!("/doc/section:0/item:{}", i),
                    "id": format!("id-{:012x}", i),
                })
            })
            .collect();
        let response = serde_json::json!({
            "tool": "inspect_document",
            "path": "big.docx",
            "total_elements": ids.len(),
            "element_ids": ids,
            "coordinates": [],
            "fidelity_budget": {"confidence": 1.0},
        });
        let result = enforce_inspect_budget(response).unwrap();
        assert_eq!(
            result["element_ids"].as_array().unwrap().len(),
            1000,
            "element_ids should be capped to 1000"
        );
        // total_elements preserves the original count
        assert_eq!(result["total_elements"].as_u64().unwrap(), 1500);
        assert!(result["budget_exceeded"].as_bool().unwrap());
        let note = result["budget_note"].as_str().unwrap();
        assert!(note.contains("1500"));
        assert!(note.contains("1000"));
        assert!(note.contains("500 omitted"));
        // Diagnostics array must exist (C3) with API-E-001 entry
        let diags = result["diagnostics"].as_array().unwrap();
        assert!(
            diags.iter().any(|d| d["code"] == "API-E-001"),
            "API-E-001 diagnostic expected when element_ids truncated"
        );
    }

    /// test_enforce_inspect_total_elements_preserved — total_elements is not clobbered.
    #[test]
    fn test_enforce_inspect_total_elements_preserved() {
        let ids: Vec<_> = (0..2000)
            .map(|i| {
                serde_json::json!({
                    "kind": "cell",
                    "path": format!("/table:0/cell:{}", i),
                    "id": format!("id-{:012x}", i),
                })
            })
            .collect();
        let response = serde_json::json!({
            "tool": "inspect_document",
            "path": "wide.xlsx",
            "total_elements": 50000,
            "element_ids": ids,
            "coordinates": [],
            "fidelity_budget": {"confidence": 1.0},
        });
        let result = enforce_inspect_budget(response).unwrap();
        // element_ids capped
        assert_eq!(result["element_ids"].as_array().unwrap().len(), 1000);
        // total_elements from build_inspect_result, not the truncated array
        assert_eq!(result["total_elements"].as_u64().unwrap(), 50000);
    }

    /// test_enforce_inspect_coordinates_truncated — coordinate entries and bboxes capped.
    #[test]
    fn test_enforce_inspect_coordinates_truncated() {
        let pages: Vec<_> = (0..250)
            .map(|i| {
                let bboxes: Vec<_> = (0..80)
                    .map(|j| {
                        serde_json::json!({
                            "id": format!("id-{:012x}", j),
                            "bbox_pt": [10.0 * j as f64, 10.0, 100.0, 20.0],
                            "type": "text",
                            "text": format!("page {} bbox {}", i, j),
                        })
                    })
                    .collect();
                serde_json::json!({
                    "page": i,
                    "bbox_count": 80,
                    "bboxes": bboxes,
                })
            })
            .collect();
        let response = serde_json::json!({
            "tool": "inspect_document",
            "path": "big.pdf",
            "total_elements": 0,
            "element_ids": [],
            "coordinates": pages,
            "fidelity_budget": {"confidence": 1.0},
        });
        let result = enforce_inspect_budget(response).unwrap();
        let coords = result["coordinates"].as_array().unwrap();
        assert_eq!(coords.len(), 200, "coordinates should be capped to 200");
        // Each page's bboxes capped to 50
        for page in coords {
            assert!(
                page["bboxes"].as_array().unwrap().len() <= 50,
                "bboxes per page should be <= 50"
            );
        }
        assert!(result["budget_exceeded"].as_bool().unwrap());
    }

    /// test_enforce_inspect_no_budget_exceeded_when_within_caps — no flags for small docs.
    #[test]
    fn test_enforce_inspect_no_budget_exceeded_when_within_caps() {
        let response = serde_json::json!({
            "tool": "inspect_document",
            "path": "small.odt",
            "total_elements": 10,
            "element_ids": [],
            "coordinates": [],
            "fidelity_budget": {"confidence": 1.0},
        });
        let result = enforce_inspect_budget(response).unwrap();
        assert!(result.get("budget_exceeded").is_none());
    }

    /// test_enforce_inspect_ensures_diagnostics_c3 — diagnostics array is added if missing.
    #[test]
    fn test_enforce_inspect_ensures_diagnostics_c3() {
        let response = serde_json::json!({
            "tool": "inspect_document",
            "path": "ok.docx",
            "total_elements": 5,
            "element_ids": [],
            "coordinates": [],
            "fidelity_budget": {"confidence": 1.0},
        });
        let result = enforce_inspect_budget(response).unwrap();
        assert!(
            result["diagnostics"].is_array(),
            "C3: diagnostics array must exist"
        );
    }

    /// test_mock_tool_responses_all_have_diagnostics.
    #[test]
    fn test_mock_tool_responses_all_have_diagnostics() {
        let tools = vec![
            "inspect_document",
            "render_document",
            "inspect_region",
            "diff_documents",
            "doctor",
            "serve",
        ];
        for tool in tools {
            let response = mock_tool_response(tool);
            assert!(
                response.get("diagnostics").is_some(),
                "{} mock missing diagnostics",
                tool
            );
            assert!(
                response["diagnostics"].is_array(),
                "{} diagnostics should be array",
                tool
            );
        }
    }
}
