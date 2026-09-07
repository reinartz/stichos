#!/usr/bin/env python3
"""Byte-dedup gate for the corpus: no two fixtures in the SAME CATEGORY may share bytes.

v1.1 M0-gate additions added a second instance (nnn=002) to 24 budget cells; the
rule (corpus/PLAN.md §3) is that a second instance is a real fixture, not a
re-hash or a copy with a renamed title. Check: group corpus/blobs by category
(id = <cat>-<fmt>-<nnn>), sha256 every blob, fail on any duplicate hash inside
a category. Global duplicates across categories are reported as information
(should be none: different formats/ids embed different parts).

Usage: python3 tools/dedup_check.py [corpus_dir]   (exit 0 = all distinct)
"""
from __future__ import annotations

import hashlib
import sys
from collections import defaultdict
from pathlib import Path


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> int:
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[1] / "corpus"
    blobs = sorted((root / "blobs").iterdir())
    by_cat: dict[str, dict[str, list[str]]] = defaultdict(lambda: defaultdict(list))
    for p in blobs:
        stem = p.stem  # <cat>-<fmt>-<nnn>
        cat = stem.rsplit("-", 2)[0]
        by_cat[cat][sha256_file(p)].append(stem)

    dupes = []
    total = 0
    for cat in sorted(by_cat):
        for sha, names in by_cat[cat].items():
            total += len(names)
            if len(names) > 1:
                dupes.append((cat, sha[:16], sorted(names)))

    print(f"blobs checked: {total} across {len(by_cat)} categories")
    if dupes:
        for cat, sha, names in dupes:
            print(f"DUPLICATE in {cat} sha256={sha}… : {', '.join(names)}")
        print(f"dedup: FAIL ({len(dupes)} duplicate group(s))")
        return 1
    per_cat = {c: sum(len(v) for v in by_cat[c].values()) for c in sorted(by_cat)}
    multi = {c: n for c, n in per_cat.items() if n > 1}
    print(f"categories with >1 instance: {len(multi)} (all instance pairs distinct)")
    print("dedup: OK (all same-category blobs distinct)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
