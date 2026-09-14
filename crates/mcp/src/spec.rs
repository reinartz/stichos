//! Tool specification types for C7 contract compliance.

/// A tool parameter definition.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ParamDef {
    pub name: &'static str,
    pub param_type: ParamType,
    pub required: bool,
    pub description: &'static str,
}

/// Parameter types.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ParamType {
    String,
    Number,
    Boolean,
    StringArray,
    NumberArray,
}

/// A tool definition compliant with the C7 contract.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    #[serde(skip, default = "default_params")]
    pub params: &'static [ParamDef],
    pub idempotent: bool,
    pub deprecated: bool,
    pub deprecation_notice: Option<&'static str>,
    /// Schema version for this tool (C7 v1).
    pub schema_version: &'static str,
}

fn default_params() -> &'static [ParamDef] {
    &[]
}

/// All Phase 1 read tools (C7).
pub const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "inspect_document",
        description: "Extract structure, element IDs, coordinates manifest, and fidelity budget.",
        params: &[
            ParamDef {
                name: "path",
                param_type: ParamType::String,
                required: true,
                description: "Path to document",
            },
            ParamDef {
                name: "format",
                param_type: ParamType::String,
                required: false,
                description: "Format hint (inferred if omitted)",
            },
        ],
        idempotent: true,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.0.0",
    },
    ToolSpec {
        name: "render_document",
        description: "Render document to page images with layout QA.",
        params: &[
            ParamDef {
                name: "path",
                param_type: ParamType::String,
                required: true,
                description: "Path to document",
            },
            ParamDef {
                name: "dpi",
                param_type: ParamType::Number,
                required: false,
                description: "Render DPI (default 150)",
            },
            ParamDef {
                name: "page_range",
                param_type: ParamType::NumberArray,
                required: false,
                description: "Page numbers to render (1-indexed)",
            },
        ],
        idempotent: true,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.0.0",
    },
    ToolSpec {
        name: "inspect_region",
        description: "Crop and zoom a region from a document for detailed inspection.",
        params: &[
            ParamDef {
                name: "path",
                param_type: ParamType::String,
                required: true,
                description: "Path to document",
            },
            ParamDef {
                name: "page",
                param_type: ParamType::Number,
                required: true,
                description: "Page number (1-indexed)",
            },
            ParamDef {
                name: "bbox_pt",
                param_type: ParamType::NumberArray,
                required: true,
                description: "Bounding box in points [x, y, w, h]",
            },
            ParamDef {
                name: "zoom",
                param_type: ParamType::Number,
                required: false,
                description: "Zoom multiplier (default 2.0)",
            },
        ],
        idempotent: true,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.0.0",
    },
    ToolSpec {
        name: "diff_documents",
        description: "Compare two documents and report structural and visual differences.",
        params: &[
            ParamDef {
                name: "path_a",
                param_type: ParamType::String,
                required: true,
                description: "Path to first document",
            },
            ParamDef {
                name: "path_b",
                param_type: ParamType::String,
                required: true,
                description: "Path to second document",
            },
            ParamDef {
                name: "format",
                param_type: ParamType::String,
                required: false,
                description: "Format hint",
            },
        ],
        idempotent: true,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.0.0",
    },
    ToolSpec {
        name: "doctor",
        description: "Probe the host: check engine availability and render test fixtures.",
        params: &[],
        idempotent: true,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.0.0",
    },
    ToolSpec {
        name: "serve",
        description: "Start the MCP transport server (stdio or HTTP).",
        params: &[
            ParamDef {
                name: "transport",
                param_type: ParamType::String,
                required: false,
                description: "Transport: stdio or http",
            },
            ParamDef {
                name: "port",
                param_type: ParamType::Number,
                required: false,
                description: "HTTP port (default 8731)",
            },
        ],
        idempotent: true,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.0.0",
    },
];

/// Check if a tool name is valid per C7.
pub fn is_valid_tool(name: &str) -> bool {
    TOOLS.iter().any(|t| t.name == name)
}

/// Get a tool spec by name.
pub fn get_tool(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|t| t.name == name)
}

/// Get all non-deprecated tool names.
pub fn active_tool_names() -> Vec<&'static str> {
    TOOLS
        .iter()
        .filter(|t| !t.deprecated)
        .map(|t| t.name)
        .collect()
}

/// Verify C7 compliance: all tools have schema_version "1.0.0" and are idempotent.
pub fn verify_c7_compliance() -> Result<Vec<&'static str>, String> {
    let mut errors = Vec::new();
    for tool in TOOLS {
        if tool.schema_version != "1.0.0" {
            errors.push(format!(
                "tool '{}' has version '{}', expected '1.0.0'",
                tool.name, tool.schema_version
            ));
        }
        if !tool.idempotent {
            errors.push(format!("tool '{}' is not idempotent", tool.name));
        }
    }
    if errors.is_empty() {
        Ok(active_tool_names())
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // test_all_tools_defined — all 6 C7 tools present
    #[test]
    fn test_all_tools_defined() {
        assert_eq!(TOOLS.len(), 6, "expected exactly 6 Phase 1 tools");
        let names: Vec<_> = TOOLS.iter().map(|t| t.name).collect();
        assert!(names.contains(&"inspect_document"));
        assert!(names.contains(&"render_document"));
        assert!(names.contains(&"inspect_region"));
        assert!(names.contains(&"diff_documents"));
        assert!(names.contains(&"doctor"));
        assert!(names.contains(&"serve"));
    }

    // test_all_tools_idempotent — all tools are idempotent
    #[test]
    fn test_all_tools_idempotent() {
        for tool in TOOLS {
            assert!(tool.idempotent, "tool '{}' must be idempotent", tool.name);
        }
    }

    // test_all_tools_schema_version — all tools have version 1.0.0
    #[test]
    fn test_all_tools_schema_version() {
        for tool in TOOLS {
            assert_eq!(
                tool.schema_version, "1.0.0",
                "tool '{}' version is '{}', expected '1.0.0'",
                tool.name, tool.schema_version
            );
        }
    }

    // test_valid_tool_names — known tools return true
    #[test]
    fn test_valid_tool_names() {
        assert!(is_valid_tool("inspect_document"));
        assert!(is_valid_tool("render_document"));
        assert!(is_valid_tool("inspect_region"));
        assert!(is_valid_tool("diff_documents"));
        assert!(is_valid_tool("doctor"));
        assert!(is_valid_tool("serve"));
    }

    // test_invalid_tool_name — unknown tool returns false
    #[test]
    fn test_invalid_tool_name() {
        assert!(!is_valid_tool("apply"));
        assert!(!is_valid_tool("delete"));
        assert!(!is_valid_tool("foobar"));
        assert!(!is_valid_tool(""));
    }

    // test_get_tool_by_name — each tool is findable
    #[test]
    fn test_get_tool_by_name() {
        assert!(get_tool("inspect_document").is_some());
        assert!(get_tool("render_document").is_some());
        assert!(get_tool("inspect_region").is_some());
        assert!(get_tool("diff_documents").is_some());
        assert!(get_tool("doctor").is_some());
        assert!(get_tool("serve").is_some());
    }

    // test_get_tool_not_found — unknown name returns None
    #[test]
    fn test_get_tool_not_found() {
        assert!(get_tool("apply").is_none());
        assert!(get_tool("foobar").is_none());
        assert!(get_tool("").is_none());
    }

    // test_active_tool_names — no deprecated tools
    #[test]
    fn test_active_tool_names() {
        let active = active_tool_names();
        assert_eq!(
            active.len(),
            6,
            "all 6 tools should be active (none deprecated)"
        );
        for name in &active {
            assert!(
                !get_tool(name).unwrap().deprecated,
                "tool '{}' should not be deprecated",
                name
            );
        }
    }

    // test_verify_c7_compliance — all tools pass compliance
    #[test]
    fn test_verify_c7_compliance() {
        let result = verify_c7_compliance();
        assert!(
            result.is_ok(),
            "C7 compliance check failed: {}",
            result.unwrap_err()
        );
        let names = result.unwrap();
        assert_eq!(
            names.len(),
            6,
            "all 6 tools should be active after compliance check"
        );
    }

    // test_tool_spec_serde — roundtrip serialization
    #[test]
    fn test_tool_spec_serde() {
        for tool in TOOLS {
            let json = serde_json::to_string(tool).expect("serialize tool");
            let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
            assert_eq!(value["name"], tool.name);
            assert_eq!(value["description"], tool.description);
            assert_eq!(value["idempotent"], true);
            assert_eq!(value["schema_version"], "1.0.0");
            assert_eq!(value["deprecated"], false);
        }
    }

    // test_param_def_serde — parameter roundtrip
    #[test]
    fn test_param_def_serde() {
        for tool in TOOLS {
            for param in tool.params {
                let json = serde_json::to_string(param).expect("serialize param");
                let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
                assert_eq!(value["name"], param.name);
                assert_eq!(value["required"], param.required);
                assert_eq!(value["description"], param.description);
                assert_eq!(
                    value["param_type"],
                    match param.param_type {
                        ParamType::String => "String",
                        ParamType::Number => "Number",
                        ParamType::Boolean => "Boolean",
                        ParamType::StringArray => "StringArray",
                        ParamType::NumberArray => "NumberArray",
                    }
                );
            }
        }
    }

    // test_no_apply_tool — verify "apply" is not in TOOLS
    #[test]
    fn test_no_apply_tool() {
        let names: Vec<_> = TOOLS.iter().map(|t| t.name).collect();
        assert!(
            !names.contains(&"apply"),
            "'apply' must not be a Phase 1 tool (read-only)"
        );
    }
}
