# stichos

**proof before publish.**

stichos is a local proof/render fidelity toolchain for office documents, built for AI agents: it
owns the file format, delegates layout to real rendering engines, and returns rendered images plus
machine-readable check results so the calling LLM can look at what it produced. The name is always
lowercase `stichos`.

## Install

Release binaries for six platforms ship on the GitHub Releases page
(`github.com/reinartz/stichos/releases`): Linux x86_64/arm64 (glibc), Linux x86_64 (musl),
macOS arm64/x86_64, and Windows x86_64 — each a `.tar.gz` (`.zip` on Windows) with a `.sha256`
checksum (target matrix in `.github/workflows/release.yml`; verify/rollback in
[`docs/release-playbook.md`](docs/release-playbook.md)).

- **winget:** `winget install stichos.stichos`
- **Homebrew:** `brew install sticho`

Or build from source:

```bash
cargo install --path crates/cli    # installs the `stichos` binary
cargo build --release -p stichos   # or build in-tree → target/release/stichos
```

## Getting started

```bash
stichos doctor
```

probes render-engine availability (LibreOffice, pandoc, Chromium, Typst, plus the raster ladder)
and reports which capabilities work on your machine — run it once before your first render.

## Repo status

Phases 0–6 are complete and gated: M0 harness (151-fixture corpus + goldens, baseline report-v1,
reproducible `fontset_hash`, nightly ×3 green), M1 read-only ship (tag `m1-closeout`), M2 surgical
edit + closed loop (tag `m2`), M3 creation + ODF/PDF write-path completion + sandbox + distribution
automation (tag `m3`, gate re-verified 2026-09-16: 1 644 tests green, clippy/fmt/deny clean,
`phase3_gate.sh` 26/26). **Phase 4 / M4 (human loop: tracked-change accept/reject, review overlay,
ODF revisions, handoff) is complete** (tag `m4`, 1 714+ tests green). **Phase 5 / M5 (PDF-native
loop + trust: C12 annotation round-trip, PDF review session + opt-in pdf.js text layer, C13
signature integrity + PAdES-B-B sign-later, launch prep only) is complete** (tag `m5`,
1 807 lib tests green; a post-acceptance correction was applied).
**Phase 6 / M6 (context budget C14 + COM/formula/revisions intake) shipped 2026-09-23**
(35/35 steps, `phase6_gate.sh --ci` green pre-tag, tag `m6`; `main` + `m6` **pushed**).
The M6 closeout rounds (§11–§13) are recorded in a local-only acceptance doc; Phase-7 Tracks
A′/B/C/D are complete (P7-3 tranches C/D landed 2026-10-15) except the owner-timed launch
(P7-9). A local-only remaining-work doc (rev 6) holds the task map; Tracks E/F/G completed
2026-10-15 and Track H was shelved (D0 Path B), so only owner launch items remain. Track-H
work (generic Office handling; H0 adoption declined 2026-10-01) is preserved on branch
`track-h-proposed`; `main` is clean.
See [`docs/support-matrix.md`](docs/support-matrix.md).

## Doc map

| Path | What it is |
|---|---|
| [`project_concept.md`](project_concept.md) | concept + decision record (D1–D29; §16 decision log is source of truth) |
| [`CODEBASE.md`](CODEBASE.md) | Rust crate structure, dependency DAG, module details, test coverage summary |
| [`docs/creation-measurement.md`](docs/creation-measurement.md) | M3 creation-path measurement matrix (C9) |
| [`docs/threat-model.md`](docs/threat-model.md) | threat model + SEC-06 review checklists (M1–M3) |
| [`docs/release-playbook.md`](docs/release-playbook.md) | release / verify / rollback procedure (DIS-08) |
| [`docs/contracts/C14-context-budget.md`](docs/contracts/C14-context-budget.md) | C14 contract (draft → freeze at M6) + [`docs/implementation-guide-context-budget.md`](docs/implementation-guide-context-budget.md) |
| [`docs/gov/`](docs/gov/) | name hygiene: clearance memo, registration checklist, distro collisions |
| [`docs/adr/`](docs/adr/) | architecture decision records (ADR-0001: PDF stack, accepted) |
| [`harness/`](harness/) | M0 Python fidelity harness (dev-only, never shipped) |
| [`corpus/`](corpus/) | fixture corpus: manifests, blobs |

Contributing: [`CONTRIBUTING.md`](CONTRIBUTING.md) (dual license, SPDX headers, DCO, harness setup).

## License

Dual-licensed **MIT OR Apache-2.0** ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)),
unblocked by G1 (ADR-0001 accepted 2026-09-08, D19).

## Visibility

The repository is currently **private** (`github.com/reinartz/stichos`, D18); public launch is a
tracked decision.
