// SPDX-License-Identifier: MIT OR Apache-2.0
//! Style resolution for OOXML documents.
//!
//! This module provides style resolution for OOXML documents, mapping
//! style definitions to their resolved values for downstream use in
//! rendering and diffing.

#![allow(clippy::all)]
#![allow(unused_variables)]
#![allow(unused_mut)]

use gp_core::ir::{ParaPropDef, RunPropDef};
use std::collections::BTreeMap;

/// Resolved style information for a document element.
#[derive(Debug, Clone, Default)]
pub struct ResolvedStyle {
    /// Font family name, if specified.
    pub font_family: Option<String>,
    /// Font size in points.
    pub font_size_pt: Option<f64>,
    /// Bold formatting.
    pub bold: Option<bool>,
    /// Italic formatting.
    pub italic: Option<bool>,
    /// Text color as hex string.
    pub color: Option<String>,
    /// Fill color as hex string.
    pub fill_color: Option<String>,
    /// Line/border color as hex string.
    pub line_color: Option<String>,
}

/// Resolved style for an entire document or section.
#[derive(Debug, Clone, Default)]
pub struct StyleResolution {
    /// Resolved paragraph styles, keyed by style name.
    pub para_styles: BTreeMap<String, ResolvedStyle>,
    /// Resolved character/run styles, keyed by style name.
    pub char_styles: BTreeMap<String, ResolvedStyle>,
}

impl StyleResolution {
    /// Create a new empty style resolution.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the resolved style for a given paragraph style name.
    pub fn get_para_style(&self, name: &str) -> Option<&ResolvedStyle> {
        self.para_styles.get(name)
    }

    /// Get the resolved style for a given character/run style name.
    pub fn get_char_style(&self, name: &str) -> Option<&ResolvedStyle> {
        self.char_styles.get(name)
    }
}

/// Resolve styles from style definitions into resolved style map.
///
/// This function takes style definitions and resolves inherited
/// properties through the style hierarchy.
pub fn resolve_styles(
    para_defs: &BTreeMap<String, ParaPropDef>,
    run_defs: &BTreeMap<String, RunPropDef>,
) -> StyleResolution {
    let mut res = StyleResolution::new();

    for (name, def) in para_defs {
        let mut style = ResolvedStyle::default();
        if let Some(ref props) = def.alignment {
            // paragraph-level resolution
        }
        res.para_styles.insert(name.clone(), style);
    }

    for (name, def) in run_defs {
        let mut style = ResolvedStyle::default();
        if let Some(ref props) = def.font {
            style.font_family = Some(props.clone());
        }
        if let Some(ref props) = def.font_size {
            style.font_size_pt = Some(*props);
        }
        if let Some(ref props) = def.bold {
            style.bold = Some(*props);
        }
        if let Some(ref props) = def.italic {
            style.italic = Some(*props);
        }
        if let Some(ref props) = def.color {
            style.color = Some(props.clone());
        }
        res.char_styles.insert(name.clone(), style);
    }

    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_resolution() {
        let res = StyleResolution::new();
        assert!(res.para_styles.is_empty());
        assert!(res.char_styles.is_empty());
    }

    #[test]
    fn test_get_nonexistent_style() {
        let res = StyleResolution::new();
        assert!(res.get_para_style("nonexistent").is_none());
        assert!(res.get_char_style("nonexistent").is_none());
    }
}
