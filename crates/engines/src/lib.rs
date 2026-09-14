//! engines: engine runners for stichos.
//!
//! Spawns external rendering engines (LibreOffice, pandoc) as subprocesses.
//! Never links external engines; they are soft dependencies probed at runtime.

pub mod cache;
pub mod capability;
pub mod doctor;
pub mod libreoffice;
pub mod mock;
pub mod negotiate;
pub mod pandoc;
pub mod probe;
pub mod raster;

/// Engine identifiers.
pub const LIBREOFFICE_ENGINE_ID: &str = "libreoffice";
pub const PANDOC_ENGINE_ID: &str = "pandoc";
pub const MSOFFICE_ENGINE_ID: &str = "ms-office";

/// Build a rendered_by field for responses.
pub fn build_rendered_by(
    engine: &dyn gp_core::traits::EngineExt,
) -> Option<gp_core::fidelity::RendererInfo> {
    let info = engine.info();
    Some(gp_core::fidelity::RendererInfo {
        engine_id: info.engine_id,
        version: info.version,
        fontset_hash: info.fontset_hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::build_capability_matrix;
    use crate::mock::MockEngine;
    use gp_core::traits::{CapabilityEntry, Engine, EngineExt, EngineInfo};

    #[test]
    fn test_mock_engine_info() {
        let engine = MockEngine::new();
        let info = engine.info();
        assert_eq!(info.engine_id, "mock");
        assert_eq!(info.version, Some("0.0.0-mock".into()));
        assert_eq!(info.fontset_hash, Some("mock-fonthash".into()));
        assert!(info.is_available);
        assert_eq!(info.capabilities.len(), 3);
        assert_eq!(
            info.capabilities[0],
            CapabilityEntry {
                source: "docx".into(),
                target: "pdf".into(),
                works: true
            }
        );
    }

    #[test]
    fn test_mock_engine_convert() {
        let engine = MockEngine::new();
        let input = b"<docx>test</docx>";
        let result = engine.convert(input, "docx", "pdf").unwrap();
        let json: serde_json::Value = serde_json::from_slice(&result).unwrap();
        assert_eq!(json["mock"], true);
        assert_eq!(json["engine"], "mock");
        assert_eq!(json["input_bytes"], 17);
    }

    #[test]
    fn test_mock_engine_not_available() {
        let mut engine = MockEngine::new();
        engine.available = false;
        assert!(!engine.is_available());
        let result = engine.convert(b"data", "docx", "pdf");
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "ENG-E-001");
    }

    #[test]
    fn test_build_capability_matrix() {
        let engines: Vec<&dyn EngineExt> = vec![Box::leak(Box::new(MockEngine::new())), {
            let mut eng = MockEngine::new();
            eng.id = "engine2".into();
            eng.version = "2.0".into();
            eng.fontset_hash = "hash2".into();
            Box::leak(Box::new(eng))
        }];
        let matrix = build_capability_matrix(&engines);
        assert_eq!(matrix.engine, "mock");
        assert_eq!(matrix.version, Some("0.0.0-mock".into()));
        assert_eq!(matrix.conversions.len(), 6); // 3 per engine x 2
    }

    #[test]
    fn test_build_rendered_by() {
        let engine = MockEngine::new();
        let rendered = build_rendered_by(&engine).unwrap();
        assert_eq!(rendered.engine_id, "mock");
        assert_eq!(rendered.version, Some("0.0.0-mock".into()));
        assert_eq!(rendered.fontset_hash, Some("mock-fonthash".into()));
    }

    #[test]
    fn test_capability_entry_serializes() {
        let entry = CapabilityEntry {
            source: "docx".into(),
            target: "pdf".into(),
            works: true,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: CapabilityEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, back);
    }

    #[test]
    fn test_engine_info_serializes() {
        let info = EngineInfo {
            engine_id: "libreoffice".into(),
            version: Some("26.2.5".into()),
            fontset_hash: Some("abc123".into()),
            is_available: true,
            capabilities: vec![
                CapabilityEntry {
                    source: "docx".into(),
                    target: "pdf".into(),
                    works: true,
                },
                CapabilityEntry {
                    source: "xlsx".into(),
                    target: "pdf".into(),
                    works: true,
                },
            ],
        };
        let json = serde_json::to_string(&info).unwrap();
        let back: EngineInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, back);
    }
}
