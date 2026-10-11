#!/bin/bash
# P5-01 verify: surface counts measured + decision rows + ADR reserve + OQ3 closed
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

echo "=== P5-01 verify: intake hygiene ==="

# 1. MCP tool count from code
MCP_COUNT=$(grep -P 'assert_eq!\s*\(\s*TOOLS\.len\(\)' "$ROOT/crates/mcp/src/spec.rs" | grep -oP '\d+' | tail -1)
check "MCP spec asserts TOOLS.len() == $MCP_COUNT" bash -c "test \"$MCP_COUNT\" = \"24\""

# 2. CLI verb count from Command match arms
VERB_MATCH=$(grep -c 'Command::' "$ROOT/crates/cli/src/main.rs" || true)
check "CLI Command match arms: $VERB_MATCH (expect ≥22)" bash -c "test \"$VERB_MATCH\" -ge 22"

# 3. Decision rows D23–D26 present
check "D23 in project_concept.md §16" grep -q 'D23' "$ROOT/project_concept.md"
check "D24 in project_concept.md §16" grep -q 'D24' "$ROOT/project_concept.md"
check "D25 in project_concept.md §16" grep -q 'D25' "$ROOT/project_concept.md"
check "D26 in project_concept.md §16" grep -q 'D26' "$ROOT/project_concept.md"

# 4. ADR-0009/0010 reserved
check "ADR-0009 reserved in adr/README.md" grep -q 'ADR-0009' "$ROOT/docs/adr/README.md"
check "ADR-0010 reserved in adr/README.md" grep -q 'ADR-0010' "$ROOT/docs/adr/README.md"

# 5. OQ3 closed
check "OQ3 closed (gp alias: no)" grep -q 'OQ3' "$ROOT/project_concept.md"

# 6. Doc counts corrected
check "support-matrix.md has '24 MCP tools'" bash -c "grep -q '24 MCP tools' '$ROOT/docs/support-matrix.md'"
check "support-matrix.md does not say '19 tools'" bash -c "! grep -q '19 tools' '$ROOT/docs/support-matrix.md'"
check "CODEBASE.md has '24 specs'" bash -c "grep -q '24 specs' '$ROOT/CODEBASE.md'"
check "CODEBASE.md does not say '19 specs'" bash -c "! grep -q '19 specs' '$ROOT/CODEBASE.md'"
check "m4-acceptance.md corrected to 24 tools" bash -c "grep -q '24 MCP tools' '$ROOT/docs/m4-acceptance.md'"

echo ""
echo "=== P5-01 results: $PASS passed, $FAIL failed ==="
[ "$FAIL" -eq 0 ]