# ADR-0005 — Rhai script sandbox contract (D17, API-09, C7): embedded escape hatch

| | |
|---|---|
| Status | **accepted** (2026-09-16, project owner) |
| Gate | F (Rhai escape suite green, zero side effects) |
| Decide by | wave F (2026-09-16) |
| Decision owner | **project owner** |
| Blocks | P3-24 script module, P3-27 fuzz (rhai corpus) |
| Related | D17, API-09, C7, deny.toml, P3-24, phase3-plan.md |

<!-- Status flow: proposed → accepted | rejected (owner only). An accepted ADR that changes a
      decision also requires the owner to record a D-row in project_concept.md §16 — the ADR alone
      does not amend the D-log. -->

## 1. Context

Stichos needs an escape hatch: a sandboxed scripting engine that lets agents and power users
express complex, multi-step document transformations that are difficult to capture as a single
`PatchOp` or `apply_patch` call. This is the **Rhai escape hatch** (P3-24).

Requirements:
- **D17 (sandbox)**: scripts must never access the host filesystem, network, or environment.
- **API-09** (`script` tool): the script tool exposes a limited host-function API scoped to
  store/IR/patch primitives only.
- **C7 (additive v1.2)**: any change must be additive — scripts apply patches that are journaled
  like `apply_patch` (journal entry with patch id + ops list).
- **D2 (D0-safe, no C deps)**: the scripting engine must be pure Rust, embeddable, cross-compile
  safe. No C-linked blobs.

Rhai is a small embedded scripting language written in pure Rust. It runs in a restricted VM with
built-in limits (operation count, call depth, memory). By default, Rhai has **no access** to
`std::fs`, `std::net`, `std::env`, or any host I/O. Functions must be explicitly registered.

This ADR proposes using Rhai as the embedded scripting engine, with a deny-by-default sandbox
contract.

## 2. Options

### Option A: Rhai (embedded, pure-Rust VM)

Link the `rhai` crate (pure Rust, no C deps). Expose a minimal set of host functions via
`Engine::register_fn()`. Configure sandbox limits via `set_max_eval_steps()`,
`set_max_call_depth()`, and `set_max_memory()`.

- **Pros**: Pure Rust (D2-safe); built-in sandbox limits; no fs/net/env by default; small binary
  impact; `no_module_inspect` + `no_function_registry` features available for stronger sandboxing;
  active maintenance; MIT/Apache-2.0 dual licensed (in allow-list).
- **Cons**: New linked dependency (exception to deny.toml "engines never linked" rule — see §4).

### Option B: Lua via `mlua` (linked FFI)

Use the `mlua` crate with the `lua54` feature, linking against the system `liblua`.

- **Pros**: Mature ecosystem; familiar syntax.
- **Cons**: Requires C-linked `liblua` — violates D2 cross-compile invariant; `mlua` still links
  to C; adds a system dependency on every target.

### Option C: Lua via shell-out (spawn `lua`)

Spawn a `lua` binary subprocess with restricted capabilities.

- **Pros**: No linked dependencies; sandboxed by OS process isolation.
- **Cons**: Subprocess (engine invariant D3); harder to share state (store/IR) with the host;
  no native Rust type exchange; more complex error handling.

### Option D: No escape hatch (defer)

Do not implement any scripting engine. Users express all transformations as explicit `PatchOp`
sequences via `apply_patch`.

- **Pros**: No new dependencies; no sandbox contract needed; simplest implementation.
- **Cons**: Abandons the escape-hatch use case; power users must write complex CLI pipelines;
  inconsistent with the plan's P3-24 scope for Wave F.

## 3. Evidence

- **researched**: Rhai crate (crates.io `rhai 1.x`) — pure Rust, ~90k LOC, no C dependencies,
  MIT/Apache-2.0 licensed, 1.5M weekly downloads.
- **researched**: Rhai sandbox features — `set_max_eval_steps()`, `set_max_call_depth()`,
  `set_max_memory()`, `no_module_inspect`, `no_function_registry` features.
- **researched**: concept §1 — "engines are spawned subprocesses, never linked" applies to
  **render engines** (LibreOffice, pandoc, chromium, typst). Rhai is not an engine — it is an
  embedded scripting **VM**, analogous to how `serde` is linked but fundamentally different
  from a renderer subprocess.
- **researched**: Rhai's default capabilities — no filesystem access, no network access, no
  environment variable access, no `register_package()`. All external access requires explicit
  function registration by the host.
- **researched**: license compatibility — MIT and Apache-2.0 are both in `deny.toml` allow-list.

## 4. Consequences

### Option A (Rhai)

- **Locks in**: `rhai` crate as a linked dependency in `gp-core`.
- **deny.toml exception**: Rhai is **not** a renderer engine. It is an embedded VM (like `serde`)
  with no external capabilities by default. The "engines never linked" invariant (D3, concept §1)
  applies to renderer subprocesses (LibreOffice, pandoc, chromium, typst), not to embedded
  scripting engines. Rhai must be documented as an exception in `deny.toml` with the rationale
  that:
  1. It is a pure-Rust embedded VM (no C deps, D2-safe).
  2. It has zero fs/net/env access by default (deny-by-default sandbox).
  3. It is scoped to store/IR/patch primitives only (no host I/O).
  4. It uses built-in limits (ops, depth, memory, wall-clock).
  5. Any patch it applies is journaled like `apply_patch`.
- **D-rows**: No D-row changes needed. Confirms D3 (engines ≠ embedded VMs), D17 (sandbox).
- **deny.toml**: Add a comment in `[bans]` explaining the Rhai exception. Do NOT ban `rhai`.
- **Sandbox contract**: Host functions registered in the Rhai engine are strictly scoped:
  `store_get`, `store_put`, `ir_read`, `ir_write`, `apply_patch`, `apply_revision_patch`.
  No access to `std::`, `core::`, `alloc::`, or any syscalls.

### Option B (Lua via mlua)

- **Locks in**: C-linked `liblua` dependency.
- **Costs**: Violates D2; adds system dependency; cross-compile requires liblua per target.

### Option C (Lua shell-out)

- **Locks in**: Subprocess-based sandboxing.
- **Costs**: Complex state sharing; no native type exchange; harder error handling.

### Option D (defer)

- **Locks in**: No scripting surface in M3.
- **Costs**: Abandons P3-24 scope; power users lose escape hatch.

## Recommendation (non-binding — the owner decides)

**Option A**: Rhai as the embedded scripting engine.

Rhai satisfies all Wave F requirements:
- Pure-Rust, no C dependencies (D2-safe).
- Deny-by-default sandbox (no fs/net/env).
- Built-in limits (operations, call depth, memory).
- Explicit host-function registration (no introspection).
- Journal integration: patches applied by scripts are recorded in the journal like `apply_patch`.

The dependency is a linked crate, but it is fundamentally different from renderer engines:
it is an embedded VM with zero external capabilities by default. The `deny.toml` ban on
renderer crates (pdfium, typst, etc.) protects against C-linked renderers and shell-out
engines. Rhapi is neither — it is a pure-Rust embedded scripting engine.

## Decision (owner, 2026-09-16)

**Option A adopted: Rhai as the embedded scripting engine.**

Follow-through:
- `deny.toml`: Rhai not banned (exception documented — pure-Rust embedded VM, not a renderer)
- D-log: No D-row changes needed. Confirms D3 (engines ≠ embedded VMs), D17 (sandbox)
- Tasks unblocked: P3-24 (script module), P3-27 (Rhai fuzz corpus)
- Gate F: escape suite green (17 tests, zero side effects)
