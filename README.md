# stichos

**proof before publish.**

stichos is a local proof/render fidelity toolchain for office documents, built for AI agents: it
owns the file format, delegates layout to real rendering engines, and returns rendered images plus
machine-readable check results so the calling LLM can look at what it produced. The name is always
lowercase `stichos`.

## Repo status

Phases 0–6 are complete and gated: M0 harness (151-fixture corpus + goldens, baseline report-v1,
reproducible `fontset_hash`, nightly ×3 green), M1 read-only ship (tag `m1-closeout`), M2 surgical
edit + closed loop (tag `m2`), M3 creation + ODF/PDF write-path completion + sandbox + distribution
automation (tag `m3`, gate re-verified 2026-09-16: 1 644 tests green, clippy/fmt/deny clean,
`phase3_gate.sh` 26/26). **Phase 4 / M4 (human loop: tracked-change accept/reject, review overlay,
ODF revisions, handoff) is complete** (tag `m4`, 1 714+ tests green). **Phase 5 / M5 (PDF-native
loop + trust: C12 annotation round-trip, PDF review session + opt-in pdf.js text layer, C13
signature integrity + PAdES-B-B sign-later, launch prep only) is complete** (tag `m5`,
1 807 lib tests green; see `docs/m5-acceptance.md` — §9 holds the post-acceptance correction).
**Phase 6 / M6 (context budget C14 + COM/formula/revisions intake) shipped 2026-09-23**
(35/35 steps, `phase6_gate.sh --ci` green pre-tag, tag `m6`; `main` + `m6` **pushed**).
`docs/m6-acceptance.md` §11–§13 record the closeout rounds; Phase-7 Tracks A′/B/C/D are complete
except P7-3 tranches C/D and the owner-timed launch (P7-9).
[`docs/remaining-work.md`](docs/remaining-work.md) (rev 6) holds the remaining-track breakdown
(Tracks E/F/G + Track H (proposed) + owner items). Track-H work (generic Office handling; H0
adoption declined 2026-10-01) is preserved on branch `track-h-proposed`; `main` is clean.
See [`docs/phase4-concept.md`](docs/phase4-concept.md) / [`docs/phase4-plan.md`](docs/phase4-plan.md) /
[`docs/support-matrix.md`](docs/support-matrix.md).

## Doc map

| Path | What it is |
|---|---|
| [`project_concept.md`](project_concept.md) | concept + decision record (D1–D27; §16 decision log is source of truth) |
| [`project_plan.md`](project_plan.md) | plan: milestones M0–M6, tracks, contract registry (§4), decision gates (§6.4) |
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
| [`docs/m4-acceptance.md`](docs/m4-acceptance.md) | M4 acceptance report (Git-State section = tag-re-anchoring correction) + Phase-5 intake |
| [`docs/phase5-concept.md`](docs/phase5-concept.md) | Phase 5 concept: PDF-native loop + trust (M5), scope, gate, owner calls C1–C5 |
| [`docs/phase5-plan.md`](docs/phase5-plan.md) | Phase 5 plan: 19 agent-sized steps in 6 waves |
| [`docs/phase5-status.json`](docs/phase5-status.json) | Phase 5 progress tracker (canonical step status) |
| [`docs/m5-acceptance.md`](docs/m5-acceptance.md) | M5 acceptance report (§9 = post-acceptance correction) + Phase-6 intake |
| [`docs/phase6-concept.md`](docs/phase6-concept.md) | Phase 6 concept: context budget C14 + M6 intake (M6, active), scope, gate, owner calls C6–C9 |
| [`docs/phase6-plan.md`](docs/phase6-plan.md) | Phase 6 plan: agent-sized steps in 6 waves |
| [`docs/phase6-status.json`](docs/phase6-status.json) | Phase 6 progress tracker (canonical step status) |
| [`docs/contracts/C14-context-budget.md`](docs/contracts/C14-context-budget.md) | C14 contract (draft → freeze at M6) + [`docs/implementation-guide-context-budget.md`](docs/implementation-guide-context-budget.md) |
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
