#!/bin/bash
# P5-15 verify: GOV hygiene pack (source headers + docs pass)
# Verdict from exit code (headless-safe, no /dev/tty).

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PASS=0
FAIL=0
check() {
    local label="$1"; shift
    if "$@" >/dev/null 2>&1; then
        echo "  PASS: $label"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: $label"
        FAIL=$((FAIL + 1))
    fi
}

echo "=== P5-15 verify: GOV hygiene pack ==="

# 1. SECURITY.md present
check "SECURITY.md present" test -f "$ROOT/SECURITY.md"

# 2. CODE_OF_CONDUCT.md present
check "CODE_OF_CONDUCT.md present" test -f "$ROOT/CODE_OF_CONDUCT.md"

# 3. CONTRIBUTING.md present
check "CONTRIBUTING.md present" test -f "$ROOT/CONTRIBUTING.md"

# 4. LICENSE dual files present
check "LICENSE-MIT present" test -f "$ROOT/LICENSE-MIT"
check "LICENSE-APACHE present" test -f "$ROOT/LICENSE-APACHE"

# 5. Workspace Cargo.toml declares dual license
check "Workspace Cargo.toml: MIT OR Apache-2.0" \
    grep -q 'license = "MIT OR Apache-2.0"' "$ROOT/Cargo.toml"

# 6. All crates use license.workspace = true
CRATE_COUNT=$(find "$ROOT/crates" -maxdepth 2 -name Cargo.toml | wc -l)
WORKSPACE_LICENSE=$(grep -rl 'license.workspace = true' "$ROOT/crates" | wc -l)
check "All $CRATE_COUNT crates use license.workspace = true ($WORKSPACE_LICENSE)" \
    bash -c "test \"$WORKSPACE_LICENSE\" -eq \"$CRATE_COUNT\""

# 7. Rust source files: SPDX header on first line
RUST_FILES=$(find "$ROOT/crates" -name "*.rs" | wc -l)
RUST_HEADERS=$(find "$ROOT/crates" -name "*.rs" -exec head -1 {} + 2>/dev/null | grep -c "// SPDX-License-Identifier: MIT OR Apache-2.0" || true)
check "Rust SPDX headers: $RUST_HEADERS / $RUST_FILES" \
    bash -c "test \"$RUST_HEADERS\" -eq \"$RUST_FILES\""

# 8. Python source files: SPDX header on first line
PYTHON_FILES=$(find "$ROOT/harness" -name "*.py" | wc -l)
PYTHON_HEADERS=$(find "$ROOT/harness" -name "*.py" -exec head -1 {} + 2>/dev/null | grep -c "# SPDX-License-Identifier: MIT OR Apache-2.0" || true)
check "Python SPDX headers: $PYTHON_HEADERS / $PYTHON_FILES" \
    bash -c "test \"$PYTHON_HEADERS\" -eq \"$PYTHON_FILES\""

# 9. Docs pass: no local absolute paths in shipped docs
LOCAL_PATHS=$(grep -rl '/localdata\|/home/\|/tmp/\|scratch' \
    "$ROOT/README.md" "$ROOT/CODEBASE.md" "$ROOT/docs/support-matrix.md" \
    2>/dev/null | wc -l || true)
check "No local absolute paths in shipped docs ($LOCAL_PATHS hits)" \
    bash -c "test \"$LOCAL_PATHS\" -eq 0"

echo ""
echo "=== P5-15 results: $PASS passed, $FAIL failed ==="
[ "$FAIL" -eq 0 ]
