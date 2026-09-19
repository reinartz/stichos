# SPDX-License-Identifier: MIT OR Apache-2.0
"""xlsx category builders (openpyxl)."""

import re
import xml.etree.ElementTree as ET
from xml.sax.saxutils import escape

from ..core import FIXED_DT, BuildResult, builder, pin_xlsx_props, zip_read, zip_write

_SHEET_NS = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"


def _freeze_modified(path):
    """openpyxl's save_workbook() overwrites dcterms:modified with wall-clock at
    save time (openpyxl/writer/excel.py), defeating pin_xlsx_props. Re-pin the
    stamp in docProps/core.xml to FIXED_DT so rebuilds are byte-identical (INV-4)."""
    entries = zip_read(path)
    core_xml = entries["docProps/core.xml"].decode("utf-8")
    stamp = FIXED_DT.strftime("%Y-%m-%dT%H:%M:%SZ")
    core_xml, n = re.subn(
        r"(<dcterms:modified[^>]*>)[^<]*(</dcterms:modified>)",
        rf"\g<1>{stamp}\g<2>",
        core_xml,
    )
    assert n == 1, "dcterms:modified anchor missing in core.xml"
    entries["docProps/core.xml"] = core_xml.encode("utf-8")
    zip_write(path, entries)


@builder("merged-cells", "xlsx")
def merged_cells(ctx, nnn):
    from openpyxl import Workbook

    wb = Workbook()
    ws = wb.active
    ws.title = "Main"
    if nnn == 1:
        grid = ["A1v", "B1v", "A2v", "B2v", "A3v", "B3v"]
        for v, (row, col) in zip(
            grid, [(1, 1), (1, 2), (2, 1), (2, 2), (3, 1), (3, 2)]
        ):
            ws.cell(row=row, column=col, value=v)
        ws.merge_cells("A1:B3")  # 3 rows x 2 cols; openpyxl keeps top-left only
        ws["D1"] = "outside"
        ref = "A1v\noutside"
        notes = (
            "real ws.merge_cells 3x2 region A1:B3; all six cells (every corner) were written "
            "pre-merge as a value-conflict probe — openpyxl's merge keeps the top-left value "
            "only and the other five are not present in sheet XML, so reference_text lists the "
            "survivors (top-left + one normal cell) only."
        )
    elif nnn == 2:
        for v, (row, col) in [
            ("M1A1", (1, 1)), ("M1B1", (1, 2)), ("M1A2", (2, 1)), ("M1B2", (2, 2)),
            ("M2D3", (3, 4)), ("M2E3", (3, 5)), ("M2D4", (4, 4)), ("M2E4", (4, 5)),
        ]:
            ws.cell(row=row, column=col, value=v)
        ws.merge_cells("A1:B2")
        ws.merge_cells("D3:E4")
        ws["C1"] = "free-c1"
        ws["F5"] = "free-f5"
        ws["G7"] = "free-g7"
        ref = "M1A1\nM2D3\nfree-c1\nfree-f5\nfree-g7"
        notes = (
            "real ws.merge_cells: two disjoint regions A1:B2 and D3:E4 plus three unmerged "
            "normal cells; pre-merge corner values outside each top-left are dropped by "
            "openpyxl, reference_text lists surviving strings only."
        )
    else:
        ws["A1"] = "MainMerge"
        ws.merge_cells("A1:B2")
        ws["D1"] = "main-normal"
        ws2 = wb.create_sheet("Second")
        ws2["B4"] = "SecondMerge"
        ws2.merge_cells("B4:C6")
        ws2["E1"] = "second-normal"
        ref = "MainMerge\nmain-normal\nSecondMerge\nsecond-normal"
        notes = (
            "real ws.merge_cells across two worksheets ('Main' A1:B2, 'Second' B4:C6), one "
            "merge per sheet plus one normal cell per sheet; merge regions never cross sheet "
            "boundaries (xlsx has no cross-sheet merge) — the point is per-sheet merge state."
        )
    pin_xlsx_props(wb)
    path = ctx.tmp / "fixture.xlsx"
    wb.save(path)
    _freeze_modified(path)
    return BuildResult(
        path=path, title=f"Merged cells grid {nnn}", reference_text=ref, notes=notes
    )


@builder("rtl-sheets", "xlsx")
def rtl_sheets(ctx, nnn):
    from openpyxl import Workbook
    from openpyxl.styles import Alignment

    wb = Workbook()
    ws = wb.active
    ws.title = "RTL"
    ws.sheet_view.rightToLeft = True
    ws["A1"] = "المنتج"
    ws["B1"] = "السعر"
    rows = [("زيت", 45.5), ("خبز", 3.25), ("عسل", 88)]
    for i, (name, price) in enumerate(rows, start=2):
        ws.cell(row=i, column=1, value=name)
        ws.cell(row=i, column=2, value=price)
    ref = "\n".join(
        [f"{ws['A1'].value} {ws['B1'].value}"]
        + [f"{name} {price}" for name, price in rows]
    )
    if nnn == 1:
        notes = (
            "real sheetView rightToLeft=1 on the whole worksheet (view-level RTL, distinct "
            "from per-cell bidi alignment); Arabic headers + three Arabic product names and "
            "three real numeric values (no formulas)."
        )
    else:
        ws.column_dimensions["B"].alignment = Alignment(horizontal="right")
        ws2 = wb.create_sheet("LTR")
        ws2.sheet_view.rightToLeft = False
        ws2["A1"] = "Widget"
        ws2["B1"] = "Count"
        ws2["A2"] = "Bolts"
        ws2["B2"] = 120
        ref += f"\n{ws2['A1'].value} {ws2['B1'].value}\n{ws2['A2'].value} {ws2['B2'].value}"
        notes = (
            "mixed-workbook case: sheet 'RTL' has sheetView rightToLeft=1 plus one "
            "column-level right alignment on column B (column_dimensions alignment), sheet "
            "'LTR' is explicitly left-to-right — extractors must not assume one bidi view "
            "direction per workbook; numeric values are real, no formulas."
        )
    pin_xlsx_props(wb)
    path = ctx.tmp / "fixture.xlsx"
    wb.save(path)
    _freeze_modified(path)
    return BuildResult(
        path=path, title=f"RTL sheet view {nnn}", reference_text=ref, notes=notes
    )


@builder("protected-readonly", "xlsx")
def protected_readonly(ctx, nnn):
    from openpyxl import Workbook
    from openpyxl.styles import Protection
    from openpyxl.workbook.protection import WorkbookProtection

    wb = Workbook()
    ws = wb.active
    ws.title = "Locked"
    ws["A1"] = "locked-cell"  # inherits default locked=1 from Normal style
    ws["A2"] = "open-cell-1"
    ws["A2"].protection = Protection(locked=False)
    ws["A3"] = "open-cell-2"
    ws["A3"].protection = Protection(locked=False)
    ws.protection.sheet = True  # password-less sheet protection (weak, intentional)
    wb.security = WorkbookProtection(lockStructure=True)  # also password-less
    ref = "locked-cell\nopen-cell-1\nopen-cell-2"
    notes = (
        "real sheetProtection (sheet=\"1\", no password hash) via ws.protection.sheet=True, "
        "plus password-less workbook structure lock (wb.security lockStructure=True); A1 is "
        "the one locked cell (inherits locked=1 from the Normal cell style), A2:A3 carry "
        "explicit applyProtection locked=0 xf entries — verified in styles.xml/sheet1.xml. "
        "Password-less protection is a UI hint, not security; weakness is intentional fixture "
        "content, not a generator bug."
    )
    pin_xlsx_props(wb)
    path = ctx.tmp / "fixture.xlsx"
    wb.save(path)
    _freeze_modified(path)
    return BuildResult(
        path=path, title=f"Protected read-only xlsx {nnn:03d}", reference_text=ref, notes=notes
    )


@builder("unicode-edge", "xlsx")
def unicode_edge(ctx, nnn):
    from openpyxl import Workbook

    wb = Workbook()
    ws = wb.active
    ws.title = "Unicode"
    cells = [
        ("A1", "ﬁﬂ"),  # ligature glyphs U+FB01 U+FB02
        ("A2", "\U0001F468\u200D\U0001F469\u200D\U0001F467\u200D\U0001F466"),  # ZWJ family emoji
        ("A3", "e\u0301clair"),  # e + combining acute U+0301 (NFD sequence)
        ("A4", "The\u00a0quick brown\u00a0fox\u00a0jumps."),  # NBSP-separated sentence
        ("A5", "床前明月光"),  # Quiet Night Thought, first line
        ("A6", "שלום world"),  # Hebrew + Latin run mix
    ]
    for addr, val in cells:
        ws[addr] = val
    ref = "\n".join(val for _, val in cells)
    notes = (
        "six real shared-string/inline cell values: typographic ligatures ﬁﬂ (compatibility "
        "codepoints, no font trickery), one ZWJ-joined family emoji (U+1F468/469/467/466 with "
        "U+200D separators), a decomposed e+U+0301 combining sequence, NBSP (U+00A0) inside a "
        "Latin sentence, CJK '床前明月光' (Quiet Night Thought line 1, per-char F1 rule applies "
        "downstream), and a Hebrew+Latin mixed run — one per cell A1..A6, no escaping or "
        "normalization applied by the generator."
    )
    pin_xlsx_props(wb)
    path = ctx.tmp / "fixture.xlsx"
    wb.save(path)
    _freeze_modified(path)
    return BuildResult(
        path=path, title=f"Unicode edge cells xlsx {nnn:03d}", reference_text=ref, notes=notes
    )


def _assert_every_f_has_v(xml_text):
    """Recount: every <c> element that carries an <f> child must also carry a
    cached <v> sibling. Returns the number of formula cells checked."""
    root = ET.fromstring(xml_text)
    checked = 0
    for c in root.iter(f"{{{_SHEET_NS}}}c"):
        if c.find(f"{{{_SHEET_NS}}}f") is not None:
            v = c.find(f"{{{_SHEET_NS}}}v")
            assert v is not None and (v.text or "").strip() != "", (
                f"formula cell {c.get('r')} has <f> but no non-empty cached <v>"
            )
            checked += 1
    return checked


def _inject_cached_values(path, sheet_part, cache):
    """openpyxl writes formula cells as <c r="A1"><f>...</f><v></v></c> — the
    cached <v> element is present but EMPTY (openpyxl 3.1.5), i.e. there is no
    cached value. zip_read/zip_write the sheet part and fill each empty <v>
    (t="str" added when the result is a string) with the value pre-computed in
    python by the builder; a missing <v> is inserted after </f> instead."""
    entries = zip_read(path)
    xml = entries[sheet_part].decode("utf-8")
    for ref, value in sorted(cache.items()):
        is_str = isinstance(value, str)
        pattern = re.compile(
            rf'(<c r="{ref}")([^>]*>)(<f>.*?</f>)(?:<v\s*/>|<v></v>)?(</c>)'
        )

        def repl(m, _is_str=is_str, _val=value):
            attrs = m.group(2)
            if _is_str and " t=" not in attrs:
                attrs = attrs[:-1] + ' t="str">'
            return (
                m.group(1)
                + attrs
                + m.group(3)
                + f"<v>{escape(str(_val))}</v>"
                + m.group(4)
            )

        xml, n = pattern.subn(repl, xml)
        assert n == 1, f"formula cell {ref} not found (or not formula-only) in {sheet_part}"
    checked = _assert_every_f_has_v(xml)
    assert checked == len(cache), f"recount {checked} != injected {len(cache)} in {sheet_part}"
    entries[sheet_part] = xml.encode("utf-8")
    zip_write(path, entries)


@builder("long-tables", "xlsx")
def long_tables(ctx, nnn):
    from openpyxl import Workbook

    wb = Workbook()
    ws = wb.active
    ws.title = "Main"
    for c in range(1, 6):
        ws.cell(row=1, column=c, value=f"h{c}")
    ref_rows = [" ".join(f"h{c}" for c in range(1, 6))]
    cache = {}
    if nnn == 1:
        for r in range(2, 201):
            vals = [f"v{r}_{c}" for c in range(1, 6)]
            for c, v in enumerate(vals, start=1):
                ws.cell(row=r, column=c, value=v)
            ref_rows.append(" ".join(vals))
        notes = (
            "real single-sheet stress grid: 200 rows x 5 cols on 'Main', row 1 is the header "
            "(h1..h5) and rows 2..200 carry deterministic string values v{r}_{c}; no formulas, "
            "no merged cells — the point is volume a text extractor must not truncate. All "
            "1000 strings are in the reference text (header line + 199 data lines)."
        )
    else:
        ws.cell(row=1, column=6, value="h6")
        ref_rows[0] += " h6"
        for r in range(2, 201):
            for c in (1, 3, 4, 5):
                ws.cell(row=r, column=c, value=f"v{r}_{c}")
            ws.cell(row=r, column=2, value=r)  # numeric so =B{r}*2 evaluates
            ws.cell(row=r, column=6, value=f"=B{r}*2")
            cache[f"F{r}"] = 2 * r
            ref_rows.append(f"v{r}_1 {r} v{r}_3 v{r}_4 v{r}_5 {2 * r}")
        notes = (
            "same 200x5 grid as 001 plus column F: a real =B{r}*2 formula on every data row "
            "(199 cells). Column B holds the integer row index r in this variant (a string "
            "would make =B{r}*2 a #VALUE! error, so 002 deviates from the v{r}_{c} pattern in "
            "column B only). openpyxl writes formulas without cached values; the cached <v> "
            "(2*r, pre-computed in python) is injected by a zip rewrite so cached-value "
            "readers and recalc-on-load renderers agree; recount asserts every <f> has a <v>."
        )
    pin_xlsx_props(wb)
    path = ctx.tmp / "fixture.xlsx"
    wb.save(path)
    if cache:
        _inject_cached_values(path, "xl/worksheets/sheet1.xml", cache)
    _freeze_modified(path)
    return BuildResult(
        path=path,
        title=f"Long tables xlsx {nnn:03d}",
        reference_text="\n".join(ref_rows),
        notes=notes,
    )


@builder("formulas-cached", "xlsx")
def formulas_cached(ctx, nnn):
    from openpyxl import Workbook

    wb = Workbook()
    ws = wb.active
    cache = {}
    sheet_part = "xl/worksheets/sheet1.xml"
    if nnn == 1:
        ws.title = "Calc"
        for i, v in enumerate([2, 4, 6, 8, 10], start=1):
            ws.cell(row=i, column=4, value=v)  # D1:D5
        spec = [
            (1, "sum", "=SUM(D1:D5)", 30),
            (2, "average", "=AVERAGE(D1:D5)", 6),
            (3, "min", "=MIN(D1:D5)", 2),
            (4, "max", "=MAX(D1:D5)", 10),
        ]
        for r, label, formula, result in spec:
            ws.cell(row=r, column=1, value=label)
            ws.cell(row=r, column=2, value=formula)
            cache[f"B{r}"] = result
        ref = (
            "\n".join(f"{label} {result}" for _, label, _, result in spec)
            + "\n"
            + "2 4 6 8 10"
        )
        notes = (
            "real SUM/AVERAGE/MIN/MAX formulas over D1:D5 (2,4,6,8,10); openpyxl emits <f> "
            "with an empty <v></v> (no cached value), so the results 30/6/2/10 (computed in "
            "python) are injected into sheet1.xml by a zip rewrite — cached-value readers see "
            "the values without recalculating, LibreOffice still recalculates identically on "
            "open; recount asserts all four <f> nodes carry a non-empty <v> sibling."
        )
    elif nnn == 2:
        ws.title = "Data"
        ws["A1"] = 7
        ws["A2"] = 11
        ws["B1"] = "base-7"
        ws["B2"] = "base-11"
        refs = wb.create_sheet("Refs")
        refs["A1"] = "triple-of-7"
        refs["A2"] = "triple-of-11"
        refs["B1"] = "=Data!A1*3"
        refs["B2"] = "=Data!A2*3"
        cache = {"B1": 21, "B2": 33}
        sheet_part = "xl/worksheets/sheet2.xml"
        ref = "base-7\nbase-11\n7\n11\ntriple-of-7 21\ntriple-of-11 33"
        notes = (
            "real cross-sheet references: 'Refs'!B1/B2 hold =Data!A1*3 / =Data!A2*3 over "
            "Data!A1=7, A2=11; caches 21/33 (computed in python) injected into "
            "xl/worksheets/sheet2.xml — a cached-value extractor that only reads its own "
            "sheet still gets the numbers from <v>, cross-sheet resolution only needed for "
            "recalc; recount asserts both <f> nodes carry a <v> sibling."
        )
    else:
        ws.title = "Mix"
        ws["A1"] = 50
        ws["A2"] = "cat"
        ws["B1"] = '=IF(A1>40,"big","small")'
        ws["B2"] = '=A2&"s"'
        ws["B3"] = "=A1/100"
        ws["B3"].number_format = "0%"
        for r, label in [(1, "if"), (2, "concat"), (3, "percent")]:
            ws.cell(row=r, column=3, value=label)
        cache = {"B1": "big", "B2": "cats", "B3": 0.5}
        ref = "50\ncat\nif big\nconcat cats\npercent 0.5"
        notes = (
            "three real formula flavours with injected caches: IF(A1>40,...) -> string 'big' "
            "(needs t=\"str\" on the cell, added during the zip rewrite), A2&\"s\" string "
            "concatenation -> 'cats' (t=\"str\"), and =A1/100 displayed through a real "
            "number_format 0% (renders 50%, raw cached value 0.5 in <v>); reference text uses "
            "the raw stored values, not the formatted rendering. Recount asserts all three "
            "<f> nodes carry a <v> sibling."
        )
    pin_xlsx_props(wb)
    path = ctx.tmp / "fixture.xlsx"
    wb.save(path)
    _inject_cached_values(path, sheet_part, cache)
    _freeze_modified(path)
    return BuildResult(
        path=path, title=f"Formulas with cached values xlsx {nnn:03d}", reference_text=ref, notes=notes
    )


@builder("conditional-formatting", "xlsx")
def conditional_formatting(ctx, nnn):
    from openpyxl import Workbook
    from openpyxl.formatting.rule import CellIsRule, ColorScaleRule, FormulaRule, IconSetRule
    from openpyxl.styles import PatternFill

    wb = Workbook()
    ws = wb.active
    ws.title = "CF"
    if nnn == 1:
        values = list(range(1, 21))
        for i, v in enumerate(values, start=1):
            ws.cell(row=i, column=1, value=v)
        ws["C1"] = "cf-colorscale"
        ws.conditional_formatting.add(
            "A1:A20",
            ColorScaleRule(
                start_type="min", start_color="FFFFFF", end_type="max", end_color="63BE7B"
            ),
        )
        red = PatternFill(start_color="FFC7CE", end_color="FFC7CE", fill_type="solid")
        ws.conditional_formatting.add(
            "A1:A20", CellIsRule(operator="greaterThan", formula=["10"], fill=red)
        )
        ref = "cf-colorscale\n" + " ".join(str(v) for v in values)
        notes = (
            "two real openpyxl rules on A1:A20 (values 1..20): a two-stop colorScale "
            "(min FFFFFF -> max 63BE7B) and a cellIs greaterThan 10 rule with a solid red "
            "fill (hits 11..20) — both written as genuine <conditionalFormatting>/<cfRule> "
            "blocks in sheet1.xml, asserted to survive the _freeze_modified zip rewrite "
            "(exactly 2 <cfRule> after the rewrite)."
        )
    else:
        values = [5, 9, 5, 12, 9, 20, 5, 12, 31, 9]
        for i, v in enumerate(values, start=1):
            ws.cell(row=i, column=1, value=v)
        ws["C1"] = "cf-dupes"
        ws.conditional_formatting.add(
            "A1:A10",
            IconSetRule(icon_style="3TrafficLights1", type="num", values=[0, 10, 20], showValue=True),
        )
        yellow = PatternFill(start_color="FFF2CC", end_color="FFF2CC", fill_type="solid")
        ws.conditional_formatting.add(
            "A1:A10", FormulaRule(formula=["COUNTIF($A$1:$A$10,A1)>1"], fill=yellow)
        )
        ref = "cf-dupes\n" + " ".join(str(v) for v in values)
        notes = (
            "two real openpyxl rules on A1:A10 (values with duplicates 5/9/12): a genuine "
            "iconSet rule with the built-in 3TrafficLights1 set (num cfvos 0/10/20, "
            "showValue) plus a formula rule COUNTIF($A$1:$A$10,A1)>1 highlighting duplicates "
            "in yellow — real <cfRule type=\"iconSet\"/>/<cfRule type=\"expression\"/> XML, "
            "asserted (2 <cfRule> + the iconSet id) to survive the _freeze_modified zip "
            "rewrite."
        )
    pin_xlsx_props(wb)
    path = ctx.tmp / "fixture.xlsx"
    wb.save(path)
    _freeze_modified(path)
    sheet_xml = zip_read(path)["xl/worksheets/sheet1.xml"].decode("utf-8")
    assert sheet_xml.count("<cfRule") == 2, "conditional formatting rules lost in zip rewrite"
    if nnn == 2:
        assert "3TrafficLights1" in sheet_xml, "iconSet id lost in zip rewrite"
    return BuildResult(
        path=path,
        title=f"Conditional formatting xlsx {nnn:03d}",
        reference_text=ref,
        notes=notes,
    )


@builder("tracked-changes", "xlsx")
def tracked_changes(ctx, nnn):
    from openpyxl import Workbook

    wb = Workbook()
    ws = wb.active
    ws.title = "Revs"
    ws["A1"] = "keep-value"
    ws["A2"] = "edit-value"
    ws["C1"] = "meta-flag"
    ref = "keep-value\nedit-value\nmeta-flag"
    notes = (
        "openpyxl has no revisions API, so the xlsx change-tracking layer (§18.16.8 "
        "worksheetRevisions, ECMA-376 part 1) is injected post-save by a zip rewrite: a "
        "minimal <worksheetRevisions> block with 2 <revision> records (revisionType "
        "deleteCell + insertCell, authorId=0, date 2026-01-01T00:00:00Z, r:id 1/2) appended "
        "before </worksheet> in sheet1.xml, with the r namespace declared on the injected "
        "element (openpyxl's sheet root omits xmlns:r). Synthetic revision records, NOT "
        "Excel-authored history: there is no author table and no workbook-level revision "
        "history (xl/revisions/*.xml, CT_Workbook/revision §18.2.11), so authorId 0 is "
        "dangling and the cell grid holds only the current-state values; LO/Excel tolerance "
        "of this minimal shape varies. Reference text = the surviving cell values only."
    )
    pin_xlsx_props(wb)
    path = ctx.tmp / "fixture.xlsx"
    wb.save(path)

    r_ns = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
    block = (
        f'<worksheetRevisions xmlns:r="{r_ns}">'
        '<revision r:id="1" r:authorId="0" r:date="2026-01-01T00:00:00Z" '
        'r:revisionType="deleteCell"/>'
        '<revision r:id="2" r:authorId="0" r:date="2026-01-01T00:00:00Z" '
        'r:revisionType="insertCell"/>'
        "</worksheetRevisions>"
    )
    entries = zip_read(path)
    xml = entries["xl/worksheets/sheet1.xml"].decode("utf-8")
    assert xml.count("</worksheet>") == 1, "sheet1.xml close anchor not unique"
    xml = xml.replace("</worksheet>", block + "</worksheet>", 1)
    assert xml.count("<revision ") == 2, "revision records lost"
    entries["xl/worksheets/sheet1.xml"] = xml.encode("utf-8")
    zip_write(path, entries)
    _freeze_modified(path)
    return BuildResult(
        path=path,
        title=f"Tracked changes xlsx {nnn:03d}",
        reference_text=ref,
        notes=notes,
    )


@builder("cjk-text", "xlsx")
def cjk_text(ctx, nnn):
    from openpyxl import Workbook

    wb = Workbook()
    ws = wb.active
    ws.title = "CJK"
    poem = ["床前明月光", "疑是地上霜", "举头望明月", "低头思故乡"]
    korean = "동해물과 백두산이 마르고 닳도록"
    for i, line in enumerate(poem, start=1):
        ws.cell(row=i, column=1, value=line)
        ws.cell(row=i, column=2, value=i)
    ws["C1"] = korean
    ref = "\n".join(f"{line} {i}" for i, line in enumerate(poem, start=1)) + f"\n{korean}"
    notes = (
        "four Chinese lines of Li Bai's Quiet Night Thought (床前明月光…) in A1:A4 with plain "
        "integer row numbers 1..4 in B1:B4, plus the first line of the Korean anthem "
        "애국가 (동해물과 백두산이 마르고 닳도록) in C1; strings stored exactly as written "
        "(no transliteration, normalization or escaping by the generator), CJK per-char F1 "
        "rule applies downstream per C9 §10.6."
    )
    pin_xlsx_props(wb)
    path = ctx.tmp / "fixture.xlsx"
    wb.save(path)
    _freeze_modified(path)
    return BuildResult(
        path=path, title=f"CJK text xlsx {nnn:03d}", reference_text=ref, notes=notes
    )


@builder("custom-styles", "xlsx")
def custom_styles(ctx, nnn):
    from openpyxl import Workbook
    from openpyxl.styles import Font, NamedStyle, PatternFill

    wb = Workbook()
    ws = wb.active
    ws.title = "Styles"
    money = NamedStyle(name="GP Money")
    money.number_format = '"$"#,##0.00'
    head = NamedStyle(name="GP Head")
    head.font = Font(bold=True, color="FBF7EE")
    head.fill = PatternFill(fill_type="solid", start_color="101A2E", end_color="101A2E")
    wb.add_named_style(money)
    wb.add_named_style(head)
    headers = ["region", "q1", "q2", "total"]
    for c, h in enumerate(headers, start=1):
        cell = ws.cell(row=1, column=c, value=h)
        cell.style = "GP Head"
    data = [("north", 10.5, 20, 30.5), ("south", 5.25, 7.75, 13.0)]
    for r, (region, q1, q2, total) in enumerate(data, start=2):
        ws.cell(row=r, column=1, value=region)
        for c, v in zip((2, 3, 4), (q1, q2, total)):
            cell = ws.cell(row=r, column=c, value=v)
            cell.style = "GP Money"
    assert set(wb.named_styles) >= {"GP Money", "GP Head"}, "named styles missing on wb"
    ref = (
        " ".join(headers)
        + "\n"
        + "\n".join(f"{region} {q1} {q2} {total}" for region, q1, q2, total in data)
    )
    notes = (
        "two real openpyxl NamedStyle definitions registered on the workbook: 'GP Money' "
        "with a currency number_format (\"$\"#,##0.00) applied to the B2:D3 numeric range, "
        "and 'GP Head' (bold ink-vs-paper font + solid 101A2E fill) applied to the A1:D1 "
        "header row; wb.named_styles contains both (asserted), and after save both style "
        "names are asserted present in xl/styles.xml. Reference text lists the raw stored "
        "values, not the currency-formatted rendering."
    )
    pin_xlsx_props(wb)
    path = ctx.tmp / "fixture.xlsx"
    wb.save(path)
    _freeze_modified(path)
    styles_xml = zip_read(path)["xl/styles.xml"].decode("utf-8")
    assert 'name="GP Money"' in styles_xml and 'name="GP Head"' in styles_xml, (
        "named styles lost in styles.xml"
    )
    return BuildResult(
        path=path, title=f"Custom named styles xlsx {nnn:03d}", reference_text=ref, notes=notes
    )


@builder("comments-annotations", "xlsx")
def comments_annotations(ctx, nnn):
    from openpyxl import Workbook
    from openpyxl.comments import Comment

    wb = Workbook()
    ws = wb.active
    ws.title = "Notes"
    cells = [("A1", "line-alpha"), ("A2", "line-beta"), ("A3", "line-gamma"), ("B2", "side-delta")]
    texts = [f"Annotation {i} on {addr}." for i, (addr, _) in enumerate(cells)]
    for (addr, value), text in zip(cells, texts):
        ws[addr] = value
        ws[addr].comment = Comment(text, "stichos-bot")
    ref = "\n".join(value for _, value in cells) + "\n" + "\n".join(texts)
    notes = (
        "four real openpyxl Comment objects (author stichos-bot, fixed texts) on A1, A2, "
        "A3 and B2: openpyxl emits the CT_Comments part (authors table + commentList with "
        "ref/authorId) as xl/comments/comment1.xml (Excel itself would name it "
        "xl/comments1.xml; same part type), plus the legacy VML shape layer "
        "xl/drawings/commentsDrawing1.vml and the sheet-level relationship; comment parts "
        "carry no timestamp, so no wall-clock leaks. Comment texts are part of the reference "
        "text. Note: the metrics OOX key table is wordprocessing-only (w:*), so features{} "
        "shows zero w:comment/w:commentReference counts for xlsx — expected, the framework "
        "fills features from metrics, builders never set it."
    )
    pin_xlsx_props(wb)
    path = ctx.tmp / "fixture.xlsx"
    wb.save(path)
    _freeze_modified(path)
    entries = zip_read(path)
    cparts = [n for n in entries if "comment" in n and n.endswith(".xml")]
    assert len(cparts) == 1, f"expected exactly one comments part, got {cparts}"
    assert entries[cparts[0]].decode("utf-8").count("<comment ") == 4, (
        "comment records lost"
    )
    return BuildResult(
        path=path, title=f"Cell comments xlsx {nnn:03d}", reference_text=ref, notes=notes
    )


@builder("huge-cell-text", "xlsx")
def huge_cell_text(ctx, nnn):
    from openpyxl import Workbook

    wb = Workbook()
    ws = wb.active
    ws.title = "Huge"
    if nnn == 1:
        huge = ("lorem ipsum " * 2731)[:32767]  # exactly the ECMA-376 cell max
        ws["A1"] = huge
        ws["B1"] = "neighbor-right"
        ws["A2"] = "neighbor-below"
        assert len(ws["A1"].value) == 32767, "giant cell truncated below spec max"
        ref = huge + "\nneighbor-right\nneighbor-below"
        notes = (
            "one giant cell A1 at the ECMA-376 cell-length maximum of 32,767 chars "
            "('lorem ipsum ' repeated, truncated to exactly 32767), plus two short neighbour "
            "cells (B1, A2) that an extractor must still surface un-truncated next to the "
            "giant cell; 48k chars in one cell is impossible in a spec-valid xlsx (openpyxl "
            "truncates silently at 32767 — asserted here at exactly the max), so the "
            "reference text holds the full 32,767-char stored value; the fixture size IS the "
            "category, truncation-tolerance probe for string readers."
        )
    else:
        rows = []
        for r in range(1, 501):
            s = f"row{r:03d}" + " shared-string-stress" * 9  # ~195 chars, deterministic
            ws.cell(row=r, column=1, value=s)
            rows.append(s)
        ref = "\n".join(rows)
        notes = (
            "500 unique deterministic strings of ~200 chars each in A1:A500 — a string-"
            "volume stress for the workbook's string storage: this openpyxl build stores "
            "them inline (t=\"inlineStr\", no sharedStrings.xml part), MS/Excel writers "
            "would use a sharedStrings table, so readers must handle both paths; all 500 "
            "lines are in the reference text, no truncation allowed."
        )
    pin_xlsx_props(wb)
    path = ctx.tmp / "fixture.xlsx"
    wb.save(path)
    _freeze_modified(path)
    return BuildResult(
        path=path, title=f"Huge cell text xlsx {nnn:03d}", reference_text=ref, notes=notes
    )


@builder("embedded-ole", "xlsx")
def embedded_ole(ctx, nnn):
    from openpyxl import Workbook

    from .docx import _OLE_CT, _OLE_RELTYPE, _ole_cfb_stub, _rel_next_id

    wb = Workbook()
    ws = wb.active
    ws.title = "Embed"
    ws["A1"] = f"embedded-ole xlsx {nnn:03d}"
    ws["B1"] = "host sheet line"
    ws["A3"] = "line below the embed region"
    ref = (
        f"embedded-ole xlsx {nnn:03d}\n"
        "host sheet line\n"
        "line below the embed region"
    )
    notes = (
        "real package-level embed plumbing with a shapeless payload: xl/embeddings/object1.bin "
        "(hand-built minimal CFB, same builder as the docx fixture: D0CF11E0A1B11AE1 magic, v3, "
        "512-byte sectors, header + Root Entry directory sector + one FAT sector, no server "
        "streams) added via zip rewrite after openpyxl save, plus a workbook-part oleObject "
        "relationship in xl/_rels/workbook.xml.rels and a Content_Types oleObject override for "
        "/xl/embeddings/object1.bin. The embed is present but NOT referenced by a visible "
        "drawing — there is no xl/drawings part and no legacy OLE object on the sheet — so this "
        "fixture probes pass-through behavior: walkers that only follow sheet drawings never "
        "see it, container-level readers must still enumerate the part. No text truth in the "
        "embed; reference_text covers the three sheet cells only."
    )
    pin_xlsx_props(wb)
    path = ctx.tmp / "fixture.xlsx"
    wb.save(path)

    entries = zip_read(path)
    rels = entries["xl/_rels/workbook.xml.rels"].decode("utf-8")
    rid = _rel_next_id(rels)
    assert "</Relationships>" in rels
    rels = rels.replace(
        "</Relationships>",
        f'<Relationship Id="rId{rid}" Type="{_OLE_RELTYPE}" Target="embeddings/object1.bin"/>'
        "</Relationships>",
        1,
    )
    entries["xl/_rels/workbook.xml.rels"] = rels.encode("utf-8")
    entries["xl/embeddings/object1.bin"] = _ole_cfb_stub()

    ct = entries["[Content_Types].xml"].decode("utf-8")
    assert "/xl/embeddings/" not in ct
    ct = ct.replace(
        "</Types>",
        f'<Override PartName="/xl/embeddings/object1.bin" ContentType="{_OLE_CT}"/></Types>',
        1,
    )
    entries["[Content_Types].xml"] = ct.encode("utf-8")
    zip_write(path, entries)
    _freeze_modified(path)

    entries = zip_read(path)
    blob = entries["xl/embeddings/object1.bin"]
    assert blob[:8] == bytes.fromhex("D0CF11E0A1B11AE1"), "CFB magic lost"
    assert f'rId{rid}' in entries["xl/_rels/workbook.xml.rels"].decode("utf-8")
    return BuildResult(
        path=path, title=f"Embedded OLE object xlsx {nnn:03d}", reference_text=ref, notes=notes
    )
