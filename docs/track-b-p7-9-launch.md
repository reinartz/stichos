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
| `phase6_gate.sh --ci` | Green (90/90 at m6 tag time) |
| `check-deps.sh` | Green |
| `main` branch | Pushed to origin |
| `m6` tag | **Pushed to origin** (`ff80f8e`) |
| Version | `0.1.0` (workspace `Cargo.toml`) |
| Track A′ (M6 closeout round 2) | **COMPLETE** — T-A/T-B/T-C/T-D all done |
| Track C (engine completion) | **COMPLETE** |
| Track D (CI green) | **COMPLETE** |
| Track B (Phase-7 backlog) | P7-1/2/4/5/6/8 complete; P7-3 C/D deferred; **P7-9 (launch) remaining** |

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

**All prerequisites for P7-9 launch are met:**

| Step | Description | Size | Gate | Status |
|------|-------------|------|------|--------|
| **T-A** | C7 v1.6 freeze consistency — update `spec.rs` ceilings, `C7.md`, `C14.md` | S+M+S | Green `cargo test -p mcp --lib` | ✅ Done (Track A′) |
| **T-B** | C14 envelope wiring — `__budget` + `rendered_by` on 100% of CLI + MCP responses | L+L+M+S | Envelope smoke script passes | ✅ Done (Track A′) |
| **T-C** | Gate hardening — fix toothless `phase6_gate.sh` checks (1b/1d/1e/1f) | M+M+S | `--planted-violation` mode proves detection | ✅ Done (Track A′) |
| **T-D1** | Re-verify full tree — build, test, clippy, fmt, deps, gate | M | All green | ✅ Done (Track A′) |
| **T-D2** | **Commit + re-anchor `m6` tag** — human-gated, tag moves to gate-green tree 👤 | S | Owner executes | ✅ Done (Track A′) |
| **T-D3** | **Push** `main` + `m6` to origin — human-gated, D26 approval 👤 | S | Owner executes | ✅ Done (Track A′) |

Track-A execution order: T-A1 → T-C3 → T-B1 → T-B3/T-B2 (parallel) → T-A2/T-A3 → T-C1/C2 → T-B4 → T-E → T-D1 → T-D2 → T-D3.

**Other prerequisites:**

| Prerequisite | Status |
|--------------|--------|
| Track C (engine completion) | ✅ Complete |
| Track D (CI green) | ✅ Complete |
| Phase U (finalization) | ✅ Complete |
| D0 ruling (Path B chosen) | ✅ Complete |

---

## Launch items

`main` + `m6` are already pushed (T-D3 complete). Remaining launch actions:

### 1. Registry publish preparation

- **crates.io** — The name `stichos` is free on crates.io (confirmed in D18 measurement, `project_concept.md` §12). Register the org/package before first publish.
- **Package name** — `stichos` (no `@scope/` prefix needed, no `-rs` suffix).
- **Publish command** — `cargo publish -p <crate>` for each crate (`gp-core`, `adapters-ooxml`, `adapters-odf`, `adapters-pdf`, `engines`, `lint`, `mcp`, `cli`) after version bump.
- **Yanked releases** — If needed, `cargo yank <crate> --vers <version>`.

### 2. Visibility actions

- **README** — Ensure `README.md` is present at repo root with clear description, install instructions, and `stichos doctor` getting-started flow.
- **Install instructions** — Cross-platform: `curl` tarball install (Linux), Homebrew (`brew install`), Winget (`winget install stichos.stichos`), GitHub Releases page.
- **Changelog** — `CHANGELOG.md` should summarize M0–M6 shipped features (or be generated via `./tools/changelog_gen.sh` at release time).
- **Docs** — `docs/` contents (phase plans, contracts, acceptance records) are public on GitHub. Consider a top-level `CONTRIBUTING.md` for future contributors.
- **Domain** — `stichos.dev` is free and unregistered; `stichos.com` is parked (D20 decision: skip `.com`, no homepage planned).
- **Social/brand** — Assets in `assets/`; mark + wordmark per `brand.md`; three-tier icons per `build-icons.sh`.

---

## Status

**READY — all prerequisites met, pending owner action.**

P7-9 is a single-owner action item. All pre-requisites for launch are satisfied (Track A′ closeout complete, Tracks C and D complete, CI green, `main` + `m6` pushed). The owner executes the release playbook (`docs/release-playbook.md`) in full:

```
Release playbook §1 (pre-flight + version bump)
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
| **T-D2** (tag re-anchor) | ✅ Done (`m6` on `ff80f8e`) | No |
| **T-D3** (push `main` + `m6`) | ✅ Done (pushed to origin) | No |
| Track A′ T-A/T-B/T-C | ✅ Complete | No — gate-green achieved |
| Track C (engine completion) | ✅ Complete | No |
| Track D (CI green) | ✅ Complete | No |
| T-E (dependabot triage, housekeeping) | Done (pre-launch) | No — can be deferred to post-launch |
| D18 (registry squatting) | Measured; `stichos` free on crates.io/npm/PyPI | No — already satisfied |
| D26 (launch timing) | Owner decision pending | No — by definition, owner decides when to flip |
| Domain `stichos.dev` | Free (RDAP 404) | No — optional, not required for launch |

---

## Summary

P7-9 is the **owner-timed launch flip**: publish to crates.io, register the GitHub Release, and submit to Winget/Homebrew. The complete step-by-step checklist is in `docs/release-playbook.md`. All pre-requisites are met (Track A′ closeout complete, Tracks C and D complete, `main` + `m6` pushed). No blockers remain — launch is ready pending owner execution of the playbook.

**Status: READY — pending owner action.**
