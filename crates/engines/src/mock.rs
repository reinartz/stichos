// SPDX-License-Identifier: MIT OR Apache-2.0
//! Mock engine for testing.
//!
//! Provides a configurable engine implementation that returns deterministic
//! results without requiring external software.

use gp_core::diagnostics::Diagnostic;
use gp_core::traits::{Engine, EngineExt, EngineInfo};

/// A mock engine for testing.
#[derive(Debug, Clone)]
pub struct MockEngine {
    /// Engine identifier.
    pub id: String,
    /// Engine version string.
    pub version: String,
    /// Fontset hash (for render cache keys, INV-4).
    pub fontset_hash: String,
    /// Whether this engine is available.
    pub available: bool,
    /// List of supported (source, target) conversions.
    pub conversions: Vec<(String, String)>,
}

impl MockEngine {
    /// Create a new MockEngine with default values.
    pub fn new() -> Self {
        Self {
            id: "mock".into(),
            version: "0.0.0-mock".into(),
            fontset_hash: "mock-fonthash".into(),
            available: true,
            conversions: vec![
                ("docx".into(), "pdf".into()),
                ("xlsx".into(), "pdf".into()),
                ("pptx".into(), "pdf".into()),
            ],
        }
    }
}

impl Default for MockEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine for MockEngine {
    fn engine_id(&self) -> &str {
        &self.id
    }

    fn version(&self) -> Option<&str> {
        if self.version.is_empty() {
            None
        } else {
            Some(&self.version)
        }
    }

    fn can_convert(&self, _source: &str, _target: &str) -> bool {
        self.available
    }

    fn convert(
        &self,
        input: &[u8],
        _source: &str,
        _target: &str,
    ) -> Result<Vec<u8>, Vec<Diagnostic>> {
        if !self.available {
            return Err(vec![Diagnostic::new(
                "ENG-E-001",
                gp_core::diagnostics::Severity::Error,
                format!("engine '{}' not available", self.id),
            )]);
        }
        let _ = input;
        let result = serde_json::json!({
            "mock": true,
            "engine": self.id,
            "input_bytes": input.len()
        });
        Ok(serde_json::to_vec(&result).unwrap())
    }

    fn is_available(&self) -> bool {
        self.available
    }
}

impl EngineExt for MockEngine {
    fn info(&self) -> EngineInfo {
        let capabilities: Vec<gp_core::traits::CapabilityEntry> = self
            .conversions
            .iter()
            .map(|(source, target)| {
                let works = self.available && self.can_convert(source, target);
                gp_core::traits::CapabilityEntry {
                    source: source.clone(),
                    target: target.clone(),
                    works,
                }
            })
            .collect();

        EngineInfo {
            engine_id: self.id.clone(),
            version: if self.version.is_empty() {
                None
            } else {
                Some(self.version.clone())
            },
            fontset_hash: if self.fontset_hash.is_empty() {
                None
            } else {
                Some(self.fontset_hash.clone())
            },
            is_available: self.available,
            capabilities,
        }
    }

    fn fontset_hash(&self) -> Option<String> {
        if self.fontset_hash.is_empty() {
            None
        } else {
            Some(self.fontset_hash.clone())
        }
    }

    fn supported_conversions(&self) -> Vec<(String, String)> {
        self.conversions.clone()
    }
}
