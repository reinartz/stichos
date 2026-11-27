use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeatureState {
    Read,
    Write,
    None,
}

impl fmt::Display for FeatureState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FeatureState::Read => write!(f, "read"),
            FeatureState::Write => write!(f, "write"),
            FeatureState::None => write!(f, "none"),
        }
    }
}

pub const CAPABILITIES: &[(&str, &[(&str, FeatureState)])] = &[
    (
        "docx",
        &[
            ("styles", FeatureState::None),
            ("hyperlinks", FeatureState::None),
            ("drawings_media", FeatureState::None),
            ("complex_fields", FeatureState::Read),
            ("images", FeatureState::None),
        ],
    ),
    (
        "xlsx",
        &[
            ("cell_styles", FeatureState::None),
            ("freeze_panes", FeatureState::None),
            ("autofilter", FeatureState::None),
            ("defined_names", FeatureState::None),
            ("conditional_formatting", FeatureState::None),
            ("data_validation", FeatureState::None),
            ("tables", FeatureState::None),
            ("protection", FeatureState::None),
            ("page_setup", FeatureState::None),
            ("hyperlinks", FeatureState::None),
            ("number_formats", FeatureState::None),
        ],
    ),
    (
        "pptx",
        &[
            ("shape_tree", FeatureState::Read),
            ("placeholders", FeatureState::Read),
            ("table_cells", FeatureState::Read),
            ("theme", FeatureState::Read),
            ("notes", FeatureState::Read),
            ("slide_size", FeatureState::Read),
            ("charts", FeatureState::Read),
        ],
    ),
    (
        "cross",
        &[
            ("capability_matrix", FeatureState::Read),
            ("inspect_flags", FeatureState::Read),
            ("csv_export", FeatureState::None),
        ],
    ),
];

pub fn capabilities_json() -> String {
    let mut map = BTreeMap::new();
    for (format, features) in CAPABILITIES {
        let mut feat_map = BTreeMap::new();
        for (feature, state) in *features {
            feat_map.insert(feature.to_string(), format!("{}", state));
        }
        map.insert(format.to_string(), feat_map);
    }
    let output = serde_json::json!({"capabilities": map});
    serde_json::to_string_pretty(&output).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_false_write_claims() {
        let json = capabilities_json();
        // Verify no feature is claimed as write when it shouldn't be
        let pptx_section: &str = &json[json.find(r#""pptx""#).unwrap_or(0)..];
        let pptx_end = pptx_section
            .find(r#""cross""#)
            .unwrap_or(pptx_section.len());
        assert!(
            !pptx_section[..pptx_end].contains(r#""write""#),
            "PPTX should not claim write"
        );
    }

    #[test]
    fn test_capabilities_json_valid() {
        let json = capabilities_json();
        assert!(json.contains("charts"));
        assert!(json.contains("read"));
    }

    #[test]
    fn test_xlsx_no_write_claims() {
        let json = capabilities_json();
        let xlsx_section: &str = &json[json.find(r#""xlsx""#).unwrap_or(0)..];
        let xlsx_end = xlsx_section.find(r#""pptx""#).unwrap_or(xlsx_section.len());
        assert!(
            !xlsx_section[..xlsx_end].contains(r#""write""#),
            "XLSX should not claim write"
        );
    }
}
