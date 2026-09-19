#!/bin/bash
# P5-08: Verify vendored pdf.js hashes against HASHES.toml
# Exit 0 = all hashes match; exit 1 = mismatch or missing file
set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
VENDOR_DIR="${REPO_DIR}/assets/vendor/pdfjs"
HASH_FILE="${VENDOR_DIR}/HASHES.toml"

if [[ ! -f "$HASH_FILE" ]]; then
  echo "FAIL: HASHES.toml not found at $HASH_FILE"
  exit 1
fi

errors=0
declare -A expected
current_file=""

while IFS= read -r line; do
  if [[ "$line" =~ ^\[([a-z0-9_\.]+)\]$ ]]; then
    current_file="${BASH_REMATCH[1]}"
  elif [[ -n "$current_file" ]] && [[ "$line" =~ ^sha256\ =\ \"([a-f0-9]{64})\" ]]; then
    expected["$current_file"]="${BASH_REMATCH[1]}"
    current_file=""
  fi
done < "$HASH_FILE"

for file in "${!expected[@]}"; do
  filepath="${VENDOR_DIR}/${file}"
  if [[ ! -f "$filepath" ]]; then
    echo "FAIL: $filepath not found"
    ((errors++))
    continue
  fi
  actual=$(sha256sum "$filepath" | cut -d' ' -f1)
  if [[ "$actual" != "${expected[$file]}" ]]; then
    echo "FAIL: $file hash mismatch: expected ${expected[$file]}, got $actual"
    ((errors++))
  else
    echo "OK: $file"
  fi
done

if [[ $errors -gt 0 ]]; then
  echo "HASH_VERIFY: $errors error(s)"
  exit 1
fi

echo "HASH_VERIFY: all ${#expected[@]} hashes verified"
exit 0
