#!/bin/bash
# P5-02 verify: PDF fixture corpus integrity
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

echo "=== P5-02 verify: PDF fixture corpus ==="

# 1. At least 12 PDF fixtures
MANIFEST_COUNT=$(find "$ROOT/corpus/manifests" -name "*-pdf-*.json" -o -name "pdfa-*.json" | wc -l)
check "≥12 PDF fixtures in manifests: $MANIFEST_COUNT" bash -c "test \"$MANIFEST_COUNT\" -ge 12"

# 2. All blobs present
BLOB_COUNT=$(find "$ROOT/corpus/blobs" -name "*-pdf-*.pdf" -o -name "pdfa-*.pdf" | wc -l)
check "Matching blobs: $BLOB_COUNT" bash -c "test \"$BLOB_COUNT\" -eq \"$MANIFEST_COUNT\""

# 3. All reference texts present
REF_COUNT=$(find "$ROOT/corpus/reference" -name "*-pdf-*.txt" -o -name "pdfa-*.txt" | wc -l)
check "Reference texts: $REF_COUNT" bash -c "test \"$REF_COUNT\" -eq \"$MANIFEST_COUNT\""

# 4. SHA256 integrity: every manifest's sha matches its blob
check "SHA256 integrity" bash -c 'python3 -c "
import json, hashlib, pathlib, sys
root = pathlib.Path(\"'\"$ROOT\"'\")
failed = 0
for mp in sorted((root / \"corpus\" / \"manifests\").glob(\"*-pdf-*.json\")) + sorted((root / \"corpus\" / \"manifests\").glob(\"pdfa-*.json\")):
    m = json.loads(mp.read_text())
    blob = root / \"corpus\" / \"blobs\" / (m[\"id\"] + \".pdf\")
    if not blob.exists():
        failed += 1; continue
    sha = hashlib.sha256(blob.read_bytes()).hexdigest()
    if sha != m[\"sha256\"]:
        failed += 1; print(f\"  SHA mismatch: {m[\"id\"]}\")
sys.exit(failed)
"
'

# 5. Manifests parseable by pikepdf
check "All fixtures parseable" bash -c "python3 -c \"
import pikepdf, pathlib, sys
root = pathlib.Path('$ROOT')
failed = 0
for blob in sorted((root / 'corpus' / 'blobs').glob('*-pdf-*.pdf')) + sorted((root / 'corpus' / 'blobs').glob('pdfa-*.pdf')):
    try:
        pikepdf.open(str(blob))
    except Exception as e:
        failed += 1; print(f'  Parse fail: {blob.name}: {e}')
sys.exit(failed)
\""

# 6. Feature counts reasonable
check "Highlight fixtures have annotations" bash -c "python3 -c \"
import json, sys
root = '$ROOT'
for fid in ['highlight-annot-pdf-001', 'mixed-annot-pdf-001', 'multi-highlight-pdf-001']:
    m = json.loads(open(f'{root}/corpus/manifests/{fid}.json').read())
    if m['features'].get('pdf:annotations', 0) < 1:
        print(f'  No annotations in {fid}')
        sys.exit(1)
sys.exit(0)
\""

# 7. Damaged fixture has unknown subtype
check "Damaged fixture has /Unknown subtype" bash -c 'python3 -c "
import pikepdf, json, pathlib
root = pathlib.Path(\"'\"$ROOT\"'\")
m = json.loads((root / \"corpus\" / \"manifests\" / \"damaged-annot-pdf-001.json\").read_text())
pdf = pikepdf.open(str(root / \"corpus\" / \"blobs\" / (m[\"id\"] + \".pdf\")))
if \"/Annots\" in pdf.Root:
    for a in pdf.Root[\"/Annots\"]:
        if \"/Subtype\" in a and str(a[\"/Subtype\"]) == \"/Unknown\":
            exit(0)
print(\"  No /Unknown subtype found\")
exit(1)
"
'

# 8. Signed fixture has AcroForm with Sig
check "Signed fixture has AcroForm with /Sig field" bash -c 'python3 -c "
import pikepdf, json, pathlib
root = pathlib.Path(\"'\"$ROOT\"'\")
m = json.loads((root / \"corpus\" / \"manifests\" / \"selfsigned-pdf-001.json\").read_text())
pdf = pikepdf.open(str(root / \"corpus\" / \"blobs\" / (m[\"id\"] + \".pdf\")))
if \"/AcroForm\" in pdf.Root and \"/Fields\" in pdf.Root.AcroForm:
    exit(0)
print(\"  No AcroForm with fields\")
exit(1)
"
'

echo ""
echo "=== P5-02 results: $PASS passed, $FAIL failed ==="
[ "$FAIL" -eq 0 ]
