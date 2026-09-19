# SPDX-License-Identifier: MIT OR Apache-2.0
"""``python -m metrics`` — JSON-first CLI for the C9 Phase-0 metrics.

Every verb prints one JSON object on stdout (``{"ok": true, ...}`` on
success, ``{"ok": false, "error": {"code", "message"}}`` on failure) and
exits nonzero on any error.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from .errors import MetricsError
from .features import feature_counts, feature_retention
from .pages import page_count_delta
from .ssim import ssim_pages
from .text import text_f1


def _read_text(path: str) -> str:
    if path == "-":
        return sys.stdin.read()
    try:
        return Path(path).read_text(encoding="utf-8", errors="replace")
    except OSError as exc:
        raise MetricsError("file_not_found", f"{path}: {exc}") from exc


def _read_counts_json(path: str) -> dict:
    raw = _read_text(path)
    try:
        data = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise MetricsError("bad_json", f"{path}: {exc}") from exc
    if not isinstance(data, dict):
        raise MetricsError("bad_json", f"{path}: expected a JSON object")
    return data


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="python -m metrics",
        description="stichos Phase-0 fidelity metrics (C9 reference implementation)",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("ssim", help="per-page SSIM between two raster page dirs (C9 §4)")
    p.add_argument("a_dir", help="directory of candidate rasters")
    p.add_argument("b_dir", help="directory of golden rasters")
    p.add_argument("--dpi", type=int, default=150, help="nominal render DPI (recorded, C9 §4)")

    p = sub.add_parser("feature-counts", help="count OOXML/ODF features in a package (C9 §6)")
    p.add_argument("path", help="docx/xlsx/pptx/odt/ods/odp file")

    p = sub.add_parser("retention", help="feature retention between two feature-counts JSON docs (C9 §6)")
    p.add_argument("before", help="before-counts JSON file, or - for stdin")
    p.add_argument("after", help="after-counts JSON file, or - for stdin")

    p = sub.add_parser("text-f1", help="text-extraction F1 (C9 §7)")
    p.add_argument("reference", help="reference text file, or - for stdin")
    p.add_argument("extracted", help="extracted text file, or - for stdin")

    p = sub.add_parser("page-delta", help="signed page-count delta (C9 §5)")
    p.add_argument("a", type=int, help="candidate page count")
    p.add_argument("b", type=int, help="golden page count")
    return parser


def _dispatch(args: argparse.Namespace) -> dict:
    if args.command == "ssim":
        return ssim_pages(args.a_dir, args.b_dir, dpi=args.dpi)
    if args.command == "feature-counts":
        return feature_counts(args.path)
    if args.command == "retention":
        return feature_retention(_read_counts_json(args.before),
                                 _read_counts_json(args.after))
    if args.command == "text-f1":
        return text_f1(_read_text(args.reference), _read_text(args.extracted))
    if args.command == "page-delta":
        return page_count_delta(args.a, args.b)
    raise MetricsError("unknown_command", args.command)


def main(argv: list[str] | None = None) -> int:
    args = _build_parser().parse_args(argv)
    try:
        result = _dispatch(args)
    except MetricsError as exc:
        print(json.dumps({"ok": False, "error": {"code": exc.code,
                                                 "message": str(exc)}},
                         ensure_ascii=False))
        return 2
    except Exception as exc:  # noqa: BLE001 — last-resort machine-readable error
        print(json.dumps({"ok": False, "error": {"code": "internal_error",
                                                 "message": f"{type(exc).__name__}: {exc}"}},
                         ensure_ascii=False))
        return 1
    print(json.dumps({"ok": True, **result}, ensure_ascii=False))
    return 0


def cli_entry(argv: list[str] | None = None) -> None:
    try:
        code = main(argv)
    except SystemExit as exc:
        code = exc.code if isinstance(exc.code, int) else 2
    sys.exit(code)


if __name__ == "__main__":
    cli_entry()
