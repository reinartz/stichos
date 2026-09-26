#!/usr/bin/env bash
# check-deps.sh — verify the dependency DAG against project policy.
#
# Checks:
#   1. No banned crates (mupdf*, lokit*, *-libreoffice*, *-officekit)
#      (pdfium* / pdfium-render allowed since G5 adoption — dynamic linking only)
#   2. No -sys crates
#   3. No wildcard (*) version specs in Cargo.toml files
#   4. Cross-crate edge enforcement (workspace deps vs project_plan.md §3.2 allow-list)
#   5. Planted violation test (injects a bad edge, verifies it's caught)
#
# Usage: bash tools/check-deps.sh [--planted-violation]
#   --planted-violation  Inject a known bad edge and verify the gate catches it
#
# Exits 0 if clean, 1 otherwise.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
ERRORS=0
PLANTED_VIOLATION=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --planted-violation)  PLANTED_VIOLATION=true; shift ;;
        *) echo "Unknown arg: $1"; exit 1 ;;
    esac
done

error() {
    echo "ERROR: $*" >&2
    ERRORS=$((ERRORS + 1))
}

warn() {
    echo "WARN: $*" >&2
}

echo "=== stichos dependency DAG check ==="
echo "Root: $ROOT_DIR"
echo ""

# 1. Find all Cargo.toml files (excluding target/)
mapfile -t CARGO_FILES < <(find "$ROOT_DIR" -name Cargo.toml -not -path '*/target/*' -not -path '*/.cargo/*' | sort)

if [ ${#CARGO_FILES[@]} -eq 0 ]; then
    error "No Cargo.toml files found"
    exit 1
fi

echo "Found ${#CARGO_FILES[@]} Cargo.toml file(s)"
echo ""

# 2. Check for banned crates
echo "--- Ban check: banned crates ---"
# pdfium* / pdfium-render bans removed 2026-09-26 by G5 adoption (dynamic linking only).
BANNED_PATTERNS=(
    "mupdf"
    "mupdf*"
    "lokit*"
    "lokit-dl"
    "*-libreoffice*"
    "*-officekit*"
)

for pattern in "${BANNED_PATTERNS[@]}"; do
    MATCHES=""
    for f in "${CARGO_FILES[@]}"; do
        m=$(grep -n "${pattern}" "$f" 2>/dev/null || true)
        if [ -n "$m" ]; then
            MATCHES="${MATCHES}  $f: $m
"
        fi
    done
    if [ -n "$MATCHES" ]; then
        error "Banned crate pattern '${pattern}' found:"
        printf "%s" "$MATCHES"
    fi
done

echo ""

# 3. Check for -sys crates
echo "--- Ban check: -sys crates ---"
SYS_MATCHES=""
for f in "${CARGO_FILES[@]}"; do
    m=$(grep -n '^[[:space:]]*[a-zA-Z0-9_-]-sys[[:space:]]*=' "$f" 2>/dev/null || true)
    if [ -n "$m" ]; then
        SYS_MATCHES="${SYS_MATCHES}  $f: $m
"
    fi
done
if [ -n "$SYS_MATCHES" ]; then
    error "Found -sys crate dependencies (C-renderer bindings must not be linked):"
    printf "%s" "$SYS_MATCHES"
fi

echo ""

# 4. Check for wildcard version specs
echo "--- Ban check: wildcard dependencies ---"
WILDCARD_MATCHES=""
for f in "${CARGO_FILES[@]}"; do
    m=$(grep -n '^[[:space:]]*[a-zA-Z_][a-zA-Z0-9_]*[[:space:]]*=[[:space:]]*"\*"' "$f" 2>/dev/null || true)
    if [ -n "$m" ]; then
        WILDCARD_MATCHES="${WILDCARD_MATCHES}  $f: $m
"
    fi
done
if [ -n "$WILDCARD_MATCHES" ]; then
    error "Found wildcard (*) version specifications:"
    printf "%s" "$WILDCARD_MATCHES"
fi

echo ""

# 6. Cross-crate edge enforcement (workspace-only deps)
#
# This section enforces the allowed dependency edges defined in project_plan.md §3.2.
# Each line is "crate: dep1,dep2,dep3" meaning "crate may depend on dep1, dep2, dep3".
# An empty allow-list (e.g., "gp-core:") means the crate may NOT depend on any workspace crate.
# "cli:" with no deps listed means cli is exempt (de-facto wiring point).
#
# Planted-violation mode: if --planted-violation is passed, inject a known bad edge
# (lint -> adapters-odf) and verify the gate catches it (exits 1).

# Allow-list of allowed workspace dependency edges (from project_plan.md §3.2)
# Format: source_crate:allowed_dep1,allowed_dep2,...
declare -A ALLOWED_DEPS
ALLOWED_DEPS[gp-core]=""
ALLOWED_DEPS[adapters-ooxml]="gp-core,lint"
ALLOWED_DEPS[adapters-odf]="gp-core"
ALLOWED_DEPS[adapters-pdf]="gp-core"
ALLOWED_DEPS[engines]="gp-core,adapters-ooxml"
ALLOWED_DEPS[lint]="gp-core"
ALLOWED_DEPS[mcp]="gp-core"
ALLOWED_DEPS[cli]="*all*"  # cli is exempt: de-facto wiring point
# Note: adapters-ooxfuzz is dev-only and not enforced

# Extract workspace deps for a single crate from its Cargo.toml
get_workspace_deps() {
    local cargo_file="$1"
    local deps=""
    local in_deps=0
    while IFS= read -r line; do
        if [[ "$line" =~ ^\[dependencies\] ]]; then
            in_deps=1
            continue
        fi
        if [[ $in_deps -eq 1 ]] && [[ "$line" =~ ^\[ ]]; then
            in_deps=0
            continue
        fi
        if [[ $in_deps -eq 1 ]] && [[ "$line" =~ ^[[:space:]]*([a-zA-Z0-9_-]+)[[:space:]]*= ]]; then
            dep_name="${BASH_REMATCH[1]}"
            # Check if this is a workspace crate (has path = "../")
            if [[ "$dep_name" == gp-core ]] || [[ "$dep_name" == adapters-* ]] || [[ "$dep_name" == engines ]] || [[ "$dep_name" == lint ]] || [[ "$dep_name" == mcp ]]; then
                if [ -n "$deps" ]; then
                    deps="${deps},${dep_name}"
                else
                    deps="${dep_name}"
                fi
            fi
        fi
    done < "$cargo_file"
    echo "$deps"
}

# Check for a planted violation (inject a bad edge and verify it's caught)
if [[ "${PLANTED_VIOLATION:-false}" == "true" ]]; then
    echo "--- Planted violation test ---"

    # Add a bad edge: lint depends on adapters-odf (should be caught)
    LINT_CARGO="${ROOT_DIR}/crates/lint/Cargo.toml"
    if [ -f "$LINT_CARGO" ]; then
        # Backup
        cp "$LINT_CARGO" "${LINT_CARGO}.bak"
        # Add the bad dependency
        echo 'adapters-odf = { path = "../adapters-odf", version = "0.1.0" }' >> "$LINT_CARGO"
    fi

    # Re-run this check — it should fail because lint -> adapters-odf is not allowed
    PLANTED_CAUGHT=false
    LINT_DEPS=$(get_workspace_deps "$LINT_CARGO" 2>/dev/null || true)
    if [[ -n "$LINT_DEPS" ]]; then
        ALLOWED="${ALLOWED_DEPS[lint]:-}"
        if [[ -n "$ALLOWED" && "$ALLOWED" != "*all*" ]]; then
            IFS=',' read -ra ALLOW_ARR <<< "$ALLOWED"
            for bad_dep in $LINT_DEPS; do
                for allowed in "${ALLOW_ARR[@]}"; do
                    if [[ "$bad_dep" == "$allowed" ]]; then
                        continue 2
                    fi
                done
                echo "  Planted violation detected: lint depends on $bad_dep (not in allow-list)"
                PLANTED_CAUGHT=true
            done
        fi
    fi

    # Restore
    if [ -f "${LINT_CARGO}.bak" ]; then
        mv "${LINT_CARGO}.bak" "$LINT_CARGO"
    fi

    if $PLANTED_CAUGHT; then
        echo "  PASS: Planted violation was caught (exit code enforcement works)"
    else
        echo "  FAIL: Planted violation was NOT caught (gate is broken)"
        ERRORS=$((ERRORS + 1))
    fi
    echo ""
    echo "=== Result: $((ERRORS == 0 ? 0 : 1)) ($ERRORS error(s)) ==="
    exit $ERRORS
fi

echo "--- Cross-crate edge enforcement ---"

# Check each workspace crate's deps against the allow-list
for cargo_file in "${CARGO_FILES[@]}"; do
    CRATE_NAME=$(basename "$(dirname "$cargo_file")")

    # Skip non-crate toml files (workspace root, etc.)
    case "$CRATE_NAME" in
        Cargo) continue ;;
    esac

    # Skip dev-only crates
    if [[ "$CRATE_NAME" == *"fuzz"* ]]; then
        continue
    fi

    ALLOWED="${ALLOWED_DEPS[$CRATE_NAME]:-}"

    # Get actual workspace deps
    ACTUAL_DEPS=$(get_workspace_deps "$cargo_file")

    if [ -z "$ACTUAL_DEPS" ]; then
        continue  # No workspace deps, nothing to check
    fi

    # Check each actual dep against the allow-list
    if [[ "$ALLOWED" == "*all*" ]]; then
        # This crate (cli) is exempt
        continue
    fi

    if [ -z "$ALLOWED" ]; then
        # Empty allow-list: crate must not depend on any workspace crate
        if [ -n "$ACTUAL_DEPS" ]; then
            error "Cross-crate edge violation: $CRATE_NAME depends on [$ACTUAL_DEPS] but has an empty allow-list"
            for dep in $(echo "$ACTUAL_DEPS" | tr ',' ' '); do
                echo "  forbidden edge: $CRATE_NAME -> $dep"
            done
        fi
    else
        IFS=',' read -ra ALLOW_ARR <<< "$ALLOWED"
        for actual_dep in $(echo "$ACTUAL_DEPS" | tr ',' ' '); do
            found=false
            for allowed in "${ALLOW_ARR[@]}"; do
                if [[ "$actual_dep" == "$allowed" ]]; then
                    found=true
                    break
                fi
            done
            if ! $found; then
                error "Cross-crate edge violation: $CRATE_NAME -> $actual_dep (not in allow-list: $ALLOWED)"
            fi
        done
    fi
done

echo ""
echo "=== Result: $((ERRORS == 0 ? 0 : 1)) ($ERRORS error(s)) ==="
exit $ERRORS
