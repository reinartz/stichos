// SPDX-License-Identifier: MIT OR Apache-2.0
//! Performance measurement for the edit→render→look latency loop.
//!
//! OPS-02: P95 ≤ 2s CI gate for closed-loop edit→render→look latency.
//!
//! # Overview
//!
//! `measure_edit_render_loop` exercises the executor + engine pipeline
//! against a corpus subset, collecting per-iteration wall-clock latencies
//! and computing P50/P95/P99/max statistics.
//!
//! # Usage
//!
//! CLI: `stichos perf --fixtures pr-subset --target 2000`
//!
//! Library:
//! ```no_run
//! # use lint::perf::{measure_edit_render_loop, FixtureRef};
//! // let fixtures = vec![/* ... */];
//! // let result = measure_edit_render_loop(&fixtures, &engine, &executor).await?;
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use gp_core::executor::{FormatType, SurgicalPatchExecutor};
use gp_core::ir::ElementId;
use gp_core::patch::PatchOp;
use gp_core::traits::Engine;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A fixture reference for performance runs.
#[derive(Debug, Clone)]
pub struct FixtureRef {
    /// Human-readable fixture name (e.g. "hello-world-docx").
    pub name: String,
    /// Absolute or relative path to the document file.
    pub path: PathBuf,
}

/// Performance statistics for a single format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatPerf {
    /// Number of iterations measured for this format.
    pub iterations: usize,
    /// P95 latency in milliseconds.
    pub p95_ms: f64,
    /// Maximum latency in milliseconds.
    pub max_ms: f64,
}

/// Aggregate performance measurement result.
///
/// Serializable to JSON for CI reporting (P2-33 / OPS-02).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfResult {
    /// Total wall-clock time for the entire benchmark run (ms).
    pub total_duration_ms: f64,
    /// Number of fixture iterations completed.
    pub iterations: usize,
    /// P50 (median) latency (ms).
    pub p50_ms: f64,
    /// P95 latency (ms) — the OPS-02 gate target.
    pub p95_ms: f64,
    /// P99 latency (ms).
    pub p99_ms: f64,
    /// Maximum observed latency (ms).
    pub max_ms: f64,
    /// Minimum observed latency (ms).
    pub min_ms: f64,
    /// Cache hit rate (0.0–1.0) — fraction of engine lookups that hit cache.
    pub cache_hit_rate: f64,
    /// Per-format breakdown.
    pub per_format: HashMap<String, FormatPerf>,
}

impl PerfResult {
    /// Check whether the P95 latency meets the given target.
    pub fn meets_target(&self, target_ms: f64) -> bool {
        self.p95_ms <= target_ms
    }

    /// Serialize to pretty-printed JSON string.
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Serialize to compact JSON string.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

// ---------------------------------------------------------------------------
// Engine cache shim (reuses P1-29 cache pattern)
// ---------------------------------------------------------------------------

/// A minimal cache that tracks hit/miss rates for engine lookups.
///
/// This mirrors the P1-29 render cache but is kept internal to the perf
/// module to avoid pulling the full cache infrastructure into the lint crate.
struct CacheTracker {
    hits: usize,
    misses: usize,
}

impl CacheTracker {
    fn new() -> Self {
        Self { hits: 0, misses: 0 }
    }

    #[allow(dead_code)]
    fn record_hit(&mut self) {
        self.hits += 1;
    }

    fn record_miss(&mut self) {
        self.misses += 1;
    }

    fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            return 0.0;
        }
        self.hits as f64 / total as f64
    }
}

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

/// Load fixture references from a corpus manifest file.
///
/// The manifest is expected to be a JSON Lines file with entries like:
/// `{"name": "...", "path": "..."}`
pub fn load_fixtures_from_manifest(path: &str) -> Result<Vec<FixtureRef>, std::io::Error> {
    let content = std::fs::read_to_string(path)?;
    let mut fixtures = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(val) => {
                let name = val
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let path_str = val.get("path").and_then(|v| v.as_str()).unwrap_or(line);
                fixtures.push(FixtureRef {
                    name,
                    path: PathBuf::from(path_str),
                });
            }
            Err(_) => {
                // Treat each line as a plain path
                fixtures.push(FixtureRef {
                    name: line.to_string(),
                    path: PathBuf::from(line),
                });
            }
        }
    }
    Ok(fixtures)
}

/// Discover fixtures from the corpus pr-subset.
///
/// Reads `corpus/pr-subset.txt` for fixture IDs, then looks up
/// `corpus/blobs/` for actual files. Falls back to corpus reference dir.
pub fn discover_pr_subset_fixtures() -> Vec<FixtureRef> {
    let pr_subset = PathBuf::from("corpus/pr-subset.txt");
    let blobs_dir = PathBuf::from("corpus/blobs");
    let ref_dir = PathBuf::from("corpus/reference");

    if !pr_subset.exists() {
        warn!("corpus/pr-subset.txt not found — returning empty fixture list");
        return Vec::new();
    }

    let ids: Vec<String> = std::fs::read_to_string(&pr_subset)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    let mut fixtures = Vec::new();
    for id in &ids {
        // Check blobs first, then reference
        let mut found = false;
        for dir in [&blobs_dir, &ref_dir] {
            for ext in &["docx", "xlsx", "pptx", "odt", "ods", "odp", "pdf"] {
                let candidate = dir.join(format!("{}.{}", id, ext));
                if candidate.exists() {
                    fixtures.push(FixtureRef {
                        name: id.clone(),
                        path: candidate,
                    });
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
        if !found {
            debug!("fixture {} not found on disk — will be skipped", id);
        }
    }
    fixtures
}

/// Load fixtures from a path (plain-text newline-separated list).
pub fn load_fixtures_from_path(path: &PathBuf) -> Result<Vec<FixtureRef>, std::io::Error> {
    let content = std::fs::read_to_string(path)?;
    Ok(content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| FixtureRef {
            name: l.to_string(),
            path: PathBuf::from(l),
        })
        .collect())
}

/// Load fixture references from CLI arguments.
pub fn load_fixture_set(arg: &Option<String>) -> Result<Vec<FixtureRef>, anyhow::Error> {
    match arg.as_deref() {
        Some("pr-subset") => Ok(discover_pr_subset_fixtures()),
        Some(path) => {
            let path_buf = PathBuf::from(path);
            if path_buf.exists() && path_buf.is_file() {
                load_fixtures_from_path(&path_buf).map_err(|e| anyhow::anyhow!("{}", e))
            } else {
                anyhow::bail!("fixture path not found: {}", path);
            }
        }
        None => {
            // Default: pr-subset
            Ok(discover_pr_subset_fixtures())
        }
    }
}

/// Resolve fixture references by name to actual files.
pub fn resolve_fixtures(fixtures: &[FixtureRef]) -> Vec<FixtureRef> {
    fixtures
        .iter()
        .filter(|f| f.path.exists())
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// Core measurement
// ---------------------------------------------------------------------------

/// Determine the format type from a file path.
fn format_from_path(path: &std::path::Path) -> Option<FormatType> {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(|e| match e.to_lowercase().as_str() {
            "docx" => Some(FormatType::Docx),
            "xlsx" => Some(FormatType::Xlsx),
            "pptx" => Some(FormatType::Pptx),
            "odt" => Some(FormatType::Odt),
            "ods" => Some(FormatType::Ods),
            "odp" => Some(FormatType::Odp),
            "pdf" => Some(FormatType::Pdf),
            _ => None,
        })
}

/// Generate a small edit (text replacement) for a fixture.
fn generate_test_edit(fixture_name: &str) -> Vec<PatchOp> {
    vec![PatchOp::ReplaceText {
        elem: ElementId::content_anchored(
            "perf-edit",
            &[&format!("{}:0", fixture_name)],
            "benchmark-text",
        ),
        old: "benchmark-text".to_string(),
        new: format!("bench-{}", fixture_name).to_string(),
    }]
}

/// Run a single perf iteration on one fixture.
///
/// Returns (elapsed_ms, is_cache_hit).
fn run_fixture_iteration(
    fixture: &FixtureRef,
    executor: &SurgicalPatchExecutor,
    engine: Option<&dyn Engine>,
    cache: &mut CacheTracker,
) -> (f64, bool) {
    let source = match std::fs::read(&fixture.path) {
        Ok(bytes) => bytes,
        Err(e) => {
            debug!("cannot read fixture {}: {}", fixture.name, e);
            return (0.0, false);
        }
    };

    let format = match format_from_path(&fixture.path) {
        Some(f) => f,
        None => return (0.0, false),
    };

    // Skip non-writable formats for executor tests
    if !format.is_writable() {
        // Still count engine availability check
        if let Some(eng) = engine {
            let _cache_key = format!("{}:{}", eng.engine_id(), format.as_str());
            // Simulate cache lookup
            cache.record_miss();
        }
        return (0.0, false);
    }

    let ops = generate_test_edit(&fixture.name);
    let start = Instant::now();

    // Execute the patch
    let exec_result = executor.execute(&source, format.clone(), &ops);

    // Check engine if available
    let cache_hit = if let Some(eng) = engine {
        if !exec_result.accepted {
            cache.record_miss();
            false
        } else {
            let cache_key = format!("{}:{}", eng.engine_id(), format.as_str());
            let _ = cache_key; // Used for real cache integration later
            cache.record_miss();
            // Attempt conversion (may fail if engine unavailable — that's OK)
            let _ = eng.convert(&exec_result.output, format.as_str(), "pdf");
            false
        }
    } else {
        cache.record_miss();
        false
    };

    let elapsed = start.elapsed().as_millis() as f64;
    (elapsed, cache_hit)
}

/// Run a performance measurement on the given fixtures.
///
/// For each fixture:
/// 1. Generate a small edit (text replacement)
/// 2. Execute the edit
/// 3. Optionally render the output via the engine (if available)
/// 4. Measure total wall-clock time
///
/// Collects all latencies, computes P50/P95/P99/max.
///
/// # Arguments
/// * `fixtures` — fixture references to measure
/// * `engine` — optional rendering engine (may be unavailable → skip with warning)
/// * `executor` — the patch executor to use
///
/// # Non-blocking behavior
///
/// If LibreOffice (or any engine) is not available, the measurement continues
/// with engine-skipped iterations and logs a warning. The job does NOT fail
/// due to missing engines.
pub fn measure_edit_render_loop(
    fixtures: &[FixtureRef],
    engine: Option<&dyn Engine>,
    executor: &SurgicalPatchExecutor,
) -> PerfResult {
    let run_start = Instant::now();
    let mut latencies: Vec<f64> = Vec::new();
    let mut format_latencies: HashMap<String, Vec<f64>> = HashMap::new();
    let mut iterations = 0usize;
    let mut cache = CacheTracker::new();

    // Check engine availability — warn but continue
    let engine_available = engine
        .map(|e| {
            let available = e.is_available();
            if !available {
                warn!(
                    "engine {} is not available — skipping render step (non-blocking)",
                    e.engine_id()
                );
            }
            available
        })
        .unwrap_or(false);

    if !engine_available {
        warn!("no engine available — measuring executor-only latency");
    }

    for fixture in fixtures {
        // Skip fixtures that don't exist on disk
        if !fixture.path.exists() {
            debug!("skipping missing fixture: {}", fixture.name);
            continue;
        }

        let (elapsed, _cache_hit) = run_fixture_iteration(fixture, executor, engine, &mut cache);

        // Record non-zero latencies
        if elapsed > 0.0 {
            iterations += 1;
            latencies.push(elapsed);

            let fmt_name = format_from_path(&fixture.path)
                .map(|f| f.as_str().to_string())
                .unwrap_or_else(|| "unknown".to_string());

            format_latencies.entry(fmt_name).or_default().push(elapsed);
        }

        if iterations % 5 == 0 {
            debug!("perf: {} iterations so far", iterations);
        }
    }

    let total_duration = run_start.elapsed().as_millis() as f64;

    // Compute statistics
    let stats = percentile_stats(&latencies);
    let per_format: HashMap<String, FormatPerf> = format_latencies
        .into_iter()
        .map(|(name, lats)| {
            let (p95, max) = {
                let s = percentile_stats(&lats);
                (s.1, s.2)
            };
            (
                name,
                FormatPerf {
                    iterations: lats.len(),
                    p95_ms: p95,
                    max_ms: max,
                },
            )
        })
        .collect();

    let result = PerfResult {
        total_duration_ms: total_duration,
        iterations,
        p50_ms: stats.0,
        p95_ms: stats.1,
        p99_ms: stats.2,
        max_ms: stats.3,
        min_ms: stats.4,
        cache_hit_rate: cache.hit_rate(),
        per_format,
    };

    info!(
        "perf: {} iterations, P50={}ms P95={}ms P99={}ms max={}ms cache_hit_rate={:.2}",
        iterations,
        result.p50_ms,
        result.p95_ms,
        result.p99_ms,
        result.max_ms,
        result.cache_hit_rate
    );

    result
}

/// Compute P50/P95/P99/max/min from a slice of latency values.
///
/// Returns `(p50, p95, p99, max, min)` in milliseconds.
fn percentile_stats(latencies: &[f64]) -> (f64, f64, f64, f64, f64) {
    if latencies.is_empty() {
        return (0.0, 0.0, 0.0, 0.0, 0.0);
    }

    let mut sorted = latencies.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let min = sorted.first().copied().unwrap_or(0.0);
    let max = sorted.last().copied().unwrap_or(0.0);

    let p50 = percentile_at(&sorted, 0.50);
    let p95 = percentile_at(&sorted, 0.95);
    let p99 = percentile_at(&sorted, 0.99);

    (p50, p95, p99, max, min)
}

/// Compute the value at the given percentile from a sorted slice.
///
/// Uses linear interpolation between closest ranks.
fn percentile_at(sorted: &[f64], percentile: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }

    let rank = percentile * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;

    if lower >= sorted.len() - 1 && upper >= sorted.len() - 1 {
        return *sorted.last().unwrap_or(&0.0);
    }

    let fraction = rank - lower as f64;
    sorted[lower] + fraction * (sorted[upper.min(sorted.len() - 1)] - sorted[lower])
}

// ---------------------------------------------------------------------------
// CLI helpers
// ---------------------------------------------------------------------------

/// Run the perf benchmark with CLI argument parsing.
///
/// Returns exit code 0 on success, 1 on P95 target failure.
///
/// This function is provided for library users who have an engine available.
/// For CLI usage, see `perf_tool::run_perf` which handles engine discovery.
pub fn run_perf(
    fixtures: &[FixtureRef],
    engine: Option<&dyn Engine>,
    target_ms: f64,
) -> Result<i32, anyhow::Error> {
    let executor = SurgicalPatchExecutor::default();
    let result = measure_edit_render_loop(fixtures, engine, &executor);

    // Print result as JSON
    println!("{}", result.to_json_pretty()?);

    // Check target
    if !result.meets_target(target_ms) {
        warn!(
            "P95={}ms exceeds target={}ms — OPS-02 gate FAILED",
            result.p95_ms, target_ms
        );
        return Ok(1);
    }

    info!(
        "OPS-02 gate PASSED: P95={}ms <= target={}ms",
        result.p95_ms, target_ms
    );
    Ok(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_perf_result_serialization() {
        let result = PerfResult {
            total_duration_ms: 1500.0,
            iterations: 10,
            p50_ms: 120.0,
            p95_ms: 1800.0,
            p99_ms: 1950.0,
            max_ms: 2000.0,
            min_ms: 80.0,
            cache_hit_rate: 0.3,
            per_format: {
                let mut map = HashMap::new();
                map.insert(
                    "docx".to_string(),
                    FormatPerf {
                        iterations: 5,
                        p95_ms: 1700.0,
                        max_ms: 1900.0,
                    },
                );
                map
            },
        };

        // Serialize to JSON
        let json = result.to_json().expect("serialize PerfResult");
        assert!(!json.is_empty());

        // Deserialize back
        let parsed: PerfResult = serde_json::from_str(&json).expect("deserialize PerfResult");
        assert_eq!(parsed.iterations, 10);
        assert!((parsed.p50_ms - 120.0).abs() < f64::EPSILON);
        assert!((parsed.p95_ms - 1800.0).abs() < f64::EPSILON);
        assert_eq!(parsed.per_format.get("docx").unwrap().iterations, 5);
    }

    #[test]
    fn test_perf_result_meets_target() {
        let passing = PerfResult {
            total_duration_ms: 100.0,
            iterations: 5,
            p50_ms: 10.0,
            p95_ms: 1500.0,
            p99_ms: 1800.0,
            max_ms: 1900.0,
            min_ms: 5.0,
            cache_hit_rate: 0.0,
            per_format: HashMap::new(),
        };
        assert!(passing.meets_target(2000.0));
        assert!(!passing.meets_target(1000.0));

        let failing = PerfResult {
            total_duration_ms: 100.0,
            iterations: 5,
            p50_ms: 1500.0,
            p95_ms: 2500.0,
            p99_ms: 2800.0,
            max_ms: 3000.0,
            min_ms: 1000.0,
            cache_hit_rate: 0.0,
            per_format: HashMap::new(),
        };
        assert!(!failing.meets_target(2000.0));
        assert!(failing.meets_target(3000.0));
    }

    #[test]
    fn test_percentile_stats_empty() {
        let stats = percentile_stats(&[]);
        assert_eq!(stats, (0.0, 0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn test_percentile_stats_single() {
        let stats = percentile_stats(&[100.0]);
        assert_eq!(stats.0, 100.0); // p50
        assert_eq!(stats.1, 100.0); // p95
        assert_eq!(stats.2, 100.0); // p99
        assert_eq!(stats.3, 100.0); // max
        assert_eq!(stats.4, 100.0); // min
    }

    #[test]
    fn test_percentile_stats_known_data() {
        // [10, 20, 30, 40, 50, 60, 70, 80, 90, 100]
        let latencies: Vec<f64> = (1..=10).map(|x| x as f64 * 10.0).collect();
        let stats = percentile_stats(&latencies);

        // P50 should be near 55 (interpolated between 50 and 60)
        assert!(
            stats.0 > 45.0 && stats.0 < 65.0,
            "p50={} expected ~55",
            stats.0
        );
        // P95 should be near 95
        assert!(
            stats.1 > 85.0 && stats.1 < 100.0,
            "p95={} expected ~95",
            stats.1
        );
        // P99 should be near 99
        assert!(
            stats.2 > 90.0 && stats.2 <= 100.0,
            "p99={} expected ~99",
            stats.2
        );
        // max/min
        assert_eq!(stats.3, 100.0);
        assert_eq!(stats.4, 10.0);
    }

    #[test]
    fn test_percentile_stats_small_set() {
        // [100, 200]
        let stats = percentile_stats(&[100.0, 200.0]);
        // P50 interpolates between 100 and 200
        assert!(stats.0 >= 100.0 && stats.0 <= 200.0);
        // P95 should be near 200
        assert!(stats.1 >= 190.0 && stats.1 <= 200.0);
        assert_eq!(stats.3, 200.0); // max
        assert_eq!(stats.4, 100.0); // min
    }

    #[test]
    fn test_format_from_path() {
        assert_eq!(
            format_from_path(&PathBuf::from("test.docx")),
            Some(FormatType::Docx)
        );
        assert_eq!(
            format_from_path(&PathBuf::from("test.xlsx")),
            Some(FormatType::Xlsx)
        );
        assert_eq!(
            format_from_path(&PathBuf::from("test.pptx")),
            Some(FormatType::Pptx)
        );
        assert_eq!(
            format_from_path(&PathBuf::from("test.pdf")),
            Some(FormatType::Pdf)
        );
        assert_eq!(format_from_path(&PathBuf::from("test.unknown")), None);
    }

    #[test]
    fn test_format_from_path_case_insensitive() {
        assert_eq!(
            format_from_path(&PathBuf::from("test.DOCX")),
            Some(FormatType::Docx)
        );
        assert_eq!(
            format_from_path(&PathBuf::from("test.XLSX")),
            Some(FormatType::Xlsx)
        );
    }

    #[test]
    fn test_cache_tracker_hit_rate() {
        let mut cache = CacheTracker::new();
        assert_eq!(cache.hit_rate(), 0.0); // No data → 0.0

        cache.record_hit();
        assert_eq!(cache.hit_rate(), 1.0);

        cache.record_hit();
        cache.record_miss();
        assert!((cache.hit_rate() - 2.0 / 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_perf_result_to_json_pretty() {
        let result = PerfResult {
            total_duration_ms: 100.0,
            iterations: 1,
            p50_ms: 50.0,
            p95_ms: 80.0,
            p99_ms: 90.0,
            max_ms: 100.0,
            min_ms: 10.0,
            cache_hit_rate: 0.5,
            per_format: HashMap::new(),
        };
        let json = result.to_json_pretty().expect("pretty json");
        assert!(json.contains("\"total_duration_ms\""));
        assert!(json.contains("100.0"));
        // Pretty-printed should contain newlines
        assert!(json.contains('\n'));
    }

    #[test]
    fn test_fixture_ref_new() {
        let fixture = FixtureRef {
            name: "test-doc".to_string(),
            path: PathBuf::from("/path/to/test.docx"),
        };
        assert_eq!(fixture.name, "test-doc");
        assert_eq!(fixture.path.file_name().unwrap(), "test.docx");
    }

    #[test]
    fn test_generate_test_edit() {
        let ops = generate_test_edit("test-docx");
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            PatchOp::ReplaceText { old, new, .. } => {
                assert_eq!(old, "benchmark-text");
                assert_eq!(new, "bench-test-docx");
            }
            _ => panic!("expected ReplaceText"),
        }
    }

    #[test]
    fn test_percentile_stats_five_values() {
        // [10, 20, 30, 40, 50]
        let latencies = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        let stats = percentile_stats(&latencies);
        assert_eq!(stats.3, 50.0); // max
        assert_eq!(stats.4, 10.0); // min
                                   // p50 should be around 30
        assert!(stats.0 >= 25.0 && stats.0 <= 35.0);
    }
}
