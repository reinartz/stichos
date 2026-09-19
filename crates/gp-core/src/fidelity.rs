// SPDX-License-Identifier: MIT OR Apache-2.0
//! Fidelity budget: what a read operation cost in terms of feature representation.
//!
//! Every read response carries a `FidelityBudget` — an accounting of what was faithfully
//! captured, what was dropped, and how confident we are in the result.

use serde::{Deserialize, Serialize};

/// A single dropped or degraded feature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DroppedFeature {
    /// Feature name, e.g. "smart_art", "ole_object", "macro".
    pub feature: String,
    /// Why it was dropped.
    pub reason: String,
    /// How many instances were affected.
    pub count: usize,
}

/// A warning about a specific part of the document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartWarning {
    /// The part name (e.g. "word/document.xml", "xl/styles.xml").
    pub part: String,
    /// Warning message.
    pub message: String,
}

/// Renderer / engine provenance info.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RendererInfo {
    /// Engine identifier.
    pub engine_id: String,
    /// Engine version.
    pub version: Option<String>,
    /// Fontset hash (INV-4).
    pub fontset_hash: Option<String>,
}

/// The fidelity budget for a read operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FidelityBudget {
    /// Features that could not be faithfully represented.
    pub dropped_features: Vec<DroppedFeature>,
    /// Warnings about specific parts.
    pub part_warnings: Vec<PartWarning>,
    /// Unknown parts encountered during parsing.
    pub unknown_parts: Vec<String>,
    /// Which engine rendered visual feedback (if any).
    pub rendered_by: Option<RendererInfo>,
    /// Overall confidence [0.0, 1.0].
    pub confidence: f32,
    /// Whether the document contained potentially unsafe content.
    pub has_potentially_unsafe: bool,
}

impl Default for FidelityBudget {
    fn default() -> Self {
        Self {
            dropped_features: Vec::new(),
            part_warnings: Vec::new(),
            unknown_parts: Vec::new(),
            rendered_by: None,
            confidence: 1.0,
            has_potentially_unsafe: false,
        }
    }
}

impl FidelityBudget {
    /// Create a new empty budget.
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a feature as dropped.
    pub fn drop_feature(
        mut self,
        feature: impl Into<String>,
        reason: impl Into<String>,
        count: usize,
    ) -> Self {
        self.dropped_features.push(DroppedFeature {
            feature: feature.into(),
            reason: reason.into(),
            count,
        });
        self
    }

    /// Add a part warning.
    pub fn warn_part(mut self, part: impl Into<String>, message: impl Into<String>) -> Self {
        self.part_warnings.push(PartWarning {
            part: part.into(),
            message: message.into(),
        });
        self
    }

    /// Add an unknown part.
    pub fn unknown_part(mut self, part: impl Into<String>) -> Self {
        self.unknown_parts.push(part.into());
        self
    }

    /// Set renderer provenance.
    pub fn rendered_with(
        mut self,
        engine: impl Into<String>,
        version: Option<String>,
        fontset_hash: Option<String>,
    ) -> Self {
        self.rendered_by = Some(RendererInfo {
            engine_id: engine.into(),
            version,
            fontset_hash,
        });
        self
    }

    /// Set confidence level.
    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    /// Mark potentially unsafe content.
    pub fn with_unsafe(mut self, has_unsafe: bool) -> Self {
        self.has_potentially_unsafe = has_unsafe;
        self
    }

    /// Calculate a summary score: 1.0 - (dropped_ratio + warning_ratio).
    pub fn summary_score(&self) -> f32 {
        let dropped_count_sum: usize = self.dropped_features.iter().map(|d| d.count).sum();
        let total = dropped_count_sum + self.part_warnings.len() + self.unknown_parts.len();
        let max = std::cmp::max(1, total + dropped_count_sum);
        (max as f32 - total as f32) / max as f32
    }

    /// Check if budget is "good" (confidence > 0.8 and no unsafe).
    pub fn is_good(&self) -> bool {
        self.confidence > 0.8 && !self.has_potentially_unsafe
    }

    /// Merge this budget with another (e.g. when combining multiple reads).
    ///
    /// Rules:
    /// - dropped_features: union (deduplicate by feature+reason)
    /// - part_warnings: union (deduplicate by part)
    /// - unknown_parts: union (deduplicate)
    /// - rendered_by: prefer the more specific (non-None) renderer
    /// - confidence: minimum of both (pessimistic)
    /// - has_potentially_unsafe: OR
    pub fn merge(mut self, other: Self) -> Self {
        // Deduplicate dropped features
        let mut dropped: Vec<DroppedFeature> = self.dropped_features;
        for o in other.dropped_features {
            if !dropped
                .iter()
                .any(|d| d.feature == o.feature && d.reason == o.reason)
            {
                dropped.push(o);
            }
        }
        self.dropped_features = dropped;

        // Deduplicate warnings
        let mut warnings: Vec<PartWarning> = self.part_warnings;
        for o in other.part_warnings {
            if !warnings.iter().any(|w| w.part == o.part) {
                warnings.push(o);
            }
        }
        self.part_warnings = warnings;

        // Deduplicate unknown parts
        let mut unknown: Vec<String> = self.unknown_parts;
        for o in other.unknown_parts {
            if !unknown.contains(&o) {
                unknown.push(o);
            }
        }
        self.unknown_parts = unknown;

        // Confidence: minimum (pessimistic)
        self.confidence = self.confidence.min(other.confidence);

        // Unsafe: OR
        self.has_potentially_unsafe = self.has_potentially_unsafe || other.has_potentially_unsafe;

        // Rendered by: prefer non-None
        if self.rendered_by.is_none() && other.rendered_by.is_some() {
            self.rendered_by = other.rendered_by;
        }

        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> FidelityBudget {
        FidelityBudget::new()
    }

    fn budget_a() -> FidelityBudget {
        FidelityBudget::new()
            .drop_feature("smart_art", "no_direct_equivalent", 2)
            .drop_feature("ole_object", "unsafe_skip", 1)
            .warn_part("word/document.xml", "large section truncated")
            .unknown_part("embeddings/v1.bin")
            .with_confidence(0.9)
    }

    fn budget_b() -> FidelityBudget {
        FidelityBudget::new()
            .drop_feature("smart_art", "no_direct_equivalent", 3)
            .drop_feature("macro", "blocked_policy", 1)
            .warn_part("xl/styles.xml", "custom theme missing")
            .unknown_part("embeddings/v2.bin")
            .with_confidence(0.7)
            .with_unsafe(true)
    }

    fn budget_c() -> FidelityBudget {
        FidelityBudget::new()
            .drop_feature("picture", "too_large", 5)
            .warn_part("ppt/slides/slide1.xml", "layout mismatch")
            .with_confidence(0.85)
            .rendered_with(
                "libreoffice",
                Some("7.5.0".to_string()),
                Some("abc123".to_string()),
            )
    }

    // ------------------------------------------------------------------
    // Basic builder tests
    // ------------------------------------------------------------------

    #[test]
    fn test_default_is_empty() {
        let fb = FidelityBudget::default();
        assert!(fb.dropped_features.is_empty());
        assert!(fb.part_warnings.is_empty());
        assert!(fb.unknown_parts.is_empty());
        assert!(fb.rendered_by.is_none());
        assert_eq!(fb.confidence, 1.0);
        assert!(!fb.has_potentially_unsafe);
    }

    #[test]
    fn test_builder_chain() {
        let fb = FidelityBudget::new()
            .drop_feature("smart_art", "no_equivalent", 2)
            .warn_part("word/doc.xml", "truncated")
            .unknown_part("extra.xml")
            .rendered_with("lo", Some("1.0".to_string()), Some("h1".to_string()))
            .with_confidence(0.5)
            .with_unsafe(true);

        assert_eq!(fb.dropped_features.len(), 1);
        assert_eq!(fb.part_warnings.len(), 1);
        assert_eq!(fb.unknown_parts.len(), 1);
        assert!(fb.rendered_by.is_some());
        assert_eq!(fb.confidence, 0.5);
        assert!(fb.has_potentially_unsafe);
    }

    #[test]
    fn test_confidence_clamping() {
        let fb_neg = FidelityBudget::new().with_confidence(-0.5);
        assert_eq!(fb_neg.confidence, 0.0);

        let fb_high = FidelityBudget::new().with_confidence(1.5);
        assert_eq!(fb_high.confidence, 1.0);
    }

    #[test]
    fn test_summary_score() {
        // Empty budget: dropped_count_sum=0, total=0, max=max(1,0)=1
        // score = (1-0)/1 = 1.0
        let empty = FidelityBudget::new();
        assert!((empty.summary_score() - 1.0).abs() < 1e-6);

        // One dropped feature with count=2: dropped_count_sum=2, total=2, max=max(1,4)=4
        // score = (4-2)/4 = 0.5
        let fb = FidelityBudget::new().drop_feature("x", "r", 2);
        assert!((fb.summary_score() - 0.5).abs() < 1e-6);

        // One warning: dropped_count_sum=0, total=1, max=max(1,1)=1
        // score = (1-1)/1 = 0.0
        let fb2 = FidelityBudget::new().warn_part("p", "m");
        assert!((fb2.summary_score() - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_is_good() {
        assert!(FidelityBudget::new().is_good());
        assert!(!FidelityBudget::new().with_confidence(0.5).is_good());
        assert!(!FidelityBudget::new().with_unsafe(true).is_good());
        assert!(FidelityBudget::new().with_confidence(0.9).is_good());
    }

    // ------------------------------------------------------------------
    // Merge algebra tests
    // ------------------------------------------------------------------

    #[test]
    fn test_merge_dropped_features_dedup() {
        // Same feature+reason should be deduplicated
        let a = FidelityBudget::new().drop_feature("smart_art", "no_eq", 2);
        let b = FidelityBudget::new().drop_feature("smart_art", "no_eq", 3);
        let merged = a.merge(b);
        assert_eq!(merged.dropped_features.len(), 1);
        assert_eq!(merged.dropped_features[0].count, 2); // keep first
    }

    #[test]
    fn test_merge_dropped_features_union() {
        // Different features both kept
        let a = FidelityBudget::new().drop_feature("a", "r1", 1);
        let b = FidelityBudget::new().drop_feature("b", "r2", 2);
        let merged = a.merge(b);
        assert_eq!(merged.dropped_features.len(), 2);
    }

    #[test]
    fn test_merge_warnings_dedup() {
        // Same part deduplicated, keep first
        let a = FidelityBudget::new().warn_part("p.xml", "msg1");
        let b = FidelityBudget::new().warn_part("p.xml", "msg2");
        let merged = a.merge(b);
        assert_eq!(merged.part_warnings.len(), 1);
        assert_eq!(merged.part_warnings[0].message, "msg1");
    }

    #[test]
    fn test_merge_warnings_union() {
        let a = FidelityBudget::new().warn_part("a.xml", "ma");
        let b = FidelityBudget::new().warn_part("b.xml", "mb");
        let merged = a.merge(b);
        assert_eq!(merged.part_warnings.len(), 2);
    }

    #[test]
    fn test_merge_unknown_dedup() {
        let a = FidelityBudget::new().unknown_part("x.xml");
        let b = FidelityBudget::new().unknown_part("x.xml");
        let merged = a.merge(b);
        assert_eq!(merged.unknown_parts.len(), 1);
    }

    #[test]
    fn test_merge_unknown_union() {
        let a = FidelityBudget::new().unknown_part("a.xml");
        let b = FidelityBudget::new().unknown_part("b.xml");
        let merged = a.merge(b);
        assert_eq!(merged.unknown_parts.len(), 2);
    }

    #[test]
    fn test_merge_confidence_minimum() {
        let a = FidelityBudget::new().with_confidence(0.9);
        let b = FidelityBudget::new().with_confidence(0.7);
        let merged = a.merge(b);
        assert_eq!(merged.confidence, 0.7);
    }

    #[test]
    fn test_merge_confidence_symmetric() {
        // min(0.7, 0.9) should be 0.7 regardless of order
        let a = FidelityBudget::new().with_confidence(0.7);
        let b = FidelityBudget::new().with_confidence(0.9);
        assert_eq!(
            a.clone().merge(b.clone()).confidence,
            b.clone().merge(a).confidence
        );
    }

    #[test]
    fn test_merge_unsafe_or() {
        let a = FidelityBudget::new().with_unsafe(false);
        let b = FidelityBudget::new().with_unsafe(true);
        let merged = a.merge(b);
        assert!(merged.has_potentially_unsafe);
    }

    #[test]
    fn test_merge_unsafe_both_false() {
        let a = FidelityBudget::new().with_unsafe(false);
        let b = FidelityBudget::new().with_unsafe(false);
        let merged = a.merge(b);
        assert!(!merged.has_potentially_unsafe);
    }

    #[test]
    fn test_merge_rendered_by_prefer() {
        // None + Some = Some
        let a = FidelityBudget::new();
        let b = FidelityBudget::new().rendered_with(
            "lo",
            Some("7.0".to_string()),
            Some("h1".to_string()),
        );
        let merged = a.merge(b);
        assert!(merged.rendered_by.is_some());
        assert_eq!(merged.rendered_by.as_ref().unwrap().engine_id, "lo");
    }

    #[test]
    fn test_merge_rendered_by_keep_self() {
        // Both Some: keep self (prefer non-None only when self is None)
        let a = FidelityBudget::new().rendered_with("a", Some("1.0".to_string()), None::<String>);
        let b = FidelityBudget::new().rendered_with("b", Some("2.0".to_string()), None::<String>);
        let merged = a.merge(b);
        assert_eq!(merged.rendered_by.as_ref().unwrap().engine_id, "a");
    }

    #[test]
    fn test_merge_rendered_by_other_only() {
        // Self None, other Some -> use other
        let a = FidelityBudget::new();
        let b = FidelityBudget::new().rendered_with("b", None::<String>, Some("h2".to_string()));
        let merged = a.merge(b);
        assert_eq!(merged.rendered_by.as_ref().unwrap().engine_id, "b");
    }

    #[test]
    fn test_merge_idempotence() {
        let fb = budget_a();
        let merged = fb.clone().merge(fb.clone());
        assert_eq!(merged.dropped_features.len(), fb.dropped_features.len());
        assert_eq!(merged.part_warnings.len(), fb.part_warnings.len());
        assert_eq!(merged.unknown_parts.len(), fb.unknown_parts.len());
        assert_eq!(merged.confidence, fb.confidence);
        assert_eq!(merged.has_potentially_unsafe, fb.has_potentially_unsafe);
    }

    #[test]
    fn test_merge_commutativity() {
        let a = budget_a();
        let b = budget_b();
        let ab = a.clone().merge(b.clone());
        let ba = b.clone().merge(a.clone());

        assert_eq!(ab.dropped_features.len(), ba.dropped_features.len());
        assert_eq!(ab.part_warnings.len(), ba.part_warnings.len());
        assert_eq!(ab.unknown_parts.len(), ba.unknown_parts.len());
        assert_eq!(ab.confidence, ba.confidence);
        assert_eq!(ab.has_potentially_unsafe, ba.has_potentially_unsafe);
    }

    #[test]
    fn test_merge_associativity() {
        let a = budget_a();
        let b = budget_b();
        let c = budget_c();

        let left = a.clone().merge(b.clone()).merge(c.clone());
        let right = a.clone().merge(b.clone().merge(c.clone()));

        assert_eq!(left.dropped_features.len(), right.dropped_features.len());
        assert_eq!(left.part_warnings.len(), right.part_warnings.len());
        assert_eq!(left.unknown_parts.len(), right.unknown_parts.len());
        assert_eq!(left.confidence, right.confidence);
        assert_eq!(left.has_potentially_unsafe, right.has_potentially_unsafe);
    }

    #[test]
    fn test_merge_empty() {
        let fb = budget_a();
        let merged = empty().merge(fb.clone());
        assert_eq!(merged.dropped_features.len(), fb.dropped_features.len());
        assert_eq!(merged.part_warnings.len(), fb.part_warnings.len());
        assert_eq!(merged.unknown_parts.len(), fb.unknown_parts.len());
        assert_eq!(merged.confidence, fb.confidence);
        assert_eq!(merged.has_potentially_unsafe, fb.has_potentially_unsafe);

        let merged2 = fb.clone().merge(empty());
        assert_eq!(merged2.dropped_features.len(), fb.dropped_features.len());
        assert_eq!(merged2.part_warnings.len(), fb.part_warnings.len());
        assert_eq!(merged2.unknown_parts.len(), fb.unknown_parts.len());
        assert_eq!(merged2.confidence, fb.confidence);
        assert_eq!(merged2.has_potentially_unsafe, fb.has_potentially_unsafe);
    }

    #[test]
    fn test_merge_empty_both() {
        let merged = empty().merge(empty());
        assert!(merged.dropped_features.is_empty());
        assert!(merged.part_warnings.is_empty());
        assert!(merged.unknown_parts.is_empty());
    }

    #[test]
    fn test_serde_roundtrip() {
        let fb = FidelityBudget::new()
            .drop_feature("smart_art", "no_eq", 2)
            .warn_part("word/doc.xml", "truncated")
            .unknown_part("extra.bin")
            .rendered_with(
                "lo",
                Some("7.5.0".to_string()),
                Some("fonthash".to_string()),
            )
            .with_confidence(0.85)
            .with_unsafe(false);

        let json = serde_json::to_string(&fb).expect("serialize");
        let decoded: FidelityBudget = serde_json::from_str(&json).expect("deserialize");
        assert!(fb == decoded);
    }

    #[test]
    fn test_serde_empty_json_shape() {
        let fb = FidelityBudget::new();
        let json = serde_json::to_string(&fb).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");

        assert!(parsed["dropped_features"].is_array());
        assert!(parsed["part_warnings"].is_array());
        assert!(parsed["unknown_parts"].is_array());
        assert!(parsed["rendered_by"].is_null());
        assert!(parsed["confidence"].is_number());
        assert!(parsed["has_potentially_unsafe"].is_boolean());
    }

    #[test]
    fn test_merge_all_fields_combined() {
        let a = FidelityBudget::new()
            .drop_feature("smart_art", "no_eq", 1)
            .warn_part("a.xml", "wa")
            .unknown_part("x.bin")
            .with_confidence(0.6)
            .with_unsafe(false);

        let b = FidelityBudget::new()
            .drop_feature("macro", "blocked", 2)
            .warn_part("b.xml", "wb")
            .unknown_part("y.bin")
            .with_confidence(0.4)
            .with_unsafe(true)
            .rendered_with("engine1", Some("2.0".to_string()), None::<String>);

        let merged = a.merge(b);

        // Dropped: smart_art + macro = 2
        assert_eq!(merged.dropped_features.len(), 2);
        // Warnings: a.xml + b.xml = 2
        assert_eq!(merged.part_warnings.len(), 2);
        // Unknown: x.bin + y.bin = 2
        assert_eq!(merged.unknown_parts.len(), 2);
        // Confidence: min(0.6, 0.4) = 0.4
        assert_eq!(merged.confidence, 0.4);
        // Unsafe: false OR true = true
        assert!(merged.has_potentially_unsafe);
        // Rendered: a had None, b had Some -> Some
        assert!(merged.rendered_by.is_some());
        assert_eq!(merged.rendered_by.as_ref().unwrap().engine_id, "engine1");
    }
}
