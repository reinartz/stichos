"""Feature counting in OOXML/ODF packages + feature retention (C9 §6).

Read-only streaming over the zip container: parts are parsed with
``xml.etree.ElementTree.iterparse`` straight from the zip stream, nothing
is ever extracted to disk. Hostile-input guards:

* compressed size > 500 MB refused before the zip is opened;
* declared uncompressed total > 2 GB aborts before any part is read
  (manifest-lie zip-bomb guard);
* actual decompressed bytes are also counted while parsing, so a part
  that lies about its size aborts at the same 2 GB cap.
"""
from __future__ import annotations

import zipfile
import xml.etree.ElementTree as ET
from pathlib import Path

from .errors import MetricsError

MAX_COMPRESSED_BYTES = 500 * 1024 * 1024
MAX_UNCOMPRESSED_BYTES = 2 * 1024 * 1024 * 1024

_FORMAT_FAMILY = {
    ".docx": "ooxml", ".xlsx": "ooxml", ".pptx": "ooxml",
    ".odt": "odf", ".ods": "odf", ".odp": "odf",
}

_W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_ODF_TABLE = "urn:oasis:names:tc:opendocument:xmlns:table:1.0"
_ODF_DRAW = "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
_ODF_TEXT = "urn:oasis:names:tc:opendocument:xmlns:text:1.0"
_ODF_OFFICE = "urn:oasis:names:tc:opendocument:xmlns:office:1.0"

_OOXML_PARTS: dict[str, dict[str, str]] = {
    "word/document.xml": {
        f"{{{_W}}}tbl": "w:tbl",
        f"{{{_W}}}drawing": "w:drawing",
        f"{{{_W}}}fldSimple": "w:fldSimple",
        f"{{{_W}}}commentReference": "w:commentReference",
    },
    "word/comments.xml": {f"{{{_W}}}comment": "w:comment"},
}

_ODF_PARTS: dict[str, dict[str, str]] = {
    "content.xml": {
        f"{{{_ODF_TABLE}}}table": "table:table",
        f"{{{_ODF_DRAW}}}frame": "draw:frame",
        f"{{{_ODF_TEXT}}}note": "text:note",
        f"{{{_ODF_TEXT}}}field": "text:field",
        f"{{{_ODF_OFFICE}}}annotation": "office:annotation",
    },
}

_COMPLEX_FIELD_ATTR = f"{{{_W}}}fldCharType"


class _Budget:
    """Shared decompressed-byte budget; trips the zip-bomb guard."""

    def __init__(self, limit: int) -> None:
        self.remaining = int(limit)

    def consume(self, n: int) -> None:
        self.remaining -= n
        if self.remaining < 0:
            raise MetricsError(
                "zip_bomb",
                f"decompressed size exceeded cap of {MAX_UNCOMPRESSED_BYTES} bytes",
            )


class _CappedReader:
    """File-like wrapper that charges every read to a shared _Budget."""

    def __init__(self, raw, budget: _Budget) -> None:
        self._raw = raw
        self._budget = budget

    def read(self, size: int = -1) -> bytes:
        if size is None or size < 0:
            chunk = self._raw.read()
        else:
            chunk = self._raw.read(size)
        self._budget.consume(len(chunk))
        return chunk


def _count_document_part(zf: zipfile.ZipFile, name: str, tag_map: dict[str, str],
                         counts: dict[str, int], budget: _Budget) -> int:
    """Stream one XML part; returns count of instrText-bearing complex fields."""
    try:
        raw = zf.open(name)
    except KeyError:
        return 0
    except RuntimeError as exc:  # encrypted member
        raise MetricsError("encrypted_zip", f"{name}: {exc}") from exc
    fld_char_tag = f"{{{_W}}}fldChar"
    instr_tag = f"{{{_W}}}instrText"
    field_stack: list[bool] = []
    instr_fields = 0
    with raw:
        source = _CappedReader(raw, budget)
        try:
            for _, elem in ET.iterparse(source, events=("end",)):
                tag = elem.tag
                if tag == fld_char_tag:
                    ftype = elem.get(_COMPLEX_FIELD_ATTR)
                    if ftype == "begin":
                        field_stack.append(False)
                    elif ftype == "end" and field_stack:
                        if field_stack.pop():
                            instr_fields += 1
                elif tag == instr_tag and field_stack:
                    field_stack[-1] = True
                mapped = tag_map.get(tag)
                if mapped is not None:
                    counts[mapped] += 1
                elem.clear()
        except ET.ParseError as exc:
            raise MetricsError("xml_parse_error", f"{name}: {exc}") from exc
        except (zipfile.BadZipFile, EOFError) as exc:
            raise MetricsError("zip_read_error", f"{name}: {exc}") from exc
    return instr_fields


def feature_counts(path) -> dict:
    """Count C9 §6 features in a docx/xlsx/pptx/odt/ods/odp package.

    Returns a flat dict: per-feature counts (``w:tbl``, ``w:drawing``,
    ``w:fldSimple``, ``w:instrText`` = complex fields bearing instrText,
    ``w:comment`` + ``w:commentReference`` as a pair; ODF: ``table:table``,
    ``draw:frame``, ``text:note``, ``text:field``, ``office:annotation``),
    plus ``format``, ``part_count`` and ``uncompressed_bytes`` (declared).
    Parts absent from the package count 0.
    """
    p = Path(path)
    if not p.is_file():
        raise MetricsError("file_not_found", f"not a file: {path}")
    family = _FORMAT_FAMILY.get(p.suffix.lower())
    if family is None:
        raise MetricsError(
            "unsupported_format",
            f"extension {p.suffix!r} not in docx/xlsx/pptx/odt/ods/odp",
        )
    compressed = p.stat().st_size
    if compressed > MAX_COMPRESSED_BYTES:
        raise MetricsError(
            "compressed_too_large",
            f"file is {compressed} bytes, cap is {MAX_COMPRESSED_BYTES}",
        )
    try:
        zf = zipfile.ZipFile(p)
    except (zipfile.BadZipFile, OSError) as exc:
        raise MetricsError("not_a_zip", f"{path}: {exc}") from exc

    with zf:
        files = [i for i in zf.infolist() if not i.is_dir()]
        declared = sum(i.file_size for i in files)
        if declared > MAX_UNCOMPRESSED_BYTES:
            raise MetricsError(
                "zip_bomb",
                f"declared uncompressed size {declared} exceeds cap "
                f"of {MAX_UNCOMPRESSED_BYTES} bytes",
            )
        tag_specs = _OOXML_PARTS if family == "ooxml" else _ODF_PARTS
        counts: dict[str, int] = {out: 0 for spec in tag_specs.values() for out in spec.values()}
        if family == "ooxml":
            counts["w:instrText"] = 0
        budget = _Budget(MAX_UNCOMPRESSED_BYTES)
        for part_name, tag_map in tag_specs.items():
            instr = _count_document_part(zf, part_name, tag_map, counts, budget)
            if "w:instrText" in counts:
                counts["w:instrText"] += instr
        counts["format"] = family
        counts["part_count"] = len(files)
        counts["uncompressed_bytes"] = int(declared)
    return counts


def feature_retention(before_counts: dict, after_counts: dict) -> dict:
    """Per-feature before/after/dropped view of two feature_counts results.

    Feature keys are those shaped ``ns:name`` (``part_count`` etc. are
    metadata and excluded). ``dropped = max(0, before - after)``;
    ``changed = before != after`` (C9 §6: any count change is a retention
    report entry). PASS = no feature changed at all; PASS against the
    manifest ``features{}`` expectation remains the caller's job.
    """
    if not isinstance(before_counts, dict) or not isinstance(after_counts, dict):
        raise MetricsError("bad_json", "both inputs must be JSON objects")
    keys = sorted(k for k in set(before_counts) | set(after_counts)
                  if isinstance(k, str) and ":" in k)
    features: dict[str, dict] = {}
    for key in keys:
        before = int(before_counts.get(key, 0))
        after = int(after_counts.get(key, 0))
        features[key] = {
            "before": before,
            "after": after,
            "dropped": max(0, before - after),
            "changed": before != after,
        }
    return {
        "features": features,
        "changed": [k for k, v in features.items() if v["changed"]],
        "pass": all(not v["changed"] for v in features.values()),
    }
