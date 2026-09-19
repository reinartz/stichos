// SPDX-License-Identifier: MIT OR Apache-2.0
//! Mock implementations and testkit for loading corpus manifests.
//!
//! This module provides mock implementations of the core traits (Codec, Engine, Probe,
//! Linter) for use in unit tests, plus utility functions for loading corpus fixture
//! manifests from the `corpus/manifests/` directory.

use crate::diagnostics::{Diagnostic, Severity};
use crate::traits::*;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// Mock Codec implementations
// ---------------------------------------------------------------------------

/// Mock DOCX codec
pub struct MockDocxCodec;
/// Mock XLSX codec
pub struct MockXlsxCodec;
/// Mock PPTX codec
pub struct MockPptxCodec;
/// Mock ODT codec
pub struct MockOdtCodec;
/// Mock ODS codec
pub struct MockOdsCodec;
/// Mock ODP codec
pub struct MockOdpCodec;
/// Mock PDF codec
pub struct MockPdfCodec;

macro_rules! impl_mock_codec {
    ($name:ident, $format:expr) => {
        impl Codec for $name {
            fn format_name(&self) -> &str {
                $format
            }

            fn read(&self, _data: &[u8]) -> Result<ReadResult, Vec<Diagnostic>> {
                let ir = serde_json::to_string(&serde_json::json!({
                    "version": "1.0.0",
                    "format": $format,
                    "pages": [],
                    "elements": []
                }))
                .unwrap();
                Ok(ReadResult {
                    ir: ir.into_bytes(),
                    diagnostics: vec![],
                })
            }

            fn write(&self, _ir: &[u8]) -> Result<Vec<u8>, Vec<Diagnostic>> {
                Err(vec![Diagnostic::new(
                    "GP-W-001",
                    Severity::Info,
                    format!("write not yet implemented for {}", $format),
                )])
            }
        }
    };
}

impl_mock_codec!(MockDocxCodec, "docx");
impl_mock_codec!(MockXlsxCodec, "xlsx");
impl_mock_codec!(MockPptxCodec, "pptx");
impl_mock_codec!(MockOdtCodec, "odt");
impl_mock_codec!(MockOdsCodec, "ods");
impl_mock_codec!(MockOdpCodec, "odp");
impl_mock_codec!(MockPdfCodec, "pdf");

// ---------------------------------------------------------------------------
// Mock Engine
// ---------------------------------------------------------------------------

/// Mock rendering engine with configurable id, version, and fontset hash.
pub struct MockEngine {
    pub id: String,
    pub version: String,
    pub fontset_hash: String,
}

impl MockEngine {
    pub fn new(id: &str, version: &str, fontset_hash: &str) -> Self {
        Self {
            id: id.to_string(),
            version: version.to_string(),
            fontset_hash: fontset_hash.to_string(),
        }
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
        true
    }

    fn convert(
        &self,
        _input: &[u8],
        _source: &str,
        _target: &str,
    ) -> Result<Vec<u8>, Vec<Diagnostic>> {
        Ok(vec![])
    }

    fn is_available(&self) -> bool {
        true
    }
}

impl EngineExt for MockEngine {
    fn info(&self) -> EngineInfo {
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
            is_available: self.is_available(),
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
                CapabilityEntry {
                    source: "pptx".into(),
                    target: "pdf".into(),
                    works: true,
                },
            ],
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
        vec![
            ("docx".into(), "pdf".into()),
            ("xlsx".into(), "pdf".into()),
            ("pptx".into(), "pdf".into()),
        ]
    }
}

// ---------------------------------------------------------------------------
// Mock Probe
// ---------------------------------------------------------------------------

/// Mock probe that returns a pre-built CapabilityMatrix.
pub struct MockProbe;

impl Probe for MockProbe {
    fn probe(&self, engine: &dyn Engine) -> CapabilityMatrix {
        CapabilityMatrix {
            engine: engine.engine_id().to_string(),
            version: engine.version().map(|v| v.to_string()),
            conversions: vec![
                CapabilityEntry {
                    source: "docx".into(),
                    target: "pdf".into(),
                    works: engine.can_convert("docx", "pdf"),
                },
                CapabilityEntry {
                    source: "xlsx".into(),
                    target: "pdf".into(),
                    works: engine.can_convert("xlsx", "pdf"),
                },
                CapabilityEntry {
                    source: "pptx".into(),
                    target: "pdf".into(),
                    works: engine.can_convert("pptx", "pdf"),
                },
            ],
        }
    }
}

// ---------------------------------------------------------------------------
// Mock Linter
// ---------------------------------------------------------------------------

/// Mock linter that returns empty diagnostics.
pub struct MockLinter;

impl Linter for MockLinter {
    fn lint(&self, _input: &[u8], _pages: &[LinterPage]) -> Vec<Diagnostic> {
        vec![]
    }
}

impl BatchLinter for MockLinter {
    fn lint_batch(&self, pages: &[LinterPage]) -> Vec<PageDiagnostic> {
        pages
            .iter()
            .map(|p| PageDiagnostic {
                page_number: p.page_number,
                diagnostics: vec![],
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Mock PatchExecutor
// ---------------------------------------------------------------------------

/// Mock patch executor implementing the PatchExecutor trait.
pub struct MockPatchExecutor;

impl PatchExecutor for MockPatchExecutor {
    type Error = std::convert::Infallible;

    fn apply(&self, patch: &[u8]) -> Result<Vec<u8>, Self::Error> {
        // Return the patch bytes as-is (identity transform for testing).
        Ok(patch.to_vec())
    }

    fn apply_op(
        &self,
        _op: &crate::patch::PatchOp,
    ) -> Result<crate::patch::ApplyResult, Vec<Diagnostic>> {
        // Return a no-op result for testing.
        Ok(crate::patch::ApplyResult {
            diagnostics: Vec::new(),
            changed_element_ids: Vec::new(),
            fidelity_budget: crate::fidelity::FidelityBudget::default(),
        })
    }
}

// ---------------------------------------------------------------------------
// Corpus fixture loading
// ---------------------------------------------------------------------------

/// Corpus fixture info loaded from manifest
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FixtureInfo {
    pub id: String,
    pub format: String,
    pub categories: Vec<String>,
    pub expected_feature_counts: HashMap<String, usize>,
}

/// Load all fixture manifests from a directory.
///
/// Reads every `.json` file in `manifest_dir` as a fixture manifest and returns
/// a vector of `FixtureInfo` with the parsed id, format, categories, and features.
pub fn load_all_manifests(manifest_dir: &str) -> Result<Vec<FixtureInfo>, Vec<Diagnostic>> {
    let path = Path::new(manifest_dir);
    if !path.exists() {
        return Err(vec![Diagnostic::new(
            "GP-E-001",
            Severity::Error,
            format!("manifest directory not found: {}", manifest_dir),
        )]);
    }

    let mut fixtures = Vec::new();

    let mut entries: Vec<_> = fs::read_dir(path)
        .map_err(|e| vec![Diagnostic::new("CLI-E-002", Severity::Error, e.to_string())])?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .collect();

    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let content = fs::read_to_string(entry.path())
            .map_err(|e| vec![Diagnostic::new("CLI-E-002", Severity::Error, e.to_string())])?;

        let value: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| vec![Diagnostic::new("GP-E-001", Severity::Error, e.to_string())])?;

        let id = value["id"].as_str().unwrap_or("").to_string();
        let format = value["format"].as_str().unwrap_or("").to_string();
        let categories: Vec<String> = if let Some(arr) = value["categories"].as_array() {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        } else {
            vec![]
        };
        let expected_feature_counts: HashMap<String, usize> = value["features"]
            .as_object()
            .unwrap_or(&serde_json::Map::new())
            .iter()
            .filter_map(|(k, v)| v.as_u64().map(|n| (k.clone(), n as usize)))
            .collect();

        fixtures.push(FixtureInfo {
            id,
            format,
            categories,
            expected_feature_counts,
        });
    }

    Ok(fixtures)
}

/// Load pr-subset manifest IDs and return FixtureInfo for each.
///
/// Reads `subset_file` for fixture IDs (one per line, lines starting with `#` are comments),
/// then loads the corresponding manifests from `manifest_dir`.
pub fn load_pr_subset(
    manifest_dir: &str,
    subset_file: &str,
) -> Result<Vec<FixtureInfo>, Vec<Diagnostic>> {
    let subset_path = Path::new(subset_file);
    if !subset_path.exists() {
        return Err(vec![Diagnostic::new(
            "GP-E-001",
            Severity::Error,
            format!("subset file not found: {}", subset_file),
        )]);
    }

    let content = fs::read_to_string(subset_path)
        .map_err(|e| vec![Diagnostic::new("CLI-E-002", Severity::Error, e.to_string())])?;

    let ids: Vec<&str> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    let all = load_all_manifests(manifest_dir)?;
    let id_set: std::collections::HashSet<&str> = ids.iter().copied().collect();

    let filtered: Vec<FixtureInfo> = all
        .into_iter()
        .filter(|f| id_set.contains(f.id.as_str()))
        .collect();

    Ok(filtered)
}

/// Get the expected corpus manifest count (151).
pub const EXPECTED_CORPUS_COUNT: usize = 169;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .unwrap()
            .to_path_buf()
    }

    /// test_mock_codecs_compile — each MockCodec implements Codec
    #[test]
    fn test_mock_codecs_compile() {
        let docx = MockDocxCodec;
        assert_eq!(docx.format_name(), "docx");
        let result = docx.read(&[]).unwrap();
        assert!(!result.ir.is_empty());
        assert!(result.diagnostics.is_empty());

        let xlsx = MockXlsxCodec;
        assert_eq!(xlsx.format_name(), "xlsx");
        let xlsx_result = xlsx.read(&[]).unwrap();
        assert!(!xlsx_result.ir.is_empty());

        let pptx = MockPptxCodec;
        assert_eq!(pptx.format_name(), "pptx");

        let odt = MockOdtCodec;
        assert_eq!(odt.format_name(), "odt");

        let ods = MockOdsCodec;
        assert_eq!(ods.format_name(), "ods");

        let odp = MockOdpCodec;
        assert_eq!(odp.format_name(), "odp");

        let pdf = MockPdfCodec;
        assert_eq!(pdf.format_name(), "pdf");
        assert!(!pdf.read(&[]).unwrap().ir.is_empty());

        // Test write returns error
        let write_err = docx.write(&[]).unwrap_err();
        assert_eq!(write_err.len(), 1);
        assert_eq!(write_err[0].code, "GP-W-001");
    }

    /// test_mock_engine — MockEngine implements Engine + EngineExt
    #[test]
    fn test_mock_engine() {
        let eng = MockEngine::new("mock-libreoffice", "26.2.5", "abc123hash");
        assert_eq!(eng.engine_id(), "mock-libreoffice");
        assert_eq!(eng.version(), Some("26.2.5"));
        assert!(eng.can_convert("docx", "pdf"));
        assert!(eng.is_available());

        let bytes = eng.convert(b"input", "docx", "pdf").unwrap();
        assert!(bytes.is_empty());

        // EngineExt
        let info = eng.info();
        assert_eq!(info.engine_id, "mock-libreoffice");
        assert_eq!(info.version, Some("26.2.5".into()));
        assert_eq!(info.fontset_hash, Some("abc123hash".into()));
        assert!(info.is_available);
        assert_eq!(info.capabilities.len(), 3);

        assert_eq!(eng.fontset_hash(), Some("abc123hash".into()));

        // Empty version/hash
        let bare = MockEngine::new("bare", "", "");
        assert_eq!(bare.version(), None);
        assert_eq!(bare.fontset_hash(), None);
    }

    /// test_mock_probe — MockProbe returns CapabilityMatrix
    #[test]
    fn test_mock_probe() {
        let probe = MockProbe;
        let eng = MockEngine::new("test-eng", "1.0", "h1");
        let matrix = probe.probe(&eng);
        assert_eq!(matrix.engine, "test-eng");
        assert_eq!(matrix.version, Some("1.0".into()));
        assert_eq!(matrix.conversions.len(), 3);
        assert!(matrix.conversions[0].works);
    }

    /// test_mock_linter — MockLinter + BatchLinter
    #[test]
    fn test_mock_linter() {
        let linter = MockLinter;
        let pages = vec![
            LinterPage {
                page_number: 1,
                width_pt: 612.0,
                height_pt: 792.0,
                text_bboxes: vec![],
                exists: true,
            },
            LinterPage {
                page_number: 2,
                width_pt: 612.0,
                height_pt: 792.0,
                text_bboxes: vec![],
                exists: true,
            },
        ];
        let diags = linter.lint(&[], &pages);
        assert!(diags.is_empty());

        let batch = linter.lint_batch(&pages);
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].page_number, 1);
        assert!(batch[0].diagnostics.is_empty());
        assert_eq!(batch[1].page_number, 2);
        assert!(batch[1].diagnostics.is_empty());
    }

    /// test_patch_executor_type_only — MockPatchExecutor implements PatchExecutor
    #[test]
    fn test_patch_executor_type_only() {
        let executor = MockPatchExecutor;
        let patch = b"some-patch-data";
        let result = executor.apply(patch).unwrap();
        assert_eq!(result, patch.as_slice());
    }

    /// test_load_all_manifests — load manifests from corpus/manifests/, count == EXPECTED_CORPUS_COUNT (169: 157 M0-era + 12 P5-02 PDF fixtures)
    #[test]
    fn test_load_all_manifests() {
        let manifests_dir = workspace_root().join("corpus").join("manifests");
        let fixtures =
            load_all_manifests(manifests_dir.to_str().unwrap()).expect("should load manifests");
        assert_eq!(
            fixtures.len(),
            EXPECTED_CORPUS_COUNT,
            "expected {} manifests, got {}",
            EXPECTED_CORPUS_COUNT,
            fixtures.len()
        );

        // Verify each fixture has a non-empty id
        for f in &fixtures {
            assert!(!f.id.is_empty(), "fixture id should not be empty");
            assert!(
                !f.format.is_empty(),
                "fixture format should not be empty for {}",
                f.id
            );
        }

        // Verify formats are from the expected set
        let valid_formats = ["docx", "xlsx", "pptx", "odt", "ods", "odp", "pdf"];
        for f in &fixtures {
            assert!(
                valid_formats.contains(&f.format.as_str()),
                "unexpected format '{}' for fixture '{}'",
                f.format,
                f.id
            );
        }

        // Verify categories are non-empty for all fixtures
        for f in &fixtures {
            assert!(
                !f.categories.is_empty(),
                "categories should not be empty for {}",
                f.id
            );
        }
    }

    /// test_load_pr_subset — load pr-subset, count == 20 (matches pr-subset.txt)
    #[test]
    fn test_load_pr_subset() {
        let manifests_dir = workspace_root().join("corpus").join("manifests");
        let subset_file = workspace_root().join("corpus").join("pr-subset.txt");
        let fixtures = load_pr_subset(
            manifests_dir.to_str().unwrap(),
            subset_file.to_str().unwrap(),
        )
        .expect("should load pr subset");
        assert_eq!(
            fixtures.len(),
            20,
            "pr-subset should have 20 fixtures, got {}",
            fixtures.len()
        );

        // Verify specific IDs from pr-subset
        let ids: Vec<&str> = fixtures.iter().map(|f| f.id.as_str()).collect();
        assert!(ids.contains(&"cjk-text-docx-001"));
        assert!(ids.contains(&"comments-annotations-docx-001"));
        assert!(ids.contains(&"conditional-formatting-xlsx-001"));
        assert!(ids.contains(&"smartart-drawing-docx-001"));
        assert!(ids.contains(&"unicode-edge-docx-001"));
    }

    /// test_fixture_info_serializes — FixtureInfo round-trip
    #[test]
    fn test_fixture_info_serializes() {
        let mut features = HashMap::new();
        features.insert("w:tbl".to_string(), 5);
        features.insert("w:drawing".to_string(), 3);

        let fixture = FixtureInfo {
            id: "test-fixture-001".into(),
            format: "docx".into(),
            categories: vec!["custom-styles".into(), "tables".into()],
            expected_feature_counts: features,
        };

        let json = serde_json::to_string(&fixture).unwrap();
        let back: FixtureInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(fixture.id, back.id);
        assert_eq!(fixture.format, back.format);
        assert_eq!(fixture.categories, back.categories);
        assert_eq!(
            fixture.expected_feature_counts,
            back.expected_feature_counts
        );
    }

    /// test_codec_builder — CodecBuilder::for_format returns correct mocks
    #[test]
    fn test_codec_builder() {
        struct TestCodecBuilder;

        impl CodecBuilder for TestCodecBuilder {
            fn for_format(format: &str) -> Option<Box<dyn Codec>> {
                match format {
                    "docx" => Some(Box::new(MockDocxCodec)),
                    "xlsx" => Some(Box::new(MockXlsxCodec)),
                    "pptx" => Some(Box::new(MockPptxCodec)),
                    "odt" => Some(Box::new(MockOdtCodec)),
                    "ods" => Some(Box::new(MockOdsCodec)),
                    "odp" => Some(Box::new(MockOdpCodec)),
                    "pdf" => Some(Box::new(MockPdfCodec)),
                    _ => None,
                }
            }

            fn supported_formats(&self) -> Vec<&'static str> {
                vec!["docx", "xlsx", "pptx", "odt", "ods", "odp", "pdf"]
            }
        }

        let builder = TestCodecBuilder;
        let formats = builder.supported_formats();
        assert_eq!(formats.len(), 7);

        let docx = TestCodecBuilder::for_format("docx").unwrap();
        assert_eq!(docx.format_name(), "docx");

        let xlsx = TestCodecBuilder::for_format("xlsx").unwrap();
        assert_eq!(xlsx.format_name(), "xlsx");

        let pdf = TestCodecBuilder::for_format("pdf").unwrap();
        assert_eq!(pdf.format_name(), "pdf");

        assert!(TestCodecBuilder::for_format("rtf").is_none());
        assert!(TestCodecBuilder::for_format("unknown").is_none());
    }
}
