// SPDX-License-Identifier: MIT OR Apache-2.0
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
        schema_version: "1.6.0",
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
        schema_version: "1.6.0",
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
        schema_version: "1.6.0",
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
            ParamDef {
                name: "max_ops",
                param_type: ParamType::Number,
                required: false,
                description: "Maximum number of diff operations to return (default unlimited)",
            },
        ],
        idempotent: true,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.6.0",
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
    ToolSpec {
        name: "apply_patch",
        description: "Apply a surgical patch to a document and return the result with diff, provenance, and fidelity budget.",
        params: &[
            ParamDef {
                name: "path",
                param_type: ParamType::String,
                required: true,
                description: "Path to the document file",
            },
            ParamDef {
                name: "ops",
                param_type: ParamType::StringArray,
                required: true,
                description: "JSON patch operations array (PatchOp variants)",
            },
            ParamDef {
                name: "dry_run",
                param_type: ParamType::Boolean,
                required: false,
                description: "If true, only compute preview (IR diff), no writes (default false)",
            },
            ParamDef {
                name: "format",
                param_type: ParamType::String,
                required: false,
                description: "Format hint (inferred from extension if omitted)",
            },
            ParamDef {
                name: "session_id",
                param_type: ParamType::String,
                required: false,
                description: "Journal session ID (auto-generated if omitted)",
            },
            ParamDef {
                name: "direct",
                param_type: ParamType::Boolean,
                required: false,
                description: "Apply directly without tracked-revisions (default false for docx)",
            },
            ParamDef {
                name: "author",
                param_type: ParamType::String,
                required: false,
                description: "Author name for revision markup (default: agent)",
            },
        ],
        idempotent: false,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.6.0",
    },
    ToolSpec {
        name: "edit_text",
        description: "Replace text in an element by ID. Idempotent: same args produces same result.",
        params: &[
            ParamDef {
                name: "path",
                param_type: ParamType::String,
                required: true,
                description: "Path to the document file",
            },
            ParamDef {
                name: "element_id",
                param_type: ParamType::String,
                required: true,
                description: "Element ID to edit",
            },
            ParamDef {
                name: "old_text",
                param_type: ParamType::String,
                required: true,
                description: "Current text to replace",
            },
            ParamDef {
                name: "new_text",
                param_type: ParamType::String,
                required: true,
                description: "New text to insert",
            },
            ParamDef {
                name: "dry_run",
                param_type: ParamType::Boolean,
                required: false,
                description: "If true, only preview changes",
            },
            ParamDef {
                name: "format",
                param_type: ParamType::String,
                required: false,
                description: "Format hint",
            },
            ParamDef {
                name: "direct",
                param_type: ParamType::Boolean,
                required: false,
                description: "Apply directly without tracked-revisions (default false for docx)",
            },
            ParamDef {
                name: "author",
                param_type: ParamType::String,
                required: false,
                description: "Author name for revision markup (default: agent)",
            },
        ],
        idempotent: true,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.6.0",
    },
    ToolSpec {
        name: "edit_cell",
        description: "Set a spreadsheet cell value by element ID. Idempotent: same args produces same result.",
        params: &[
            ParamDef {
                name: "path",
                param_type: ParamType::String,
                required: true,
                description: "Path to the spreadsheet file",
            },
            ParamDef {
                name: "element_id",
                param_type: ParamType::String,
                required: true,
                description: "Cell element ID",
            },
            ParamDef {
                name: "value",
                param_type: ParamType::String,
                required: true,
                description: "New cell value",
            },
            ParamDef {
                name: "dry_run",
                param_type: ParamType::Boolean,
                required: false,
                description: "If true, only preview changes",
            },
            ParamDef {
                name: "format",
                param_type: ParamType::String,
                required: false,
                description: "Format hint",
            },
            ParamDef {
                name: "direct",
                param_type: ParamType::Boolean,
                required: false,
                description: "Apply directly without tracked-revisions (default false for docx)",
            },
            ParamDef {
                name: "author",
                param_type: ParamType::String,
                required: false,
                description: "Author name for revision markup (default: agent)",
            },
        ],
        idempotent: true,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.6.0",
    },
    ToolSpec {
        name: "edit_structure",
        description: "Insert or delete a structural element (paragraph, etc.) by element ID. Idempotent: same args produces same result.",
        params: &[
            ParamDef {
                name: "path",
                param_type: ParamType::String,
                required: true,
                description: "Path to the document file",
            },
            ParamDef {
                name: "element_id",
                param_type: ParamType::String,
                required: true,
                description: "Element ID for the operation",
            },
            ParamDef {
                name: "action",
                param_type: ParamType::String,
                required: true,
                description: "Action: insert or delete",
            },
            ParamDef {
                name: "kind",
                param_type: ParamType::String,
                required: false,
                description: "Element kind (e.g., paragraph) for insert",
            },
            ParamDef {
                name: "dry_run",
                param_type: ParamType::Boolean,
                required: false,
                description: "If true, only preview changes",
            },
            ParamDef {
                name: "format",
                param_type: ParamType::String,
                required: false,
                description: "Format hint",
            },
            ParamDef {
                name: "direct",
                param_type: ParamType::Boolean,
                required: false,
                description: "Apply directly without tracked-revisions (default false for docx)",
            },
            ParamDef {
                name: "author",
                param_type: ParamType::String,
                required: false,
                description: "Author name for revision markup (default: agent)",
            },
        ],
        idempotent: true,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.6.0",
    },
    ToolSpec {
        name: "annotate_pages",
        description: "Annotate rendered pages with numbered lint boxes. Returns marked+clean image pairs.",
        params: &[
            ParamDef {
                name: "path",
                param_type: ParamType::String,
                required: true,
                description: "Path to the document file",
            },
            ParamDef {
                name: "dpi",
                param_type: ParamType::Number,
                required: false,
                description: "Render DPI for annotation (default 150)",
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
        schema_version: "1.6.0",
    },
    ToolSpec {
        name: "revert_document",
        description: "Revert a document to a previous journal entry. Returns byte-identical content from the target version.",
        params: &[
            ParamDef {
                name: "path",
                param_type: ParamType::String,
                required: true,
                description: "Path to the document file",
            },
            ParamDef {
                name: "steps",
                param_type: ParamType::Number,
                required: false,
                description: "Number of steps to revert (default 1)",
            },
            ParamDef {
                name: "session_id",
                param_type: ParamType::String,
                required: false,
                description: "Journal session ID",
            },
            ParamDef {
                name: "format",
                param_type: ParamType::String,
                required: false,
                description: "Format hint",
            },
        ],
        idempotent: false,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.1.0",
    },
    ToolSpec {
        name: "journal",
        description: "List journal entries for a document session. Shows version chain with pre/post hashes.",
        params: &[
            ParamDef {
                name: "path",
                param_type: ParamType::String,
                required: true,
                description: "Path to the document file",
            },
            ParamDef {
                name: "session_id",
                param_type: ParamType::String,
                required: false,
                description: "Journal session ID (list current if omitted)",
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
        schema_version: "1.6.0",
    },
    // P3-23: create_document tool (C7 v1.2 additive)
    ToolSpec {
        name: "create_document",
        description: "Create a new document from source content (markdown, html, typst) and return the result with provenance and fidelity budget. Response includes store_root path (null for dry_run) for blob retrieval when not dry_run.",
        params: &[
            ParamDef {
                name: "source_type",
                param_type: ParamType::String,
                required: true,
                description: "Source format: markdown, html, or typ",
            },
            ParamDef {
                name: "format",
                param_type: ParamType::String,
                required: true,
                description: "Target format: docx, odt, or pdf",
            },
            ParamDef {
                name: "content",
                param_type: ParamType::String,
                required: true,
                description: "Source content as string",
            },
            ParamDef {
                name: "template",
                param_type: ParamType::String,
                required: false,
                description: "Path to template/reference document",
            },
            ParamDef {
                name: "dry_run",
                param_type: ParamType::Boolean,
                required: false,
                description: "If true, only compute preview without writing (default false)",
            },
            ParamDef {
                name: "target",
                param_type: ParamType::String,
                required: false,
                description: "Path to existing docx for create-as-revisions mode",
            },
            ParamDef {
                name: "author",
                param_type: ParamType::String,
                required: false,
                description: "Author name for revision markup (default: agent)",
            },
            ParamDef {
                name: "direct",
                param_type: ParamType::Boolean,
                required: false,
                description: "Apply directly without tracked-revisions (default false)",
            },
        ],
        idempotent: false,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.2.1",
    },
    // P3-24 Wave F: script_tool (C7 v1.2 additive)
    ToolSpec {
        name: "script_tool",
        description: "Execute a sandboxed Rhai script against the document store.",
        params: &[
            ParamDef {
                name: "script",
                param_type: ParamType::String,
                required: true,
                description: "Rhai script source code to execute",
            },
            ParamDef {
                name: "ir",
                param_type: ParamType::String,
                required: false,
                description: "IR document as JSON (optional)",
            },
        ],
        idempotent: false,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.2.0",
    },
    // P3-29 Wave G: check_update tool (C7 v1.2 additive)
    ToolSpec {
        name: "check_update",
        description: "Check for available stichos updates. Returns version info and whether an update is available. Does not install.",
        params: &[
            ParamDef {
                name: "channel",
                param_type: ParamType::String,
                required: false,
                description: "Update channel to check (default: stable)",
            },
        ],
        idempotent: true,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.2.0",
    },
    // P4-08: list_revisions tool (C7 v1.3.0 additive)
    ToolSpec {
        name: "list_revisions",
        description: "List all tracked revisions in a document with metadata (kind, author, date, anchor).",
        params: &[
            ParamDef {
                name: "path",
                param_type: ParamType::String,
                required: true,
                description: "Path to document",
            },
            ParamDef {
                name: "author",
                param_type: ParamType::String,
                required: false,
                description: "Filter by author",
            },
        ],
        idempotent: true,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.3.0",
    },
    // P4-08: accept_revisions tool (C7 v1.3.0 additive)
    ToolSpec {
        name: "accept_revisions",
        description: "Accept tracked revisions in a document. Returns the modified document with revisions applied.",
        params: &[
            ParamDef {
                name: "path",
                param_type: ParamType::String,
                required: true,
                description: "Path to document",
            },
            ParamDef {
                name: "ids",
                param_type: ParamType::StringArray,
                required: false,
                description: "Revision IDs to accept (accepts all if omitted)",
            },
            ParamDef {
                name: "all",
                param_type: ParamType::Boolean,
                required: false,
                description: "Accept all revisions (default false)",
            },
            ParamDef {
                name: "author",
                param_type: ParamType::String,
                required: false,
                description: "Accept only revisions by this author",
            },
            ParamDef {
                name: "dry_run",
                param_type: ParamType::Boolean,
                required: false,
                description: "Preview changes without applying (default false)",
            },
            ParamDef {
                name: "session_id",
                param_type: ParamType::String,
                required: false,
                description: "Journal session ID",
            },
        ],
        idempotent: false,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.3.0",
    },
    // P4-08: reject_revisions tool (C7 v1.3.0 additive)
    ToolSpec {
        name: "reject_revisions",
        description: "Reject tracked revisions in a document. Returns the modified document with revisions removed.",
        params: &[
            ParamDef {
                name: "path",
                param_type: ParamType::String,
                required: true,
                description: "Path to document",
            },
            ParamDef {
                name: "ids",
                param_type: ParamType::StringArray,
                required: false,
                description: "Revision IDs to reject (rejects all if omitted)",
            },
            ParamDef {
                name: "all",
                param_type: ParamType::Boolean,
                required: false,
                description: "Reject all revisions (default false)",
            },
            ParamDef {
                name: "author",
                param_type: ParamType::String,
                required: false,
                description: "Reject only revisions by this author",
            },
            ParamDef {
                name: "dry_run",
                param_type: ParamType::Boolean,
                required: false,
                description: "Preview changes without applying (default false)",
            },
            ParamDef {
                name: "session_id",
                param_type: ParamType::String,
                required: false,
                description: "Journal session ID",
            },
        ],
        idempotent: false,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.3.0",
    },
    // P4-14: add_comment tool (C7 v1.3.0 additive)
    ToolSpec {
        name: "add_comment",
        description: "Add a comment to a document. The comment is anchored to an element ID (docx) or inserted as office:annotation (odt).",
        params: &[
            ParamDef {
                name: "path",
                param_type: ParamType::String,
                required: true,
                description: "Path to the document file.",
            },
            ParamDef {
                name: "text",
                param_type: ParamType::String,
                required: true,
                description: "Comment text.",
            },
            ParamDef {
                name: "element_id",
                param_type: ParamType::String,
                required: true,
                description: "Element ID to anchor the comment.",
            },
            ParamDef {
                name: "author",
                param_type: ParamType::String,
                required: false,
                description: "Comment author. Default: reviewer.",
            },
            ParamDef {
                name: "initials",
                param_type: ParamType::String,
                required: false,
                description: "Author initials.",
            },
            ParamDef {
                name: "dry_run",
                param_type: ParamType::Boolean,
                required: false,
                description: "Preview without writing.",
            },
        ],
        idempotent: false,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.3.0",
    },
    // P5-06: list_annotations tool (C7 v1.4.0 additive)
    ToolSpec {
        name: "list_annotations",
        description: "List all PDF annotations in a document. Returns annotation IDs, types, pages, contents, and review state.",
        params: &[
            ParamDef {
                name: "path",
                param_type: ParamType::String,
                required: true,
                description: "Path to the PDF document.",
            },
        ],
        idempotent: true,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.4.0",
    },
    // P5-06: resolve_annotations tool (C7 v1.4.0 additive)
    ToolSpec {
        name: "resolve_annotations",
        description: "Resolve PDF annotations by ID. Applies incremental write-back to mark annotations as resolved.",
        params: &[
            ParamDef {
                name: "path",
                param_type: ParamType::String,
                required: true,
                description: "Path to the PDF document.",
            },
            ParamDef {
                name: "annotation_ids",
                param_type: ParamType::StringArray,
                required: true,
                description: "Annotation IDs to resolve.",
            },
            ParamDef {
                name: "dry_run",
                param_type: ParamType::Boolean,
                required: false,
                description: "Preview changes without writing (default false).",
            },
        ],
        idempotent: false,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.4.0",
    },
    // P5-13: sign_document tool (C7 v1.5.0 additive — PAdES-B-B sign-later)
    ToolSpec {
        name: "sign_document",
        description: "Sign a PDF document using PAdES-B-B sign-later. Creates a digital signature over the document.",
        params: &[
            ParamDef {
                name: "path",
                param_type: ParamType::String,
                required: true,
                description: "Path to the unsigned PDF document.",
            },
            ParamDef {
                name: "subject",
                param_type: ParamType::String,
                required: false,
                description: "Certificate subject CN (default: 'stichos').",
            },
        ],
        idempotent: false,
        deprecated: false,
        deprecation_notice: None,
        schema_version: "1.5.0",
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

/// Verify C7 compliance: read-only tools have version "1.0.0" and are idempotent;
/// additive tools (v1.1+) may be non-idempotent but must be documented.
pub fn verify_c7_compliance() -> Result<Vec<&'static str>, String> {
    let mut errors = Vec::new();
    for tool in TOOLS {
        match tool.schema_version {
            "1.0.0" => {
                if !tool.idempotent {
                    errors.push(format!("tool '{}' (1.0.0) is not idempotent", tool.name));
                }
            }
            "1.1.0" | "1.2.0" | "1.2.1" | "1.3.0" | "1.4.0" | "1.5.0" | "1.6.0" => {
                // v1.1/v1.2/v1.2.1/v1.3/v1.4/v1.5/v1.6 additive: non-idempotent tools allowed
            }
            ver => {
                errors.push(format!(
                    "tool '{}' has unexpected version '{}', expected '1.0.0', '1.1.0', '1.2.0', '1.2.1', '1.3.0', '1.4.0', '1.5.0', or '1.6.0'",
                    tool.name, ver
                ));
            }
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
    //! C7 v1.1 — Tool Schema Contract (additive freeze).
    //! All changes since v1.0.0 are additive: new tools, new optional params.
    //! Breaking changes require a minor version bump.
    use super::*;

    // test_all_tools_defined — all 23 tools present (20 original + list_annotations + resolve_annotations + sign_document)
    #[test]
    fn test_all_tools_defined() {
        assert_eq!(TOOLS.len(), 23, "expected exactly 23 tools");
        let names: Vec<_> = TOOLS.iter().map(|t| t.name).collect();
        assert!(names.contains(&"inspect_document"));
        assert!(names.contains(&"render_document"));
        assert!(names.contains(&"inspect_region"));
        assert!(names.contains(&"diff_documents"));
        assert!(names.contains(&"doctor"));
        assert!(names.contains(&"serve"));
        assert!(names.contains(&"apply_patch"));
        assert!(names.contains(&"edit_text"));
        assert!(names.contains(&"edit_cell"));
        assert!(names.contains(&"edit_structure"));
        assert!(names.contains(&"annotate_pages"));
        assert!(names.contains(&"revert_document"));
        assert!(names.contains(&"journal"));
        assert!(names.contains(&"create_document"));
        assert!(names.contains(&"script_tool"));
        assert!(names.contains(&"check_update"));
        assert!(names.contains(&"list_revisions"));
        assert!(names.contains(&"accept_revisions"));
        assert!(names.contains(&"reject_revisions"));
        assert!(names.contains(&"add_comment"));
        assert!(names.contains(&"list_annotations"));
        assert!(names.contains(&"resolve_annotations"));
        assert!(names.contains(&"sign_document"));
    }

    // test_all_tools_idempotent — read-only tools are idempotent (apply_patch is not)
    #[test]
    fn test_all_tools_idempotent() {
        for tool in TOOLS {
            if tool.schema_version == "1.0.0" {
                assert!(tool.idempotent, "tool '{}' must be idempotent", tool.name);
            }
        }
    }

    // test_all_tools_schema_version — tools have version 1.0.0, 1.1.0, or 1.2.0
    #[test]
    fn test_all_tools_schema_version() {
        for tool in TOOLS {
            let expected = match tool.name {
                "inspect_document" | "render_document" | "inspect_region" | "diff_documents"
                | "apply_patch" | "edit_text" | "edit_cell" | "edit_structure"
                | "annotate_pages" | "journal" => "1.6.0",
                "revert_document" => "1.1.0",
                "check_update" | "script_tool" => "1.2.0",
                "create_document" => "1.2.1",
                "list_revisions" | "accept_revisions" | "reject_revisions" | "add_comment" => {
                    "1.3.0"
                }
                "list_annotations" | "resolve_annotations" => "1.4.0",
                "sign_document" => "1.5.0",
                _ => "1.0.0",
            };
            assert_eq!(
                tool.schema_version, expected,
                "tool '{}' version is '{}', expected '{}'",
                tool.name, tool.schema_version, expected
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
        assert!(is_valid_tool("apply_patch"));
        assert!(is_valid_tool("edit_text"));
        assert!(is_valid_tool("edit_cell"));
        assert!(is_valid_tool("edit_structure"));
        assert!(is_valid_tool("annotate_pages"));
        assert!(is_valid_tool("revert_document"));
        assert!(is_valid_tool("journal"));
        assert!(is_valid_tool("create_document"));
        assert!(is_valid_tool("script_tool"));
        assert!(is_valid_tool("check_update"));
        assert!(is_valid_tool("list_revisions"));
        assert!(is_valid_tool("accept_revisions"));
        assert!(is_valid_tool("reject_revisions"));
        assert!(is_valid_tool("add_comment"));
        assert!(is_valid_tool("list_annotations"));
        assert!(is_valid_tool("resolve_annotations"));
        assert!(is_valid_tool("sign_document"));
    }

    // test_invalid_tool_name — unknown tool returns false
    #[test]
    fn test_invalid_tool_name() {
        assert!(!is_valid_tool("apply"));
        assert!(!is_valid_tool("delete"));
        assert!(!is_valid_tool("foobar"));
        assert!(!is_valid_tool(""));
        assert!(!is_valid_tool("create"));
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
        assert!(get_tool("apply_patch").is_some());
        assert!(get_tool("edit_text").is_some());
        assert!(get_tool("edit_cell").is_some());
        assert!(get_tool("edit_structure").is_some());
        assert!(get_tool("annotate_pages").is_some());
        assert!(get_tool("revert_document").is_some());
        assert!(get_tool("journal").is_some());
        assert!(get_tool("create_document").is_some());
        assert!(get_tool("script_tool").is_some());
        assert!(get_tool("check_update").is_some());
        assert!(get_tool("list_revisions").is_some());
        assert!(get_tool("accept_revisions").is_some());
        assert!(get_tool("reject_revisions").is_some());
        assert!(get_tool("add_comment").is_some());
        assert!(get_tool("list_annotations").is_some());
        assert!(get_tool("resolve_annotations").is_some());
        assert!(get_tool("sign_document").is_some());
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
            23,
            "all 23 tools should be active (none deprecated)"
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
            23,
            "all 23 tools should be active after compliance check"
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
            assert_eq!(value["idempotent"], tool.idempotent);
            assert_eq!(value["schema_version"], tool.schema_version);
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

    // test_apply_patch_is_non_idempotent — verify apply_patch is not idempotent
    #[test]
    fn test_apply_patch_is_non_idempotent() {
        let tool = get_tool("apply_patch").expect("apply_patch tool must exist");
        assert!(!tool.idempotent, "apply_patch must not be idempotent");
        assert_eq!(tool.schema_version, "1.6.0");
    }

    // test_create_document_is_non_idempotent — verify create_document is not idempotent (C7 v1.2.1)
    #[test]
    fn test_create_document_is_non_idempotent() {
        let tool = get_tool("create_document").expect("create_document tool must exist");
        assert!(!tool.idempotent, "create_document must not be idempotent");
        assert_eq!(tool.schema_version, "1.2.1");
        assert_eq!(tool.params.len(), 8);
        let param_names: Vec<_> = tool.params.iter().map(|p| p.name).collect();
        assert!(param_names.contains(&"source_type"));
        assert!(param_names.contains(&"format"));
        assert!(param_names.contains(&"content"));
        assert!(param_names.contains(&"template"));
        assert!(param_names.contains(&"dry_run"));
        assert!(param_names.contains(&"target"));
        assert!(param_names.contains(&"author"));
        assert!(param_names.contains(&"direct"));
    }

    // test_script_tool_is_non_idempotent — verify script_tool is not idempotent (C7 v1.2)
    #[test]
    fn test_script_tool_is_non_idempotent() {
        let tool = get_tool("script_tool").expect("script_tool tool must exist");
        assert!(!tool.idempotent, "script_tool must not be idempotent");
        assert_eq!(tool.schema_version, "1.2.0");
        assert_eq!(tool.params.len(), 2);
        let param_names: Vec<_> = tool.params.iter().map(|p| p.name).collect();
        assert!(param_names.contains(&"script"));
        assert!(param_names.contains(&"ir"));
    }
}
