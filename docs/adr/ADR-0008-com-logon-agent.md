# ADR-0008 — COM logon-agent process model (D10, ENG-09)

| | |
|---|---|
| Status | **proposed** (2026-09-18, contributor) — flip to **accepted**/**rejected** by the **project owner** |
| Gate | P4-16 (Phase 4 Wave H) |
| Decide by | Wave H or 2026-10-01 |
| Decision owner | **project owner** |
| Blocks | P4-16 tasks (skeleton implementation, CI compile gate), future Phase 5 COM implementation |
| Related | project_concept.md §9a/D10 (MS Office automation), ENG-09 (engine abstraction), ADR-0000 (template), `crates/engines/src/ms_office.rs`, `crates/engines/src/negotiate.rs` |

<!-- Status flow: proposed → accepted | rejected (owner only). An accepted ADR that changes a
      decision also requires the owner to record a D-row in project_concept.md §16 — the ADR alone
      does not amend the D-log. -->

## 1. Context

The M4 concept (§9a/D10) describes a Windows COM worker process model for MS Office automation
as the primary renderer on Windows. This covers document conversion paths: docx→pdf (Word),
xlsx→pdf (Excel), pptx→pdf (PowerPoint).

Phase 4 delivers the design document + compile-only skeleton. Actual COM interop implementation
is deferred to Phase 5+ when a Windows host is available.

Key constraints from COM/Office:
- MS Office applications do not run as Windows services — they require an interactive logon session.
- In-process COM creates apartment threading complexity (single-threaded apartment = STA).
- Office applications must be probed at runtime (not all Windows machines have Office installed).
- The `stichos` binary must compile on Linux CI even when targeting Windows.

## 2. Options

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| **A: Out-of-process COM worker** (recommended) | Each ms-office engine instance spawns a separate `stichos-com-worker.exe` that initializes COM (`CoInitializeEx`), creates Word/Excel/PowerPoint app objects, performs conversion, and exits. | Avoids STA apartment threading; clean process isolation; matches how engines already work (subprocess model). | Slightly higher overhead per conversion (process spawn); requires worker binary distribution. |
| B: In-process COM | Link the `windows` crate into the main `stichos` binary and run COM in-process. | Lower overhead; single binary. | STA apartment threading complexity; single Office crash takes down entire process; cannot run on non-Windows at all. |
| C: DDE protocol | Use Windows DDE to communicate with existing Office instances. | No new process spawn if Office already running. | DDE is legacy/deprecated; unreliable; security concerns; requires interactive session anyway. |
| D: Skip MS Office entirely | Never implement MS Office on Windows. | Simpler codebase. | Loses fidelity on Windows; contradicts D10 concept (§9a); competitor parity gap. |

## 3. Evidence

| Item | Detail | Status |
|------|--------|--------|
| COM process model | Microsoft documentation states Office apps require interactive desktop; COM STA apartment model is complex | Researched (MS docs) |
| `stichos` engine architecture | All engines (LibreOffice, pandoc, chromium, typst) are subprocesses — never linked | Measured (source) |
| `ms-office` feature flag | Added as compile gate; module always compiles, stubs on non-Windows | Measured (implementation) |
| `check_ms_office_available()` | Already in `negotiate.rs` with `cfg(target_os = "windows")` path probe | Measured (`negotiate.rs:118-134`) |
| `windows` crate availability | Not yet a dependency; would be gated behind `ms-office` feature | Researched (Cargo.toml) |

## 4. Consequences

### Option A (out-of-process COM worker) — recommended

**What it locks in:**
- Two-binary distribution: `stichos` CLI + `stichos-com-worker.exe` on Windows.
- The `ms-office` feature flag controls whether the worker binary is bundled.
- Each conversion spawns the worker → COM init → Office automation → convert → exit.
- Honest absence: on non-Windows or without Office, `is_available = false` always.

**Costs:**
- Process spawn overhead (~100-500ms) per conversion on Windows.
- Requires worker binary distribution for Windows targets.
- Version 0 skeleton always returns false on Linux CI (by design).

**What it makes easier:**
- Future Phase 5 COM implementation is structurally isolated.
- The `windows` crate dependency is optional (behind `ms-office` feature).
- Linux CI can verify compilation via `cargo check --target x86_64-pc-windows-msvc`.

### Option B (in-process COM)
**Costs:**
- All non-Windows targets cannot compile without `cfg` gymnastics.
- STA apartment threading requires careful `CoInitializeEx` / `CoUninitialize` lifecycle management.
- Office crash crashes the entire `stichos` process.

### Option C (DDE)
**Costs:**
- DDE is deprecated by Microsoft.
- Still requires interactive logon session.

### Option D (skip MS Office)
**Costs:**
- Violates D10 concept (§9a) which explicitly calls out MS Office as the primary Windows renderer.
- Creates a permanent fidelity gap on Windows.

## Recommendation (non-binding — the owner decides)

**Recommend Option A: out-of-process COM worker.**

This matches the existing `stichos` engine architecture (all engines are subprocesses, never linked),
avoids COM apartment threading complexity, and provides a clean path for Phase 5 implementation.
The compile-only skeleton in Session 1 ensures CI can verify the types and trait bounds are correct
before any runtime code lands.

## Decision (owner, <date>)

<!-- Owner-only section: the decision as taken, and its follow-through — D-log entry (§16),
     tasks unblocked, status flip in the header table and docs/adr/README.md. -->
