# Contributing to stichos

stichos is dual-licensed **MIT OR Apache-2.0** ([LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE)); the choice was unblocked by G1 (ADR-0001 accepted 2026-09-08,
concept D19). Contributing means you license your contribution under both grants. No CLA — DCO only.

## License headers (SPDX)

Every source file carries the SPDX identifier as its **first line**:

```
// SPDX-License-Identifier: MIT OR Apache-2.0    # Rust and other C-style files
# SPDX-License-Identifier: MIT OR Apache-2.0     # Python (harness/)
```

The `harness/` Python is dev-only fidelity tooling — never shipped. The product is Rust with zero
hard external deps (concept D3/D8); engines are spawned subprocesses, never linked. Dev-only Python
deps are still license-gated (allow-list MIT/Apache-2.0/BSD/ISC; `odfpy` deliberately excluded —
GPLv2, see `harness/requirements-dev.txt`).

## DCO sign-off

Every commit must be signed: `git commit -s` appends a `Signed-off-by: Name <email>` line.

The DCO (Developer Certificate of Origin) is a per-commit assertion: you wrote the change or
otherwise have the right to submit it under the project's licenses, and you accept that the
contribution and your sign-off are recorded publicly and kept indefinitely. Unsigned commits are
not merged.

## Harness dev env (M0)

Python 3.12+ (CI pins 3.12), from the repo root:

```
pip install -r harness/requirements-dev.txt
python -m pytest harness/metrics/tests -q
```

The pins in `harness/requirements-dev.txt` are load-bearing. `pillow`, `numpy`, `lxml` (and
`et_xmlfile`) are fixture-critical: they determine the XML bytes and PNG encodings that fixture
sha256 values depend on. Do not bump them casually — two of the 2026-09-08 nightly breaks were
exactly this class of bug (measured, see the CI bug-fix log in
[`docs/phase0-execution.md`](docs/phase0-execution.md)).

## ADRs

Write an ADR when you add a new external crate (plan §4: "ADR required for any new external crate")
or when a decision or gate outcome changes. Copy
[`docs/adr/ADR-0000-template.md`](docs/adr/ADR-0000-template.md), take the next number, status
`proposed`. Only the project owner flips a status to `accepted` or `rejected`. Index:
[`docs/adr/README.md`](docs/adr/README.md).

## Decision-log discipline (concept §16)

The D-log in `project_concept.md` §16 is owned by the project owner. Contributors never edit D-rows;
`project_concept.md` and `project_plan.md` are not edited by contributors or subagents. If a decision
should change: open an issue, or draft an ADR with evidence — the owner records the outcome as a
D-row. Proof, not edits: mark claims measured (command + result) vs researched (source) vs
unverified.

## Contracts

Versioned interfaces between tracks: registry in [`project_plan.md`](project_plan.md) §4, specs in
[`docs/contracts/`](docs/contracts/) (C9 — corpus manifest + metric definitions — is live today).
Contract changes go through the same ADR + owner path; a contract freeze gates the work graph.

## Naming

The name is always lowercase `stichos`. Never "Stichos", never bold the wordmark.

## Definition of done

Per-task definition of done: [`project_plan.md`](project_plan.md) §8 — tests green, dependency and
license gates clean, contracts/diagnostics respected, docs updated, corpus-subset CI green for
anything touching rendering or edits.
