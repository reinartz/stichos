# SPDX-License-Identifier: MIT OR Apache-2.0
"""Fixture generator framework (HAR-02).

A builder registers for (category, format), produces a package file in a temp
dir and returns a BuildResult. The framework then normalizes the zip
deterministically, writes corpus/blobs/<id>.<fmt>, counts features with the
normative `metrics` lib, validates the manifest against
corpus/schema/fixture.schema.json, and writes manifest + reference text.

Determinism (INV-4): zip entries get a fixed date_time and sorted order;
builders must pin in-package timestamps (core.xml / meta.xml) via the helpers
below and seed any randomness from Ctx.rng. `gen verify` re-builds every
fixture and fails on any sha256 drift.
"""
from __future__ import annotations

import dataclasses
import hashlib
import json
import random
import sys
import tempfile
import zipfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable

REPO = Path(__file__).resolve().parents[3]
CORPUS = REPO / "corpus"
sys.path.insert(0, str(REPO / "harness" / "metrics"))

import jsonschema  # noqa: E402
from metrics import feature_counts  # noqa: E402

GENERATOR = {"name": "harness/generator", "version": "0.1.0"}
FIXED_ZIP_DT = (2026, 1, 1, 0, 0, 0)
FIXED_DT = datetime(2026, 1, 1, 0, 0, 0, tzinfo=timezone.utc)
RENDER = {"dpis": [150], "engines": ["soffice-26.2.5"]}

# Generated-fixture budget, verbatim from corpus/PLAN.md §3 (Σ=151).
# v1.1 M0-gate additions (+24, entries ending ": 2  # ✚"): the M0 exit gate needs the
# harness to run >=150 fixtures; v1.0 shipped 127 generated + 24 curated *planned* = 151,
# but curated-24 ingest (PLAN.md §5) is an outstanding owner follow-up (per-file license
# checks), so the gate is met on generated fixtures alone by adding second instances in
# the under-represented formats (pptx 15->26, odp 11->22, ods 19->21). Each 002 builder
# emits content that DIFFERS from 001 (strings/layout/counts), not a re-hash; same-category
# blobs are byte-dedup-checked. See PLAN.md §3 for the marked table.
BUDGET: dict[tuple[str, str], int] = {
    ("merged-cells", "docx"): 2, ("merged-cells", "xlsx"): 3, ("merged-cells", "pptx"): 2,  # ✚ pptx 1->2
    ("merged-cells", "odt"): 1, ("merged-cells", "ods"): 2,
    ("smartart-drawing", "docx"): 3, ("smartart-drawing", "pptx"): 2,
    ("embedded-ole", "docx"): 2, ("embedded-ole", "xlsx"): 1, ("embedded-ole", "pptx"): 2,  # ✚ pptx 1->2
    ("embedded-ole", "odt"): 1, ("embedded-ole", "ods"): 2, ("embedded-ole", "odp"): 2,  # ✚ ods 1->2, odp 1->2
    ("tracked-changes", "docx"): 3, ("tracked-changes", "xlsx"): 1,
    ("tracked-changes", "odt"): 2, ("tracked-changes", "ods"): 1,
    ("scanned-ocr", "docx"): 1, ("scanned-ocr", "pptx"): 2,  # ✚ pptx 1->2
    ("scanned-ocr", "odt"): 1, ("scanned-ocr", "odp"): 2,  # ✚ odp 1->2
    ("cjk-text", "docx"): 2, ("cjk-text", "xlsx"): 1, ("cjk-text", "pptx"): 2,  # ✚ pptx 1->2
    ("cjk-text", "odt"): 2, ("cjk-text", "ods"): 1, ("cjk-text", "odp"): 2,  # ✚ odp 1->2
    ("rtl-text", "docx"): 2, ("rtl-text", "pptx"): 2,  # ✚ pptx 1->2
    ("rtl-text", "odt"): 2, ("rtl-text", "odp"): 2,  # ✚ odp 1->2
    ("rtl-sheets", "xlsx"): 2, ("rtl-sheets", "ods"): 2,
    ("long-deck-300", "pptx"): 2, ("long-deck-300", "odp"): 2,  # ✚ odp 1->2
    ("long-tables", "docx"): 2, ("long-tables", "xlsx"): 2,
    ("long-tables", "odt"): 1, ("long-tables", "ods"): 1,
    ("footnotes-endnotes", "docx"): 2, ("footnotes-endnotes", "odt"): 2,
    ("headers-fields", "docx"): 2, ("headers-fields", "odt"): 2,
    ("toc-fields", "docx"): 2, ("toc-fields", "odt"): 2,
    ("image-textwrap", "docx"): 2, ("image-textwrap", "pptx"): 2,  # ✚ pptx 1->2
    ("image-textwrap", "odt"): 1, ("image-textwrap", "odp"): 2,  # ✚ odp 1->2
    ("formulas-cached", "xlsx"): 3, ("formulas-cached", "ods"): 3,
    ("conditional-formatting", "xlsx"): 2, ("conditional-formatting", "ods"): 2,
    ("custom-styles", "docx"): 2, ("custom-styles", "xlsx"): 1, ("custom-styles", "pptx"): 2,  # ✚ pptx 1->2
    ("custom-styles", "odt"): 2, ("custom-styles", "ods"): 1, ("custom-styles", "odp"): 2,  # ✚ odp 1->2
    ("numbering-restarts", "docx"): 2, ("numbering-restarts", "pptx"): 2,  # ✚ pptx 1->2
    ("numbering-restarts", "odt"): 2, ("numbering-restarts", "odp"): 2,  # ✚ odp 1->2
    ("comments-annotations", "docx"): 2, ("comments-annotations", "xlsx"): 1,
    ("comments-annotations", "pptx"): 2, ("comments-annotations", "odt"): 1,  # ✚ pptx 1->2
    ("comments-annotations", "ods"): 2, ("comments-annotations", "odp"): 2,  # ✚ ods 1->2, odp 1->2
    ("protected-readonly", "docx"): 1, ("protected-readonly", "xlsx"): 1,
    ("protected-readonly", "pptx"): 2, ("protected-readonly", "odt"): 1,  # ✚ pptx 1->2
    ("protected-readonly", "ods"): 1, ("protected-readonly", "odp"): 2,  # ✚ odp 1->2
    ("unicode-edge", "docx"): 2, ("unicode-edge", "xlsx"): 1, ("unicode-edge", "pptx"): 2,  # ✚ pptx 1->2
    ("unicode-edge", "odt"): 2, ("unicode-edge", "ods"): 1, ("unicode-edge", "odp"): 2,  # ✚ odp 1->2
    ("huge-cell-text", "docx"): 1, ("huge-cell-text", "xlsx"): 2,
    ("huge-cell-text", "odt"): 1, ("huge-cell-text", "ods"): 2,
}

BUILDERS: dict[tuple[str, str], Callable[["Ctx", int], "BuildResult"]] = {}


def builder(category: str, fmt: str):
    def deco(fn: Callable[["Ctx", int], "BuildResult"]):
        if (category, fmt) in BUILDERS:
            raise ValueError(f"duplicate builder {category}/{fmt}")
        BUILDERS[(category, fmt)] = fn
        return fn

    return deco


@dataclasses.dataclass
class Ctx:
    fixture_id: str
    tmp: Path
    rng: random.Random


@dataclasses.dataclass
class BuildResult:
    path: Path
    title: str
    reference_text: str | None
    notes: str


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def png_rgb_bytes(img) -> bytes:
    """Deterministic PNG container for PIL-drawn images (INV-4).

    Pixels come from PIL (same pinned version rasterizes identically), but the
    file is hand-encoded: fixed IHDR, one IDAT of filter-0 scanlines compressed
    with zlib level 0. Stored-deflate blocks are a pure RFC-1951 format — no
    encoder choices — so the bytes are identical across zlib/Pillow wheel
    builds. (Nightly 34214500425: Pillow 12.2.0 cp314 vs cp312 wheels produced
    different IDAT for identical pixels; only PIL's own PNG encoder is bypassed.)"""
    import struct
    import zlib

    if img.mode != "RGB":
        img = img.convert("RGB")
    w, h = img.size
    raw = img.tobytes()
    rows = b"".join(b"\x00" + raw[y * w * 3:(y + 1) * w * 3] for y in range(h))
    co = zlib.compressobj(0)
    idat = co.compress(rows) + co.flush()

    def chunk(typ: bytes, data: bytes) -> bytes:
        return (struct.pack(">I", len(data)) + typ + data
                + struct.pack(">I", zlib.crc32(typ + data) & 0xFFFFFFFF))

    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", idat)
            + chunk(b"IEND", b""))


def normalize_zip(path: Path) -> None:
    """Sorted entries, fixed date_time, fixed attrs, ZIP_STORED. Entries are
    stored UNCOMPRESSED on purpose: deflate streams differ across zlib builds
    (host zlib-ng vs CI stock zlib — nightly run 34200949975), so container
    bytes must not depend on a compressor to stay byte-reproducible across
    environments (INV-4). `create_system=3` pins the Unix creator byte (ZipInfo
    defaults to the build host's OS). ODF special case: `mimetype` entry goes
    FIRST, STORED (ODF §3.1.1). In-package timestamps are the builder's job
    (pin core.xml/meta.xml via FIXED_DT)."""
    with zipfile.ZipFile(path) as zf:
        entries = {i.filename: zf.read(i.filename) for i in zf.infolist()}
    with zipfile.ZipFile(path, "w", zipfile.ZIP_STORED) as zf:
        names = sorted(n for n in entries if n != "mimetype")
        if "mimetype" in entries:
            zi = zipfile.ZipInfo("mimetype", date_time=FIXED_ZIP_DT)
            zi.compress_type = zipfile.ZIP_STORED
            zi.create_system = 3
            zi.external_attr = 0o644 << 16
            zf.writestr(zi, entries["mimetype"])
        for name in names:
            zi = zipfile.ZipInfo(name, date_time=FIXED_ZIP_DT)
            zi.compress_type = zipfile.ZIP_STORED
            zi.create_system = 3
            zi.external_attr = 0o644 << 16
            zf.writestr(zi, entries[name])


def zip_read(path: Path) -> dict[str, bytes]:
    with zipfile.ZipFile(path) as zf:
        return {i.filename: zf.read(i.filename) for i in zf.infolist()}


def zip_write(path: Path, entries: dict[str, bytes]) -> None:
    """Write entries as-is; call normalize_zip afterwards (or build via tmp + normalize)."""
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as zf:
        for name, data in entries.items():
            zf.writestr(name, data)


def pin_docx_props(doc) -> None:
    cp = doc.core_properties
    cp.author = cp.last_modified_by = "stichos"
    cp.revision = 1
    cp.created = cp.modified = FIXED_DT.replace(tzinfo=None)


def pin_xlsx_props(wb) -> None:
    wb.properties.creator = "stichos"
    wb.properties.lastModifiedBy = "stichos"
    wb.properties.created = FIXED_DT.replace(tzinfo=None)
    wb.properties.modified = FIXED_DT.replace(tzinfo=None)


def pin_pptx_props(prs) -> None:
    cp = prs.core_properties
    cp.author = cp.last_modified_by = "stichos"
    cp.revision = 1
    cp.created = cp.modified = FIXED_DT.replace(tzinfo=None)


def _load_schema() -> dict:
    return json.loads((CORPUS / "schema" / "fixture.schema.json").read_text(encoding="utf-8"))


def _manifest(fid: str, fmt: str, cat: str, res: BuildResult, sha: str) -> dict:
    feats = {k: v for k, v in feature_counts(res.path).items() if ":" in k}
    m = {
        "schema_version": 1,
        "id": fid,
        "title": res.title,
        "source": "generated",
        "generator": GENERATOR,
        "format": fmt,
        "categories": [cat],
        "features": feats,
        "reference_text": f"corpus/reference/{fid}.txt" if res.reference_text is not None else None,
        "expected": {"page_count_min": None, "page_count_max": None},
        "render": RENDER,
        "notes": res.notes,
        "sha256": sha,
    }
    jsonschema.validate(m, _load_schema())
    return m


def build_one(cat: str, fmt: str, nnn: int, out: Path | None = None) -> dict:
    fn = BUILDERS[(cat, fmt)]
    fid = f"{cat}-{fmt}-{nnn:03d}"
    with tempfile.TemporaryDirectory(prefix="gpfx-") as td:
        ctx = Ctx(fixture_id=fid, tmp=Path(td), rng=random.Random(fid))
        res = fn(ctx, nnn)
        normalize_zip(res.path)
        out_dir = out or CORPUS
        blob = out_dir / "blobs" / f"{fid}.{fmt}"
        blob.parent.mkdir(parents=True, exist_ok=True)
        blob.write_bytes(res.path.read_bytes())
        manifest = _manifest(fid, fmt, cat, res, sha256_file(blob))
    if out is None:
        mdir = CORPUS / "manifests"
        mdir.mkdir(parents=True, exist_ok=True)
        (mdir / f"{fid}.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n",
                                          encoding="utf-8")
        if res.reference_text is not None:
            rdir = CORPUS / "reference"
            rdir.mkdir(parents=True, exist_ok=True)
            (rdir / f"{fid}.txt").write_text(res.reference_text, encoding="utf-8")
    return manifest


def select(fmt: str | None, cat: str | None) -> list[tuple[str, str, int]]:
    jobs = []
    for (c, f), n in sorted(BUDGET.items()):
        if fmt and f != fmt:
            continue
        if cat and c != cat:
            continue
        if (c, f) not in BUILDERS:
            continue
        jobs += [(c, f, i) for i in range(1, n + 1)]
    return jobs


def build_all(fmt: str | None, cat: str | None) -> list[dict]:
    manifests = []
    for c, f, i in select(fmt, cat):
        m = build_one(c, f, i)
        manifests.append(m)
        print(f"built {m['id']} sha256={m['sha256'][:12]}…")
    return manifests


def _no_blob_problems(mp: Path, m: dict, fid: str) -> list[str]:
    """Manifest-only internal consistency (blobs are gitignored, corpus/PLAN.md §6):
    id<->filename, id<->format/category, budget membership, generator identity,
    reference-text path convention + presence. No fixture bytes are read."""
    problems: list[str] = []
    if mp.stem != fid:
        problems.append(f"{fid}: manifest filename {mp.name} != id")
    parts = fid.rsplit("-", 2)
    if len(parts) != 3 or not parts[2].isdigit():
        problems.append(f"{fid}: id does not parse as <cat>-<format>-<nnn>")
        return problems
    cat, fmt, nnn = parts[0], parts[1], int(parts[2])
    if fmt != m["format"]:
        problems.append(f"{fid}: id format '{fmt}' != manifest format '{m['format']}'")
    if cat not in m["categories"]:
        problems.append(f"{fid}: id category '{cat}' not in categories {m['categories']}")
    rt = m.get("reference_text")
    if rt and not (REPO / rt).exists():
        problems.append(f"{fid}: reference text missing: {rt}")
    if m["source"] == "generated":
        if m["categories"] != [cat]:
            problems.append(f"{fid}: generated fixture must carry exactly ['{cat}']")
        if rt is not None and rt != f"corpus/reference/{fid}.txt":
            problems.append(f"{fid}: reference_text {rt} != corpus/reference/{fid}.txt")
        if m["generator"]["name"] != GENERATOR["name"]:
            problems.append(f"{fid}: generator.name != {GENERATOR['name']}")
        if (cat, fmt) not in BUDGET:
            problems.append(f"{fid}: ({cat}, {fmt}) not in corpus/PLAN.md §3 budget")
        elif not 1 <= nnn <= BUDGET[(cat, fmt)]:
            problems.append(f"{fid}: nnn {nnn:03d} outside budget 1..{BUDGET[(cat, fmt)]:03d}")
        if m["sha256"] is None:
            problems.append(f"{fid}: generated manifest sha256 is null")
    return problems


def verify_all(no_blobs: bool = False) -> list[str]:
    """Check every manifest: schema, blob sha, feature recount, rebuild determinism.

    no_blobs=True switches to manifest-only mode (CI: corpus/blobs is gitignored,
    corpus/PLAN.md §6): schema + internal consistency + reference-file presence;
    blob sha, feature recount and rebuild determinism are skipped."""
    problems: list[str] = []
    for mp in sorted((CORPUS / "manifests").glob("*.json")):
        m = json.loads(mp.read_text(encoding="utf-8"))
        fid = m["id"]
        try:
            jsonschema.validate(m, _load_schema())
        except jsonschema.ValidationError as e:
            problems.append(f"{fid}: schema: {e.message}")
            continue
        if no_blobs:
            problems.extend(_no_blob_problems(mp, m, fid))
            continue
        blob = CORPUS / "blobs" / f"{fid}.{m['format']}"
        if not blob.exists():
            problems.append(f"{fid}: blob missing")
            continue
        if sha256_file(blob) != m["sha256"]:
            problems.append(f"{fid}: sha256 drift")
            continue
        recount = {k: v for k, v in feature_counts(blob).items() if ":" in k}
        if recount != m["features"]:
            problems.append(f"{fid}: feature recount {recount} != manifest {m['features']}")
        key = (fid.rsplit("-", 2)[0], m["format"])
        cat, nnn = key[0], int(fid.rsplit("-", 1)[1])
        if key in BUILDERS:
            with tempfile.TemporaryDirectory() as td:
                m2 = build_one(cat, m["format"], nnn, out=Path(td))
            if m2["sha256"] != m["sha256"]:
                problems.append(f"{fid}: rebuild not deterministic")
        rt = m.get("reference_text")
        if rt and not (REPO / rt).exists():
            problems.append(f"{fid}: reference text missing: {rt}")
    return problems
