#!/usr/bin/env bash
# check-deps.sh — verify the dependency DAG against project policy.
#
# Checks:
#   1. No banned crates (pdfium*, mupdf*, lokit*, *-libreoffice*, *-officekit*)
#   2. No -sys crates
#   3. No wildcard (*) version specs in Cargo.toml files
#
# Exits 0 if clean, 1 otherwise.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
ERRORS=0

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
BANNED_PATTERNS=(
    "pdfium*"
    "pdfium-render"
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

# 5. Print dependency DAG summary
echo "--- Dependency DAG ---"
for cargo_file in "${CARGO_FILES[@]}"; do
    CRATE_NAME=$(basename "$(dirname "$cargo_file")")
    # Extract direct dependencies (simplified: look for lines after [dependencies])
    in_deps=0
    DEPS=""
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
            if [ -n "$DEPS" ]; then
                DEPS="${DEPS},${dep_name}"
            else
                DEPS="${dep_name}"
            fi
        fi
    done < "$cargo_file"
    if [ -n "$DEPS" ]; then
        echo "  $CRATE_NAME -> $DEPS"
    fi
done

echo ""
echo "=== Result: $((ERRORS == 0 ? 0 : 1)) ($ERRORS error(s)) ==="
exit $ERRORS
