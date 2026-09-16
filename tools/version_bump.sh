#!/usr/bin/env bash
# version_bump.sh — Bump the unified workspace version
#
# Usage: ./tools/version_bump.sh <new-version>
#
# Reads the current version from the workspace Cargo.toml [workspace.package],
# updates it in-place, appends a changelog entry to CHANGELOG.md,
# and prints the changes (does NOT commit).
#
# Examples:
#   ./tools/version_bump.sh 0.2.0
#   ./tools/version_bump.sh 1.0.0-beta.1

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

usage() {
    echo "Usage: $0 <new-version>"
    echo ""
    echo "Bumps the unified workspace version and appends a changelog entry."
    echo ""
    echo "Arguments:"
    echo "  new-version    Semver version (e.g. 0.2.0, 1.0.0-beta.1)"
    exit 1
}

if [ $# -ne 1 ]; then
    usage
fi

NEW_VERSION="$1"

# Validate semver-ish format (relaxed: must start with digit)
case "$NEW_VERSION" in
    [0-9]*) ;;
    *)
        echo "Error: version must start with a digit (got: $NEW_VERSION)"
        exit 1
        ;;
esac

# -------------------------------------------------------------------
# 1. Read current version from workspace Cargo.toml
# -------------------------------------------------------------------
WORKSPACE_CARGO="$REPO_ROOT/Cargo.toml"

if [ ! -f "$WORKSPACE_CARGO" ]; then
    echo "Error: workspace Cargo.toml not found at $WORKSPACE_CARGO"
    exit 1
fi

CURRENT_VERSION=$(grep -E '^version = "' "$WORKSPACE_CARGO" | head -1 | sed 's/version = "//;s/"//')

if [ -z "$CURRENT_VERSION" ]; then
    echo "Error: could not read version from $WORKSPACE_CARGO"
    exit 1
fi

echo "=== Version Bump ==="
echo "Current version: $CURRENT_VERSION"
echo "New version:     $NEW_VERSION"
echo ""

# -------------------------------------------------------------------
# 2. Update workspace Cargo.toml
# -------------------------------------------------------------------
echo "--- Updating workspace Cargo.toml ---"
sed -i "s/^version = \"$CURRENT_VERSION\"/version = \"$NEW_VERSION\"/" "$WORKSPACE_CARGO"
echo "Updated workspace.package.version -> $NEW_VERSION"

# -------------------------------------------------------------------
# 3. Verify no other crates override version (they should use .workspace)
# -------------------------------------------------------------------
echo ""
echo "--- Verifying workspace version usage ---"
CRATE_COUNT=0
WORKSPACE_COUNT=0
OVERIDE_COUNT=0

for f in "$REPO_ROOT"/crates/*/Cargo.toml; do
    CRATE_COUNT=$((CRATE_COUNT + 1))
    if grep -q '^version = ' "$f"; then
        OVERIDE_COUNT=$((OVERIDE_COUNT + 1))
        echo "  WARNING: $f has its own version line (should use workspace = true)"
    else
        WORKSPACE_COUNT=$((WORKSPACE_COUNT + 1))
    fi
done

echo "  Crates using workspace version: $WORKSPACE_COUNT / $CRATE_COUNT"
if [ "$OVERIDE_COUNT" -gt 0 ]; then
    echo "  WARNING: $OVERIDE_COUNT crate(s) override version — they won't be updated"
fi

# -------------------------------------------------------------------
# 4. Generate changelog entry
# -------------------------------------------------------------------
CHANGELOG="$REPO_ROOT/CHANGELOG.md"
DATE=$(date +%Y-%m-%d)
TIMESTAMP=$(date +%Y-%m-%dT%H:%M:%S%z)

echo ""
echo "--- Generating changelog entry ---"
CHANGELOG_ENTRY="## [$NEW_VERSION] — $DATE

- Version bump: $CURRENT_VERSION → $NEW_VERSION
- Generated: $TIMESTAMP

"

if [ -f "$CHANGELOG" ]; then
    # Prepend new entry (after first line which is the title)
    FIRST_LINE=$(head -1 "$CHANGELOG")
    REST=$(tail -n +2 "$CHANGELOG")
    {
        echo "$FIRST_LINE"
        echo ""
        echo "$CHANGELOG_ENTRY"
        echo "$REST"
    } > "$CHANGELOG"
    echo "Appended entry to $CHANGELOG"
else
    {
        echo "# CHANGELOG"
        echo ""
        echo "$CHANGELOG_ENTRY"
    } > "$CHANGELOG"
    echo "Created $CHANGELOG"
fi

# -------------------------------------------------------------------
# 5. Print diff summary
# -------------------------------------------------------------------
echo ""
echo "=== Changes ==="
echo ""
echo "Modified files:"
echo "  Cargo.toml              (version $CURRENT_VERSION → $NEW_VERSION)"
echo "  CHANGELOG.md            (appended [$NEW_VERSION] entry)"
echo ""
echo "Run 'git diff' to see the full diff."
echo "This script does NOT commit or push."
