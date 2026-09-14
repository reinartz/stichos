//! Capability matrix aggregation.
//!
//! Builds a capability matrix from a list of engines.

use gp_core::traits::{CapabilityEntry, CapabilityMatrix, EngineExt};

/// Build a capability matrix from multiple engines.
///
/// Aggregates capabilities across all discovered engines into a single
/// `CapabilityMatrix` suitable for reporting via the `doctor` tool.
pub fn build_capability_matrix(engines: &[&dyn EngineExt]) -> CapabilityMatrix {
    let engine_entries: Vec<CapabilityEntry> = engines
        .iter()
        .flat_map(|e| e.info().capabilities.clone())
        .collect();

    let first = engines
        .first()
        .map(|e| e.info())
        .unwrap_or_else(|| gp_core::traits::EngineInfo {
            engine_id: "none".into(),
            version: None,
            fontset_hash: None,
            is_available: false,
            capabilities: Vec::new(),
        });

    CapabilityMatrix {
        engine: first.engine_id,
        version: first.version,
        conversions: engine_entries,
    }
}
