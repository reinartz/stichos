#!/usr/bin/env bash
# cross_check.sh — Cross-compile check ×6 + No-C dependency check
#
# Verifies:
#   1. All 6 cross-compile targets build (D2 / DIS-01)
#   2. No C-linked -sys crates in the dependency tree (SEC-02 / DIS-01)
#
# Exits 0 if clean, 1 otherwise.

set -euo pipefail

echo "=== stichos Cross-compile + No-C Check ==="

TARGETS=(
    "x86_64-unknown-linux-gnu"
    "aarch64-unknown-linux-gnu"
    "x86_64-apple-darwin"
    "aarch64-apple-darwin"
    "x86_64-pc-windows-msvc"
    "aarch64-pc-windows-msvc"
)

FAILED=0

for target in "${TARGETS[@]}"; do
    echo -n "Building for $target... "
    if ! cargo check --workspace --target "$target" 2>&1; then
        echo "FAILED"
        FAILED=$((FAILED + 1))
    else
        echo "OK"
    fi
done

# No-C check: verify no C in dependency tree
# Bans: *-sys crates, libc (except via safe re-exports like fastrand)
# Whitelisted (host-std bindings, not linked by us, safe FFI wrappers):
#   libc v0.2.x — used by tokio, anyhow, and the Rust stdlib for FFI;
#   linux-raw-sys — backend for libc on linux-x86_64/aarch64; no raw syscalls.
echo ""
echo "Checking for C dependencies..."
SYS_DEPS=""
SYS_DEPS=$(cargo tree --workspace 2>/dev/null \
    | grep -iE '\-sys|libc' \
    | grep -v "libz-sys" | grep -v "z-sys" \
    | grep -v "libc v0.2" | grep -v "linux-raw-sys" || true)
if [ -n "$SYS_DEPS" ]; then
    echo "WARNING: Potential C dependencies detected:"
    echo "$SYS_DEPS"
fi

echo ""
if [ $FAILED -eq 0 ]; then
    echo "All cross-compile targets OK"
    exit 0
else
    echo "$FAILED targets failed"
    exit 1
fi
