#!/usr/bin/env bash
# tools/measure_creation.sh — P3-28 creation-path corpus measurement
#
# For each creation path (pandoc→docx, pandoc→odt, chromium→pdf,
# typst→pdf, native→docx):
#   1. Take pr-subset fixtures (at least 10, up to 15)
#   2. Create output documents via the path
#   3. Render outputs to PDF (using LO) for comparison
#   4. Compare against LO-rendered golden: page-count delta, SSIM, feature drops
#
# Outputs: docs/creation-measurement.csv
#
# Prerequisites: pandoc, chromium-browser, typst, LibreOffice, stichos CLI,
#   python3 with harness/metrics (Pillow, numpy, scikit-image).
# Missing tools cause graceful skip of that path.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$ROOT"

# ---- helpers ----

csv_header="path,fixture,latency_ms,page_delta,ssim_mean,feature_drops"
CSV_OUTPUT="$ROOT/docs/creation-measurement.csv"
TMPDIR_MEASURE="$(mktemp -d)"
# Uncomment to keep temp dir for debugging:
# trap '' EXIT
trap 'rm -rf "$TMPDIR_MEASURE"' EXIT

echo "$csv_header" > "$CSV_OUTPUT"

log() { echo "[measure] $*"; }

# Check tool availability
has_tool() { command -v "$1" &>/dev/null; }

# Measure page count from a PDF directory (page-*.png files)
pdf_dir_page_count() {
    local dir="$1"
    ls "$dir"/page-*.png 2>/dev/null | wc -l
}

# Measure page count from a PDF file using pdftoppm
pdf_file_page_count() {
    local pdf="$1"
    local tmpdir="$TMPDIR_MEASURE/$(basename "$pdf").pagecount"
    mkdir -p "$tmpdir"
    pdftoppm -r 1 -gray "$pdf" "$tmpdir/p" 2>/dev/null && ls "$tmpdir"/p-*.png 2>/dev/null | wc -l || echo "0"
    rm -rf "$tmpdir"
}

# Get golden page count for a fixture
get_golden_pages() {
    local fid="$1"
    local golden_dir="$ROOT/goldens/staging/${fid}"
    if [[ -d "$golden_dir/pages" ]]; then
        pdf_dir_page_count "$golden_dir/pages"
    else
        echo "0"
    fi
}

# ---- fixture discovery ----

log "Discovering pr-subset fixtures..."

declare -a FIXTURES=()
declare -A FIXTURE_FORMAT=()
while IFS= read -r line; do
    [[ "$line" =~ ^#.*$ || -z "$line" ]] && continue
    fid="$line"
    blob="$ROOT/corpus/blobs/${fid}.docx"
    if [[ -f "$blob" ]]; then
        FIXTURES+=("$fid")
        FIXTURE_FORMAT["$fid"]="docx"
    else
        for ext in xlsx pptx odt ods odp; do
            blob="$ROOT/corpus/blobs/${fid}.${ext}"
            if [[ -f "$blob" ]]; then
                FIXTURES+=("$fid")
                FIXTURE_FORMAT["$fid"]="$ext"
                break
            fi
        done
    fi
done < "$ROOT/corpus/pr-subset.txt"

TOTAL=${#FIXTURES[@]}
log "Found $TOTAL fixtures"

# Use all fixtures (all 20 have golden renders)
SUBSET=("${FIXTURES[@]}")
SUBSET_SIZE=${#SUBSET[@]}
log "Using $SUBSET_SIZE fixtures"

# ---- helper: run one fixture through a creation path and measure ----

# Args: path_name fixture markdown_content [extra_args...]
# Sets: MEASURED_LATENCY, MEASURED_PAGE_DELTA, MEASURED_SSIM, MEASURED_DROPS
measure_fixture() {
    local path_name="$1"
    local fid="$2"
    local md_content="$3"
    local doc_path="$4"          # path to created document
    local pdf_path="$5"          # path to rendered PDF
    local golden_dir="$ROOT/goldens/staging/${fid}"
    local latency="$6"           # latency in ms (already measured by caller)

    # Get golden page count
    local golden_pages
    golden_pages=$(get_golden_pages "$fid")

    # Get candidate page count
    local candidate_pages=0
    if [[ -f "$pdf_path" ]]; then
        candidate_pages=$(pdf_file_page_count "$pdf_path")
    fi

    # Page delta
    local page_delta
    page_delta=$(cd "$ROOT/harness/metrics" && python3 -m metrics page-delta "$candidate_pages" "$golden_pages" 2>/dev/null \
        | python3 -c "import sys,json;print(json.load(sys.stdin)['page_delta'])" 2>/dev/null) || page_delta="0"

    # SSIM
    local ssim_mean="null"
    if [[ -f "$pdf_path" && -d "$golden_dir/pages" ]]; then
        local cand_pages_dir="$TMPDIR_MEASURE/${path_name}_${fid}_pages"
        mkdir -p "$cand_pages_dir"
        pdftoppm -r 150 -gray -png "$pdf_path" "$cand_pages_dir/cand" 2>/dev/null
        if ls "$cand_pages_dir"/cand-*.png &>/dev/null; then
            ssim_mean=$(cd "$ROOT/harness/metrics" && python3 -m metrics ssim "$cand_pages_dir" "$golden_dir/pages" 2>/dev/null \
                | python3 -c "import sys,json;d=json.load(sys.stdin);print(round(d.get('mean',0),4))" 2>/dev/null || echo "null") || ssim_mean="null"
        fi
    fi

    # Feature drops
    local feature_drops=0
    if [[ "$path_name" == "pandoc-docx" || "$path_name" == "pandoc-odt" || "$path_name" == "native-docx" ]]; then
        if [[ -f "$doc_path" && -d "$golden_dir" ]]; then
            # Determine the original fixture path based on format
            local orig_path=""
            for ext in docx xlsx pptx odt ods odp; do
                if [[ -f "$ROOT/corpus/blobs/${fid}.${ext}" ]]; then
                    orig_path="$ROOT/corpus/blobs/${fid}.${ext}"
                    break
                fi
            done
            if [[ -n "$orig_path" && -f "$orig_path" ]]; then
                local feat_before="$TMPDIR_MEASURE/feat_before.json"
                local feat_after="$TMPDIR_MEASURE/feat_after.json"
                # Get feature counts for original
                cd "$ROOT/harness/metrics" && python3 -m metrics feature-counts "$orig_path" > "$feat_before" 2>/dev/null
                # Get feature counts for created doc
                cd "$ROOT/harness/metrics" && python3 -m metrics feature-counts "$doc_path" > "$feat_after" 2>/dev/null
                # Compare retention
                feature_drops=$(cd "$ROOT/harness/metrics" && python3 -m metrics retention "$feat_before" "$feat_after" 2>/dev/null \
                    | python3 -c "
import sys,json
d = json.load(sys.stdin)
drops = sum(1 for v in d.get('features',{}).values() if v.get('dropped',0) > 0)
print(drops)
" 2>/dev/null) || feature_drops="0"
            fi
        fi
    fi

    # Write CSV row
    echo "${path_name},${fid},${latency},${page_delta},${ssim_mean},${feature_drops}" >> "$CSV_OUTPUT"
}

# ---- pandoc path: docx ----

run_pandoc_docx() {
    has_tool pandoc || { log "SKIP pandoc→docx: pandoc not available"; return; }
    log "Starting pandoc→docx path..."

    for fid in "${SUBSET[@]}"; do
        local ref_txt="$ROOT/corpus/reference/${fid}.txt"
        [[ -f "$ref_txt" ]] || continue

        local md_file="$TMPDIR_MEASURE/${fid}.md"
        local out_file="$TMPDIR_MEASURE/${fid}.docx"
        local pdf_file="$TMPDIR_MEASURE/${fid}.pdf"

        # Create markdown from reference text
        local title
        title=$(head -1 "$ref_txt")
        {
            echo "# $title"
            tail -n +2 "$ref_txt" | while IFS= read -r line; do
                if [[ -z "$line" ]]; then
                    echo ""
                else
                    echo "$line"
                fi
            done
        } > "$md_file"

        # Time pandoc conversion
        local t0 t1 elapsed
        t0=$(date +%s%N)
        pandoc --to docx "$md_file" -o "$out_file" 2>/dev/null || { log "  pandoc failed for $fid"; continue; }
        t1=$(date +%s%N)
        elapsed=$(( (t1 - t0) / 1000000 ))

        # Render to PDF via LO
        soffice --headless --convert-to pdf --outdir "$TMPDIR_MEASURE" "$out_file" 2>/dev/null
        [[ -f "$pdf_file" ]] || { log "  LO render failed for $fid"; continue; }

        measure_fixture "pandoc-docx" "$fid" "" "$out_file" "$pdf_file" "$elapsed"
    done
    log "  pandoc→docx complete"
}

# ---- pandoc path: odt ----

run_pandoc_odt() {
    has_tool pandoc || { log "SKIP pandoc→odt: pandoc not available"; return; }
    log "Starting pandoc→odt path..."

    for fid in "${SUBSET[@]}"; do
        local ref_txt="$ROOT/corpus/reference/${fid}.txt"
        [[ -f "$ref_txt" ]] || continue

        local md_file="$TMPDIR_MEASURE/${fid}.md"
        local out_file="$TMPDIR_MEASURE/${fid}.odt"
        local pdf_file="$TMPDIR_MEASURE/${fid}.pdf"

        # Create markdown from reference text
        local title
        title=$(head -1 "$ref_txt")
        {
            echo "# $title"
            tail -n +2 "$ref_txt" | while IFS= read -r line; do
                if [[ -z "$line" ]]; then
                    echo ""
                else
                    echo "$line"
                fi
            done
        } > "$md_file"

        # Time pandoc conversion
        local t0 t1 elapsed
        t0=$(date +%s%N)
        pandoc --to odt "$md_file" -o "$out_file" 2>/dev/null || { log "  pandoc failed for $fid"; continue; }
        t1=$(date +%s%N)
        elapsed=$(( (t1 - t0) / 1000000 ))

        # Render to PDF via LO
        soffice --headless --convert-to pdf --outdir "$TMPDIR_MEASURE" "$out_file" 2>/dev/null
        [[ -f "$pdf_file" ]] || { log "  LO render failed for $fid"; continue; }

        measure_fixture "pandoc-odt" "$fid" "" "$out_file" "$pdf_file" "$elapsed"
    done
    log "  pandoc→odt complete"
}

# ---- chromium path ----

run_chromium() {
    has_tool chromium-browser || { log "SKIP chromium→pdf: chromium-browser not available"; return; }
    log "Starting chromium→pdf path..."

    for fid in "${SUBSET[@]}"; do
        local ref_txt="$ROOT/corpus/reference/${fid}.txt"
        [[ -f "$ref_txt" ]] || continue

        local html_file="$TMPDIR_MEASURE/${fid}.html"
        local pdf_file="$TMPDIR_MEASURE/${fid}.pdf"

        # Create HTML from reference text
        local title
        title=$(head -1 "$ref_txt")
        cat > "$html_file" <<HTMLEOF
<!DOCTYPE html>
<html><head><meta charset="utf-8"><style>
body{font-family:DejaVu Sans,sans-serif;margin:2in;font-size:11pt;line-height:1.4}
h1{font-size:16pt;margin-bottom:1em}
</style></head><body>
<h1>$title</h1>
$(tail -n +2 "$ref_txt" | sed 's/^/ <p>/;s/$/<\/p>/')
</body></html>
HTMLEOF

        # Time chromium rendering
        local t0 t1 elapsed
        t0=$(date +%s%N)
        chromium-browser --headless --disable-gpu --no-sandbox \
            --print-to-pdf="$pdf_file" \
            --print-to-pdf-no-header \
            "$html_file" 2>/dev/null || { log "  chromium failed for $fid"; continue; }
        t1=$(date +%s%N)
        elapsed=$(( (t1 - t0) / 1000000 ))

        measure_fixture "chromium-pdf" "$fid" "" "" "$pdf_file" "$elapsed"
    done
    log "  chromium→pdf complete"
}

# ---- typst path ----

run_typst() {
    has_tool typst || { log "SKIP typst→pdf: typst not available"; return; }
    log "Starting typst→pdf path..."

    for fid in "${SUBSET[@]}"; do
        local ref_txt="$ROOT/corpus/reference/${fid}.txt"
        [[ -f "$ref_txt" ]] || continue

        local typ_file="$TMPDIR_MEASURE/${fid}.typ"
        local pdf_file="$TMPDIR_MEASURE/${fid}.pdf"

        # Create typst source from reference text (proper quoting)
        local title
        title=$(head -1 "$ref_txt")
        {
            echo '#set page(margin: 1in)'
            echo '#set text(font: "Noto Sans", size: 11pt)'
            echo "#heading(\"$title\")"
            tail -n +2 "$ref_txt" | while IFS= read -r line; do
                if [[ -z "$line" ]]; then
                    echo "#"
                else
                    echo "#text(\"$line\")"
                fi
            done
        } > "$typ_file"

        # Time typst compilation
        local t0 t1 elapsed
        t0=$(date +%s%N)
        typst compile "$typ_file" "$pdf_file" 2>/dev/null || { log "  typst failed for $fid"; continue; }
        t1=$(date +%s%N)
        elapsed=$(( (t1 - t0) / 1000000 ))

        measure_fixture "typst-pdf" "$fid" "" "" "$pdf_file" "$elapsed"
    done
    log "  typst→pdf complete"
}

# ---- native path: IR→OOXML via stichos create (IR→docx) ----
# Note: The create-as-revisions path (stichos create --target) has a known bug
# where the output package has an empty _rels/.rels, making it unreadable by LO.
# The native IR→OOXML writer path cannot be exercised directly via CLI for
# non-revision creation. This path is measured via unit tests instead.
# We note this limitation in the report.

run_native() {
    local binary="$ROOT/target/debug/stichos"
    [[ -f "$binary" ]] || { log "SKIP native path: stichos binary not found"; return; }
    log "SKIP native path: create-as-revisions has empty _rels/.rels bug (skip via CLI; measured via unit tests)"
}

# ---- run all paths ----

log "=== Starting P3-28 creation-path corpus measurement ==="
log "Fixtures: ${SUBSET_SIZE}"

run_pandoc_docx
run_pandoc_odt
run_chromium
run_typst
run_native

log "=== Measurement complete ==="
log "Results written to $CSV_OUTPUT"
log "Total rows (including header): $(wc -l < "$CSV_OUTPUT")"

# ---- summary stats ----

log ""
log "=== Summary by path ==="
tail -n +2 "$CSV_OUTPUT" | cut -d',' -f1 | sort -u | while read -r path; do
    local_count=$(grep -c "^${path}," "$CSV_OUTPUT" || true)
    avg_lat=$(grep "^${path}," "$CSV_OUTPUT" | cut -d',' -f3 | python3 -c "import sys;vals=[float(l) for l in sys.stdin if l.strip()];print(round(sum(vals)/len(vals),1) if vals else 0)" 2>/dev/null || echo "N/A")
    avg_pd=$(grep "^${path}," "$CSV_OUTPUT" | cut -d',' -f4 | python3 -c "import sys;vals=[float(p) for p in sys.stdin if p.strip()];print(round(sum(vals)/len(vals),1) if vals else 0)" 2>/dev/null || echo "N/A")
    avg_ssim=$(grep "^${path}," "$CSV_OUTPUT" | cut -d',' -f5 | python3 -c "
import sys
vals = []
for l in sys.stdin:
    v = l.strip()
    if v and v != 'null':
        vals.append(float(v))
print(round(sum(vals)/len(vals),4) if vals else 'N/A')
" 2>/dev/null || echo "N/A")
    avg_drops=$(grep "^${path}," "$CSV_OUTPUT" | cut -d',' -f6 | python3 -c "import sys;vals=[int(d) for d in sys.stdin if d.strip()];print(round(sum(vals)/len(vals),1) if vals else 0)" 2>/dev/null || echo "N/A")
    p95_lat=$(grep "^${path}," "$CSV_OUTPUT" | cut -d',' -f3 | python3 -c "
import sys,math
vals = sorted(float(l) for l in sys.stdin if l.strip())
if vals:
    rank = 0.95 * (len(vals) - 1)
    lo = int(rank)
    hi = min(lo + 1, len(vals) - 1)
    frac = rank - lo
    print(round(vals[lo] + frac * (vals[hi] - vals[lo]), 1))
else:
    print('N/A')
" 2>/dev/null || echo "N/A")
    p50_lat=$(grep "^${path}," "$CSV_OUTPUT" | cut -d',' -f3 | python3 -c "
import sys,math
vals = sorted(float(l) for l in sys.stdin if l.strip())
if vals:
    rank = 0.50 * (len(vals) - 1)
    lo = int(rank)
    hi = min(lo + 1, len(vals) - 1)
    frac = rank - lo
    print(round(vals[lo] + frac * (vals[hi] - vals[lo]), 1))
else:
    print('N/A')
" 2>/dev/null || echo "N/A")

    log "  $path: count=$local_count avg_lat=${avg_lat}ms avg_pd=${avg_pd} avg_ssim=${avg_ssim} avg_drops=${avg_drops} P50=${p50_lat}ms P95=${p95_lat}ms"
done
