#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────
# run_goldens.sh — Generate parity golden files for stichos inspect
#
# Phase G.2: golden/parity/ directory with per-feature golden files.
#
# Usage:  ./goldens/parity/run_goldens.sh          # regenerate in place (dev)
#         ./goldens/parity/run_goldens.sh --check  # compare against committed goldens (CI)
# Runs from the repository root (workspace root).
# Idempotent: overwrites existing golden files safely in regenerate mode.
# --check regenerates into a temp dir, diffs against the committed goldens, and exits
# non-zero on any drift (this is the CI gate).
# ──────────────────────────────────────────────────────────────────────
set -euo pipefail

# Resolve paths relative to the repository root
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# Allow an explicit binary via $STICHOS_BIN, else prefer release, then debug.
if [ -n "${STICHOS_BIN:-}" ]; then
    STICHOS="${STICHOS_BIN}"
elif [ -x "${REPO_ROOT}/target/release/stichos" ]; then
    STICHOS="${REPO_ROOT}/target/release/stichos"
elif [ -x "${REPO_ROOT}/target/debug/stichos" ]; then
    STICHOS="${REPO_ROOT}/target/debug/stichos"
else
    STICHOS="${REPO_ROOT}/target/release/stichos"
fi
CORPUS="${REPO_ROOT}/corpus/blobs"
GOLDEN_DIR="${REPO_ROOT}/goldens/parity"

# Parse mode: --check compares; default regenerates.
CHECK_MODE=0
if [ "${1:-}" = "--check" ]; then
    CHECK_MODE=1
fi

# Verify stichos binary exists
if [ ! -x "${STICHOS}" ]; then
    echo "ERROR: stichos binary not found at ${STICHOS}" >&2
    echo "  Run 'cargo build --release' (or 'cargo build') first." >&2
    exit 1
fi

BINARY_VERSION=$("${STICHOS}" --version 2>&1 || true)
echo "stichos binary: ${STICHOS} (${BINARY_VERSION})"
echo "corpus dir:     ${CORPUS}"
echo "golden dir:     ${GOLDEN_DIR}"
echo ""

# Ensure golden directory exists
mkdir -p "${GOLDEN_DIR}"

# Counter for reporting
pass=0
fail=0
total=0

# Helper: run inspect and save/compare golden file
# Usage: save_golden <golden_filename> <corpus_filename> <description> [flags...]
save_golden() {
    local golden_name="$1"
    local corpus_name="$2"
    local description="$3"
    shift 3
    local golden_path="${GOLDEN_DIR}/${golden_name}"
    local corpus_path="${CORPUS}/${corpus_name}"
    # Always include --json; it's required for machine-readable output
    local flags=("--json" "$@")

    total=$((total + 1))
    if [ "${CHECK_MODE}" -eq 1 ]; then
        echo "[${total}] Checking ${golden_name} ..."
    else
        echo "[${total}] Generating ${golden_name} ..."
    fi

    if [ ! -f "${corpus_path}" ]; then
        echo "       SKIP: corpus file not found: ${corpus_path}" >&2
        fail=$((fail + 1))
        return 0
    fi

    # In check mode write to a temp file; otherwise write straight to the golden.
    local target_path="${golden_path}"
    if [ "${CHECK_MODE}" -eq 1 ]; then
        target_path="$(mktemp)"
    fi

    # Run inspect; capture stdout to target, stderr to temp file for diagnostics
    local stderr_file
    stderr_file=$(mktemp)
    if "${STICHOS}" inspect "${corpus_path}" "${flags[@]}" > "${target_path}" 2>"${stderr_file}"; then
        local size
        size=$(wc -c < "${target_path}")
        if [ "${size}" -eq 0 ]; then
            echo "       WARN: golden output is empty (${size} bytes)" >&2
            if [ -s "${stderr_file}" ]; then
                echo "       STDERR: $(head -5 "${stderr_file}")" >&2
            fi
            fail=$((fail + 1))
        elif [ "${CHECK_MODE}" -eq 1 ]; then
            if [ ! -f "${golden_path}" ]; then
                echo "       FAIL: committed golden missing: ${golden_path}" >&2
                fail=$((fail + 1))
            elif diff -u "${golden_path}" "${target_path}" >/tmp/golden_diff.$$ 2>&1; then
                echo "       OK:   ${golden_name} matches (${size} bytes)"
                pass=$((pass + 1))
            else
                echo "       DRIFT: ${golden_name} differs from committed golden" >&2
                head -40 /tmp/golden_diff.$$ >&2 || true
                fail=$((fail + 1))
            fi
        else
            echo "       OK:   ${golden_path}  (${size} bytes)"
            pass=$((pass + 1))
        fi
    else
        echo "       FAIL: inspect returned non-zero exit code" >&2
        cat "${stderr_file}" >&2
        if [ "${CHECK_MODE}" -eq 0 ]; then
            rm -f "${golden_path}"
        fi
        fail=$((fail + 1))
    fi
    rm -f "${stderr_file}"
    if [ "${CHECK_MODE}" -eq 1 ]; then
        rm -f "${target_path}" /tmp/golden_diff.$$ 2>/dev/null || true
    fi
}

# ──────────────────────────────────────────────────────────────────────
# DOCX goldens
# ──────────────────────────────────────────────────────────────────────
save_golden "docx_styles.json" "custom-styles-docx-001.docx" "custom styles" --styles
save_golden "docx_hyperlinks.json" "headers-fields-docx-001.docx" "headers, fields, hyperlinks" --styles
save_golden "docx_media.json" "image-textwrap-docx-001.docx" "images and media" --styles --media

# ──────────────────────────────────────────────────────────────────────
# XLSX goldens
# ──────────────────────────────────────────────────────────────────────
save_golden "xlsx_styles.json" "custom-styles-xlsx-001.xlsx" "custom styles" --styles
save_golden "xlsx_conditional_formatting.json" "conditional-formatting-xlsx-001.xlsx" "conditional formatting" --styles
save_golden "xlsx_freeze_panes.json" "merged-cells-xlsx-001.xlsx" "merged cells / freeze panes" --styles
save_golden "xlsx_autofilter.json" "long-tables-xlsx-001.xlsx" "autofilter / table structures" --styles
save_golden "xlsx_tables.json" "long-tables-xlsx-002.xlsx" "ListObject tables" --styles

# ──────────────────────────────────────────────────────────────────────
# PPTX goldens
# ──────────────────────────────────────────────────────────────────────
save_golden "pptx_shape_tree.json" "smartart-drawing-pptx-001.pptx" "shape trees and SmartArt" --styles
save_golden "pptx_theme.json" "custom-styles-pptx-001.pptx" "slide layouts, masters, theme data" --styles
save_golden "pptx_notes.json" "scanned-ocr-pptx-001.pptx" "slide notes content" --styles
save_golden "pptx_charts.json" "smartart-drawing-pptx-001.pptx" "chart / drawing elements" --styles

# ──────────────────────────────────────────────────────────────────────
echo ""
echo "════════════════════════════════════════"
if [ "${CHECK_MODE}" -eq 1 ]; then
    echo "  Golden check: ${total} total, ${pass} match, ${fail} drift/skipped"
else
    echo "  Summary: ${total} total, ${pass} OK, ${fail} skipped/failed"
fi
echo "════════════════════════════════════════"

if [ "${fail}" -gt 0 ]; then
    exit 1
fi
exit 0
