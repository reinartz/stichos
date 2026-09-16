#!/usr/bin/env bash
# changelog_gen.sh — Generate a changelog section from git log since last tag
#
# Usage: ./tools/changelog_gen.sh [--since <date>] [--until <tag>]
#
# Reads git log, groups commits by area, and outputs a markdown section.
# If no tag exists, shows all commits.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$REPO_ROOT"

# Default: since last tag, or all history
SINCE_ARG=""
LAST_TAG=""
LAST_TAG=$(git describe --tags --abbrev=0 2>/dev/null || true)
if [ -n "$LAST_TAG" ]; then
    SINCE_ARG="${LAST_TAG}..HEAD"
fi

# Parse args
while [ $# -gt 0 ]; do
    case "$1" in
        --since)
            shift
            SINCE_ARG="$1..HEAD"
            ;;
        --until)
            shift
            SINCE_ARG="$1"
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 [--since <date>] [--until <tag>]"
            exit 1
            ;;
    esac
    shift
done

# Collect commits
COMMITS=""
if [ -n "$SINCE_ARG" ]; then
    COMMITS=$(git log "$SINCE_ARG" --pretty=format:"%s" --no-merges 2>/dev/null || true)
else
    COMMITS=$(git log --pretty=format:"%s" --no-merges 2>/dev/null || true)
fi

if [ -z "$COMMITS" ]; then
    echo "No commits found for the given range."
    exit 0
fi

# Classification: classify a commit line into an area.
# Uses commit message keywords and patterns.
classify() {
    local msg="$1"
    local lower
    lower=$(echo "$msg" | tr '[:upper:]' '[:lower:]')

    # CLI / MCP (cli, mcp, tool, verb, command)
    if echo "$lower" | grep -qE '(cli| mcp |^cli| tool |verb |command |subcommand |mcp)'; then
        echo "CLI"
        return
    fi

    # adapters-ooxml
    if echo "$lower" | grep -qE '(ooxml|docx|xlsx|pptx|office)'; then
        echo "adapters-ooxml"
        return
    fi

    # adapters-odf
    if echo "$lower" | grep -qE '(odf|odt|ods|odp)'; then
        echo "adapters-odf"
        return
    fi

    # adapters-pdf
    if echo "$lower" | grep -qE '\bpdf\b'; then
        echo "adapters-pdf"
        return
    fi

    # engines (chromium, typst, pandoc, libreoffice, engine)
    if echo "$lower" | grep -qE '(chromium|typst|pandoc|engine|renderer|layout)'; then
        echo "engines"
        return
    fi

    # gp-core (core, kernel, ir, patch, executor, journal, diff, policy)
    if echo "$lower" | grep -qE '(core| kernel |ir |patch |executor|journal|diff |policy|ir\.)'; then
        echo "gp-core"
        return
    fi

    # lint
    if echo "$lower" | grep -qE '\blint\b|budget|ssim|fidelity|annotat|severity'; then
        echo "lint"
        return
    fi

    # mcp (MCP, jsonrpc, transport, tool)
    if echo "$lower" | grep -qE '\bmcp\b|jsonrpc|transport|manifest '; then
        echo "mcp"
        return
    fi

    # docs
    if echo "$lower" | grep -qE '(docs?|readme|changelog|plan|status|adr|doc)'; then
        echo "docs"
        return
    fi

    # tools
    if echo "$lower" | grep -qE '(tools?|scripts?)'; then
        echo "tools"
        return
    fi

    # CI
    if echo "$lower" | grep -qE '(ci|workflow|github|action|nightly|pr\.yml)'; then
        echo "CI"
        return
    fi

    echo "other"
}

# Build per-area lists using temp files
TMPDIR_WORK=$(mktemp -d)
trap 'rm -rf "$TMPDIR_WORK"' EXIT

while IFS= read -r line; do
    [ -z "$line" ] && continue
    area=$(classify "$line")
    echo "- $line" >> "$TMPDIR_WORK/$area"
done <<< "$COMMITS"

# Print output
echo "# Changelog"
echo ""

if [ -n "$LAST_TAG" ]; then
    echo "Commits since **$LAST_TAG**:"
else
    echo "**Full history** (no tags found):"
fi
echo ""

# Print by area in order
for area in "CLI" "gp-core" "adapters-ooxml" "adapters-odf" "adapters-pdf" "engines" "lint" "mcp" "docs" "tools" "CI" "other"; do
    file="$TMPDIR_WORK/$area"
    if [ -f "$file" ]; then
        echo "## $area"
        echo ""
        sort -u "$file"
        echo ""
    fi
done
