# P7-9 Launch — Status Note

**Date:** 2026-09-24  
**Owner:** 👤 (not delegable — per D26 + P7-9 definition in `docs/remaining-work.md`)  
**Related decisions:** D18 (registry publishes), D26 (visibility)  
**Checklist:** `docs/release-playbook.md` (owner-gated, full checklist)

---

## Task description

D18/D26 flip: **registry publishes, visibility.**

- **D18** — GitHub presence established as private repo under owner's personal account (`reinartz/stichos`); no org. MCP namespace `io.github.reinartz/stichos`.
- **D26** — Launch is **owner-timed**; M5 shipped prep only; the D18/D26 flip is one-way (registry registration, publish, visibility actions).
- **P7-9** in `docs/remaining-work.md` references the owner-gated release checklist in `docs/release-playbook.md`.

---

## Current state

| Item | Status |
|------|--------|
| Phases M0–M6 | All shipped and tagged (`m1-closeout` through `m6`) |
| `phase6_gate.sh --ci` | Green (86/88 — expected `m6`-tag-exists artifact) |
| `check-deps.sh` | Green (Wave 0 DAG edge removed) |
| `main` branch | 8 commits, **local only — not pushed** |
| `m6` tag | **local only — not pushed** |
| Version | `0.1.0` (workspace `Cargo.toml`) |
| Track A (M6 closeout) | **Incomplete** — T-A (C7 v1.6 freeze), T-B (C14 envelope on 100% of responses), T-C (gate hardening), T-D (commit/re-tag/push) not yet done |
| Track B (Phase-7 backlog) | Pending — P7-9 depends on Track A |

---

## Owner-gated checklist reference

The full owner-gated launch checklist is in **`docs/release-playbook.md`**. This status note references the key steps:

1. **Pre-flight checks** — `git status`, `cargo test --workspace`, `cargo clippy`, `cargo fmt --check`
2. **Version bump** — `./tools/version_bump.sh <new-version>` (updates workspace version, all crates, CHANGELOG)
3. **Generate changelog** — `./tools/changelog_gen.sh`
4. **Commit and tag** — commit version bump, create annotated tag `v*`, push
5. **CI builds** — push `v*` tag triggers GitHub Actions Release workflow (6 targets)
6. **Verify artifacts** — confirm 6 platform binaries on GitHub Release draft
7. **Publish** — un-draft the release on GitHub
8. **Update Winget** — submit manifest to `winget-pkgs`
9. **Update Homebrew** — submit formula to `homebrew-core`
10. **Rollback plan** — documented in playbook §6

---

## Prerequisites

**Track A closeout must be complete before P7-9 can proceed:**

| Step | Description | Size | Gate |
|------|-------------|------|------|
| **T-A** | C7 v1.6 freeze consistency — update `spec.rs` ceilings, `C7.md`, `C14.md` | S+M+S | Green `cargo test -p mcp --lib` |
| **T-B** | C14 envelope wiring — `__budget` + `rendered_by` on 100% of CLI + MCP responses | L+L+M+S | Envelope smoke script passes |
| **T-C** | Gate hardening — fix toothless `phase6_gate.sh` checks (1b/1d/1e/1f) | M+M+S | `--planted-violation` mode proves detection |
| **T-D1** | Re-verify full tree — build, test, clippy, fmt, deps, gate | M | All green |
| **T-D2** | **Commit + re-anchor `m6` tag** — human-gated, tag moves to gate-green tree 👤 | S | Owner executes |
| **T-D3** | **Push** `main` + `m6` to origin — human-gated, D26 approval 👤 | S | Owner executes |

Track-A execution order: T-A1 → T-C3 → T-B1 → T-B3/T-B2 (parallel) → T-A2/T-A3 → T-C1/C2 → T-B4 → T-E → T-D1 → T-D2 → T-D3.

---

## Pre-launch items

Once Track A is complete and `main` + `m6` are pushed:

### 1. Push `main` and `m6` to origin

- **T-D3** executes `git push origin main` + `git push origin m6`
- Verify via `git ls-remote origin`

### 2. Registry publish preparation

- **crates.io** — The name `stichos` is free on crates.io (confirmed in D18 measurement, `project_concept.md` §12). Register the org/package before first publish.
- **Package name** — `stichos` (no `@scope/` prefix needed, no `-rs` suffix).
- **Publish command** — `cargo publish -p <crate>` for each crate (`gp-core`, `adapters-ooxml`, `adapters-odf`, `adapters-pdf`, `engines`, `lint`, `mcp`, `cli`) after version bump.
- **Yanked releases** — If needed, `cargo yank <crate> --vers <version>`.

### 3. Visibility actions

- **README** — Ensure `README.md` is present at repo root with clear description, install instructions, and `stichos doctor` getting-started flow.
- **Install instructions** — Cross-platform: `curl` tarball install (Linux), Homebrew (`brew install`), Winget (`winget install stichos.stichos`), GitHub Releases page.
- **Changelog** — `CHANGELOG.md` should summarize M0–M6 shipped features (or be generated via `./tools/changelog_gen.sh` at release time).
- **Docs** — `docs/` contents (phase plans, contracts, acceptance records) are public on GitHub. Consider a top-level `CONTRIBUTING.md` for future contributors.
- **Domain** — `stichos.dev` is free and unregistered; `stichos.com` is parked (D20 decision: skip `.com`, no homepage planned).
- **Social/brand** — Assets in `assets/`; mark + wordmark per `brand.md`; three-tier icons per `build-icons.sh`.

---

## Status

**UNBLOCKED — awaiting Track A closeout (T-D1 through T-D3).**

P7-9 is a single-owner action item. Once Track A completes (T-D3 pushes `main` + `m6`), the owner executes the release playbook (`docs/release-playbook.md`) in full:

```
Track A T-D3 complete
    → Release playbook §1 (pre-flight + version bump)
    → Release playbook §2 (verify artifacts)
    → Release playbook §3 (publish)
    → Release playbook §4 (Winget)
    → Release playbook §5 (Homebrew)
    → D18/D26 flip complete: stichos is live
```

---

## Dependencies

| Dependency | Status | Blocks P7-9? |
|------------|--------|-------------|
| **T-D2** (tag re-anchor) | Not done | Yes — tag must point to gate-green tree |
| **T-D3** (push `main` + `m6`) | Not done | Yes — owner-gated, D26 approval required |
| Track A T-A/T-B/T-C | Not done | Yes — must be gate-green before release tag |
| T-E (dependabot triage, housekeeping) | Not done | No — can be deferred to post-launch |
| D18 (registry squatting) | Measured; `stichos` free on crates.io/npm/PyPI | No — already satisfied |
| D26 (launch timing) | Owner decision pending | No — by definition, owner decides when to flip |
| Domain `stichos.dev` | Free (RDAP 404) | No — optional, not required for launch |

---

## Summary

P7-9 is the **owner-timed launch flip**: push `main` + `m6`, publish to crates.io, register the GitHub Release, and submit to Winget/Homebrew. The complete step-by-step checklist is in `docs/release-playbook.md`. The only hard blockers are Track A closeout (T-D2 tag re-anchor and T-D3 push), both of which require owner action.

**Status: UNBLOCKED — awaiting Track A closeout.**
