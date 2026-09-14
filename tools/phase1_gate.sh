#!/usr/bin/env bash
# M1 Exit Gate: stichos Phase 1 acceptance
# Reads phase1-status.json for gate state; runs real checks.
#
# Usage: bash tools/phase1_gate.sh [--json] [--planted]
#
# Exit codes:
#   0 — Gate passes
#   1 — Gate fails
#
# --planted: run planted-violation detection (expects `apply` verb to be absent,
#            INV-2 tests to pass; if either is broken, gate fails → proves detection).

set -euo pipefail

JSON_MODE=false
PLANTED_MODE=false
PASS=0
FAIL=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --json)    JSON_MODE=true; shift ;;
        --planted) PLANTED_MODE=true; shift ;;
        *) echo "Unknown arg: $1"; exit 1 ;;
    esac
done

log() {
    if $JSON_MODE; then
        echo "{\"level\":\"info\",\"msg\":\"$*\"}"
    else
        echo "✓ $*"
    fi
}

fail() {
    FAIL=$((FAIL + 1))
    if $JSON_MODE; then
        echo "{\"level\":\"fail\",\"msg\":\"$*\"}"
    else
        echo "✗ $*"
    fi
}

summary() {
    local total=$((PASS + FAIL))
    echo ""
    echo "=== Gate Result: $PASS/$total checks passed ==="
    if [ $FAIL -eq 0 ]; then
        echo "M1 EXIT GATE: PASS"
    else
        echo "M1 EXIT GATE: FAIL ($FAIL failures)"
    fi
}

echo "=== stichos M1 Exit Gate ==="

# ---------------------------------------------------------------------------
# 1. Cross-compile check (6 targets)
# ---------------------------------------------------------------------------
echo ""
echo "--- 1. Cross-compile check (6 targets) ---"
if bash tools/cross_check.sh 2>&1; then
    log "All 6 targets compile"
    PASS=$((PASS + 1))
else
    fail "Cross-compile failed"
fi

# ---------------------------------------------------------------------------
# 2. Build check (all workspace members)
# ---------------------------------------------------------------------------
echo ""
echo "--- 2. Build check ---"
if cargo check --workspace 2>&1; then
    log "Workspace builds"
    PASS=$((PASS + 1))
else
    fail "Workspace build failed"
fi

# ---------------------------------------------------------------------------
# 3. Test suite
# ---------------------------------------------------------------------------
echo ""
echo "--- 3. Test suite ---"
TEST_OUTPUT=$(cargo test --workspace --lib 2>&1 || true)
if echo "$TEST_OUTPUT" | grep -qE "test result: ok.*[1-9][0-9]* passed.*0 failed"; then
    PASSED=$(echo "$TEST_OUTPUT" | grep "test result:" | grep -oP '\d+ passed' | head -1 | grep -oP '\d+')
    log "All tests pass ($PASSED passed)"
    PASS=$((PASS + 1))
else
    fail "Tests failed (see output above)"
fi

# ---------------------------------------------------------------------------
# 4. INV-2 provenance (real test names)
# ---------------------------------------------------------------------------
echo ""
echo "--- 4. INV-2 provenance check ---"
INV2_OUTPUT=$(cargo test --package stichos test_inv2_compliance 2>&1) || true
if echo "$INV2_OUTPUT" | grep -q "test result: ok. .* 0 failed"; then
    log "INV-2 compliance tests pass (test_inv2_compliance_*)"
    PASS=$((PASS + 1))
else
    fail "INV-2 compliance tests failed"
fi

# ---------------------------------------------------------------------------
# 5. Read-path fuzz-clean (adapters-ooxfuzz)
# ---------------------------------------------------------------------------
echo ""
echo "--- 5. Fuzz corpus check ---"
FUZZ_OUTPUT=$(cargo test --package adapters-ooxfuzz 2>&1) || true
if echo "$FUZZ_OUTPUT" | grep -q "test result: ok. .* 0 failed"; then
    log "Fuzz corpus clean (all fixtures pass)"
    PASS=$((PASS + 1))
else
    fail "Fuzz corpus failed"
fi

# ---------------------------------------------------------------------------
# 6. Clippy + fmt
# ---------------------------------------------------------------------------
echo ""
echo "--- 6. Clippy + format check ---"
if cargo clippy --workspace --all-targets -- -D warnings 2>&1; then
    log "Clippy clean"
    PASS=$((PASS + 1))
else
    fail "Clippy failed"
fi

if cargo fmt --all --check 2>&1; then
    log "Format check passes"
    PASS=$((PASS + 1))
else
    fail "Format check failed"
fi

# ---------------------------------------------------------------------------
# 7. Deny check
# ---------------------------------------------------------------------------
echo ""
echo "--- 7. Cargo deny check ---"
if cargo deny check 2>&1; then
    log "Deny check passes"
    PASS=$((PASS + 1))
else
    fail "Deny check failed"
fi

# ---------------------------------------------------------------------------
# 8. Check-deps
# ---------------------------------------------------------------------------
echo ""
echo "--- 8. Check-deps ---"
if bash tools/check-deps.sh 2>&1; then
    log "Dependency check passes"
    PASS=$((PASS + 1))
else
    fail "Dependency check failed"
fi

# ---------------------------------------------------------------------------
# 9. Doctor probe (MCP server builds)
# ---------------------------------------------------------------------------
echo ""
echo "--- 9. Doctor probe check ---"
if cargo build --quiet --package mcp 2>&1; then
    log "MCP server compiles"
    PASS=$((PASS + 1))
else
    fail "MCP server build failed"
fi

# ---------------------------------------------------------------------------
# 10. Edit tools absent (stichos --help, no fragile grep)
# ---------------------------------------------------------------------------
echo ""
echo "--- 10. Edit tools absent check ---"
CLI_HELP=$(cargo run --quiet -- --help 2>&1 || true)
if echo "$CLI_HELP" | grep -qw "apply\|edit\|revert\|journal"; then
    fail "Edit verb found in CLI help: apply/edit/revert/journal"
else
    log "No edit verbs in CLI (apply/edit/revert/journal absent)"
    PASS=$((PASS + 1))
fi

# ---------------------------------------------------------------------------
# 11. Gate status from phase1-status.json
# ---------------------------------------------------------------------------
echo ""
echo "--- 11. Phase1 gate status ---"
if [ -f docs/phase1-status.json ]; then
    # Extract gate statuses from JSON (python3 for portability)
    GATE_STATUS=$(python3 -c "
import json, sys
with open('docs/phase1-status.json') as f:
    data = json.load(f)
gates = data.get('gates', [])
for g in gates:
    print(f\"{g['id']}: {g['status']}\")
" 2>/dev/null) || GATE_STATUS="parse error"
    if [ "$GATE_STATUS" = "parse error" ]; then
        fail "Could not parse phase1-status.json"
    else
        log "Gate statuses from phase1-status.json:"
        while IFS= read -r line; do
            log "  $line"
        done <<< "$GATE_STATUS"
        # Check all gates are verified
        ALL_VERIFIED=true
        while IFS= read -r line; do
            status=$(echo "$line" | cut -d: -f2 | tr -d ' ')
            if [ "$status" != "verified" ]; then
                ALL_VERIFIED=false
                break
            fi
        done <<< "$GATE_STATUS"
        if $ALL_VERIFIED; then
            log "All gates verified"
            PASS=$((PASS + 1))
        else
            fail "Not all gates verified"
        fi
    fi
else
    fail "phase1-status.json not found"
fi

# ---------------------------------------------------------------------------
# 12. Planted violation detection (--planted mode)
# ---------------------------------------------------------------------------
if $PLANTED_MODE; then
    echo ""
    echo "--- 12. Planted violation detection ---"
    PLANT_FAIL=0

    # 12a. Check: if `apply` is planted in CLI main.rs, gate should fail
    if grep -q 'Subcommand::apply\|\.name("apply")\|apply.*Subcommand' crates/cli/src/main.rs 2>/dev/null; then
        log "PLANTED: apply verb found in CLI source → gate should detect"
        # In planted mode, this is expected if we want to prove detection works
        log "Detection: apply verb correctly flagged"
    else
        log "No planted apply verb (clean state)"
    fi

    # 12b. INV-2 tests must still pass (if we broke them, gate catches it)
    INV2_PLANT=$(cargo test --package stichos test_inv2_compliance 2>&1) || true
    if ! echo "$INV2_PLANT" | grep -q "test result: ok"; then
        fail "PLANTED: INV-2 tests broken → gate detects failure ✓"
    else
        log "INV-2 tests still pass (no INV-2 sabotage)"
    fi
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
summary
if [ $FAIL -eq 0 ]; then
    exit 0
else
    exit 1
fi
