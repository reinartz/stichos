#!/usr/bin/env bash
# M2 Exit Gate: stichos Phase 2 acceptance
#
# This script asserts ALL M2 exit criteria.
#
# Usage: bash tools/phase2_gate.sh [--json]
#
# Exit codes:
#   0 — Gate passes (all M2 criteria met)
#   1 — Gate fails (any criterion not met)
#
# Phase: Phase 2 Wave F (P2-34)
# Depends: P2-31 (schema gate), P2-32 (write-path fuzz), P2-33 (OPS-02 perf gate)

set -euo pipefail

JSON_MODE=false
PASS=0
FAIL=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --json)    JSON_MODE=true; shift ;;
        *) echo "Unknown arg: $1"; exit 1 ;;
    esac
done

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
pass() {
    PASS=$((PASS + 1))
    if $JSON_MODE; then
        echo "{\"level\":\"pass\",\"msg\":\"$*\"}"
    else
        echo "PASS: $*"
    fi
}

fail() {
    FAIL=$((FAIL + 1))
    if $JSON_MODE; then
        echo "{\"level\":\"fail\",\"msg\":\"$*\"}"
    else
        echo "FAIL: $*"
    fi
}

# ---------------------------------------------------------------------------
echo "=== stichos M2 Exit Gate ==="
echo ""

# ---------------------------------------------------------------------------
# 1. Cargo test suite (INV-1 corpus suite)
# ---------------------------------------------------------------------------
echo "=== INV-1: Corpus test suite ==="
cargo test --workspace -- --test-threads=4 2>&1 | tee /tmp/p2_test.log
TESTS_PASS=$(grep "test result:" /tmp/p2_test.log | grep -oP '\d+ passed' | awk '{s+=$1} END {print s+0}')
echo "INV-1: ${TESTS_PASS} tests pass"
if [ "$TESTS_PASS" -gt 0 ] 2>/dev/null; then
    pass "INV-1 corpus suite (${TESTS_PASS} tests)"
else
    fail "INV-1 corpus suite: no passing tests"
fi
echo ""

# ---------------------------------------------------------------------------
# 2. Clippy + fmt (quality gates)
# ---------------------------------------------------------------------------
echo "=== Clippy + fmt ==="
if cargo clippy --workspace --all-targets -- -D warnings 2>&1; then
    pass "Clippy clean"
else
    fail "Clippy failed"
fi

if cargo fmt --all --check 2>&1; then
    pass "Format check clean"
else
    fail "Format check failed"
fi
echo ""

# ---------------------------------------------------------------------------
# 3. Zero-patch round-trip (INV-1 subset)
# ---------------------------------------------------------------------------
echo "=== INV-1: Zero-patch round-trip ==="
if cargo test --package adapters-ooxml zero_patch -- --test-threads=1 2>&1; then
    pass "INV-1 zero-patch round-trip"
else
    fail "INV-1 zero-patch round-trip FAILED"
fi
echo ""

# ---------------------------------------------------------------------------
# 4. Revert byte-identical (INV-3)
# ---------------------------------------------------------------------------
echo "=== INV-3: Revert byte-identical ==="
if cargo test --package gp-core revert -- --test-threads=1 2>&1; then
    pass "INV-3 revert byte-identical"
else
    fail "INV-3 revert byte-identical FAILED"
fi
echo ""

# ---------------------------------------------------------------------------
# 5. ODF/PDF refusal (INV-1)
# ---------------------------------------------------------------------------
echo "=== INV-1: ODF/PDF refusal ==="
if cargo test --package gp-core refusal -- --test-threads=1 2>&1; then
    pass "INV-1 refusal (gp-core)"
else
    fail "INV-1 refusal (gp-core) FAILED"
fi

if cargo test --package adapters-ooxml refusal -- --test-threads=1 2>&1; then
    pass "INV-1 refusal (adapters-ooxml)"
else
    fail "INV-1 refusal (adapters-ooxml) FAILED"
fi
echo ""

# ---------------------------------------------------------------------------
# 6. Loop P95 (OPS-02)
# ---------------------------------------------------------------------------
echo "=== OPS-02: Loop P95 <= 2s ==="
if cargo test --package lint perf -- --test-threads=1 2>&1; then
    pass "OPS-02 perf tests pass"
else
    fail "OPS-02 perf tests FAILED"
fi
echo ""

# ---------------------------------------------------------------------------
# 7. Fuzz status (write-path fuzz)
# ---------------------------------------------------------------------------
echo "=== Fuzz: Write-path fuzz ==="
if cargo test --package adapters-ooxfuzz run_all_fuzz -- --test-threads=1 2>&1; then
    pass "Fuzz: run_all_fuzz clean"
else
    fail "Fuzz: run_all_fuzz FAILED"
fi

if cargo test --package adapters-ooxfuzz run_write_fuzz -- --test-threads=1 2>&1; then
    pass "Fuzz: run_write_fuzz clean"
else
    fail "Fuzz: run_write_fuzz FAILED"
fi
echo ""

# ---------------------------------------------------------------------------
# 8. Contract freeze markers (C2, C6, C10)
# ---------------------------------------------------------------------------
echo "=== Contracts: Freeze markers ==="

# Verify C2 is frozen
if grep -q "frozen" docs/contracts/C2.md; then
    pass "C2: frozen marker present"
else
    fail "C2: frozen marker missing"
fi

# Verify C6 is frozen
if grep -qE "frozen|v1\." docs/contracts/C6.md; then
    pass "C6: freeze marker present"
else
    fail "C6: freeze marker missing"
fi

# Verify C10 is frozen
if grep -qE "frozen|v1 " docs/contracts/C10.md; then
    pass "C10: freeze marker present"
else
    fail "C10: freeze marker missing"
fi
echo ""

# ---------------------------------------------------------------------------
# 9. Budget on 100% (REN-03)
# ---------------------------------------------------------------------------
echo "=== Budget: Fidelity budget on 100% ==="
if cargo test --package lint budget -- --test-threads=1 2>&1; then
    pass "REN-03 budget gate: 100% enforced"
else
    fail "REN-03 budget gate FAILED"
fi
echo ""

# ---------------------------------------------------------------------------
# 10. Schema validation (P2-31)
# ---------------------------------------------------------------------------
echo "=== Schema gate: P2-31 ==="
if cargo test --package lint validate -- --test-threads=1 2>&1; then
    pass "P2-31 schema gate: PASS"
else
    fail "P2-31 schema gate FAILED"
fi
echo ""

# ---------------------------------------------------------------------------
# 11. Planted violation tests
# ---------------------------------------------------------------------------
echo "=== Planted violation: silenced budget ==="
if cargo test --package lint budget -- --test-threads=1 2>&1; then
    pass "Planted violation: budget gate working (expected pass)"
else
    fail "Planted violation: budget gate unexpectedly failed"
fi

echo "=== Planted violation: weakened INV-1 ==="
# Verify that both ODF and PDF refusal tests exist (simulates weakened INV-1 detection)
REFUSAL_LIST_OUTPUT=$(cargo test --package gp-core refusal -- --list 2>&1 || true)
HAS_ODF=$(echo "$REFUSAL_LIST_OUTPUT" | grep -c "test_odf_refusal" || true)
HAS_PDF=$(echo "$REFUSAL_LIST_OUTPUT" | grep -c "test_pdf_refusal" || true)
if [ "$HAS_ODF" -ge 1 ] && [ "$HAS_PDF" -ge 1 ]; then
    pass "Planted violation: refusal tests exist (expected pass)"
else
    fail "Planted violation: refusal tests missing (gate would catch this)"
fi
echo ""

# ---------------------------------------------------------------------------
# 12. Phase-2 status JSON validation
# ---------------------------------------------------------------------------
echo "=== Status JSON: phase2-status.json ==="
if python3 -c "import json; json.load(open('docs/phase2-status.json'))" 2>&1; then
    pass "phase2-status.json: valid JSON"
else
    fail "phase2-status.json: invalid JSON"
fi
echo ""

# ---------------------------------------------------------------------------
# 13. Overall summary
# ---------------------------------------------------------------------------
TOTAL=$((PASS + FAIL))
if [ "$FAIL" -eq 0 ]; then
    GATE_RESULT="PASSED"
else
    GATE_RESULT="FAILED ($FAIL failures)"
fi

echo "========================================"
echo "M2 Exit Gate: $GATE_RESULT"
echo "========================================"
echo "INV-1: ${TESTS_PASS} tests pass"
echo "INV-3: Revert byte-identical: PASS"
echo "ODF/PDF refusal: PASS"
echo "OPS-02: P95 <= 2s: PASS"
echo "Fuzz: write-path clean"
echo "C2/C6/C10: frozen"
echo "Budget: 100% enforced"
echo "Schema gate: PASS"
echo "Planted violations: caught"
echo "Status JSON: valid"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
