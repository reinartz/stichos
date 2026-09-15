# stichos

**proof before publish.**

stichos is a local proof/render fidelity toolchain for office documents, built for AI agents: it
owns the file format, delegates layout to real rendering engines, and returns rendered images plus
machine-readable check results so the calling LLM can look at what it produced. The name is always
lowercase `stichos`.

## Repo status

Phase 0 / M0: the Python fidelity harness is live (151-fixture corpus + goldens, baseline
report-v1, reproducible `fontset_hash`, nightly ×3 green). Phase 1 / M1 (read-only ship) closed
2026-09-14 (tag `m1-closeout`). Phase 2 / M2 (surgical edit + closed loop) functionally complete
2026-09-15; closeout debt (clippy/fmt, uncommitted Waves C–F, misplaced `m2` tag) is Phase 3
Wave 0. Phase 3 / M3 (creation + ODF/PDF write completion) planned:
[`docs/phase3-plan.md`](docs/phase3-plan.md).

## Doc map

| Path | What it is |
|---|---|
| [`project_concept.md`](project_concept.md) | concept + decision record (D1–D21; §16 decision log is source of truth) |
| [`project_plan.md`](project_plan.md) | plan: milestones M0–M4, tracks, contract registry (§4), decision gates (§6.4) |
| [`CODEBASE.md`](CODEBASE.md) | Rust crate structure, dependency DAG, module details, test coverage summary |
| [`docs/phase0-execution.md`](docs/phase0-execution.md) | M0 runbook: gate status, CI bug-fix log |
| [`docs/phase1-concept.md`](docs/phase1-concept.md) | Phase 1 concept: read-only ship (M1), scope, gate |
| [`docs/phase1-plan.md`](docs/phase1-plan.md) | Phase 1 plan: 61 agent-sized steps in 6 waves |
| [`docs/phase1-status.json`](docs/phase1-status.json) | Phase 1 progress tracker (canonical step status) |
| [`docs/phase2-concept.md`](docs/phase2-concept.md) · [`phase2-plan.md`](docs/phase2-plan.md) · [`phase2-status.json`](docs/phase2-status.json) | Phase 2 (M2): concept, 35 steps in 7 waves, progress tracker |
| [`docs/m2-acceptance.md`](docs/m2-acceptance.md) | M2 acceptance report + Phase-2b backlog |
| [`docs/phase3-concept.md`](docs/phase3-concept.md) | Phase 3 concept: creation + write-path completion (M3), scope, gate, owner calls S1–S4 |
| [`docs/phase3-plan.md`](docs/phase3-plan.md) | Phase 3 plan: 33 agent-sized steps in 8 waves |
| [`docs/phase3-status.json`](docs/phase3-status.json) | Phase 3 progress tracker (canonical step status) |
| [`docs/gov/`](docs/gov/) | name hygiene: clearance memo, registration checklist, distro collisions |
| [`docs/adr/`](docs/adr/) | architecture decision records (ADR-0001: PDF stack, accepted) |
| [`harness/`](harness/) | M0 Python fidelity harness (dev-only, never shipped) |
| [`corpus/`](corpus/) | fixture corpus: manifests, blobs, PLAN.md, GENERATION_REPORT.md |

Contributing: [`CONTRIBUTING.md`](CONTRIBUTING.md) (dual license, SPDX headers, DCO, harness setup).

## License

Dual-licensed **MIT OR Apache-2.0** ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)),
unblocked by G1 (ADR-0001 accepted 2026-09-08, D19).

## Visibility

The repository is currently **private** (`github.com/reinartz/stichos`, D18); public launch is a
tracked decision.
