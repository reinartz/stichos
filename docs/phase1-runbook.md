# M1 Acceptance Runbook

## Pre-flight
1. `cargo build --workspace` — must compile
2. `cargo test --workspace` — 0 new failures
3. `cargo clippy --workspace -- -D warnings` — clean
4. `cargo fmt --all --check` — clean

## Gate Execution
```bash
bash tools/phase1_gate.sh
```

## Owner-Bound Items
- macOS notarization: requires Apple Developer account
- Windows Authenticode signing: requires code-signing certificate
- Registry registration: stichos on crates.io, npm, PyPI (verified free)

## Rollout
1. Push tag `v0.1.0-m1`
2. CI builds 6 targets + packages
3. Upload to GitHub Releases
4. Submit winget + Homebrew manifests
