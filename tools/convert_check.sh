#!/usr/bin/env bash
# SA-5v: LO conversion check for every corpus fixture inside the pinned image.
# Resumable: skips ids already present in baseline/convert-check.csv.
set -u
cd "$(dirname "$0")/.."
CSV=baseline/convert-check.csv
mkdir -p baseline
[ -f "$CSV" ] || echo "id,format,status,pages,seconds" > "$CSV"
for m in corpus/manifests/*.json; do
  id=$(basename "$m" .json)
  fmt=$(python3 -c "import json;print(json.load(open('$m'))['format'])")
  grep -q "^$id," "$CSV" && continue
  t0=$(date +%s)
  out=$(docker run --rm -v "$PWD/corpus/blobs:/in:ro" stichos-harness:26.2.5 \
    bash -c "cp /in/$id.$fmt /tmp/f.$fmt && cd /tmp && timeout 240 soffice --headless -env:UserInstallation=file:///tmp/u --convert-to pdf f.$fmt >/dev/null 2>&1 && pdfinfo f.pdf 2>/dev/null | awk '/^Pages:/{print \$2}'" 2>/dev/null)
  t1=$(date +%s)
  if [[ "$out" =~ ^[0-9]+$ ]]; then status=ok; pages=$out; else status=fail; pages=; fi
  echo "$id,$fmt,$status,${pages},$((t1-t0))" >> "$CSV"
  echo "$id $fmt $status ${pages}p $((t1-t0))s"
done
echo "done: $(($(wc -l < "$CSV")-1))/151 checked"
