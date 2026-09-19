// SPDX-License-Identifier: MIT OR Apache-2.0
//! Performance measurement CLI subcommand.
//!
//! `stichos perf --fixtures pr-subset --target 2000`
//!
//! Measures the edit→render→look latency loop against a corpus subset
//! and reports P50/P95/P99/max statistics. OPS-02 gate: P95 ≤ 2000ms.

use gp_core::executor::SurgicalPatchExecutor;
use gp_core::traits::Engine;
use lint::perf;

/// Run the performance benchmark from CLI arguments.
///
/// # Arguments
/// * `fixtures_arg` — corpus subset name or path (default: pr-subset)
/// * `target_ms` — P95 target in milliseconds (default: 2000)
///
/// # Returns
/// Exit code: 0 if P95 ≤ target, 1 if exceeded.
pub fn run_perf(fixtures_arg: Option<String>, target_ms: f64) -> Result<i32, anyhow::Error> {
    let fixtures = perf::resolve_fixtures(&perf::load_fixture_set(&fixtures_arg)?);

    if fixtures.is_empty() {
        tracing::warn!("no fixtures found — skipping perf measurement (non-blocking)");
        let empty_result = perf::PerfResult {
            total_duration_ms: 0.0,
            iterations: 0,
            p50_ms: 0.0,
            p95_ms: 0.0,
            p99_ms: 0.0,
            max_ms: 0.0,
            min_ms: 0.0,
            cache_hit_rate: 0.0,
            per_format: std::collections::HashMap::new(),
        };
        println!("{}", empty_result.to_json_pretty()?);
        return Ok(0);
    }

    // Try to create a LibreOffice engine (soft dependency — skip if unavailable)
    let engine: Option<&dyn Engine> = match engines::libreoffice::LibreOfficeEngine::new_auto() {
        Ok(eng) => {
            tracing::info!(
                "using engine: {} ({})",
                eng.engine_id(),
                eng.version().unwrap_or("unknown")
            );
            let boxed: Box<dyn Engine> = Box::new(eng);
            Some(Box::leak(boxed))
        }
        Err(_) => {
            tracing::warn!(
                "LibreOffice not found — measuring executor-only latency (non-blocking)"
            );
            None
        }
    };

    let executor = SurgicalPatchExecutor::default();
    let result = perf::measure_edit_render_loop(&fixtures, engine, &executor);

    // Print result as JSON
    println!("{}", result.to_json_pretty()?);

    // Check target
    if !result.meets_target(target_ms) {
        tracing::warn!(
            "P95={}ms exceeds target={}ms — OPS-02 gate FAILED",
            result.p95_ms,
            target_ms
        );
        return Ok(1);
    }

    tracing::info!(
        "OPS-02 gate PASSED: P95={}ms <= target={}ms",
        result.p95_ms,
        target_ms
    );
    Ok(0)
}
