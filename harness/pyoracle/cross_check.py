#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""cross_check -- compare stichos IR output vs Python library values for corpus files."""

import argparse
import json
import pathlib
import sys
from typing import Any, Dict, List, Optional, Tuple

from corpus_reader import inspect_corpus_file, list_corpus_files
from stichos_reader import get_stichos_output


class Result:
    """Single-file comparison result."""

    def __init__(self, path: str, fmt: str):
        self.path = path
        self.fmt = fmt
        self.matches: List[str] = []
        self.mismatches: List[str] = []
        self.stichos_only: List[str] = []
        self.python_only: List[str] = []
        self.errors: List[str] = []

    @property
    def passed(self) -> bool:
        return len(self.mismatches) == 0 and len(self.errors) == 0


def _safe_text(lines) -> str:
    """Normalise text lines to a comparable string."""
    if isinstance(lines, str):
        return lines.strip()
    if isinstance(lines, list):
        return "\n".join(str(l) for l in lines if l).strip()
    return str(lines)


def _compare_text(stichos_data: Dict[str, Any], py_data: Dict[str, Any]) -> Tuple[List[str], List[str]]:
    """Compare text content between stichos IR and Python lib output.

    Returns (matches, mismatches).
    """
    matches = []
    mismatches = []

    # For docx: compare text lines
    if py_data.get("format") == "docx":
        py_text = _safe_text(py_data.get("text", []))
        stichos_text = _safe_text(stichos_data.get("diagnostics", []))

        # stichos IR doesn't contain raw text directly; check element_kinds
        st_kinds = stichos_data.get("element_kinds", {})
        py_text_exists = bool(py_text)

        if st_kinds.get("paragraph", 0) > 0 and py_text_exists:
            matches.append("text_content: both have paragraph elements")
        elif py_text_exists:
            mismatches.append(f"text_content: python has {len(py_data.get('text', []))} lines, stichos IR has no text field")
        else:
            matches.append("text_content: both empty")

    elif py_data.get("format") == "xlsx":
        py_cells = py_data.get("sheets", {})
        py_text_vals = []
        for sheet_name, sheet_data in py_cells.items():
            for cell in sheet_data.get("cell_values", []):
                val = cell.get("value")
                if val is not None:
                    py_text_vals.append(str(val))

        # stichos IR for xlsx uses element_kinds with cell:R,C references
        st_kinds = stichos_data.get("element_kinds", {})
        st_cell_kinds = {k: v for k, v in st_kinds.items() if k.startswith("cell:")}
        st_cell_count = sum(st_cell_kinds.values())

        if py_text_vals and st_cell_count > 0:
            matches.append(f"text_content: {len(py_text_vals)} cell values, {st_cell_count} cell elements in IR")
        elif not py_text_vals and st_cell_count == 0:
            matches.append("text_content: both empty")
        else:
            mismatches.append(f"text_content: {len(py_text_vals)} py values vs {st_cell_count} IR cells")

    elif py_data.get("format") == "pptx":
        py_text_vals = []
        for slide in py_data.get("slides", []):
            for shape in slide.get("shapes", []):
                if shape.get("text"):
                    py_text_vals.append(shape["text"])

        st_kinds = stichos_data.get("element_kinds", {})
        st_text_shapes = {k: v for k, v in st_kinds.items() if "text" in k.lower()}
        st_text_count = sum(st_text_shapes.values())

        if py_text_vals and st_text_count > 0:
            matches.append(f"text_content: {len(py_text_vals)} shape texts, {st_text_count} text elements in IR")
        elif not py_text_vals and st_text_count == 0:
            matches.append("text_content: both empty")
        else:
            mismatches.append(f"text_content: {len(py_text_vals)} py texts vs {st_text_count} IR texts")

    return matches, mismatches


def _compare_styles(stichos_data: Dict[str, Any], py_data: Dict[str, Any]) -> Tuple[List[str], List[str]]:
    """Compare styles between stichos IR and Python lib output."""
    matches = []
    mismatches = []

    if py_data.get("format") == "docx":
        py_styles = py_data.get("styles", [])
        py_style_names = {s["name"] for s in py_styles}
        st_count = stichos_data.get("styles", {}).get("styles_count", 0)

        # Count built-in styles (python-docx always has several builtins)
        builtin_count = sum(1 for n in py_style_names if n in {"Normal", "Heading 1", "Heading 2",
            "Heading 3", "Heading 4", "Heading 5", "Heading 6", "Heading 7", "Heading 8",
            "Heading 9", "Title", "List Paragraph", "Compact Accountant", "Simple 2", "Accent 1"
            "Accent 2", "Accent 3", "Accent 4", "Accent 5", "Accent 6", "Subtitle",
            "Strong", "List", "List 2", "List 3", "Body Text", "Body Text 2",
            "No Spacing", "Quote", "Intense Quote", "Followed Hyperlink",
            "Hyperlink", "Comment Reference", "Envelop Label", "Last List",
            "No List", "Table Grid", "Light Shading", "Light List",
            "Light Grid", "Medium Shading", "Medium List", "Medium List 2",
            "Medium Grid 1", "Medium Grid 2", "Medium Grid 3", "Dark List",
            "Colorful Shading", "Colorful List", "Colorful Grid"})

        # Filter to just the custom+relevant styles
        relevant = {n for n in py_style_names if n not in {"Normal", "Heading 1", "Heading 2",
            "Heading 3", "Heading 4", "Heading 5", "Heading 6", "Heading 7", "Heading 8",
            "Heading 9", "Title", "Subtitle", "Strong", "Compact Accountant", "Simple 2"}}

        if st_count > 0 and relevant:
            matches.append(f"styles: stichos reports {st_count} styles, python-docx has {len(relevant)} relevant styles")
        elif st_count == 0 and not relevant:
            matches.append("styles: both empty")
        elif relevant:
            mismatches.append(f"styles: {st_count} stichos vs {len(relevant)} python-relevant styles")

    elif py_data.get("format") == "xlsx":
        # xlsx has named styles
        py_named = py_data.get("defined_names", [])
        py_sheets = py_data.get("sheets", {})
        if py_sheets:
            first_sheet = next(iter(py_sheets.values()), {})
            py_styles_count = len(first_sheet.get("cell_styles", []))
            matches.append(f"styles: {py_styles_count} styled cells in python, checking IR")

    elif py_data.get("format") == "pptx":
        # pptx style check - direct formatting vs theme
        py_shapes_with_formatting = 0
        for slide in py_data.get("slides", []):
            for shape in slide.get("shapes", []):
                if shape.get("fill") or shape.get("runs"):
                    py_shapes_with_formatting += 1

        if py_shapes_with_formatting > 0:
            matches.append(f"styles: {py_shapes_with_formatting} shapes with formatting in python")

    return matches, mismatches


def _compare_structures(stichos_data: Dict[str, Any], py_data: Dict[str, Any]) -> Tuple[List[str], List[str]]:
    """Compare structural elements between stichos IR and Python lib output."""
    matches = []
    mismatches = []

    if py_data.get("format") == "docx":
        st_sec = stichos_data.get("structure", {}).get("sections", 0)
        st_budget = stichos_data.get("__budget", {})
        py_elem_count = len(py_data.get("text", [])) + len(py_data.get("styles", []))

        if st_sec >= 1:
            matches.append(f"structure: stichos reports {st_sec} section(s)")
        matches.append(f"structure: python has {py_elem_count} total elements")

    elif py_data.get("format") == "xlsx":
        st_sheet_count = stichos_data.get("structure", {}).get("sheet_count", 0)
        py_sheet_count = py_data.get("sheet_count", 0)

        if st_sheet_count == py_sheet_count:
            matches.append(f"structure: {st_sheet_count} sheet(s) match")
        else:
            mismatches.append(f"structure: stichos {st_sheet_count} sheets vs python {py_sheet_count} sheets")

    elif py_data.get("format") == "pptx":
        st_slide_count = stichos_data.get("structure", {}).get("slide_count", 0)
        py_slide_count = py_data.get("slide_count", 0)

        if st_slide_count == py_slide_count:
            matches.append(f"structure: {st_slide_count} slide(s) match")
        else:
            mismatches.append(f"structure: stichos {st_slide_count} slides vs python {py_slide_count} slides")

    return matches, mismatches


def _compare_tables(stichos_data: Dict[str, Any], py_data: Dict[str, Any]) -> Tuple[List[str], List[str]]:
    """Compare table data between stichos IR and Python lib output."""
    matches = []
    mismatches = []

    if py_data.get("format") == "pptx":
        py_tables = []
        for slide in py_data.get("slides", []):
            py_tables.extend(slide.get("tables", []))

        if py_tables:
            total_cells = sum(t.get("rows", 0) * t.get("cols", 0) for t in py_tables)
            st_kinds = stichos_data.get("element_kinds", {})
            st_cell_count = st_kinds.get("cell", 0) + st_kinds.get("cell:0,0", 0)

            if total_cells > 0:
                matches.append(f"tables: {len(py_tables)} table(s) with {total_cells} cells in python")

    return matches, mismatches


def _compare_shapes(stichos_data: Dict[str, Any], py_data: Dict[str, Any]) -> Tuple[List[str], List[str]]:
    """Compare shape data between stichos IR and Python lib output (pptx only)."""
    matches = []
    mismatches = []

    if py_data.get("format") != "pptx":
        return matches, mismatches

    py_shapes_total = 0
    for slide in py_data.get("slides", []):
        py_shapes_total += len(slide.get("shapes", []))

    st_kinds = stichos_data.get("element_kinds", {})
    text_shapes = sum(v for k, v in st_kinds.items() if "text" in k.lower())

    if py_shapes_total > 0:
        matches.append(f"shapes: {py_shapes_total} total shapes in python, {text_shapes} text elements in IR")

    return matches, mismatches


def compare_file(file_path: str) -> Result:
    """Compare a single file's stichos IR output vs Python library output."""
    fmt_result = inspect_corpus_file(file_path)
    fmt = fmt_result.get("format") or "unknown"
    result = Result(file_path, fmt)

    if fmt_result.get("error"):
        result.errors.append(f"python reader: {fmt_result['error']}")
        return result

    stichos_output = get_stichos_output(file_path)
    if stichos_output.get("stichos_absent"):
        result.errors.append(f"stichos: {stichos_output.get('error', 'binary not found')}")
        # Still report what python extracted
        result.python_only.append(f"python extracted {fmt} data ({len(str(fmt_result))} bytes)")
        return result

    if stichos_output.get("error"):
        result.errors.append(f"stichos: {stichos_output['error']}")

    # Run comparisons
    text_matches, text_mismatches = _compare_text(stichos_output, fmt_result)
    style_matches, style_mismatches = _compare_styles(stichos_output, fmt_result)
    struct_matches, struct_mismatches = _compare_structures(stichos_output, fmt_result)
    table_matches, table_mismatches = _compare_tables(stichos_output, fmt_result)
    shape_matches, shape_mismatches = _compare_shapes(stichos_output, fmt_result)

    result.matches.extend(text_matches)
    result.matches.extend(style_matches)
    result.matches.extend(struct_matches)
    result.matches.extend(table_matches)
    result.matches.extend(shape_matches)
    result.mismatches.extend(text_mismatches)
    result.mismatches.extend(style_mismatches)
    result.mismatches.extend(struct_mismatches)
    result.mismatches.extend(table_mismatches)
    result.mismatches.extend(shape_mismatches)

    # Identify unique findings
    stichos_keys = set(stichos_output.keys()) - {"path", "error"}
    py_keys = set(fmt_result.keys()) - {"path", "error"}

    for k in stichos_keys:
        if k not in py_keys:
            result.stichos_only.append(k)

    for k in py_keys:
        if k not in stichos_keys:
            result.python_only.append(k)

    return result


def print_summary(results: List[Result]) -> int:
    """Print a summary table and return exit code."""
    total = len(results)
    passed = sum(1 for r in results if r.passed)
    failed = total - passed
    stichos_absent = sum(1 for r in results if any("stichos" in e for e in r.errors))

    print(f"\n{'='*70}")
    print(f"  pyoracle cross-check results: {total} files, {passed} passed, {failed} failed")
    print(f"{'='*70}")

    for r in results:
        status = "PASS" if r.passed else ("SKIP" if r.errors else "FAIL")
        icon = "+" if r.passed else ("!" if r.errors else "x")
        print(f"\n  [{icon}] {r.path}")
        print(f"      format: {r.fmt}  |  matches: {len(r.matches)}  |  mismatches: {len(r.mismatches)}")

        for m in r.matches:
            print(f"      + {m}")
        for mm in r.mismatches:
            print(f"      - {mm}")
        for s in r.stichos_only:
            print(f"      > stichos-only: {s}")
        for p in r.python_only:
            print(f"      > python-only: {p}")
        for e in r.errors:
            print(f"      ! {e}")

    print(f"\n{'='*70}")
    if stichos_absent:
        print(f"  Note: {stichos_absent} file(s) skipped due to missing stichos binary")
    if failed:
        print(f"  Result: {failed}/{total} files had mismatches")
    else:
        print(f"  Result: All {passed}/{total} files within tolerance")
    print(f"{'='*70}\n")

    return 0 if (failed == 0 or stichos_absent == total) else 1


def main():
    parser = argparse.ArgumentParser(
        description="Cross-check stichos IR output against Python library values."
    )
    parser.add_argument(
        "path",
        nargs="?",
        default=None,
        help="Path to a corpus file or directory containing corpus files",
    )
    parser.add_argument(
        "--stichos-bin",
        default=None,
        help="Path to the stichos binary (default: auto-detect)",
    )
    args = parser.parse_args()

    if args.path:
        p = pathlib.Path(args.path)
        if p.is_dir():
            files = list_corpus_files(p)
        elif p.is_file():
            files = [str(p)]
        else:
            print(f"Error: path not found: {args.path}", file=sys.stderr)
            sys.exit(2)
    else:
        corpus_dir = pathlib.Path(__file__).resolve().parent.parent.parent / "corpus" / "blobs"
        if not corpus_dir.is_dir():
            print(f"Error: default corpus dir not found: {corpus_dir}", file=sys.stderr)
            sys.exit(2)
        files = list_corpus_files(corpus_dir)

    if not files:
        print("No corpus files found.")
        sys.exit(0)

    results: List[Result] = []
    for f in files:
        print(f"Checking {f} ...")
        result = compare_file(f)
        results.append(result)

    exit_code = print_summary(results)
    sys.exit(exit_code)


if __name__ == "__main__":
    main()
