# stichos

**proof before publish.**

stichos is a local proof/render fidelity toolchain for office documents, built for AI agents: it
owns the file format, delegates layout to real rendering engines, and returns rendered images plus
machine-readable check results so the calling LLM can look at what it produced. The name is always
lowercase `stichos`.

## Repo status

Phases 0–3 are complete and gated: M0 harness (151-fixture corpus + goldens, baseline report-v1,
reproducible `fontset_hash`, nightly ×3 green), M1 read-only ship (tag `m1-closeout`), M2 surgical
edit + closed loop (tag `m2`), M3 creation + ODF/PDF write-path completion + sandbox + distribution
automation (tag `m3`, gate re-verified 2026-09-16: 1 644 tests green, clippy/fmt/deny clean,
`phase3_gate.sh` 26/26). **Phase 4 / M4 (human loop: tracked-change accept/reject, review overlay,
ODF revisions, handoff) is complete** (tag `m4`, 1 714+ tests green). See
[`docs/phase4-concept.md`](docs/phase4-concept.md) / [`docs/phase4-plan.md`](docs/phase4-plan.md) /
[`docs/support-matrix.md`](docs/support-matrix.md).

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
| [`docs/m3-acceptance.md`](docs/m3-acceptance.md) | M3 acceptance report (§7 = post-acceptance corrections) + Phase-4 intake |
| [`docs/creation-measurement.md`](docs/creation-measurement.md) | M3 creation-path measurement matrix (C9) |
| [`docs/threat-model.md`](docs/threat-model.md) | threat model + SEC-06 review checklists (M1–M3) |
| [`docs/release-playbook.md`](docs/release-playbook.md) | release / verify / rollback procedure (DIS-08) |
| [`docs/phase4-concept.md`](docs/phase4-concept.md) | Phase 4 concept: human loop — accept/reject, review overlay, ODF revisions, handoff (M4) |
| [`docs/phase4-plan.md`](docs/phase4-plan.md) | Phase 4 plan: 21 agent-sized steps in 6 waves |
| [`docs/phase4-status.json`](docs/phase4-status.json) | Phase 4 progress tracker (canonical step status) |
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
