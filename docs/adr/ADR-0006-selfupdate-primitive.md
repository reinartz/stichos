# ADR-0006 — Selfupdate primitive (DIS-05, G4/S4): `stichos update` opt-in channel

| | |
|---|---|
| Status | **accepted** (2026-09-16, project owner) |
| Gate | G (release CI ×6 + selfupdate fixture proof) |
| Decide by | wave G (2026-09-16) |
| Decision owner | **project owner** |
| Blocks | P3-29 selfupdate module, P3-31 release automation, P3-32 phase3_gate.sh |
| Related | G4 (distribution), S4 (supply-chain), DIS-05, phase3-plan.md, deny.toml |

<!-- Status flow: proposed → accepted | rejected (owner only). An accepted ADR that changes a
      decision also requires the owner to record a D-row in project_concept.md §16 — the ADR alone
      does not amend the D-log. -->

## 1. Context

stichos needs a self-update mechanism as part of Wave G distribution (G4). The plan (§3, P3-29)
requires:

- An opt-in `stichos update` command — never ambient, never auto-triggered.
- GitHub Releases as the update channel (G4 decision).
- Signature verification (minisign) + SHA-256 hash verification **before** any filesystem swap.
- Safety: backup → swap → smoke test → rollback on failure.
- A local fixture channel for testing the full verify → backup → swap → smoke test → rollback
  flow without network access.

Constraints from the project:
- **D2** (zero hard external deps): the selfupdate crate must not link TLS/crypto stacks;
  network is shell-out (curl/wget), signature verification uses `minisign-verify` (pure Rust).
- **S4** (supply-chain): every binary is verified by signature BEFORE any swap.
- **G4** (distribution): GitHub Releases is the canonical update channel; a fixture channel
  (`file://`) is provided for CI/testing.

## 2. Options

### Option A: `stichos update` — shell-out curl/wget, minisign verify, backup+rollback

- Download `manifest.json` from the update channel (GitHub Releases asset or local fixture).
- Verify minisign signature on `manifest.json` using the bundled public key.
- Compute SHA-256 of the downloaded binary, compare to `manifest.json` hash.
- If both pass: create backup of current binary, atomically swap, run `stichos --version` smoke
  test, remove backup on success, restore backup on failure.
- **Pros**: Meets all safety requirements (verify-before-swap, rollback); no new linked crypto
  stack; shell-out respects engine-invariant; opt-in only.
- **Cons**: Requires `minisign-verify` crate dependency; relies on system `curl`/`wget`.

### Option B: Full Rust TLS + HTTP client (reqwest)

- Use `reqwest` for HTTP + `minisign` (not `minisign-verify`) for verification.
- **Pros**: No shell-out; integrated network stack.
- **Cons**: New large linked dependency; introduces TLS/crypto linking (violates D2 spirit);
  harder to cross-compile; `reqwest` pulls in system OpenSSL.

### Option C: Defer to external updater (e.g., `cargo install`, Homebrew)

- Rely on package managers; no `stichos update` command.
- **Pros**: No selfupdate code needed.
- **Cons**: Does not satisfy G4 (release CI ×6 + selfupdate fixture proof); no single-command
  update for users who build from source; violates P3-29 scope.

## 3. Evidence

- **researched**: `minisign-verify` crate (crates.io, 0.2.x) — pure Rust, zero dependencies,
  no C deps, MIT/Apache-2.0 licensed, suitable for D2.
- **researched**: `minisign` spec — Ed25519 signature, public key is 64 hex chars + `r6` prefix.
  Signature file format: `minisign-signature` header + base64 payload.
- **researched**: GitHub Releases API — `https://api.github.com/repos/<owner>/<repo>/releases/latest`
  returns manifest URL; assets are downloadable via `browser_download_url`.
- **researched**: Atomic file swap on Linux — `renameat2(RENAME_NOREPLACE)` / `fs::rename` is
  atomic on the same filesystem; backup must be on the same filesystem.
- **researched**: `rsign2` (crates.io `rsign2`) provides CLI-compatible `rsign generate/sign/verify`
  compatible with `minisign` format — usable for test fixture generation.

## 4. Consequences

### Option A (shell-out + minisign-verify)

- **Locks in**: `minisign-verify` crate as a gp-core dependency.
- **deny.toml**: `minisign-verify` must be **allowed** (not banned). It is a pure-Rust crypto
  verification library, not a renderer engine.
- **D-rows**: Confirms D2 (no C deps, no linked crypto — verification only); confirms S4
  (supply-chain: signature before swap); no changes to D3 (engines stay subprocess).
- **Threat mitigation**:
  - **Supply chain**: minisign Ed25519 signature verified before any filesystem change.
  - **DoS**: Network operations have timeouts (curl `--max-time` / wget `--timeout`).
  - **Tampering**: Unsigned or tampered manifest → refuse, no-op.
- **Safety**: backup → swap → smoke test → rollback is atomic per-invocation.

### Option B (reqwest)

- **Locks in**: `reqwest` (large, TLS-dependent).
- **Costs**: Violates D2 cross-compile invariant; adds system OpenSSL dependency; pulls in
  many transitive crates.

### Option C (defer)

- **Locks in**: No self-update in M3.
- **Costs**: Fails G4 gate (selfupdate fixture proof required); does not complete P3-29 scope.

## Recommendation (non-binding — the owner decides)

**Option A**: Shell-out curl/wget + `minisign-verify` + backup/rollback.

This satisfies all Wave G requirements:
- Pure-Rust verification (D2-safe, no linked crypto).
- Shell-out for network (respects engine-invariant; curl is ubiquitous).
- Signature verified BEFORE swap (S4 supply-chain).
- Opt-in only via explicit `stichos update` command.
- Local fixture channel for testable verify → rollback flow without network.
- Atomic rollback on any failure (signature, smoke test).

## Decision (owner, 2026-09-16)

**Option A adopted: Shell-out curl/wget + minisign-verify + backup/rollback.**

Follow-through:
- `deny.toml`: `minisign-verify` allowed (not banned — pure-Rust crypto verification lib)
- D-log: Confirms D2 (no C deps, no linked crypto — verification only); confirms S4 (supply-chain)
- Tasks unblocked: P3-29 (selfupdate module), P3-31 (release automation), P3-32 (phase3_gate.sh)
- Gate G: verify/rollback e2e against local fixture channel proven
