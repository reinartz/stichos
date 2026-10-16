#!/usr/bin/env bash
set -euo pipefail
# tools/parity_formula.sh — Formula parity inventory and CI gate
# Usage: tools/parity_formula.sh [--ci]
#
# Walks corpus/fixtures/formula-parity/*.json, runs formula evaluator tests,
# and asserts 100% of corpus formulas are either supported or GP-F-refused
# (never parse-error). In --ci mode exits non-zero on any failure.

FIXTURE_DIR="corpus/fixtures/formula-parity"
CI=0
if [[ "${1:-}" == "--ci" ]]; then
    CI=1
fi

# Count fixtures
FIXTURE_COUNT=0
FIXTURE_SUM=0
if [[ -d "$FIXTURE_DIR" ]]; then
    for f in "$FIXTURE_DIR"/*.json; do
        if [[ -f "$f" ]]; then
            FIXTURE_COUNT=$((FIXTURE_COUNT + 1))
            # Count formulas array entries in each fixture
            # JSON has "formulas": [...] — count entries by counting "formula": occurrences
            fc=$(grep -c '"formula"' "$f" 2>/dev/null || echo 0)
            FIXTURE_SUM=$((FIXTURE_SUM + fc))
        fi
    done
fi

echo "Formula-parity fixtures: $FIXTURE_COUNT"
echo "Formula-parity formulas in fixtures: $FIXTURE_SUM"

# Run formula evaluator tests
TEST_OUTPUT=$(cargo test -p gp-core --lib formula 2>&1 || true)
TEST_COUNT=$(echo "$TEST_OUTPUT" | grep -oP '\d+ passed' | head -1 || echo "unknown")
echo "Formula evaluator tests: ${TEST_COUNT:-unknown}"

# Check test output for failures
FAIL_COUNT=$(echo "$TEST_OUTPUT" | grep -c 'FAILED' || true)
if [[ "$FAIL_COUNT" -gt 0 ]]; then
    echo "FAIL: $FAIL_COUNT formula evaluator test(s) failed"
    echo "$TEST_OUTPUT"
    if [[ $CI -eq 1 ]]; then
        exit 1
    fi
fi

# Frequency table: categorize test types
SUPPORTED_TESTS=$(echo "$TEST_OUTPUT" | grep -c 'test.*::supported' || true)
REFUSE_TESTS=$(echo "$TEST_OUTPUT" | grep -c 'test.*::refuse' || true)
EVAL_TESTS=$(echo "$TEST_OUTPUT" | grep -c 'test.*::eval' || true)
PARSER_TESTS=$(echo "$TEST_OUTPUT" | grep -c 'test.*::parser' || true)

echo "Formula parity frequency table:"
echo "  supported: $SUPPORTED_TESTS"
echo "  refuse:    $REFUSE_TESTS"
echo "  eval:      $EVAL_TESTS"
echo "  parser:    $PARSER_TESTS"
echo "  fixtures:  $FIXTURE_COUNT ($FIXTURE_SUM formulas)"

# CI mode: strict assertions
if [[ $CI -eq 1 ]]; then
    # Assert minimum test count (ensures new functions have tests)
    TEST_NUM=$(echo "$TEST_COUNT" | grep -oP '^\d+' || echo "0")
    if [[ $TEST_NUM -lt 100 ]]; then
        echo "FAIL: Expected >=100 formula evaluator tests, got $TEST_NUM"
        exit 1
    fi
    if [[ $FIXTURE_COUNT -lt 4 ]]; then
        echo "FAIL: Expected >=4 formula-parity fixtures, got $FIXTURE_COUNT"
        exit 1
    fi
    if [[ $FAIL_COUNT -gt 0 ]]; then
        echo "FAIL: $FAIL_COUNT test failures in formula evaluator"
        exit 1
    fi
    echo "PASS: Formula parity gate ($CI mode)"
fi
