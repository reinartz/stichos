// SPDX-License-Identifier: MIT OR Apache-2.0
//! Style resolution for OOXML documents.
//!
//! This module provides style resolution for OOXML documents, mapping
//! style definitions to their resolved values for downstream use in
//! rendering and diffing.
//!
//! The resolution traverses the style chain (`based_on` links) to fold
//! inherited properties from `docDefaults` through to direct run/paragraph
//! properties.

#![allow(clippy::all)]
#![allow(unused_variables)]

use gp_core::ir::{ParaPropDef, RunPropDef, StyleDef, StyleResolution};
use std::collections::BTreeMap;

/// Resolve styles from style definitions into a resolved style map.
///
/// This function takes style definitions and resolves inherited
/// properties through the style hierarchy by following `based_on`
/// chains.  Direct run/paragraph properties are extracted from each
/// `StyleDef` and the results are returned as a `StyleResolution`.
pub fn resolve_styles(style_map: &BTreeMap<String, StyleDef>) -> StyleResolution {
    let mut res = StyleResolution::default();

    // First pass: extract direct properties from each style definition
    for (name, def) in style_map {
        if let Some(ref pp) = def.para_props {
            let mut style = gp_core::ir::ResolvedStyle::default();
            extract_para_props(pp, &mut style);
            res.para_styles.insert(name.clone(), style);
        }

        // Character/run styles
        if let Some(ref rp) = def.run_props {
            let mut style = gp_core::ir::ResolvedStyle::default();
            extract_run_props(rp, &mut style);
            res.char_styles.insert(name.clone(), style);
        }
    }

    // Second pass: resolve inherited properties through based_on chains.
    // Collect all chains first, then build new maps to avoid borrow issues.
    let para_keys: Vec<String> = res.para_styles.keys().cloned().collect();
    let mut new_para_styles = BTreeMap::new();
    for name in &para_keys {
        let chain = collect_chain(name, style_map);
        let mut resolved = res.para_styles[name].clone();
        apply_chain(&mut resolved, &chain, &res.para_styles, &res.char_styles);
        new_para_styles.insert(name.clone(), resolved);
    }
    res.para_styles = new_para_styles;

    let char_keys: Vec<String> = res.char_styles.keys().cloned().collect();
    let mut new_char_styles = BTreeMap::new();
    for name in &char_keys {
        let chain = collect_chain(name, style_map);
        let mut resolved = res.char_styles[name].clone();
        apply_chain(&mut resolved, &chain, &res.para_styles, &res.char_styles);
        new_char_styles.insert(name.clone(), resolved);
    }
    res.char_styles = new_char_styles;

    res
}

/// Collect the style chain (ancestor-first order) for a given style name.
fn collect_chain(name: &str, style_map: &BTreeMap<String, StyleDef>) -> Vec<String> {
    let mut chain: Vec<String> = Vec::new();
    let mut current = name.to_string();
    while !chain.contains(&current) {
        chain.push(current.clone());
        if let Some(def) = style_map.get(&current) {
            if let Some(ref based) = def.based_on {
                current = based.clone();
            } else {
                break;
            }
        } else {
            break;
        }
    }
    chain.reverse();
    chain
}

/// Apply a pre-collected style chain to a resolved style.
fn apply_chain(
    style: &mut gp_core::ir::ResolvedStyle,
    chain: &[String],
    para_styles: &BTreeMap<String, gp_core::ir::ResolvedStyle>,
    char_styles: &BTreeMap<String, gp_core::ir::ResolvedStyle>,
) {
    for ancestor_name in chain {
        let is_para = para_styles.contains_key(ancestor_name);
        let is_char = char_styles.contains_key(ancestor_name);

        if is_para {
            if let Some(ancestor_style) = para_styles.get(ancestor_name) {
                merge_style(style, ancestor_style);
            }
        }

        if is_char {
            if let Some(ancestor_style) = char_styles.get(ancestor_name) {
                merge_style(style, ancestor_style);
            }
        }
    }
}

/// Merge source style into target, only filling in None values (child overrides parent).
fn merge_style(target: &mut gp_core::ir::ResolvedStyle, source: &gp_core::ir::ResolvedStyle) {
    if target.font_family.is_none() && source.font_family.is_some() {
        target.font_family = source.font_family.clone();
    }
    if target.font_size_pt.is_none() && source.font_size_pt.is_some() {
        target.font_size_pt = source.font_size_pt;
    }
    if target.bold.is_none() && source.bold.is_some() {
        target.bold = source.bold;
    }
    if target.italic.is_none() && source.italic.is_some() {
        target.italic = source.italic;
    }
    if target.color.is_none() && source.color.is_some() {
        target.color = source.color.clone();
    }
    if target.fill_color.is_none() && source.fill_color.is_some() {
        target.fill_color = source.fill_color.clone();
    }
    if target.line_color.is_none() && source.line_color.is_some() {
        target.line_color = source.line_color.clone();
    }
}

/// Extract direct run property overrides into a ResolvedStyle.
fn extract_run_props(props: &RunPropDef, style: &mut gp_core::ir::ResolvedStyle) {
    if let Some(ref font) = props.font {
        style.font_family = Some(font.clone());
    }
    if let Some(ref size) = props.font_size {
        style.font_size_pt = Some(*size);
    }
    if let Some(ref b) = props.bold {
        style.bold = Some(*b);
    }
    if let Some(ref i) = props.italic {
        style.italic = Some(*i);
    }
    if let Some(ref c) = props.color {
        style.color = Some(c.clone());
    }
    if let Some(ref h) = props.highlight {
        style.fill_color = Some(h.clone());
    }
}

/// Extract direct paragraph property overrides into a ResolvedStyle.
fn extract_para_props(props: &ParaPropDef, style: &mut gp_core::ir::ResolvedStyle) {
    // Paragraph properties map to para-level resolution;
    // font/color info comes from run_props but we preserve
    // alignment info for downstream use.
    if let Some(ref _align) = props.alignment {
        // alignment is paragraph-only, not stored in ResolvedStyle yet
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_core::ir::StyleDef;

    #[test]
    fn test_empty_resolution() {
        let map: BTreeMap<String, StyleDef> = BTreeMap::new();
        let res = resolve_styles(&map);
        assert!(res.para_styles.is_empty());
        assert!(res.char_styles.is_empty());
    }

    #[test]
    fn test_resolution_with_run_props() {
        let mut map: BTreeMap<String, StyleDef> = BTreeMap::new();
        map.insert(
            "Heading1".to_string(),
            StyleDef {
                style_id: "Heading1".to_string(),
                style_type: Some("paragraph".to_string()),
                run_props: Some(Box::new(RunPropDef {
                    font: Some("Arial".to_string()),
                    font_size: Some(24.0),
                    bold: Some(true),
                    ..Default::default()
                })),
                para_props: Some(Box::new(ParaPropDef {
                    alignment: Some("left".to_string()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );

        let res = resolve_styles(&map);
        assert!(res.para_styles.contains_key("Heading1"));
        assert!(res.char_styles.contains_key("Heading1"));

        let char_style = res.char_styles.get("Heading1").unwrap();
        assert_eq!(char_style.font_family, Some("Arial".to_string()));
        assert_eq!(char_style.font_size_pt, Some(24.0));
        assert_eq!(char_style.bold, Some(true));
    }

    #[test]
    fn test_resolution_inherits_from_based_on() {
        let mut map: BTreeMap<String, StyleDef> = BTreeMap::new();

        // Base style
        map.insert(
            "Normal".to_string(),
            StyleDef {
                style_id: "Normal".to_string(),
                run_props: Some(Box::new(RunPropDef {
                    font: Some("Calibri".to_string()),
                    font_size: Some(11.0),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );

        // Derived style
        map.insert(
            "Heading1".to_string(),
            StyleDef {
                style_id: "Heading1".to_string(),
                based_on: Some("Normal".to_string()),
                run_props: Some(Box::new(RunPropDef {
                    bold: Some(true),
                    ..Default::default()
                })),
                para_props: Some(Box::new(ParaPropDef::default())),
                ..Default::default()
            },
        );

        let res = resolve_styles(&map);

        // Heading1 should inherit font/size from Normal and add bold
        let para_style = res.para_styles.get("Heading1").unwrap();
        assert_eq!(para_style.font_family, Some("Calibri".to_string()));
        assert_eq!(para_style.font_size_pt, Some(11.0));
        assert_eq!(para_style.bold, Some(true));
    }
}
