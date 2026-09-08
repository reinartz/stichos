"""ODF category builders (odt/ods/odp): hand-written zip+XML.

odfpy is GPLv2 and deliberately unused (README); the package is assembled with
zipfile + string-built XML. `write_odf` below is the shared helper for the odt/ods/odp
builders (`body` selects the office:body child: office:text for odt,
office:spreadsheet for ods, office:presentation for odp); ODF §3.1.1: `mimetype` entry FIRST, STORED, exact mime, no trailing
newline (gen.core.normalize_zip re-asserts this order at framework level).

LO is strict about declared prefixes, so every emitted root element carries the
full ODF namespace set (office/text/table(+covered)/draw/style/meta/xlink/fo,
svg where used). Dates in meta.xml are fixed 2026-01-01T00:00:00 / PT0S (INV-4).
"""
from __future__ import annotations

import zipfile
from pathlib import Path
from xml.sax.saxutils import escape

from ..core import FIXED_ZIP_DT, BuildResult, builder, normalize_zip

MIME = {
    "odt": "application/vnd.oasis.opendocument.text",
    "ods": "application/vnd.oasis.opendocument.spreadsheet",
    "odp": "application/vnd.oasis.opendocument.presentation",
}

_XML_DECL = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'

# covered: uses the table namespace URI with its mandated own prefix (ODF 1.2).
_NSDECLS = (
    'xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"'
    ' xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"'
    ' xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"'
    ' xmlns:covered="urn:oasis:names:tc:opendocument:xmlns:table:1.0"'
    ' xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"'
    ' xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"'
    ' xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0"'
    ' xmlns:xlink="http://www.w3.org/1999/xlink"'
    ' xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"'
)
_DC_NS = 'xmlns:dc="http://purl.org/dc/elements/1.1/"'
_MANIFEST_NS = 'xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"'

_FIXED_META_STAMP = "2026-01-01T00:00:00"


def _meta_xml() -> str:
    return (
        _XML_DECL
        + f'<office:document-meta office:version="1.2" {_NSDECLS} {_DC_NS}>'
        "<office:meta>"
        "<meta:generator>stichos-harness</meta:generator>"
        f"<meta:creation-date>{_FIXED_META_STAMP}</meta:creation-date>"
        f"<dc:date>{_FIXED_META_STAMP}</dc:date>"
        "<meta:editing-duration>PT0S</meta:editing-duration>"
        "</office:meta>"
        "</office:document-meta>"
    )


def _master_page_styles(header: str = "", footer: str = "") -> str:
    """styles.xml with a Standard master-page (A4 page layout pm1); `header`/
    `footer` are optional inner XML (e.g. a <text:p> with fields)."""
    return (
        _XML_DECL
        + f'<office:document-styles office:version="1.2" {_NSDECLS} {_DC_NS}>'
        "<office:styles>"
        '<style:default-style style:family="paragraph">'
        '<style:text-properties fo:font-size="12pt"/>'
        "</style:default-style>"
        "</office:styles>"
        "<office:automatic-styles>"
        '<style:page-layout style:name="pm1">'
        '<style:page-layout-properties fo:page-width="8.27in" fo:page-height="11.69in"'
        ' style:num-format="1" style:print-orientation="portrait"/>'
        "</style:page-layout>"
        "</office:automatic-styles>"
        "<office:master-styles>"
        f'<style:master-page style:name="Standard" style:page-layout-name="pm1">'
        + header
        + footer
        + "</style:master-page>"
        "</office:master-styles>"
        "</office:document-styles>"
    )


def _manifest_xml(fmt: str, parts: list[str], media: dict[str, str] | None = None) -> str:
    lines = [
        _XML_DECL,
        f'<manifest:manifest {_MANIFEST_NS} manifest:version="1.2">',
        f'<manifest:file-entry manifest:full-path="/" manifest:media-type="{MIME[fmt]}"/>',
    ]
    for name in sorted(parts):
        if media and name in media:
            mt = media[name]
        else:
            mt = "text/xml" if name.endswith(".xml") else "application/octet-stream"
        lines.append(
            f'<manifest:file-entry manifest:full-path="{name}" manifest:media-type="{mt}"/>'
        )
    lines.append("</manifest:manifest>")
    return "".join(lines)


def write_odf(
    ctx,
    fmt: str,
    content: str,
    extra_parts: dict[str, str | tuple[bytes, str]] | None = None,
    automatic: str = "",
    body: str = "text",
) -> Path:
    """Assemble an ODF package at ctx.tmp/fixture.<fmt>: mimetype (exact mime, no
    trailing newline), content.xml (content = inner XML wrapped in
    <office:body><office:{body}> — office:text for odt, office:spreadsheet for ods;
    `automatic` goes into office:automatic-styles), styles.xml, meta.xml (fixed dates),
    META-INF/manifest.xml. extra_parts overrides/adds zip entries: a str value is
    a text part (e.g. a custom "styles.xml"), a (bytes, media-type) tuple is a
    binary part whose manifest entry uses that media-type. Manifest lists every
    part except mimetype (plus the "/" root entry) with its media-type."""
    assert fmt in MIME, f"unknown odf format {fmt!r}"
    aut = (
        "<office:automatic-styles/>"
        if not automatic
        else f"<office:automatic-styles>{automatic}</office:automatic-styles>"
    )
    content_xml = (
        _XML_DECL
        + f'<office:document-content office:version="1.2" {_NSDECLS} {_DC_NS}>'
        + aut
        + f"<office:body><office:{body}>"
        + content
        + f"</office:{body}></office:body>"
        "</office:document-content>"
    )
    entries: dict[str, str | bytes] = {
        "content.xml": content_xml,
        "styles.xml": _master_page_styles(),
        "meta.xml": _meta_xml(),
    }
    media: dict[str, str] = {}
    if extra_parts:
        for name, value in extra_parts.items():
            if isinstance(value, tuple):
                entries[name], media[name] = value
            else:
                entries[name] = value
    entries["META-INF/manifest.xml"] = _manifest_xml(fmt, list(entries), media)
    path = ctx.tmp / f"fixture.{fmt}"
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as zf:
        zi = zipfile.ZipInfo("mimetype", date_time=FIXED_ZIP_DT)
        zi.compress_type = zipfile.ZIP_STORED
        zi.external_attr = 0o644 << 16
        zf.writestr(zi, MIME[fmt])
        for name in sorted(entries):
            zi = zipfile.ZipInfo(name, date_time=FIXED_ZIP_DT)
            zi.compress_type = zipfile.ZIP_DEFLATED
            zi.external_attr = 0o644 << 16
            data = entries[name]
            zf.writestr(zi, data.encode("utf-8") if isinstance(data, str) else data)
    return path


def _cell(text: str = "", attrs: str = "") -> str:
    if text == "":
        return "<table:table-cell/>"
    return (
        f'<table:table-cell{attrs} office:value-type="string"'
        f' office:string-value="{escape(text)}"><text:p>{escape(text)}</text:p></table:table-cell>'
    )


_COVERED = "<covered:table-cell/>"


@builder("merged-cells", "odt")
def merged_cells(ctx, nnn):
    rows = [
        "<table:table-row>"
        + _cell("A1", ' table:number-rows-spanned="2"')
        + _cell("B1")
        + _cell("C1")
        + "</table:table-row>",
        "<table:table-row>" + _COVERED + _cell("B2") + _cell("C2") + "</table:table-row>",
        "<table:table-row>"
        + _cell("A3", ' table:number-columns-spanned="2"')
        + _COVERED
        + _cell("C3")
        + "</table:table-row>",
    ]
    table = (
        '<table:table table:name="MergedGrid">'
        '<table:table-column table:number-columns-repeated="3"/>'
        + "".join(rows)
        + "</table:table>"
    )
    title = f"merged-cells odt {nnn:03d}"
    content = f"<text:p>{escape(title)}</text:p>" + table
    ref = f"{title}\nA1 B1 C1\nB2 C2\nA3 C3"
    notes = (
        "real ODF covered-cell merges (ODF 1.2 §9.4): table:number-rows-spanned=\"2\" on cell "
        "A1 with a covered:table-cell placeholder in the same column of row 2, plus "
        "table:number-columns-spanned=\"2\" on A3 with a covered:table-cell to its right in row "
        "3; covered: is the mandated prefix for the table namespace URI. Reference text lists "
        "each surviving cell's text once in row-major order (covered cells carry no text)."
    )
    return BuildResult(path=write_odf(ctx, "odt", content), title=f"Merged cells grid odt {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("long-tables", "odt")
def long_tables(ctx, nnn):
    intro = f"long-tables odt {nnn:03d}: intro paragraph before the 120-row grid."
    closing = "Closing paragraph after the 120-row grid."
    rows_xml, ref_rows = [], []
    for r in range(120):
        texts = [f"r{r}c{c}" for c in range(4)]
        rows_xml.append(
            "<table:table-row>" + "".join(_cell(t) for t in texts) + "</table:table-row>"
        )
        ref_rows.append(" ".join(texts))
    table = (
        '<table:table table:name="LongGrid">'
        '<table:table-column table:number-columns-repeated="4"/>'
        + "".join(rows_xml)
        + "</table:table>"
    )
    content = (
        f"<text:p>{escape(intro)}</text:p>" + table + f"<text:p>{escape(closing)}</text:p>"
    )
    ref = "\n".join([intro] + ref_rows + [closing])
    notes = (
        "real table stress, hand-written ODF: one table:table with 120 table:table-rows x 4 "
        "table:table-cells, every cell carrying r{r}c{c} text, intro + closing text:p around "
        "it so LibreOffice paginates past two pages; all cell strings are in the reference text."
    )
    return BuildResult(path=write_odf(ctx, "odt", content), title=f"Long tables odt {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("footnotes-endnotes", "odt")
def footnotes_endnotes(ctx, nnn):
    cls = "footnote" if nnn == 1 else "endnote"
    n = 3 if nnn == 1 else 2
    prefix = "ftn" if nnn == 1 else "etn"
    greek = ["alpha", "beta", "gamma"]
    note_texts = [
        f"{cls.capitalize()} {i} text: supporting note for {greek[i - 1]}." for i in range(1, n + 1)
    ]
    body_lines, paras = [], []
    for i in range(1, n + 1):
        head, tail = f"Body claim {i} cites a {cls}", f" marker of type {i}."
        paras.append(
            f"<text:p>{escape(head)}"
            f'<text:note text:id="{prefix}{i}" text:note-class="{cls}">'
            f"<text:note-citation>{i}</text:note-citation>"
            f"<text:note-body><text:p>{escape(note_texts[i - 1])}</text:p></text:note-body>"
            f"</text:note>{escape(tail)}</text:p>"
        )
        body_lines.append(head + tail)
    title = f"footnotes-endnotes odt {nnn:03d}"
    content = f"<text:p>{escape(title)}</text:p>" + "".join(paras)
    ref = "\n".join([title] + body_lines + note_texts)
    notes = (
        f"real ODF notes: inline text:note (text:id {prefix}1..{prefix}{n}, "
        f"text:note-class=\"{cls}\") with text:note-citation + text:note-body/text:p, "
        "splitting the body paragraph around the note. Citation markers are authored digits: "
        "LibreOffice may render citations differently (numbering/format), so the reference "
        f"text covers the two body halves and the note bodies only, never the citation digit."
    )
    return BuildResult(path=write_odf(ctx, "odt", content),
                       title=f"Footnotes and endnotes odt {nnn:03d}", reference_text=ref, notes=notes)


@builder("headers-fields", "odt")
def headers_fields(ctx, nnn):
    title = f"headers-fields odt {nnn:03d}"
    filler = "Body paragraph {i:02d} of 60: fixed filler sentence for pagination stress. " * 3
    body = [filler.format(i=i) for i in range(1, 61)]
    if nnn == 1:
        header = (
            "<style:header><text:p>"
            '<text:field office:value-type="integer" office:value="1"'
            ' text:field-name="page-number">1</text:field>'
            " / "
            '<text:field office:value-type="integer" office:value="1"'
            ' text:field-name="page-count">1</text:field>'
            "</text:p></style:header>"
        )
        footer = ""
        feat = (
            "header carries two text:field elements (text:field-name page-number / page-count, "
            "integer value-type, cached display \"1 / 1\")"
        )
    else:
        header = ""
        footer = (
            "<style:footer><text:p>"
            '<text:date-time office:value-type="date" office:date-value="2026-01-01"'
            ' text:fixed="true">2026-01-01</text:date-time>'
            "</text:p></style:footer>"
        )
        feat = (
            "footer carries a text:date-time field with text:fixed=\"true\" and fixed cached "
            "text 2026-01-01 (office:date-value pinned, deterministic)"
        )
    content = f"<text:p>{escape(title)}</text:p>" + "".join(
        f"<text:p>{escape(p)}</text:p>" for p in body
    )
    ref = "\n".join([title] + body)
    notes = (
        f"real ODF master-page header/footer in styles.xml (style:master-page \"Standard\", A4 "
        f"page-layout pm1): {feat}. Honest caveats: (a) the normative text:field counter scans "
        "content.xml only, so these styles.xml field elements count 0 in the manifest; (b) the "
        "page fields are generic text:field user-field elements carrying cached display text, "
        "not dedicated text:page-number/text:page-count — LibreOffice shows the cached values; "
        "(c) header/footer strings are excluded from the reference text because renderers "
        "re-evaluate/re-position header content per page."
    )
    return BuildResult(path=write_odf(ctx, "odt", content, {"styles.xml": _master_page_styles(header, footer)}),
                       title=f"Headers and fields odt {nnn:03d}", reference_text=ref, notes=notes)


_TOC_PLACEHOLDER = "Table of contents placeholder — field not evaluated"


def _toc_block() -> str:
    return (
        '<text:table-of-content text:name="Contents1">'
        '<text:toc-source text:outline-level="2" text:use-outline-level="true"/>'
        "<text:toc-body>"
        f"<text:p>{escape(_TOC_PLACEHOLDER)}</text:p>"
        "</text:toc-body>"
        "</text:table-of-content>"
    )


def _toc_styles() -> str:
    base = _master_page_styles()
    styles = (
        '<style:style style:name="TocHeading1" style:family="paragraph"'
        ' style:default-outline-level="1">'
        '<style:text-properties fo:font-size="16pt" fo:font-weight="bold"/></style:style>'
        '<style:style style:name="TocHeading2" style:family="paragraph"'
        ' style:default-outline-level="2">'
        '<style:text-properties fo:font-size="14pt" fo:font-weight="bold"/></style:style>'
    )
    marker = "</style:default-style>"
    assert marker in base
    return base.replace(marker, marker + styles, 1)


@builder("toc-fields", "odt")
def toc_fields(ctx, nnn):
    title = f"toc-fields odt {nnn:03d}"
    headings = [
        (1, f"Chapter {nnn}0 Introduction"),
        (2, f"Chapter {nnn}0 Purpose"),
        (1, f"Chapter {nnn}1 Design"),
        (2, f"Chapter {nnn}1 Layout"),
        (1, f"Chapter {nnn}2 Implementation"),
        (2, f"Chapter {nnn}2 Build"),
        (1, f"Chapter {nnn}3 Validation"),
        (2, f"Chapter {nnn}3 Results"),
    ]
    paras = [
        f'<text:h text:style-name="TocHeading{lvl}" text:outline-level="{lvl}">{escape(t)}</text:h>'
        for lvl, t in headings
    ]
    link_line = None
    if nnn == 2:
        link_line = "Reference: example.org external reference end of paragraph."
        paras.append(
            f"<text:p>Reference: <text:a xlink:type=\"simple\""
            " xlink:href=\"https://example.org\">example.org external reference</text:a>"
            " end of paragraph.</text:p>"
        )
    content = (
        f"<text:p>{escape(title)}</text:p>" + _toc_block() + "".join(paras)
    )
    ref = "\n".join([title, _TOC_PLACEHOLDER] + [t for _, t in headings] + ([link_line] if link_line else []))
    notes = (
        "real text:table-of-content with text:toc-source (outline level 2) and a text:toc-body "
        "holding a single placeholder paragraph (unpopulated TOC; LibreOffice does not "
        "auto-update on load), plus 8 text:h headings with text:outline-level 1/2 whose custom "
        "paragraph styles also carry style:default-outline-level"
        + (
            "; 002 adds a real text:a hyperlink (xlink:type=\"simple\", "
            "xlink:href=\"https://example.org\", external ref) — only its visible text is "
            "reference text, the URL is not"
            if nnn == 2
            else ""
        )
        + ". Honest caveat: none of TOC/headings/hyperlink are in the five normative ODF "
        "counters (table:table, draw:frame, text:note, text:field, office:annotation), so "
        "feature counts stay 0."
    )
    return BuildResult(path=write_odf(ctx, "odt", content, {"styles.xml": _toc_styles()}),
                       title=f"TOC fields odt {nnn:03d}", reference_text=ref, notes=notes)


def _change_info(creator: str) -> str:
    return (
        "<office:change-info>"
        f"<dc:creator>{creator}</dc:creator>"
        f"<dc:date>{_FIXED_META_STAMP}</dc:date>"
        "</office:change-info>"
    )


def _ins_region(rid: str, creator: str, para: str) -> str:
    return (
        f'<text:changed-region text:id="{rid}"><text:insert>{_change_info(creator)}'
        f"<text:p>{escape(para)}</text:p>"
        "</text:insert></text:changed-region>"
    )


def _del_region(rid: str, creator: str, text: str) -> str:
    return (
        f'<text:changed-region text:id="{rid}"><text:delete>{_change_info(creator)}'
        f"{escape(text)}</text:delete></text:changed-region>"
    )


@builder("tracked-changes", "odt")
def tracked_changes(ctx, nnn):
    title = f"tracked-changes odt {nnn:03d}"
    if nnn == 1:
        ins_a = "Inserted paragraph by bot-a covering the design section."
        content = (
            f"<text:p>{escape(title)}</text:p>"
            + "<text:p>Base text"
            + _del_region("ct2", "bot-a", " (draft clause)")
            + " remains after the deletion.</text:p>"
            + _ins_region("ct1", "bot-a", ins_a)
        )
        ref = "\n".join([title, "Base text remains after the deletion.", ins_a])
        regions, ins_n, a_n, b_n = 2, 1, 2, 0
        who = "single author bot-a on both regions"
    else:
        ins_a = "Alpha insert by bot-a."
        ins_b = "Beta insert by bot-b."
        content = (
            f"<text:p>{escape(title)}</text:p>"
            + "<text:p>Section alpha revised"
            + _del_region("ct4", "bot-a", " alpha-note-deleted")
            + " stable tail.</text:p>"
            + _ins_region("ct3", "bot-a", ins_a)
            + "<text:p>Section beta revised"
            + _del_region("ct2", "bot-b", " beta-note-deleted")
            + " stable tail.</text:p>"
            + _ins_region("ct1", "bot-b", ins_b)
        )
        ref = "\n".join(
            [
                title,
                "Section alpha revised stable tail.",
                ins_a,
                "Section beta revised stable tail.",
                ins_b,
            ]
        )
        regions, ins_n, a_n, b_n = 4, 2, 2, 2
        who = 'two authors (bot-a owns the alpha insert/delete pair, bot-b the beta pair)'
    path = write_odf(ctx, "odt", content)
    xml = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert xml.count("<text:changed-region ") == regions
    assert xml.count("<text:insert>") == ins_n and xml.count("<text:delete>") == ins_n
    assert xml.count("<office:change-info>") == regions
    assert xml.count("<dc:creator>bot-a</dc:creator>") == a_n
    assert xml.count("<dc:creator>bot-b</dc:creator>") == b_n
    assert xml.count(_FIXED_META_STAMP) == regions
    notes = (
        f"real ODF tracked revisions (ODF 1.2 §2.4): {who}; text:changed-region ids ct1..ct{regions} "
        f"with {ins_n} text:insert (wrapping a whole inserted text:p) and {ins_n} text:delete "
        "(inline in the base paragraph, deleted words carried as text inside text:delete), each "
        "with office:change-info (dc:creator + fixed dc:date 2026-01-01T00:00:00). Reference text "
        "states the ACCEPTED view: inserted paragraph text is included, deleted fragment text is "
        "excluded (matches the docx tracked-changes convention). None of the five normative ODF "
        "counters match changed-region/insert/delete, so feature counts stay 0; whether "
        "LibreOffice shows or hides the revision marks at render time is a per-view setting."
    )
    return BuildResult(path=path, title=f"Tracked changes odt {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("comments-annotations", "odt")
def comments_annotations(ctx, nnn):
    title = f"comments-annotations odt {nnn:03d}"
    n = 4
    authors = ["bot-a", "bot-b"]
    annos, claims, comment_texts = [], [], []
    for i in range(1, n + 1):
        creator = authors[i % 2]
        ctext = f"Comment {i}: reviewer note on claim {i}."
        comment_texts.append(ctext)
        claims.append(f"Claim {i} opens anchor-text-{i} closes claim {i}.")
        annos.append(
            f"<text:p>Claim {i} opens "
            f'<office:annotation office:name="anno{i}">'
            f"<dc:creator>{creator}</dc:creator>"
            f"<dc:date>{_FIXED_META_STAMP}</dc:date>"
            f"<text:p>{escape(ctext)}</text:p>"
            f"</office:annotation>"
            f"anchor-text-{i} closes claim {i}.</text:p>"
        )
    content = f"<text:p>{escape(title)}</text:p>" + "".join(annos)
    ref = "\n".join([title] + claims + comment_texts)
    path = write_odf(ctx, "odt", content)
    xml = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert xml.count("<office:annotation ") == n
    for i in range(1, n + 1):
        assert xml.count(f'<office:annotation office:name="anno{i}">') == 1
    assert "office:annotation-end" not in xml
    notes = (
        f"real ODF comments: {n} inline office:annotation elements (office:name anno1..anno{n}, "
        "alternating dc:creator bot-a/bot-b, fixed dc:date 2026-01-01T00:00:00), each holding the "
        "annotation body as text:p and anchored inline between the runs of its claim paragraph "
        "(point annotation — no annotation-start/end, so only the four annotation elements exist). "
        "The normative office:annotation counter scans content.xml, so the manifest must read "
        "office:annotation=4. Reference text includes every annotation body string (all visible "
        "strings, annotation text included) — honest caveat: PDF export may omit comment bubbles "
        "unless the renderer is told to export annotations."
    )
    return BuildResult(path=path, title=f"Comments and annotations odt {nnn:03d}",
                       reference_text=ref, notes=notes)


def _named_styles_xml(extra: str) -> str:
    base = _master_page_styles()
    marker = "</style:default-style>"
    assert marker in base
    return base.replace(marker, marker + extra, 1)


@builder("custom-styles", "odt")
def custom_styles(ctx, nnn):
    title = f"custom-styles odt {nnn:03d}"
    if nnn == 1:
        named = (
            '<style:style style:name="GP Body" style:family="paragraph">'
            '<style:text-properties fo:font-size="12pt"/></style:style>'
            '<style:style style:name="GP Head" style:family="paragraph"'
            ' style:parent-style-name="GP Body">'
            '<style:text-properties fo:font-weight="bold" fo:font-size="16pt"/>'
            "</style:style>"
        )
        texts = [
            title,
            "Body paragraph one rendered through GP Body.",
            "Body paragraph two rendered through GP Body.",
        ]
        content = (
            f'<text:p text:style-name="GP Head">{escape(texts[0])}</text:p>'
            + f'<text:p text:style-name="GP Body">{escape(texts[1])}</text:p>'
            + f'<text:p text:style-name="GP Body">{escape(texts[2])}</text:p>'
        )
        ref = "\n".join(texts)
        checks = (
            'style:name="GP Body" style:family="paragraph"',
            'style:name="GP Head" style:family="paragraph" style:parent-style-name="GP Body"',
            'fo:font-size="12pt"',
            'fo:font-weight="bold" fo:font-size="16pt"',
        )
        feat = (
            'named paragraph styles in styles.xml office:styles: "GP Body" (style:family='
            '"paragraph", fo:font-size 12pt) and "GP Head" (family paragraph, '
            'style:parent-style-name="GP Body", bold 16pt), applied from content.xml via '
            "text:p text:style-name (named, not automatic, styles)"
        )
    else:
        named = (
            '<style:style style:name="GP Base" style:family="paragraph">'
            '<style:text-properties fo:font-size="12pt"/></style:style>'
            '<style:style style:name="GP Mid" style:family="paragraph"'
            ' style:parent-style-name="GP Base">'
            '<style:text-properties fo:font-style="italic"/></style:style>'
            '<style:style style:name="GP Leaf" style:family="paragraph"'
            ' style:parent-style-name="GP Mid">'
            '<style:text-properties fo:font-size="14pt"/></style:style>'
            '<style:style style:name="GP Char" style:family="character">'
            '<style:text-properties fo:font-weight="bold"/></style:style>'
        )
        texts = [
            title,
            "GP Mid chain paragraph, italic 12pt through inheritance.",
            "Leaf chain paragraph with bolded character style inside.",
        ]
        content = (
            f'<text:p text:style-name="GP Leaf">{escape(texts[0])}</text:p>'
            + f'<text:p text:style-name="GP Mid">{escape(texts[1])}</text:p>'
            + f'<text:p text:style-name="GP Leaf">Leaf chain paragraph with '
            f'<text:span text:style-name="GP Char">bolded character style</text:span>'
            f" inside.</text:p>"
        )
        ref = "\n".join(texts)
        checks = (
            'style:name="GP Base" style:family="paragraph"',
            'style:name="GP Mid" style:family="paragraph" style:parent-style-name="GP Base"',
            'style:name="GP Leaf" style:family="paragraph" style:parent-style-name="GP Mid"',
            'style:name="GP Char" style:family="character"',
        )
        feat = (
            "3-level style:parent-style-name inheritance chain GP Base (12pt) -> GP Mid "
            "(italic) -> GP Leaf (14pt), plus a CHARACTER-family style GP Char "
            "(style:family=\"character\", fo:font-weight bold) applied inline via "
            "text:span text:style-name"
        )
    path = write_odf(
        ctx, "odt", content, {"styles.xml": _named_styles_xml(named)}
    )
    sx = zipfile.ZipFile(path).read("styles.xml").decode("utf-8")
    for c in checks:
        assert c in sx, f"missing in styles.xml: {c}"
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert "text:style-name=" in cx
    if nnn == 2:
        assert '<text:span text:style-name="GP Char">' in cx
    notes = (
        f"real ODF named styles: {feat}. None of the five normative ODF counters match style "
        "elements, so feature counts stay 0; reference text is every visible paragraph string "
        "(style names themselves are not visible text)."
    )
    return BuildResult(path=path, title=f"Custom styles odt {nnn:03d}",
                       reference_text=ref, notes=notes)


def _lvl_number(level: int, fmt: str, start: str | None = None) -> str:
    start_attr = f' text:start-value="{start}"' if start else ""
    pos = {1: "706", 2: "1412", 3: "2118"}[level]
    return (
        f'<text:list-level-style-number text:level="{level}" text:num-format="{fmt}"'
        f' style:num-suffix="."{start_attr}>'
        f'<style:list-level-properties text:list-level-position="{pos}"'
        ' text:list-space-retrieve="false" text:list-space="353"/>'
        "<style:text-properties/>"
        "</text:list-level-style-number>"
    )


def _num_list(inner: str, style: str | None = None, restart: bool = False) -> str:
    attrs = (f' text:style-name="{style}"' if style else "") + (
        ' text:continue-numbering="false"' if restart else ""
    )
    return f"<text:list{attrs}>{inner}</text:list>"


def _num_item(text: str, children: str = "") -> str:
    return f"<text:list-item><text:p>{escape(text)}</text:p>{children}</text:list-item>"


@builder("numbering-restarts", "odt")
def numbering_restarts(ctx, nnn):
    title = f"numbering-restarts odt {nnn:03d}"
    if nnn == 1:
        named = (
            '<text:list-style style:name="L1">'
            + _lvl_number(1, "1", "1")
            + "</text:list-style>"
        )
        alpha = [f"Alpha item {i}" for i in range(1, 6)]
        bravo = [f"Bravo item {i}" for i in range(1, 6)]
        content = (
            f"<text:p>{escape(title)}</text:p>"
            + _num_list("".join(_num_item(t) for t in alpha), style="L1")
            + _num_list("".join(_num_item(t) for t in bravo), style="L1", restart=True)
        )
        ref = "\n".join([title] + alpha + bravo)
        feat = (
            'text:list-style "L1" in styles.xml (level 1 text:list-level-style-number, '
            'text:num-format="1", text:start-value="1"), two text:list of 5 text:list-items '
            'each referencing it; the second list carries text:continue-numbering="false" so '
            "its counter restarts at 1"
        )
        restarts = 1
    else:
        named = (
            '<text:list-style style:name="L2">'
            + _lvl_number(1, "1", "1")
            + _lvl_number(2, "a")
            + _lvl_number(3, "i")
            + "</text:list-style>"
        )
        tree = _num_list(
            _num_item(
                "Tree item 1",
                _num_list(
                    _num_item("Tree item 2", _num_list(_num_item("Tree item 3")))
                    + _num_item("Tree item 4")
                ),
            )
            + _num_item("Tree item 5"),
            style="L2",
        )
        restart = _num_list(
            _num_item("Restart item 1", _num_list(_num_item("Restart item 2")))
            + _num_item("Restart item 3"),
            style="L2",
            restart=True,
        )
        items = [
            "Tree item 1",
            "Tree item 2",
            "Tree item 3",
            "Tree item 4",
            "Tree item 5",
            "Restart item 1",
            "Restart item 2",
            "Restart item 3",
        ]
        content = f"<text:p>{escape(title)}</text:p>" + tree + restart
        ref = "\n".join([title] + items)
        feat = (
            'text:list-style "L2" declaring three levels — text:num-format "1" / "a" / "i" '
            "(decimal/lowercase-letter/lowercase-roman, start-value 1 on level 1) — used by a "
            "3-deep nested text:list-item tree and a second top-level list with "
            'text:continue-numbering="false" restarting the counters'
        )
        restarts = 1
    path = write_odf(ctx, "odt", content, {"styles.xml": _named_styles_xml(named)})
    sx = zipfile.ZipFile(path).read("styles.xml").decode("utf-8")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert '<text:list-style style:name="L' in sx
    assert 'text:num-format="1"' in sx and 'text:start-value="1"' in sx
    if nnn == 1:
        assert cx.count('text:list text:style-name="L1"') == 2
    else:
        assert 'text:num-format="a"' in sx and 'text:num-format="i"' in sx
        assert "<text:list><text:list" in cx
    assert cx.count('text:continue-numbering="false"') == restarts
    notes = (
        f"real ODF list numbering: {feat}. ODF restart semantics stated honestly: "
        "text:continue-numbering DEFAULTS to false, so a sibling text:list sharing a list-style "
        "does not continue the previous counter anyway — the explicit "
        'text:continue-numbering="false" pins the restart (continuation would require "true"); '
        "text:start-value on the level style sets the per-level start. Reference text is the "
        "exact item strings only — the number markers are render-derived, never in the file as "
        "text. No normative counter matches list elements, so feature counts stay 0."
    )
    return BuildResult(path=path, title=f"Numbering restarts odt {nnn:03d}",
                       reference_text=ref, notes=notes)


try:  # reuse the docx builder's CFB stub when the docx module is importable
    from .docx import _ole_cfb_stub
except ImportError:  # fallback duplicate: same hand-built minimal CFB (v3)

    def _ole_cfb_stub() -> bytes:
        import struct

        endofchain, fatsect, freesect = 0xFFFFFFFE, 0xFFFFFFFD, 0xFFFFFFFF
        hdr = bytearray(512)
        hdr[0:8] = bytes.fromhex("D0CF11E0A1B11AE1")
        struct.pack_into("<HH", hdr, 24, 0x003E, 0x0003)
        struct.pack_into("<H", hdr, 28, 0xFFFE)
        struct.pack_into("<HH", hdr, 30, 9, 6)
        struct.pack_into("<I", hdr, 40, 0)
        struct.pack_into("<I", hdr, 44, 1)
        struct.pack_into("<I", hdr, 48, 0)
        struct.pack_into("<I", hdr, 52, 0)
        struct.pack_into("<I", hdr, 56, 4096)
        struct.pack_into("<I", hdr, 60, endofchain)
        struct.pack_into("<I", hdr, 64, 0)
        struct.pack_into("<I", hdr, 68, endofchain)
        struct.pack_into("<I", hdr, 72, 0)
        struct.pack_into("<I", hdr, 76, 1)
        for i in range(1, 109):
            struct.pack_into("<I", hdr, 76 + 4 * i, freesect)
        d = bytearray(512)
        name = "Root Entry".encode("utf-16-le")
        d[0 : len(name)] = name
        struct.pack_into("<H", d, 64, len(name) + 2)
        d[66] = 0x05
        d[67] = 0x01
        for off in (68, 72, 76):
            struct.pack_into("<I", d, off, 0xFFFFFFFF)
        struct.pack_into("<I", d, 116, endofchain)
        struct.pack_into("<Q", d, 120, 0)
        fat = bytearray(512)
        struct.pack_into("<I", fat, 0, endofchain)
        struct.pack_into("<I", fat, 4, fatsect)
        for i in range(2, 128):
            struct.pack_into("<I", fat, 4 * i, freesect)
        return bytes(hdr + d + fat)


_SVG_NS = 'xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"'


@builder("embedded-ole", "odt")
def embedded_ole(ctx, nnn):
    title = f"embedded-ole odt {nnn:03d}"
    lead = "This paragraph precedes an embedded legacy OLE object part."
    tail = "Body line after the embedded object."
    frame = (
        f'<draw:frame draw:name="OleObject1" text:anchor-type="as-char" {_SVG_NS}'
        ' svg:width="5cm" svg:height="5cm" draw:opacity="100%">'
        '<draw:object draw:mime-type="application/oleobject"'
        ' draw:object-type="application/oleobject"'
        ' xlink:type="simple" xlink:href="object.bin"'
        ' xlink:show="embed" xlink:actuate="onLoad"/>'
        "</draw:frame>"
    )
    content = (
        f"<text:p>{escape(title)}</text:p>"
        + f"<text:p>{escape(lead)}</text:p>"
        + f"<text:p>{frame}</text:p>"
        + f"<text:p>{escape(tail)}</text:p>"
    )
    ref = "\n".join([title, lead, tail])
    path = write_odf(
        ctx, "odt", content, {"object.bin": (_ole_cfb_stub(), "application/x-ole-object")}
    )
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    mf = zipfile.ZipFile(path).read("META-INF/manifest.xml").decode("utf-8")
    assert cx.count("<draw:frame ") == 1 and '<draw:object ' in cx
    assert 'xlink:href="object.bin"' in cx
    assert 'manifest:media-type="application/x-ole-object"' in mf
    notes = (
        'legacy OLE embed in ODF: real binary part object.bin (hand-built minimal CFB v3 '
        'stub, imported _ole_cfb_stub from the docx builders) with manifest media-type '
        'application/x-ole-object, referenced by draw:frame (as-char anchor, svg 5cm x 5cm, '
        'draw:opacity="100%") > draw:object (draw:mime-type + draw:object-type '
        '"application/oleobject", xlink:type simple, xlink:href object.bin, show=embed, '
        'actuate=onLoad). Honest caveat: the CFB stub carries no real payload/streams, so '
        'LibreOffice may show an icon placeholder or render nothing for the object — '
        'reference text covers only the three surrounding text paragraphs.'
    )
    return BuildResult(path=path, title=f"Embedded OLE object odt {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("scanned-ocr", "odt")
def scanned_ocr(ctx, nnn):
    from .docx import _sc_scan_png

    png_path = ctx.tmp / "scan-page.png"
    _sc_scan_png(png_path, ctx.rng.randrange(1 << 32))
    caption = "Figure 1 — scanned fixture"
    frame = (
        f'<draw:frame draw:name="Scan1" text:anchor-type="as-char" {_SVG_NS}'
        ' svg:width="16cm" svg:height="22cm">'
        '<draw:image xlink:type="simple" xlink:href="Pictures/scan1.png"'
        ' xlink:show="embed" xlink:actuate="onLoad"/>'
        "</draw:frame>"
    )
    content = f"<text:p>{frame}</text:p>" + f"<text:p>{escape(caption)}</text:p>"
    path = write_odf(
        ctx, "odt", content, {"Pictures/scan1.png": (png_path.read_bytes(), "image/png")}
    )
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    mf = zipfile.ZipFile(path).read("META-INF/manifest.xml").decode("utf-8")
    assert cx.count("<draw:frame ") == 1 and '<draw:image ' in cx
    assert 'xlink:href="Pictures/scan1.png"' in cx
    assert 'manifest:media-type="image/png"' in mf
    notes = (
        "scanned-page emulation in ODF: synthetic 300x360 raster (off-white paper gray, gray "
        "pseudo-text word bars, seeded per-pixel noise — same PIL/numpy recipe as the docx "
        "fixture, noise seed derived from the fixture-seeded ctx.rng) stored as real PNG bytes "
        "at Pictures/scan1.png (manifest media-type image/png), shown in a draw:frame > "
        'draw:image (xlink:type="simple", 16cm x 22cm as-char anchor) with one caption line '
        "below. The picture has no text layer (bars, not glyphs), so there is no OCR truth "
        "inside it: reference text is the caption ONLY — the honest near-None case."
    )
    return BuildResult(path=path, title=f"Scanned page odt {nnn:03d}",
                       reference_text=caption, notes=notes)


_TW_LINE = (
    "Wrap line {i:02d}: fixed filler sentence that should flow around the anchored "
    "picture and keep the wrap boundary exercised across several lines. "
)


@builder("image-textwrap", "odt")
def image_textwrap(ctx, nnn):
    import io

    from PIL import Image, ImageDraw

    img = Image.new("RGB", (160, 120), (244, 241, 234))
    draw = ImageDraw.Draw(img)
    draw.rectangle([3, 3, 156, 116], outline=(147, 160, 184), width=4)
    buf = io.BytesIO()
    img.save(buf, "PNG")
    title = f"image-textwrap odt {nnn:03d}"
    head = "Text before the anchored picture in the same paragraph."
    tail = " text after the anchored picture."
    lines = [_TW_LINE.format(i=i) for i in range(1, 7)]
    automatic = (
        '<style:style style:name="GpFloat1" style:family="graphic">'
        '<style:graphic-properties style:paragraph-anchor="paragraph"'
        ' style:vertical-rel="paragraph" style:vertical-pos="top"'
        ' style:horizontal-rel="paragraph" style:horizontal-pos="left"'
        ' style:wrap-influence-on-position="once"/></style:style>'
    )
    frame = (
        f'<draw:frame draw:name="WrapPic1" draw:style-name="GpFloat1"'
        f' text:anchor-type="paragraph" {_SVG_NS}'
        ' svg:width="4cm" svg:height="3cm">'
        '<draw:image xlink:type="simple" xlink:href="Pictures/wrap1.png"'
        ' xlink:show="embed" xlink:actuate="onLoad"/>'
        "</draw:frame>"
    )
    content = (
        f"<text:p>{escape(title)}</text:p>"
        + f"<text:p>{escape(head)}{frame}{escape(tail)}</text:p>"
        + "".join(f"<text:p>{escape(line)}</text:p>" for line in lines)
    )
    ref = "\n".join(
        [title, head + tail.strip()] + [line.strip() for line in lines]
    )
    path = write_odf(
        ctx, "odt", content,
        {"Pictures/wrap1.png": (buf.getvalue(), "image/png")}, automatic=automatic,
    )
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert cx.count("<draw:frame ") == 1
    assert 'text:anchor-type="paragraph"' in cx
    assert 'draw:style-name="GpFloat1"' in cx
    assert 'style:wrap-influence-on-position="once"' in cx
    notes = (
        "real anchored picture with text around it: deterministic solid-color 160x120 PNG "
        "(PIL, no rng needed) at Pictures/wrap1.png (manifest media-type image/png) inside "
        "draw:frame > draw:image, paragraph-anchored (text:anchor-type=\"paragraph\") via an "
        "automatic family=\"graphic\" style GpFloat1 carrying style:paragraph-anchor + "
        "paragraph-relative x/y positions and style:wrap-influence-on-position=\"once\"; the "
        "frame sits inside a paragraph whose runs continue after it, six filler paragraphs "
        "follow. Honest caveat: ODF wrap semantics (whether text actually flows beside the "
        "frame) are renderer-policy territory — LibreOffice wrap behavior may vary with "
        "anchor/wrap settings; reference text is every visible paragraph string in order."
    )
    return BuildResult(path=path, title=f"Image text wrap odt {nnn:03d}",
                       reference_text=ref, notes=notes)


_CJK_STYLE = (
    '<style:style style:name="GP CJK" style:family="paragraph">'
    '<style:text-properties fo:font-size="12pt"'
    ' style:font-name-asian="Noto Serif CJK SC"'
    ' fo:language="zh" fo:country="CN"/>'
    "</style:style>"
)


@builder("cjk-text", "odt")
def cjk_text(ctx, nnn):
    title = f"cjk-text odt {nnn:03d}"
    if nnn == 1:
        lines = [
            title,
            "静夜思 — 李白",
            "床前明月光，",
            "疑是地上霜。",
            "举头望明月，",
            "低头思故乡。",
        ]
        feat = "静夜思 (Li Bai, Quiet Night Thought) as five CJK paragraphs"
    else:
        lines = [
            title,
            "古池や 蛙飛びこむ 水の音",
            "동해물과 백두산이 마르고 닳도록",
        ]
        feat = (
            "Bashō haiku 古池や 蛙飛びこむ 水の音 (Japanese kana+kanji) and an 애국가 "
            "line 동해물과 백두산이 마르고 닳도록 (Korean Hangul)"
        )
    content = f"<text:p>{escape(title)}</text:p>" + "".join(
        f'<text:p text:style-name="GP CJK">{escape(t)}</text:p>' for t in lines[1:]
    )
    ref = "\n".join(lines)
    path = write_odf(ctx, "odt", content, {"styles.xml": _named_styles_xml(_CJK_STYLE)})
    sx = zipfile.ZipFile(path).read("styles.xml").decode("utf-8")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert 'style:font-name-asian="Noto Serif CJK SC"' in sx
    assert 'fo:language="zh" fo:country="CN"' in sx
    assert cx.count('text:style-name="GP CJK"') == len(lines) - 1
    notes = (
        f"real CJK fixture: {feat}, every CJK paragraph styled with the named paragraph "
        'style "GP CJK" whose text-properties set style:font-name-asian="Noto Serif CJK SC" '
        'plus fo:language="zh" fo:country="CN" (the asian font-name slot, not the latin '
        "fo:font-name). Honest caveat: one shared style tags zh/CN even in 002, so Japanese "
        "and Korean text are zh-tagged — font substitution for kana/Hangul is renderer-font "
        "dependent (Noto CJK covers all three scripts when installed). Reference text is the "
        "exact character strings; no normative ODF counter matches, feature counts stay 0."
    )
    return BuildResult(path=path, title=f"CJK text odt {nnn:03d}",
                       reference_text=ref, notes=notes)


_RTL_STYLES = (
    '<style:style style:name="GP RTL" style:family="paragraph">'
    '<style:paragraph-properties fo:text-align="end"/>'
    '<style:text-properties fo:direction="rtl"/></style:style>'
    '<style:style style:name="GP LTR" style:family="character">'
    '<style:text-properties fo:direction="ltr"/></style:style>'
)


@builder("rtl-text", "odt")
def rtl_text(ctx, nnn):
    title = f"rtl-text odt {nnn:03d}"
    if nnn == 1:
        lines = [
            "هذا اختبار للنص العربي في ملف الثبوت.",
            "يكتب النص العربي من اليمين إلى اليسار.",
            "يجب أن يحافظ المحرك على الاتجاه والمحاذاة.",
        ]
        content = f"<text:p>{escape(title)}</text:p>" + "".join(
            f'<text:p text:style-name="GP RTL">{escape(t)}</text:p>' for t in lines
        )
        ref = "\n".join([title] + lines)
        feat = "three Arabic sentences as GP RTL paragraphs"
        checks = [
            'style:name="GP RTL" style:family="paragraph"',
            'fo:text-align="end"',
            '<style:text-properties fo:direction="rtl"/>',
        ]
    else:
        items = [
            "בדיקה עברית ראשונה של יישור מימין לשמאל",
            "רשימת תבליטים בעברית עם ",
            "שורה שלישית לשמירת היישור לאורך הרשימה",
        ]
        ltr = "hello stichos"
        item_xml = [
            f'<text:p text:style-name="GP RTL">{escape(items[0])}</text:p>',
            f'<text:p text:style-name="GP RTL">{escape(items[1])}'
            f'<text:span text:style-name="GP LTR">{ltr}</text:span> בתוכה</text:p>',
            f'<text:p text:style-name="GP RTL">{escape(items[2])}</text:p>',
        ]
        ref = "\n".join(
            [title, items[0], items[1] + ltr + " בתוכה", items[2]]
        )
        content = (
            f"<text:p>{escape(title)}</text:p>"
            + "<text:list>"
            + "".join(f"<text:list-item>{x}</text:list-item>" for x in item_xml)
            + "</text:list>"
        )
        feat = (
            "three Hebrew bullets as text:list > text:list-item > GP RTL paragraphs, the "
            "second embedding an LTR run (text:span with character style GP LTR, "
            'fo:direction="ltr") holding "hello stichos"'
        )
        checks = [
            '<style:text-properties fo:direction="ltr"/>',
        ]
    path = write_odf(ctx, "odt", content, {"styles.xml": _named_styles_xml(_RTL_STYLES)})
    sx = zipfile.ZipFile(path).read("styles.xml").decode("utf-8")
    for c in checks:
        assert c in sx, f"missing in styles.xml: {c}"
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert 'text:style-name="GP RTL"' in cx
    if nnn == 2:
        assert '<text:span text:style-name="GP LTR">' in cx
        assert cx.count("<text:list-item>") == 3
    notes = (
        f"real RTL fixture: named paragraph style GP RTL (style:paragraph-properties "
        'fo:text-align="end" + style:text-properties fo:direction="rtl"); ' + feat + ". Honest "
        "caveats: ODF has no dedicated paragraph-bidi attribute, so direction rides on "
        'fo:direction on the style text-properties and end-alignment via fo:text-align="end" '
        "(mirror logic left-to-right renderers differ on); embedded LTR/bidi reordering is "
        "performed by the renderer's bidi engine, not stored as layout. No normative ODF "
        "counter matches; feature counts stay 0."
    )
    return BuildResult(path=path, title=f"RTL text odt {nnn:03d}",
                       reference_text=ref, notes=notes)


try:  # reuse the docx unicode-edge codepoint strings when the docx module is importable
    from .docx import (
        _UE_BIDI_SANDWICH,
        _UE_COMBINING,
        _UE_LIGATURE,
        _UE_MATH_DIGITS,
        _UE_MATH_SANS,
        _UE_NBSP,
        _UE_SOFT_HYPHEN,
        _UE_VS_PAIR,
        _UE_ZWJ_FAMILY,
        _UE_ZWNBSP,
    )
except ImportError:  # fallback duplicate: same codepoint strings as docx unicode-edge
    _UE_LIGATURE = "e\ufb03ciently o\ufb04ine \u2014 a \ufb01le and a \ufb02ag"
    _UE_ZWJ_FAMILY = "\U0001F468\u200D\U0001F469\u200D\U0001F467\u200D\U0001F466"
    _UE_COMBINING = "deja\u0301 vu cafe\u0301 stack \u0061\u0302\u0301"
    _UE_NBSP = "Non\u00A0breaking\u00A0spaces\u00A0keep\u00A0this\u00A0sentence\u00A0on\u00A0one\u00A0line."
    _UE_SOFT_HYPHEN = "one long word: ex\u00ADtraor\u00ADdi\u00ADnary\u00ADhy\u00ADphe\u00ADnat\u00ADic\u00ADal\u00ADly ends here."
    _UE_MATH_SANS = "Sans double-struck variables: \U0001D54F and \U0001D550."
    _UE_MATH_DIGITS = "Mathematical bold digits: \U0001D7CF\U0001D7D0\U0001D7D1."
    _UE_VS_PAIR = "VS15 text heart \u2764\uFE0E next to VS16 emoji heart \u2764\uFE0F."
    _UE_BIDI_SANDWICH = "\u200Eopen LRM \u200F\u05E2\u05D1\u05E8\u05D9\u05EA\u200F close RLM tail\u200E"
    _UE_ZWNBSP = "\uFEFFparagraph that starts with a zero-width no-break space"


@builder("unicode-edge", "odt")
def unicode_edge(ctx, nnn):
    if nnn == 1:
        lines = [
            f"unicode-edge odt {nnn:03d}",
            _UE_LIGATURE,
            f"Family emoji (ZWJ sequence): {_UE_ZWJ_FAMILY} end",
            _UE_COMBINING,
            _UE_NBSP,
            _UE_SOFT_HYPHEN,
        ]
        notes = (
            "real codepoint edge content (same string constants as docx unicode-edge, imported "
            "from .docx): precomposed ligature GLYPHS in the text stream (e\ufb03ciently "
            "o\ufb04ine = U+FB03/U+FB04, plus U+FB01/U+FB02 in '\ufb01le'/'\ufb02ag') — actual "
            "characters, not a typographic setting; ZWJ family emoji U+1F468 U+200D U+1F469 "
            "U+200D U+1F467 U+200D U+1F466; decomposed combining stacks (e+U+0301, "
            "a+U+0302+U+0301); U+00A0 non-breaking spaces between every word of one sentence; "
            "soft hyphens U+00AD inside one long word. Reference text is the byte-exact "
            "inserted codepoints. LibreOffice font coverage varies per glyph class: missing "
            "glyphs rendering as tofu .notdef boxes is a REAL fidelity finding, not a "
            "generator bug."
        )
    else:
        lines = [
            f"unicode-edge odt {nnn:03d}",
            _UE_MATH_SANS,
            _UE_MATH_DIGITS,
            _UE_VS_PAIR,
            _UE_BIDI_SANDWICH,
            _UE_ZWNBSP,
        ]
        notes = (
            "real codepoint edge content (same string constants as docx unicode-edge, imported "
            "from .docx): mathematical double-struck (sans) letters U+1D54F/U+1D550 outside "
            "the BMP; mathematical bold digit numerals U+1D7CF/U+1D7D0/U+1D7D1 (non-ASCII "
            "digit forms that break naive isdigit/number extraction); U+2764 heart paired with "
            "VS15 U+FE0E (text) vs VS16 U+FE0F (emoji) — same base codepoint, different "
            "presentation; a bidi sandwich of LRM U+200E + RLM U+200F-wrapped Hebrew inside an "
            "LTR paragraph; and U+FEFF zero-width no-break space as the FIRST character of a "
            "paragraph. Reference text is the byte-exact inserted codepoints. LibreOffice font "
            "coverage varies per glyph class: missing glyphs rendering as tofu .notdef boxes "
            "is a REAL fidelity finding, not a generator bug."
        )
    ref = "\n".join(lines)
    content = "".join(f"<text:p>{escape(t)}</text:p>" for t in lines)
    path = write_odf(ctx, "odt", content)
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    for text in lines:
        assert f"<text:p>{text}</text:p>" in cx, f"unicode-edge line missing: {text!r}"
    if nnn == 1:
        assert "\ufb01" in cx and "\ufb02" in cx and "\ufb03" in cx and "\ufb04" in cx
        assert cx.count("\u200d") == 3
        assert "\u0301" in cx and "\u0302" in cx
        assert "\u00a0" in cx and "\u00ad" in cx
    else:
        assert "\U0001d54f" in cx and "\U0001d7cf" in cx
        assert cx.count("\ufe0e") == 1 and cx.count("\ufe0f") == 1
        assert cx.count("\u200e") == 2 and cx.count("\u200f") == 2
        assert cx.count("<text:p>\ufeff") == 1
    return BuildResult(path=path, title=f"Unicode edge cases odt {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("huge-cell-text", "odt")
def huge_cell_text(ctx, nnn):
    unit = "lorem ipsum "
    big = unit * 5000  # 60,000 chars in one text:p
    cell_big = "filler " * 1500  # 10,500 chars in one table cell
    head = f"huge-cell-text odt {nnn:03d}"
    rows = [["r0c0", "r0c1"], ["r1c0", cell_big]]
    rows_xml = "".join(
        "<table:table-row>" + "".join(_cell(t) for t in row) + "</table:table-row>"
        for row in rows
    )
    table = (
        '<table:table table:name="HugeCell">'
        '<table:table-column table:number-columns-repeated="2"/>'
        + rows_xml
        + "</table:table>"
    )
    content = (
        f"<text:p>{escape(head)}</text:p>"
        + f"<text:p>{escape(big)}</text:p>"
        + table
    )
    ref = "\n".join([head, big, "r0c0 r0c1", f"r1c0 {cell_big}"])
    path = write_odf(ctx, "odt", content)
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert cx.count(f">{big}</text:p>") == 1, "60k paragraph not written as one text:p"
    assert cx.count(f">{cell_big}</text:p>") == 1, "10.5k cell text not written as one text:p"
    normalize_zip(path)  # measure the FINAL (deterministic, STORED) container
    size = path.stat().st_size
    assert size < 10 * 1024 * 1024, f"blob {size} bytes >= 10 MB"
    notes = (
        "real huge single-string text (not an image or overflow hack): one text:p holding the "
        "deterministic 60,000-char string 'lorem ipsum ' * 5000, plus a 2x2 table:table "
        "(table:name HugeCell) whose r1c1 cell holds 'filler ' * 1500 (10,500 chars) as its "
        "office:string-value and text:p; other cells are short r{r}c{c} markers. Reference "
        "text is the exact same strings, byte-for-byte as inserted. The repetitive lorem "
        "would compress hard, but the normalized container is STORED for cross-environment "
        f"byte determinism (INV-4): saved blob is {size} bytes, far under the 10 MB cap. The "
        "2x2 table is a real table:table, so the normative counter reads table:table=1. "
        "Overflow/wrap/pagination behavior at render time is the fixture's point."
    )
    return BuildResult(path=path, title=f"Huge cell text odt {nnn:03d}",
                       reference_text=ref, notes=notes)


_CONFIG_NS = 'xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0"'

_SETTINGS_XML = (
    _XML_DECL
    + '<office:document-settings office:version="1.2"'
    ' xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"'
    + " "
    + _CONFIG_NS
    + ">"
    "<office:settings>"
    '<config:config-item-set config:name="OpenOffice.org/Writer">'
    '<config:config-item config:name="ProtectSections" config:type="boolean">false'
    "</config:config-item>"
    "</config:config-item-set>"
    "</office:settings>"
    "</office:document-settings>"
)


@builder("protected-readonly", "odt")
def protected_readonly(ctx, nnn):
    title = f"protected-readonly odt {nnn:03d}"
    body = "This document carries a settings.xml protection hint, not a real readonly flag."
    content = f"<text:p>{escape(title)}</text:p>" + f"<text:p>{escape(body)}</text:p>"
    ref = "\n".join([title, body])
    path = write_odf(ctx, "odt", content, {"settings.xml": _SETTINGS_XML})
    names = zipfile.ZipFile(path).namelist()
    assert "settings.xml" in names
    mf = zipfile.ZipFile(path).read("META-INF/manifest.xml").decode("utf-8")
    assert (
        '<manifest:file-entry manifest:full-path="settings.xml" '
        'manifest:media-type="text/xml"/>' in mf
    )
    sx = zipfile.ZipFile(path).read("settings.xml").decode("utf-8")
    assert '<config:config-item-set config:name="OpenOffice.org/Writer">' in sx
    assert '<config:config-item config:name="ProtectSections" config:type="boolean">' in sx
    notes = (
        "ODF has no first-class readonly flag — settings.xml heuristic, honest stub. Real "
        "ODF document protection lives in settings.xml (office:document-settings, "
        "config:config-item-set), and even there Writer-level items are feature toggles "
        "(ProtectBookmarks/ProtectSections/...), not a document-wide read-only bit; a real "
        "password hash (config:document-passwordprotected-style) is not emitted here. This "
        "fixture adds a well-formed settings.xml part (media type text/xml, manifest entry) "
        "with an OpenOffice.org/Writer config-item-set carrying ProtectSections boolean "
        "false: a protection-flavored marker a renderer may legitimately ignore. Reference "
        "text covers only the two body paragraphs; no normative ODF counter matches "
        "settings/config elements, so feature counts stay 0."
    )
    return BuildResult(path=path, title=f"Protected readonly odt {nnn:03d}",
                       reference_text=ref, notes=notes)


# --------------------------------------------------------------------------
# ods builders: spreadsheet content lives in office:body > office:spreadsheet >
# table:table (write_odf(body="spreadsheet")); cells reuse the odt table:table-cell
# model, numbers ride office:value-type="float" + office:value, and formulas ride
# table:formula (OpenFormula "of:" addresses) alongside the cached value natively.
# --------------------------------------------------------------------------


def _ods_num_cell(value: str, attrs: str = "") -> str:
    return (
        f'<table:table-cell{attrs} office:value-type="float" office:value="{value}">'
        f"<text:p>{escape(value)}</text:p></table:table-cell>"
    )


def _ods_formula_cell(formula: str, value: str, value_type: str = "float") -> str:
    if value_type == "string":
        vattr = f'office:value-type="string" office:string-value="{escape(value)}"'
    else:
        vattr = f'office:value-type="float" office:value="{value}"'
    return (
        f'<table:table-cell table:formula="{escape(formula, {chr(34): "&quot;"})}" {vattr}>'
        f"<text:p>{escape(value)}</text:p></table:table-cell>"
    )


@builder("merged-cells", "ods")
def merged_cells_ods(ctx, nnn):
    if nnn == 1:
        rows = [
            "<table:table-row>"
            + _cell("A1", ' table:number-rows-spanned="2"')
            + _cell("B1")
            + _cell("C1")
            + "</table:table-row>",
            "<table:table-row>" + _COVERED + _cell("B2") + _cell("C2") + "</table:table-row>",
            "<table:table-row>"
            + _cell("A3", ' table:number-columns-spanned="2"')
            + _COVERED
            + _cell("C3")
            + "</table:table-row>",
        ]
        tables = (
            '<table:table table:name="MergedGrid">'
            '<table:table-column table:number-columns-repeated="3"/>'
            + "".join(rows)
            + "</table:table>"
        )
        ref = "A1 B1 C1\nB2 C2\nA3 C3"
        tables_n = 1
        feat = (
            "one 3x3 table (table:name MergedGrid): A1 spans two rows "
            '(table:number-rows-spanned="2") with a covered:table-cell placeholder in the '
            'same column of row 2, A3 spans two columns (table:number-columns-spanned="2") '
            "with a covered:table-cell to its right in row 3"
        )
    else:
        tables = (
            '<table:table table:name="MergeRows">'
            '<table:table-column table:number-columns-repeated="2"/>'
            "<table:table-row>"
            + _cell("Top", ' table:number-rows-spanned="2"')
            + _cell("B1")
            + "</table:table-row>"
            + "<table:table-row>" + _COVERED + _cell("B2") + "</table:table-row>"
            + "</table:table>"
            + '<table:table table:name="MergeCols">'
            '<table:table-column table:number-columns-repeated="2"/>'
            "<table:table-row>"
            + _cell("Wide", ' table:number-columns-spanned="2"')
            + _COVERED
            + "</table:table-row>"
            + "</table:table>"
        )
        ref = "Top B1\nB2\nWide"
        tables_n = 2
        feat = (
            "two tables, each with exactly one merge: MergeRows has Top spanning two rows "
            "(covered:table-cell below), MergeCols has Wide spanning two columns "
            "(covered:table-cell to its right)"
        )
    path = write_odf(ctx, "ods", tables, body="spreadsheet")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert "<office:spreadsheet>" in cx and "<office:text>" not in cx
    assert cx.count("<table:table ") == tables_n
    assert cx.count('table:number-rows-spanned="2"') == 1
    assert cx.count('table:number-columns-spanned="2"') == 1
    assert cx.count(_COVERED) == 2
    notes = (
        "real ODF spreadsheet merges (same covered-cell model as the odt merged builder, "
        f"ODF 1.2 §9.4, under office:spreadsheet): {feat}; covered: is the mandated prefix "
        f"for the table namespace URI. The normative counter reads table:table={tables_n}. "
        "Reference text lists each surviving cell's text once in row-major order (covered "
        "cells carry no text)."
    )
    return BuildResult(path=path, title=f"Merged cells grid ods {nnn:03d}",
                       reference_text=ref, notes=notes)


_ODS_RTL_STYLE = (
    '<style:style style:name="GP RTL Table" style:family="table">'
    '<style:table-properties table:writing-mode="rl-tb"/></style:style>'
    '<style:style style:name="GP LTR Table" style:family="table">'
    '<style:table-properties table:writing-mode="lr-tb"/></style:style>'
)


@builder("rtl-sheets", "ods")
def rtl_sheets_ods(ctx, nnn):
    head = ["المنتج", "السعر"]
    rows_data = [("زيت", "45.5"), ("خبز", "3.25"), ("عسل", "88")]
    rtl_rows = [
        "<table:table-row>" + _cell(head[0]) + _cell(head[1]) + "</table:table-row>"
    ] + [
        "<table:table-row>" + _cell(name) + _ods_num_cell(price) + "</table:table-row>"
        for name, price in rows_data
    ]
    tables = (
        '<table:table table:name="RTL" table:style-name="GP RTL Table">'
        '<table:table-column table:number-columns-repeated="2"/>'
        + "".join(rtl_rows)
        + "</table:table>"
    )
    ref = "\n".join(
        [f"{head[0]} {head[1]}"] + [f"{name} {price}" for name, price in rows_data]
    )
    tables_n = 1
    feat = (
        'table "RTL" is applied table:style-name="GP RTL Table", a NAMED style '
        '(style:family="table") declared in styles.xml whose style:table-properties sets '
        'table:writing-mode="rl-tb" — sheet-level RTL is per-table, not per-document'
    )
    if nnn == 2:
        tables += (
            '<table:table table:name="LTR" table:style-name="GP LTR Table">'
            '<table:table-column table:number-columns-repeated="2"/>'
            "<table:table-row>" + _cell("Widget") + _cell("Count") + "</table:table-row>"
            + "<table:table-row>" + _cell("Bolts") + _ods_num_cell("120")
            + "</table:table-row>"
            + "</table:table>"
        )
        ref += "\nWidget Count\nBolts 120"
        tables_n = 2
        feat += (
            '; the second table "LTR" is explicitly left-to-right via GP LTR Table '
            '(table:writing-mode="lr-tb"), so extractors must read the writing mode per '
            "table, not assume one direction per document"
        )
    path = write_odf(
        ctx, "ods", tables, {"styles.xml": _named_styles_xml(_ODS_RTL_STYLE)},
        body="spreadsheet",
    )
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    sx = zipfile.ZipFile(path).read("styles.xml").decode("utf-8")
    assert 'style:name="GP RTL Table" style:family="table"' in sx
    assert 'table:writing-mode="rl-tb"' in sx
    assert 'table:style-name="GP RTL Table"' in cx
    assert cx.count("<table:table ") == tables_n
    if nnn == 2:
        assert 'table:writing-mode="lr-tb"' in sx
        assert 'table:style-name="GP LTR Table"' in cx
    notes = (
        f"real ODF RTL sheet: {feat}. Prices are real float cells "
        '(office:value-type="float" office:value=...) whose text:p shows the number; Arabic '
        "headers المنتج/السعر with three Arabic product rows mirror the xlsx rtl-sheets "
        "content. Honest caveats: writing-mode mirrors the sheet layout (column order and "
        "scroll direction); in-cell bidi of the Arabic strings is performed by the "
        "renderer's bidi engine, not stored as layout. Reference text is the visible cell "
        f"strings in row-major order; the normative counter reads table:table={tables_n}."
    )
    return BuildResult(path=path, title=f"RTL sheet view ods {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("long-tables", "ods")
def long_tables_ods(ctx, nnn):
    rows_xml, ref_rows = [], []
    for r in range(150):
        texts = [f"r{r}c{c}" for c in range(4)]
        rows_xml.append(
            "<table:table-row>" + "".join(_cell(t) for t in texts) + "</table:table-row>"
        )
        ref_rows.append(" ".join(texts))
    table = (
        '<table:table table:name="LongGrid">'
        '<table:table-column table:number-columns-repeated="4"/>'
        + "".join(rows_xml)
        + "</table:table>"
    )
    ref = "\n".join(ref_rows)
    path = write_odf(ctx, "ods", table, body="spreadsheet")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert "<office:spreadsheet>" in cx and "<office:text>" not in cx
    assert cx.count("<table:table-row>") == 150
    assert cx.count("<table:table ") == 1
    notes = (
        "real ODF spreadsheet stress: one table:table (table:name LongGrid) under "
        "office:spreadsheet with 150 table:table-rows x 4 table:table-cells, every cell "
        "carrying r{r}c{c} string text; the reference text is all 600 cell strings in "
        "row-major order. The normative counter reads table:table=1; print/pagination "
        "behavior over a long sheet is the fixture's point."
    )
    return BuildResult(path=path, title=f"Long tables ods {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("formulas-cached", "ods")
def formulas_cached_ods(ctx, nnn):
    if nnn == 1:
        inputs = ["1", "2", "3", "4"]
        spec = [
            ("of:=SUM([.A1:.A4])", "10"),
            ("of:=AVERAGE([.A1:.A4])", "2.5"),
            ("of:=MIN([.A1:.A4])", "1"),
        ]
        rows, ref_rows = [], []
        for r in range(4):
            cells = _ods_num_cell(inputs[r])
            if r < 3:
                cells += _ods_formula_cell(spec[r][0], spec[r][1])
                ref_rows.append(f"{inputs[r]} {spec[r][1]}")
            else:
                cells += "<table:table-cell/>"
                ref_rows.append(inputs[r])
            rows.append("<table:table-row>" + cells + "</table:table-row>")
        tables = (
            '<table:table table:name="Calc">'
            '<table:table-column table:number-columns-repeated="2"/>'
            + "".join(rows)
            + "</table:table>"
        )
        ref = "\n".join(ref_rows)
        tables_n, formulas_n = 1, 3
        checks = [
            'table:formula="of:=SUM([.A1:.A4])"',
            'table:formula="of:=AVERAGE([.A1:.A4])"',
            'table:formula="of:=MIN([.A1:.A4])"',
            'office:value="2.5"',
            "<text:p>10</text:p>",
        ]
        feat = (
            "B1/B2/B3 carry of:=SUM([.A1:.A4]) / of:=AVERAGE([.A1:.A4]) / "
            "of:=MIN([.A1:.A4]) over A1:A4 = 1,2,3,4, each with its cached float "
            "(office:value 10 / 2.5 / 1) and a display text:p showing the same number"
        )
    elif nnn == 2:
        tables = (
            '<table:table table:name="Data">'
            '<table:table-column table:number-columns-repeated="1"/>'
            "<table:table-row>"
            + _ods_formula_cell("of:=[.Other.A1]*3", "21")
            + "</table:table-row>"
            + "<table:table-row>"
            + _ods_formula_cell("of:=[.Other.A2]*3", "33")
            + "</table:table-row>"
            + "</table:table>"
            + '<table:table table:name="Other">'
            '<table:table-column table:number-columns-repeated="1"/>'
            + "<table:table-row>" + _ods_num_cell("7") + "</table:table-row>"
            + "<table:table-row>" + _ods_num_cell("11") + "</table:table-row>"
            + "</table:table>"
        )
        ref = "21\n33\n7\n11"
        tables_n, formulas_n = 2, 2
        checks = [
            'table:formula="of:=[.Other.A1]*3"',
            'table:formula="of:=[.Other.A2]*3"',
            'office:value="21"',
            'office:value="33"',
        ]
        feat = (
            "cross-table references: table Data A1/A2 hold of:=[.Other.A1]*3 / "
            "of:=[.Other.A2]*3 over table Other's A1=7, A2=11, with the results 21/33 "
            "cached in the formula cells themselves — a cached-value reader gets the "
            "numbers without resolving the cross-table address"
        )
    else:
        tables = (
            '<table:table table:name="Switch">'
            '<table:table-column table:number-columns-repeated="2"/>'
            "<table:table-row>"
            + _ods_num_cell("9")
            + _ods_formula_cell('of:=IF([.A1]>5;"big";"small")', "big", value_type="string")
            + "</table:table-row>"
            + "</table:table>"
        )
        ref = "9\nbig"
        tables_n, formulas_n = 1, 1
        checks = [
            'table:formula="of:=IF([.A1]&gt;5;&quot;big&quot;;&quot;small&quot;)"',
            'office:value-type="string" office:string-value="big"',
            "<text:p>big</text:p>",
        ]
        feat = (
            "IF cell: A1=9 (float), B1 carries of:=IF([.A1]>5;\"big\";\"small\") with a "
            "STRING cache — office:value-type=\"string\" office:string-value=\"big\" "
            '(no office:value) and display text:p "big"; the OpenFormula argument '
            "separator is the semicolon"
        )
    path = write_odf(ctx, "ods", tables, body="spreadsheet")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert "<office:spreadsheet>" in cx and "<office:text>" not in cx
    assert cx.count("<table:table ") == tables_n
    assert cx.count("table:formula=") == formulas_n
    for c in checks:
        assert c in cx, f"missing in content.xml: {c}"
    notes = (
        "OpenFormula cells with NATIVE cached values (the xlsx counterpart needs a zip "
        "rewrite because openpyxl cannot emit <v>; here table:formula and the cached "
        f"office:value/string-value are authored together, no patching): {feat}. The text:p "
        "child is the display string of the cached result — exactly what a cached-value "
        "extractor reads without recalculating. Honest caveat: LibreOffice recalculates "
        "of: formulas on load and reproduces these exact results, so recalc-vs-cache agree. "
        f"The normative counter reads table:table={tables_n}; text:field stays 0 "
        "(table:formula is not a text:field)."
    )
    return BuildResult(path=path, title=f"Formulas with cached values ods {nnn:03d}",
                       reference_text=ref, notes=notes)


_NUMBER_NS = 'xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0"'


def _ods_styles_xml(extra: str) -> str:
    """styles.xml (named styles in office:styles) with the datastyle namespace
    added to the root — needed once number:number-style / number:currency-style
    elements are emitted."""
    base = _named_styles_xml(extra)
    marker = f'office:version="1.2" {_NSDECLS} {_DC_NS}>'
    assert marker in base
    return base.replace(marker, f'office:version="1.2" {_NSDECLS} {_DC_NS} {_NUMBER_NS}>', 1)


def _ods_date_cell(value: str, attrs: str = "") -> str:
    return (
        f'<table:table-cell{attrs} office:value-type="date" office:date-value="{value}">'
        f"<text:p>{escape(value)}</text:p></table:table-cell>"
    )


@builder("tracked-changes", "ods")
def tracked_changes_ods(ctx, nnn):
    ins_text = "Inserted cell paragraph by bot-a."
    base, deleted, tail = "Base cell text", " (draft clause)", " remains after the deletion."
    accepted = base + tail
    cell_del = (
        '<table:table-cell office:value-type="string"'
        f' office:string-value="{escape(accepted)}">'
        f"<text:p>{escape(base)}"
        + _del_region("ct2", "bot-a", deleted)
        + f"{escape(tail)}</text:p>"
        + "</table:table-cell>"
    )
    cell_ins = (
        '<table:table-cell office:value-type="string"'
        f' office:string-value="{escape(ins_text)}">'
        + _ins_region("ct1", "bot-a", ins_text)
        + "</table:table-cell>"
    )
    tables = (
        '<table:table table:name="Tracked">'
        '<table:table-column table:number-columns-repeated="2"/>'
        + "<table:table-row>" + cell_del + cell_ins + "</table:table-row>"
        + "</table:table>"
    )
    ref = f"{accepted}\n{ins_text}"
    path = write_odf(ctx, "ods", tables, body="spreadsheet")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert "<office:spreadsheet>" in cx and "<office:text>" not in cx
    assert cx.count("<table:table ") == 1
    assert cx.count("<text:changed-region ") == 2
    assert cx.count("<text:insert>") == 1 and cx.count("<text:delete>") == 1
    assert cx.count("<office:change-info>") == 2
    assert cx.count("<dc:creator>bot-a</dc:creator>") == 2
    assert cx.count(_FIXED_META_STAMP) == 2
    notes = (
        "real ODF spreadsheet revisions: the same text:changed-region machinery as the odt "
        "counterpart (ODF 1.2 §2.4) but inside table:table-cell content — text:delete ct2 "
        "(dc:creator bot-a, fixed dc:date 2026-01-01T00:00:00) sits inline between the runs of "
        "the first cell's text:p, and text:insert ct1 wraps the whole inserted text:p of the "
        "second cell; both cells' office:string-value carry the ACCEPTED view. Reference text "
        "states the ACCEPTED view: inserted paragraph text included, deleted fragment excluded "
        "(same convention as the docx/odt tracked-changes fixtures). Honest caveats: whether "
        "LibreOffice shows or hides the revision marks is a per-view show-changes setting, and "
        "LO's per-cell change bookkeeping beyond the region elements is not emitted — neither "
        "verified (LibreOffice is not run in this environment). changed-region/insert/delete "
        "match no normative counter except table:table=1."
    )
    return BuildResult(path=path, title=f"Tracked changes ods {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("comments-annotations", "ods")
def comments_annotations_ods(ctx, nnn):
    if nnn == 1:
        table_name = "Commented"
        labels = ["Revenue", "Margin", "Forecast"]
        creators = ["bot-a", "bot-b", "bot-a"]
        comment_texts = [
            f"Comment {i}: reviewer note on {labels[i - 1]}." for i in range(1, 4)
        ]
    else:
        # second instance: FOUR annotated cells (001: three) with different labels and a
        # different creator pattern (bot-c/bot-a/bot-d/bot-c — a consumer keying by author
        # sees a different partition; 001 alternates two authors over three cells).
        table_name = "ReviewNotes"
        labels = ["Costs", "Ratio", "Outlook", "Budget"]
        creators = ["bot-c", "bot-a", "bot-d", "bot-c"]
        comment_texts = [
            f"Second-pass note on {labels[i - 1]}: figure needs re-check."
            for i in range(1, len(labels) + 1)
        ]
    cells = [
        f'<table:table-cell office:value-type="string"'
        f' office:string-value="{escape(labels[i - 1])}">'
        f"<text:p>{escape(labels[i - 1])}</text:p>"
        f'<office:annotation office:name="anno{i}">'
        f"<dc:creator>{creators[i - 1]}</dc:creator>"
        f"<dc:date>{_FIXED_META_STAMP}</dc:date>"
        f"<text:p>{escape(comment_texts[i - 1])}</text:p>"
        f"</office:annotation>"
        f"</table:table-cell>"
        for i in range(1, len(labels) + 1)
    ]
    tables = (
        f'<table:table table:name="{table_name}">'
        '<table:table-column table:number-columns-repeated="1"/>'
        + "".join(f"<table:table-row>{c}</table:table-row>" for c in cells)
        + "</table:table>"
    )
    ref = "\n".join(labels + comment_texts)
    path = write_odf(ctx, "ods", tables, body="spreadsheet")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert "<office:spreadsheet>" in cx and "<office:text>" not in cx
    assert cx.count("<table:table ") == 1
    assert cx.count("<office:annotation ") == len(labels)
    for i in range(1, len(labels) + 1):
        assert cx.count(f'<office:annotation office:name="anno{i}">') == 1
    assert "office:annotation-end" not in cx
    if nnn == 2:
        assert f'table:name="{table_name}"' in cx
        assert cx.count("<dc:creator>bot-c</dc:creator>") == 2, "author pattern wrong"
    notes = (
        "real ODF sheet comments: three office:annotation blocks (office:name anno1..anno3, "
        "dc:creator alternating bot-a/bot-b/bot-a, fixed dc:date 2026-01-01T00:00:00), each "
        "holding its comment body as text:p and anchored inside a table:table-cell right after "
        "the cell's text:p — the on-disk position LibreOffice writes spreadsheet notes (ODF "
        "permits office:annotation inside cell content). Point anchors: no "
        "office:annotation-end or draw:annotation-name markers exist. The normative "
        "office:annotation counter scans content.xml, so the manifest must read "
        "office:annotation=3 next to table:table=1. Reference text is every visible string: "
        "the three cell labels plus all three comment bodies. Honest caveat: whether a "
        "renderer shows/exports comment bubbles is a per-renderer setting; LibreOffice was not "
        "run here, so its exact sheet-comment serialization (LO names notes by cell address) "
        "is not byte-compared."
    )
    if nnn == 2:
        notes = (
            "real ODF sheet comments, second instance: FOUR annotations anno1..anno4 (001: "
            "three) on a table named ReviewNotes (001: Commented) over four different cell "
            "labels (Costs/Ratio/Outlook/Budget), dc:creators cycling bot-c/bot-a/bot-d/bot-c "
            "— three distinct authors with one repeat, a partition 001's two-author "
            "alternation never produces (asserted). Same cell-anchored point-annotation "
            "machinery (office:annotation right after the cell's text:p, fixed dc:date, no "
            "annotation-end), same honest caveat about renderer comment-bubble policy and LO "
            "not being run here. Normative counters: office:annotation=4, table:table=1. "
            "Reference text is the four labels plus all four comment bodies."
        )
    return BuildResult(path=path, title=f"Comments and annotations ods {nnn:03d}",
                       reference_text=ref, notes=notes)


_GP_CELL_STYLES = (
    '<style:style style:name="GP Money" style:family="table-cell"'
    ' style:data-style-name="GPNum">'
    '<style:table-cell-properties fo:background-color="#FFC7CE"/>'
    "</style:style>"
    '<style:style style:name="GP Head" style:family="table-cell">'
    '<style:text-properties fo:font-weight="bold"/></style:style>'
    '<style:style style:name="GP Col" style:family="table-column"'
    ' style:number-style="GPNum"></style:style>'
    '<number:currency-style style:name="GPNum">'
    '<number:currency-symbol number:language="en" number:country="US">$'
    "</number:currency-symbol>"
    '<number:number number:decimal-places="2" number:min-decimal-places="2"'
    ' number:min-integer-digits="1" number:grouping="true"/>'
    "</number:currency-style>"
)


@builder("custom-styles", "ods")
def custom_styles_ods(ctx, nnn):
    head_cells = (
        _cell("Product", ' table:style-name="GP Head"')
        + _cell("Amount", ' table:style-name="GP Head"')
    )
    money_rows = [("Coffee", "12.5"), ("Tin", "1234.5")]
    rows = ["<table:table-row>" + head_cells + "</table:table-row>"]
    for name, amt in money_rows:
        rows.append(
            "<table:table-row>"
            + _cell(name)
            + _ods_num_cell(amt, ' table:style-name="GP Money"')
            + "</table:table-row>"
        )
    tables = (
        '<table:table table:name="Styled">'
        '<table:table-column table:number-columns-repeated="1"/>'
        '<table:table-column table:style-name="GP Col" table:number-columns-repeated="1"/>'
        + "".join(rows)
        + "</table:table>"
    )
    ref = "Product Amount\nCoffee 12.5\nTin 1234.5"
    path = write_odf(
        ctx, "ods", tables, {"styles.xml": _ods_styles_xml(_GP_CELL_STYLES)},
        body="spreadsheet",
    )
    sx = zipfile.ZipFile(path).read("styles.xml").decode("utf-8")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert 'xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0"' in sx
    assert '<number:currency-style style:name="GPNum">' in sx
    assert ">$</number:currency-symbol>" in sx
    assert (
        'style:name="GP Money" style:family="table-cell" style:data-style-name="GPNum"' in sx
    )
    assert 'fo:background-color="#FFC7CE"' in sx
    assert 'style:name="GP Head" style:family="table-cell"' in sx
    assert 'fo:font-weight="bold"' in sx
    assert 'style:name="GP Col" style:family="table-column" style:number-style="GPNum"' in sx
    assert cx.count('table:style-name="GP Head"') == 2
    assert cx.count('table:style-name="GP Money"') == 2
    assert 'table:style-name="GP Col"' in cx
    notes = (
        'real ODF named cell styles: styles.xml office:styles holds table-cell family styles '
        '"GP Money" (style:table-cell-properties fo:background-color #FFC7CE, data style '
        'GPNum) and "GP Head" (style:text-properties fo:font-weight bold), plus the data style '
        'GPNum itself as a number:currency-style — number:currency-symbol "$" plus '
        'number:number with 2 decimals and thousands grouping. The GPNum definition rides in '
        'styles.xml office:styles (with the datastyle xmlns:number added to the root) so the '
        'named reference resolves package-wide. ODF 1.2 attribute reality on the data-style '
        'link: a table-CELL style references a number style via style:data-style-name, while '
        'style:number-style is the table-COLUMN style attribute — both spellings are exercised: '
        'GP Money uses style:data-style-name="GPNum", and a GP Col family="table-column" style '
        'carries style:number-style="GPNum" applied to column B via table:table-column '
        'table:style-name. Honest caveats: currency formatting is display-time — the stored '
        'cells hold raw floats (office:value-type="float") and the text:p shows the plain '
        'number, so the reference text lists "12.5"/"1234.5", never "$1,234.50"; LibreOffice '
        'was not run here, so its tolerance of this exact number:number/number:currency-symbol '
        'attribute spelling is recorded as unverified. The normative counter reads '
        'table:table=1; style elements match no counter.'
    )
    return BuildResult(path=path, title=f"Custom styles ods {nnn:03d}",
                       reference_text=ref, notes=notes)


_GP_CF_STYLES = (
    '<style:style style:name="GP Bad" style:family="table-cell">'
    '<style:table-cell-properties fo:background-color="#FFC7CE"/></style:style>'
    '<style:style style:name="GP Warn" style:family="table-cell">'
    '<style:table-cell-properties fo:background-color="#FFEB9C"/></style:style>'
)


def _cf_condition(test: str, apply_style: str, base: str) -> str:
    return (
        f'<table:condition table:test="{escape(test, {chr(34): "&quot;"})}"'
        f' table:apply-style="{apply_style}" table:base-cell-address="{base}"/>'
    )


@builder("conditional-formatting", "ods")
def conditional_formatting_ods(ctx, nnn):
    if nnn == 1:
        values = ["1", "4", "7", "10", "13", "16", "19", "22", "25", "28"]
        tables = (
            '<table:table table:name="Numbers">'
            '<table:table-column table:number-columns-repeated="1"/>'
            + "".join(
                f"<table:table-row>{_ods_num_cell(v)}</table:table-row>" for v in values
            )
            + "</table:table>"
            + '<table:conditional-format table:target-range-address="Numbers.A1:Numbers.A10">'
            + _cf_condition("this-cell() > 10", "GP Bad", "A1")
            + "</table:conditional-format>"
        )
        ref = "\n".join(values)
        cf_n, cond_n = 1, 1
        style_checks = ['style:name="GP Bad" style:family="table-cell"']
        test_checks = ['table:test="this-cell() &gt; 10"']
        feat = (
            'one table:conditional-format (table:target-range-address '
            '"Numbers.A1:Numbers.A10") with a single table:condition test "this-cell() > 10" '
            'applying the named cell style GP Bad (fo:background-color #FFC7CE)'
        )
    else:
        rows_data = [
            ("10", "urgent item", "2026-06-01"),
            ("50", "calm item", "2025-12-31"),
            ("90", "urgent tail", "2027-02-14"),
        ]
        tables = (
            '<table:table table:name="Mixed">'
            '<table:table-column table:number-columns-repeated="3"/>'
            + "".join(
                f"<table:table-row>{_ods_num_cell(num)}{_cell(text)}{_ods_date_cell(dt)}"
                "</table:table-row>"
                for num, text, dt in rows_data
            )
            + "</table:table>"
            + '<table:conditional-format table:target-range-address="Mixed.A1:Mixed.A3">'
            + _cf_condition("this-cell() >= 50", "GP Warn", "A1")
            + _cf_condition('contains(this-cell(); "urgent")', "GP Bad", "A1")
            + "</table:conditional-format>"
            + '<table:conditional-format table:target-range-address="Mixed.C1:Mixed.C3">'
            + _cf_condition("this-cell() > DATE(2026;1;1)", "GP Warn", "C1")
            + "</table:conditional-format>"
        )
        ref = "\n".join(f"{num} {text} {dt}" for num, text, dt in rows_data)
        cf_n, cond_n = 2, 3
        style_checks = [
            'style:name="GP Bad" style:family="table-cell"',
            'style:name="GP Warn" style:family="table-cell"',
        ]
        test_checks = [
            'table:test="this-cell() &gt;= 50"',
            'table:test="contains(this-cell(); &quot;urgent&quot;)"',
            'table:test="this-cell() &gt; DATE(2026;1;1)"',
        ]
        feat = (
            "two table:conditional-formats: one over Mixed.A1:A3 with TWO table:conditions — "
            'numeric "this-cell() >= 50" -> GP Warn (#FFEB9C) and text test '
            '"contains(this-cell(); \\"urgent\\")" -> GP Bad — and one over the date column '
            'Mixed.C1:C3 with the date condition string "this-cell() > DATE(2026;1;1)" -> '
            "GP Warn; column C holds real date cells (office:value-type=\"date\")"
        )
    path = write_odf(
        ctx, "ods", tables, {"styles.xml": _ods_styles_xml(_GP_CF_STYLES)},
        body="spreadsheet",
    )
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    sx = zipfile.ZipFile(path).read("styles.xml").decode("utf-8")
    assert "<office:spreadsheet>" in cx and "<office:text>" not in cx
    assert cx.count("<table:table ") == 1
    assert cx.count("<table:conditional-format ") == cf_n
    assert cx.count("<table:condition ") == cond_n
    for c in test_checks:
        assert c in cx, f"missing in content.xml: {c}"
    assert cx.count("table:apply-style=") == cond_n
    for c in style_checks:
        assert c in sx, f"missing in styles.xml: {c}"
    notes = (
        f"real ODF 1.2 conditional formats (ODF 1.2 §9.4, the LO 4.x on-disk shape that "
        f"replaced the ODF 1.1 style:map workaround): {feat}. table:conditional-format sits "
        "under office:spreadsheet after the table element; conditions are table:condition "
        "elements with table:test (OpenFormula with this-cell(), semicolon argument "
        "separator), table:apply-style and table:base-cell-address. LO-tolerance items NOT "
        "verified here (LibreOffice is not run in this environment, so they are recorded, not "
        "claimed): (a) table:apply-style pointing at a NAMED style — LO's own writer usually "
        "emits automatic styles there, named-style references are valid style-name references "
        "but LO round-trip tolerance is unverified; (b) address qualification — "
        "target-range-address is table-qualified (Numbers.A1:Numbers.A10) while "
        "base-cell-address is unqualified (A1), matching LO's on-disk form as best known; "
        "(c) whether the conditional-format element must sit after vs before the tables — "
        "emitted after; (d) LO evaluation of the exact this-cell()/contains()/DATE() test "
        "strings at render time. Reference text is the plain stored cell strings (condition "
        "colors are display-time, never text); the normative counter reads table:table=1."
    )
    return BuildResult(path=path, title=f"Conditional formatting ods {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("protected-readonly", "ods")
def protected_readonly_ods(ctx, nnn):
    rows = [
        "<table:table-row>" + _cell("Locked note A1") + _cell("Locked note B1") + "</table:table-row>",
        "<table:table-row>" + _cell("Unlocked note A2", ' table:protect="false"')
        + _cell("Locked note B2") + "</table:table-row>",
    ]
    tables = (
        '<table:table table:name="Locked" table:protected="true">'
        '<table:table-column table:number-columns-repeated="2"/>'
        + "".join(rows)
        + "</table:table>"
    )
    ref = "Locked note A1 Locked note B1\nUnlocked note A2 Locked note B2"
    path = write_odf(ctx, "ods", tables, body="spreadsheet")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert "<office:spreadsheet>" in cx and "<office:text>" not in cx
    assert cx.count("<table:table ") == 1
    assert 'table:table table:name="Locked" table:protected="true"' in cx
    assert cx.count('table:protect="false"') == 1
    notes = (
        'real ODF sheet protection at cell granularity: the table:table (table:name Locked) '
        'carries table:protected="true" and one cell opts back in to editing with '
        'table:protect="false" (ODF 1.2 §9.4: table:protect toggles protection of a single '
        "cell inside a protected table). No table:protection-key digest or digest-algorithm "
        "attribute is emitted — the fixture is password-less by design. Honest caveats: ODF "
        "sheet protection is advisory and renderer-policy — LibreOffice will open the sheet "
        "flagged protected but, without a protection-key, unprotecting needs no password; "
        "whether a render engine honors table:protected at all, and its tolerance of a "
        "boolean-only protected flag without key attributes, is UNVERIFIED (LibreOffice is "
        "not run in this environment). Reference text is the four cell strings; the normative "
        "counter reads table:table=1."
    )
    return BuildResult(path=path, title=f"Protected readonly ods {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("embedded-ole", "ods")
def embedded_ole_ods(ctx, nnn):
    title = f"embedded-ole ods {nnn:03d}"
    if nnn == 1:
        table_name = "Embeds"
        label = "embedded object"
        frame = (
            f'<draw:frame draw:name="OleObject1" text:anchor-type="as-char" {_SVG_NS}'
            ' svg:width="5cm" svg:height="5cm" draw:opacity="100%">'
            '<draw:object draw:mime-type="application/oleobject"'
            ' draw:object-type="application/oleobject"'
            ' xlink:type="simple" xlink:href="object.bin"'
            ' xlink:show="embed" xlink:actuate="onLoad"/>'
            "</draw:frame>"
        )
    else:
        # second instance: the embed cell MOVES (row 3 of a 4-row sheet vs row 2 of a
        # 3-row sheet in 001), the frame grows to 6cm and is named OleObject2, and every
        # visible label differs — same as-char in-cell anchor machinery underneath.
        table_name = "EmbedsB"
        label = "embedded picture cell"
        frame = (
            f'<draw:frame draw:name="OleObject2" text:anchor-type="as-char" {_SVG_NS}'
            ' svg:width="6cm" svg:height="4cm" draw:opacity="100%">'
            '<draw:object draw:mime-type="application/oleobject"'
            ' draw:object-type="application/oleobject"'
            ' xlink:type="simple" xlink:href="object.bin"'
            ' xlink:show="embed" xlink:actuate="onLoad"/>'
            "</draw:frame>"
        )
    embed_cell = (
        f'<table:table-cell office:value-type="string"'
        f' office:string-value="{escape(label)}">'
        f"<text:p>{escape(label)}{frame}</text:p>"
        "</table:table-cell>"
    )
    if nnn == 1:
        tables = (
            f'<table:table table:name="{table_name}">'
            '<table:table-column table:number-columns-repeated="2"/>'
            "<table:table-row>" + _cell(title) + _cell("host sheet line") + "</table:table-row>"
            "<table:table-row>" + _cell("embed row marker") + embed_cell + "</table:table-row>"
            "<table:table-row>" + _cell("line below the embed region") + "</table:table-row>"
            + "</table:table>"
        )
        ref = (
            f"{title} host sheet line\nembed row marker {label}\n"
            "line below the embed region"
        )
    else:
        tables = (
            f'<table:table table:name="{table_name}">'
            '<table:table-column table:number-columns-repeated="2"/>'
            "<table:table-row>" + _cell(title) + _cell("second host sheet") + "</table:table-row>"
            "<table:table-row>" + _cell("quiet row one") + _cell("no object here") + "</table:table-row>"
            "<table:table-row>" + _cell("carry row") + embed_cell + "</table:table-row>"
            "<table:table-row>" + _cell("trailing row under the frame") + "</table:table-row>"
            + "</table:table>"
        )
        ref = (
            f"{title} second host sheet\nquiet row one no object here\ncarry row {label}\n"
            "trailing row under the frame"
        )
    path = write_odf(
        ctx, "ods", tables,
        {"object.bin": (_ole_cfb_stub(), "application/x-ole-object")},
        body="spreadsheet",
    )
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    mf = zipfile.ZipFile(path).read("META-INF/manifest.xml").decode("utf-8")
    assert "<office:spreadsheet>" in cx and "<office:text>" not in cx
    assert cx.count("<table:table ") == 1
    assert cx.count("<draw:frame ") == 1 and '<draw:object ' in cx
    assert 'text:anchor-type="as-char"' in cx
    assert 'xlink:href="object.bin"' in cx
    assert 'manifest:media-type="application/x-ole-object"' in mf
    assert zipfile.ZipFile(path).read("object.bin")[:8] == bytes.fromhex("D0CF11E0A1B11AE1")
    if nnn == 2:
        assert f'table:name="{table_name}"' in cx and 'draw:name="OleObject2"' in cx
        assert cx.count("<table:table-row>") == 4, "row layout wrong"
        assert 'svg:width="6cm"' in cx
    notes = (
        "legacy OLE embed in ODF, spread-sheet placement: real binary part object.bin "
        "(hand-built minimal CFB v3 stub, shared _ole_cfb_stub helper, manifest media-type "
        "application/x-ole-object, package-root href) referenced by draw:frame > draw:object "
        '(draw:mime-type + draw:object-type "application/oleobject", xlink show=embed '
        'actuate=onLoad, svg 5cm x 5cm) placed inside a table:table-cell text:p with '
        'text:anchor-type="as-char" — the in-cell frame slot LibreOffice itself uses for '
        "cell-anchored media; the cell keeps a plain-text label run next to the frame so its "
        "text survives even if the object is dropped. Placement decision, stated honestly: "
        "cell-hosted draw frames are the LO on-disk shape for images in ODS, but ODF 1.2's "
        "cell content grammar for frames is thin — embed reachability varies by reader and "
        "LibreOffice is not run here, so downstream tests must treat the frame as "
        "pass-through content (never text truth). The CFB stub carries no payload streams, so "
        "an icon placeholder or nothing may render. The normative counters read "
        "table:table=1 and draw:frame=1. Reference text covers only the four visible cell "
        "strings."
    )
    if nnn == 2:
        notes = (
            "legacy OLE embed in ODF, second instance: same CFB-stub part and same "
            "as-char in-cell draw:frame > draw:object machinery, different PLACEMENT and "
            "copy — a FOUR-row table EmbedsB (001: three rows, table Embeds) with the embed "
            "cell in ROW THREE (001: row two) and two quiet rows in between, frame "
            "OleObject2 at 6cm x 4cm (001: OleObject1, 5cm square), every visible label "
            "different. Same honest caveats (thin cell-frame grammar, LO not run, stub "
            "renders as icon or nothing, frame is pass-through content never text truth); "
            "normative counters stay table:table=1, draw:frame=1; reference text covers the "
            "seven visible cell strings."
        )
    return BuildResult(path=path, title=f"Embedded OLE object ods {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("cjk-text", "ods")
def cjk_text_ods(ctx, nnn):
    poem = ["床前明月光，", "疑是地上霜。", "举头望明月，", "低头思故乡。"]
    korean = "동해물과 백두산이 마르고 닳도록"
    haiku = "古池や 蛙飛びこむ 水の音"

    def cjk_cell(text: str) -> str:
        return (
            f'<table:table-cell office:value-type="string"'
            f' office:string-value="{escape(text)}">'
            f'<text:p text:style-name="GP CJK">{escape(text)}</text:p>'
            "</table:table-cell>"
        )

    rows = (
        ["<table:table-row>" + cjk_cell(poem[0]) + cjk_cell(korean) + cjk_cell(haiku)
         + "</table:table-row>"]
        + [f"<table:table-row>{cjk_cell(line)}</table:table-row>" for line in poem[1:]]
    )
    tables = (
        '<table:table table:name="CJK">'
        '<table:table-column table:number-columns-repeated="3"/>'
        + "".join(rows)
        + "</table:table>"
    )
    ref = "\n".join([f"{poem[0]} {korean} {haiku}"] + poem[1:])
    path = write_odf(
        ctx, "ods", tables, {"styles.xml": _named_styles_xml(_CJK_STYLE)},
        body="spreadsheet",
    )
    sx = zipfile.ZipFile(path).read("styles.xml").decode("utf-8")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert "<office:spreadsheet>" in cx and "<office:text>" not in cx
    assert cx.count("<table:table ") == 1
    assert 'style:font-name-asian="Noto Serif CJK SC"' in sx
    assert cx.count('text:style-name="GP CJK"') == 6
    for text in poem + [korean, haiku]:
        assert f'<text:p text:style-name="GP CJK">{escape(text)}</text:p>' in cx, (
            f"CJK string lost: {text!r}"
        )
    notes = (
        "real CJK cells in ODF: the four verse lines of 静夜思 (Quiet Night Thought: "
        "床前明月光，/ 疑是地上霜。/ 举头望明月，/ 低头思故乡。) in A1:A4, an 애국가 line "
        "동해물과 백두산이 마르고 닳도록 in B1 and the Bashō haiku 古池や 蛙飛びこむ 水の音 "
        'in C1, exact strings byte-for-byte (same content as the odt/xlsx cjk fixtures). Every '
        'cell text:p references the named paragraph style "GP CJK" (shared with the odt '
        'builder) whose text-properties set style:font-name-asian="Noto Serif CJK SC" — the '
        'asian font-name slot, not the latin fo:font-name. Honest caveats: the one shared '
        'style tags zh/CN for all three scripts, so kana/Hangul substitution is renderer-font '
        'dependent (Noto CJK covers all three when installed); cell strings are stored '
        'un-normalized. The normative counter reads table:table=1. Reference text is the six '
        "visible cell strings in row-major order."
    )
    return BuildResult(path=path, title=f"CJK text ods {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("unicode-edge", "ods")
def unicode_edge_ods(ctx, nnn):
    consts = [
        _UE_LIGATURE,
        f"Family emoji (ZWJ sequence): {_UE_ZWJ_FAMILY} end",
        _UE_COMBINING,
        _UE_NBSP,
        _UE_SOFT_HYPHEN,
        _UE_MATH_SANS,
        _UE_MATH_DIGITS,
        _UE_VS_PAIR,
        _UE_BIDI_SANDWICH,
        _UE_ZWNBSP,
    ]
    tables = (
        '<table:table table:name="Unicode">'
        '<table:table-column table:number-columns-repeated="1"/>'
        + "".join(f"<table:table-row>{_cell(t)}</table:table-row>" for t in consts)
        + "</table:table>"
    )
    ref = "\n".join(consts)
    path = write_odf(ctx, "ods", tables, body="spreadsheet")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert "<office:spreadsheet>" in cx and "<office:text>" not in cx
    assert cx.count("<table:table ") == 1
    assert cx.count("<table:table-row>") == 10
    for text in consts:
        assert f"<text:p>{text}</text:p>" in cx, f"unicode-edge cell missing: {text!r}"
    assert "\ufb01" in cx and "\ufb02" in cx and "\ufb03" in cx and "\ufb04" in cx
    assert "\u0301" in cx and "\u0302" in cx
    assert "\u00a0" in cx and "\u00ad" in cx
    assert "\U0001d54f" in cx and "\U0001d7cf" in cx
    assert cx.count("<text:p>\ufeff") == 1
    # every string rides twice on disk (office:string-value + text:p): doubled counts.
    assert cx.count("\u200d") == 6
    assert cx.count("\ufe0e") == 2 and cx.count("\ufe0f") == 2
    assert cx.count("\u200e") == 4 and cx.count("\u200f") == 4
    notes = (
        "real codepoint edge content in ODS cells: the full docx unicode-edge constant set "
        "(imported _UE_* from .docx — ligature glyphs U+FB01..FB04, ZWJ family emoji "
        "U+1F468/200D/…, decomposed combining stacks, U+00A0 sentence, U+00AD soft hyphens, "
        "math double-struck U+1D54F/U+1D550, bold math digits U+1D7CF.., VS15/VS16 heart "
        "pair, LRM/RLM bidi sandwich with Hebrew, leading U+FEFF) — ten constants, one per "
        "cell in column A rows 1..10; the nominal A1..A6 window cannot hold ten one-per-cell, "
        "so the column extends to A10, placement recorded here. Cells store the raw "
        "codepoints un-normalized in office:string-value AND the display text:p (ODF repeats "
        "the string twice on disk — both must survive). Reference text is the byte-exact "
        "inserted codepoints. LibreOffice font coverage varies per glyph class: tofu boxes "
        "for missing glyphs are a REAL fidelity finding, not a generator bug. The normative "
        "counter reads table:table=1."
    )
    return BuildResult(path=path, title=f"Unicode edge cells ods {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("huge-cell-text", "ods")
def huge_cell_text_ods(ctx, nnn):
    if nnn == 1:
        big = "lorem ipsum " * 4000  # 48,000 chars in one cell text:p
        tables = (
            '<table:table table:name="Huge">'
            '<table:table-column table:number-columns-repeated="1"/>'
            "<table:table-row>" + _cell(big) + "</table:table-row>"
            + "</table:table>"
        )
        ref = big
        rows_n = 1
        feat = (
            "ONE cell holding the deterministic 48,000-char string 'lorem ipsum ' * 4000 in "
            "both office:string-value and its display text:p"
        )
        caveat = (
            "ODF has NO ECMA-376-style 32,767-char cell cap, so 48k in one cell is "
            "spec-legal here (the xlsx counterpart must pin at 32767) — readers that truncate "
            "at OOX limits are the fidelity finding this fixture hunts"
        )
    else:
        rows_txt = [
            f"row{r:03d}" + " shared-text-stress" * 8 for r in range(1, 401)
        ]  # 158 chars each, deterministic
        tables = (
            '<table:table table:name="Stress">'
            '<table:table-column table:number-columns-repeated="1"/>'
            + "".join(f"<table:table-row>{_cell(s)}</table:table-row>" for s in rows_txt)
            + "</table:table>"
        )
        ref = "\n".join(rows_txt)
        rows_n = 400
        feat = (
            "400 rows x one cell, each holding the unique deterministic ~158-char string "
            "row{r:03d} + ' shared-text-stress' * 8"
        )
        caveat = (
            "ODS has no sharedStrings table at all — every cell repeats its string literally "
            "in XML, so this is shared-text stress in the deflate/DOM sense: 63 KB of literal "
            "text with massive substring repetition across cells, which SAX/DOM readers must "
            "surface without truncation"
        )
    path = write_odf(ctx, "ods", tables, body="spreadsheet")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert "<office:spreadsheet>" in cx and "<office:text>" not in cx
    assert cx.count("<table:table ") == 1
    assert cx.count("<table:table-row>") == rows_n
    if nnn == 1:
        assert cx.count(f">{big}</text:p>") == 1, "48k cell text not written as one text:p"
    normalize_zip(path)  # measure the FINAL (deterministic, STORED) container
    size = path.stat().st_size
    assert size < 10 * 1024 * 1024, f"blob {size} bytes >= 10 MB"
    notes = (
        f"real spreadsheet text stress: {feat}. {caveat}. Reference text is the exact same "
        "string(s), byte-for-byte as inserted, no truncation allowed. The repetitive text "
        "would collapse hard under deflate, but the normalized container is STORED for "
        f"cross-environment byte determinism (INV-4): saved blob is {size} bytes, far under "
        "the 10 MB cap. The "
        "normative counter reads table:table=1. Overflow/wrap/print-area behavior over the "
        "giant strings is the fixture's point."
    )
    return BuildResult(path=path, title=f"Huge cell text ods {nnn:03d}",
                       reference_text=ref, notes=notes)


# --------------------------------------------------------------------------
# odp builders: slide content lives under office:body > office:presentation >
# draw:page (write_odf(body="presentation"), mime
# application/vnd.oasis.opendocument.presentation); each slide is a draw:page
# (10in x 7.5in via svg:width/svg:height) whose draw:frame slots carry
# presentation:class="title"/"body" and hold draw:text-box > text:p. The shared
# odp styles.xml below defines a landscape pm1 page-layout, one draw:master-page
# "mp1", the named presentation style dp1 and the named graphic style gp1, with
# frame position/size as fo:x/fo:y + svg:width/svg:height on
# style:graphic-properties (fo:x/fo:y are the ODF drawing-position properties,
# ODF 1.2 §20.36/§20.37); builders add automatic graphic styles for
# custom-placed frames via write_odf's `automatic` arg. LibreOffice is not run
# in this environment: asserts are structural only, render behavior is recorded
# as unverified.
# --------------------------------------------------------------------------

_PRESENTATION_NS = (
    'xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0"'
)


def _odp_styles_xml() -> str:
    """odp styles.xml: landscape 10x7.5in page-layout pm1, master-page mp1,
    presentation style dp1 (title slot, bold 32pt) and graphic style gp1
    (body slot) with fo:x/fo/y/width/height carried as fo:x/fo:y +
    svg:width/svg:height."""
    return (
        _XML_DECL
        + f'<office:document-styles office:version="1.2" {_NSDECLS} {_DC_NS} {_SVG_NS}>'
        "<office:styles>"
        '<style:default-style style:family="presentation">'
        '<style:text-properties fo:font-size="18pt"/>'
        "</style:default-style>"
        '<style:style style:name="dp1" style:family="presentation">'
        '<style:graphic-properties fo:x="0.5in" fo:y="0.4in" svg:width="9in" svg:height="1.2in"/>'
        '<style:text-properties fo:font-size="32pt" fo:font-weight="bold"/>'
        "</style:style>"
        '<style:style style:name="gp1" style:family="graphic">'
        '<style:graphic-properties fo:x="0.5in" fo:y="1.9in" svg:width="9in" svg:height="5in"/>'
        "</style:style>"
        "</office:styles>"
        "<office:automatic-styles>"
        '<style:page-layout style:name="pm1">'
        '<style:page-layout-properties fo:page-width="10in" fo:page-height="7.5in"'
        ' style:num-format="1"/>'
        "</style:page-layout>"
        "</office:automatic-styles>"
        "<office:master-styles>"
        '<style:master-page style:name="mp1" style:page-layout-name="pm1"/>'
        "</office:master-styles>"
        "</office:document-styles>"
    )


def _odp_named_styles_xml(extra: str) -> str:
    base = _odp_styles_xml()
    marker = "</style:default-style>"
    assert marker in base
    return base.replace(marker, marker + extra, 1)


def _odp_page(name: str, frames: str) -> str:
    return (
        f'<draw:page draw:name="{name}" draw:master-page-name="mp1"'
        f' {_SVG_NS} {_PRESENTATION_NS} svg:width="10in" svg:height="7.5in">'
        + frames
        + "</draw:page>"
    )


def _odp_paras(paras: list[str]) -> str:
    return "".join(f"<text:p>{escape(p)}</text:p>" for p in paras)


def _odp_textbox(name: str, cls: str, style: str, inner: str, z: int = 0) -> str:
    """draw:frame placeholder: `inner` is pre-built XML (escaped text:p /
    text:list) placed inside draw:text-box."""
    return (
        f'<draw:frame draw:name="{name}" draw:style-name="{style}" draw:z-index="{z}"'
        f' presentation:class="{cls}" presentation:placeholder="true">'
        f"<draw:text-box>{inner}</draw:text-box>"
        "</draw:frame>"
    )


@builder("embedded-ole", "odp")
def embedded_ole_odp(ctx, nnn):
    title = f"embedded-ole odp {nnn:03d}"
    if nnn == 1:
        lead = "Slide one carries the deck title and this lead line."
        tail = "Slide two body line next to the embedded object."
        ole_frame = (
            '<draw:frame draw:name="OleObject1" draw:style-name="gpOle" draw:z-index="1">'
            '<draw:object draw:mime-type="application/oleobject"'
            ' draw:object-type="application/oleobject"'
            ' xlink:type="simple" xlink:href="object.bin"'
            ' xlink:show="embed" xlink:actuate="onLoad"/>'
            "</draw:frame>"
        )
        automatic = (
            '<style:style style:name="gpOle" style:family="graphic">'
            f'<style:graphic-properties {_SVG_NS} fo:x="5.6in" fo:y="2.2in"'
            ' svg:width="4in" svg:height="3in"/></style:style>'
        )
        content = (
            _odp_page("page1", _odp_textbox("Title1", "title", "dp1", _odp_paras([title, lead])))
            + _odp_page(
                "page2",
                _odp_textbox("Body1", "body", "gp1", _odp_paras([tail])) + ole_frame,
            )
        )
    else:
        # second instance: the object moves to PAGE ONE (001: page two) under a different
        # frame name/style/position, the page roles swap (page1 = title+object, page2 =
        # plain body), and every visible string differs from 001.
        lead = "The embed rides the FIRST slide here, not the second."
        tail = "Second page keeps a plain body frame — no object on it."
        ole_frame = (
            '<draw:frame draw:name="OleObjectB" draw:style-name="gpOleB" draw:z-index="2">'
            '<draw:object draw:mime-type="application/oleobject"'
            ' draw:object-type="application/oleobject"'
            ' xlink:type="simple" xlink:href="object.bin"'
            ' xlink:show="embed" xlink:actuate="onLoad"/>'
            "</draw:frame>"
        )
        automatic = (
            '<style:style style:name="gpOleB" style:family="graphic">'
            f'<style:graphic-properties {_SVG_NS} fo:x="6.2in" fo:y="3.4in"'
            ' svg:width="3in" svg:height="3in"/></style:style>'
        )
        content = (
            _odp_page(
                "page1",
                _odp_textbox("Title1", "title", "dp1", _odp_paras([title, lead])) + ole_frame,
            )
            + _odp_page("page2", _odp_textbox("Body1", "body", "gp1", _odp_paras([tail])))
        )
    ref = "\n".join([title, lead, tail])
    path = write_odf(
        ctx, "odp", content,
        {
            "styles.xml": _odp_styles_xml(),
            "object.bin": (_ole_cfb_stub(), "application/x-ole-object"),
        },
        automatic=automatic,
        body="presentation",
    )
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    mf = zipfile.ZipFile(path).read("META-INF/manifest.xml").decode("utf-8")
    assert "<office:presentation>" in cx and "<office:text>" not in cx
    assert cx.count("<draw:page ") == 2
    assert cx.count("<draw:frame ") == 3
    assert '<draw:object ' in cx and 'xlink:href="object.bin"' in cx
    assert 'manifest:media-type="application/x-ole-object"' in mf
    assert zipfile.ZipFile(path).read("object.bin")[:8] == bytes.fromhex("D0CF11E0A1B11AE1")
    if nnn == 2:
        p2 = cx.index('draw:name="page2"')
        assert cx.index('draw:name="OleObjectB"') < p2, "object must sit on page one in 002"
    notes = (
        "legacy OLE embed in ODP: real binary part object.bin (hand-built minimal CFB v3 "
        "stub, shared _ole_cfb_stub, manifest media-type application/x-ole-object, "
        "package-root href) referenced on PAGE 2 by draw:frame > draw:object "
        '(draw:mime-type + draw:object-type "application/oleobject", xlink type=simple '
        "show=embed actuate=onLoad), custom-placed by automatic graphic style gpOle "
        "(fo:x 5.6in, fo:y 2.2in, 4in x 3in). Deck skeleton is the working minimal odp "
        "shape: draw:page slots (10in x 7.5in, draw:master-page-name mp1) with "
        'presentation:class="title"/"body" text frames styled by named dp1/gp1 from '
        "styles.xml. Honest caveats: the CFB stub carries no payload streams, so "
        "LibreOffice may show an icon placeholder or render nothing, and a presentation "
        "with no layout/presentation-styles part is at the thin end of the ODF grammar — "
        "LO was not run here, tolerance unverified; reference text covers only the three "
        "visible text lines (title, lead, tail). The normative counters read draw:frame=3 "
        "on this fixture."
    )
    if nnn == 2:
        notes = (
            "legacy OLE embed in ODP, second instance: the object is placed on PAGE ONE next "
            "to the title (001 puts it on page two beside the body) under frame OleObjectB "
            "(asserted before page2) with automatic style gpOleB (fo:x 6.2in, fo:y 3.4in, "
            "3in x 3in, z-index 2) — same CFB-stub part, same draw:object attributes, "
            "different placement dimension. All three visible strings differ from 001. Same "
            "honest caveats as 001 (stub payload, thin grammar, LO tolerance unverified); "
            "normative counters stay draw:frame=3."
        )
    return BuildResult(path=path, title=f"Embedded OLE object odp {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("scanned-ocr", "odp")
def scanned_ocr_odp(ctx, nnn):
    from .docx import _sc_scan_png

    png_path = ctx.tmp / "scan-slide.png"
    _sc_scan_png(png_path, ctx.rng.randrange(1 << 32))
    if nnn == 1:
        caption = "Slide 1 — scanned fixture"
        automatic = (
            '<style:style style:name="gpScan" style:family="graphic">'
            f'<style:graphic-properties {_SVG_NS} fo:x="0in" fo:y="0in"'
            ' svg:width="10in" svg:height="7.5in"/></style:style>'
        )
        frame_style, frame_name = "gpScan", "Scan1"
    else:
        # second instance: half-page raster (gpScanB: 6in x 4.5in at 0.5in/0.5in, 001 is
        # full-bleed 10x7.5), mirrored pixel stream, and a two-line caption.
        from PIL import Image, ImageOps

        with Image.open(png_path) as im:
            ImageOps.mirror(im).save(png_path, "PNG")
        caption = "Slide 1 — scanned fixture (second scan)\nraster-only body over half the slide"
        automatic = (
            '<style:style style:name="gpScanB" style:family="graphic">'
            f'<style:graphic-properties {_SVG_NS} fo:x="0.5in" fo:y="0.5in"'
            ' svg:width="6in" svg:height="4.5in"/></style:style>'
        )
        frame_style, frame_name = "gpScanB", "ScanB"
    img_frame = (
        f'<draw:frame draw:name="{frame_name}" draw:style-name="{frame_style}" draw:z-index="0">'
        '<draw:image xlink:type="simple" xlink:href="Pictures/scan1.png"'
        ' xlink:show="embed" xlink:actuate="onLoad"/>'
        "</draw:frame>"
    )
    caption_paras = caption.split("\n")
    content = _odp_page(
        "page1",
        img_frame + _odp_textbox("Title1", "title", "dp1", _odp_paras(caption_paras), z=1),
    )
    path = write_odf(
        ctx, "odp", content,
        {
            "styles.xml": _odp_styles_xml(),
            "Pictures/scan1.png": (png_path.read_bytes(), "image/png"),
        },
        automatic=automatic,
        body="presentation",
    )
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    mf = zipfile.ZipFile(path).read("META-INF/manifest.xml").decode("utf-8")
    assert "<office:presentation>" in cx and "<office:text>" not in cx
    assert cx.count("<draw:page ") == 1 and cx.count("<draw:frame ") == 2
    assert '<draw:image ' in cx and 'xlink:href="Pictures/scan1.png"' in cx
    assert 'manifest:media-type="image/png"' in mf
    notes = (
        "scanned-slide emulation in ODP: the shared PIL/numpy scan recipe "
        "(_sc_scan_png imported from the docx builder — off-white paper gray, gray "
        "pseudo-text word bars, seeded per-pixel noise from the fixture-seeded ctx.rng) "
        "stored as real PNG bytes at Pictures/scan1.png (manifest media-type image/png), "
        "shown FULL-PAGE (automatic graphic style gpScan: fo:x/fo:y 0in, "
        "svg 10in x 7.5in) in draw:frame > draw:image (xlink type=simple, "
        "show=embed actuate=onLoad), z-index 0; a presentation:title frame (named "
        "style dp1, z-index 1) above it carries the one caption line so it stays "
        "visible over the raster. The picture has no text layer (bars, not glyphs), so "
        "there is no OCR truth inside it: reference text is the caption line "
        f"{caption_paras[0]!r} ONLY — the honest near-None case. Normative counters read "
        "draw:frame=2."
    )
    if nnn == 2:
        notes = (
            "scanned-slide emulation, second instance: differs from 001 visibly — the shared "
            "_sc_scan_png raster is MIRRORED horizontally on top of the per-fixture ctx.rng "
            "seed (different pixel stream), it is placed HALF-PAGE by automatic style gpScanB "
            "(0.5in/0.5in, 6in x 4.5in; 001 is full-bleed 10in x 7.5in via gpScan), and the "
            "title frame carries a TWO-line caption (001: one line), both lines listed in "
            "reference text. Still the honest near-None case (no text layer in the raster); "
            "normative counters stay draw:frame=2."
        )
    return BuildResult(path=path, title=f"Scanned slide odp {nnn:03d}",
                       reference_text="\n".join(caption_paras), notes=notes)


@builder("image-textwrap", "odp")
def image_textwrap_odp(ctx, nnn):
    import io

    from PIL import Image

    if nnn == 1:
        img = Image.new("RGB", (64, 48), (16, 26, 46))
        lines = [
            f"image-textwrap odp {nnn:03d}",
            "This body frame overlaps the opaque picture block from its lower-right corner:",
            "z-order decides visibility, not wrap — the frame listed later paints on top.",
        ]
        automatic = (
            '<style:style style:name="gpPic" style:family="graphic">'
            f'<style:graphic-properties {_SVG_NS} fo:x="1in" fo:y="1in"'
            ' svg:width="4in" svg:height="2.2in"/></style:style>'
        )
        pic_frame = (
            '<draw:frame draw:name="WrapPic1" draw:style-name="gpPic" draw:z-index="0">'
            '<draw:image xlink:type="simple" xlink:href="Pictures/wrap1.png"'
            ' xlink:show="embed" xlink:actuate="onLoad"/>'
            "</draw:frame>"
        )
    else:
        # second instance: mirrored overlap (picture right, body enters from the left edge),
        # slate fill and a different raster size, four lines instead of three.
        img = Image.new("RGB", (96, 72), (147, 160, 184))
        lines = [
            f"image-textwrap odp {nnn:03d}",
            "Mirror layout: the slate block sits right, the body frame starts left.",
            "It overlaps the picture's upper-left corner region:",
            "z-order decides visibility, not wrap — the frame listed later paints on top.",
        ]
        automatic = (
            '<style:style style:name="gpPicB" style:family="graphic">'
            f'<style:graphic-properties {_SVG_NS} fo:x="5.4in" fo:y="0.8in"'
            ' svg:width="3.6in" svg:height="2.6in"/></style:style>'
        )
        pic_frame = (
            '<draw:frame draw:name="WrapPicB" draw:style-name="gpPicB" draw:z-index="0">'
            '<draw:image xlink:type="simple" xlink:href="Pictures/wrap1.png"'
            ' xlink:show="embed" xlink:actuate="onLoad"/>'
            "</draw:frame>"
        )
    buf = io.BytesIO()
    img.save(buf, "PNG")
    body_frame = _odp_textbox("Body1", "body", "gp1", _odp_paras(lines), z=1)
    content = _odp_page("page1", pic_frame + body_frame)
    ref = "\n".join(lines)
    path = write_odf(
        ctx, "odp", content,
        {
            "styles.xml": _odp_styles_xml(),
            "Pictures/wrap1.png": (buf.getvalue(), "image/png"),
        },
        automatic=automatic,
        body="presentation",
    )
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert "<office:presentation>" in cx and "<office:text>" not in cx
    assert cx.count("<draw:frame ") == 2 and '<draw:image ' in cx
    pic_name = "WrapPic1" if nnn == 1 else "WrapPicB"
    assert cx.index(f'draw:name="{pic_name}"') < cx.index('draw:name="Body1"'), (
        "z-order: picture frame not before body frame"
    )
    assert 'draw:z-index="0"' in cx and 'draw:z-index="1"' in cx
    notes = (
        "honest category divergence (same call as the pptx counterpart): presentation "
        "formats have NO text-wrap concept — wrap is a wordprocessing feature — so this "
        "is a z-order overlap test, not a text-wrap test. A deterministic opaque "
        "solid-ink 64x48 PNG (PIL, no rng) sits at Pictures/wrap1.png (manifest "
        "media-type image/png) in draw:frame > draw:image, placed by automatic graphic "
        "style gpPic (fo:x/fo:y 1in, 4in x 2.2in, draw:z-index=0); the presentation:body "
        "frame (named graphic style gp1: fo:x 0.5in, fo:y 1.9in, 9in x 5in, "
        "draw:z-index=1) covers x 0.5..9.5in y 1.9..6.9in and therefore overlaps the "
        "picture's lower-right corner region 1..5 x 1.9..3.2in. Later document order + "
        "higher draw:z-index both say the text frame paints on top (asserted: WrapPic1 "
        "precedes Body1). Honest caveats: gp1 is fill-less so whether the frame is "
        "transparent over the raster is renderer policy, and LibreOffice was not run "
        "here. The picture carries no text; reference text is the three frame lines. "
        "Normative counters read draw:frame=2."
    )
    if nnn == 2:
        notes = (
            "z-order overlap test, second instance — MIRROR of 001: a slate 96x72 PNG "
            "(different size AND fill from 001's ink 64x48, both deterministic PIL bytes) "
            "is placed at the RIGHT by automatic style gpPicB (5.4in/0.8in, 3.6in x 2.6in, "
            "z 0); the gp1 body frame (0.5in/1.9in, 9in x 5in, z 1) now overlaps the "
            "picture's UPPER-LEFT corner instead of the lower-right, with FOUR body lines "
            "(001: three). Document order + z-index still put the frame on top (asserted: "
            "WrapPicB precedes Body1). Same honest caveats as 001 (fill-less gp1, LO not "
            "run); reference text is the four frame lines; counters stay draw:frame=2."
        )
    return BuildResult(path=path, title=f"Image overlap odp {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("cjk-text", "odp")
def cjk_text_odp(ctx, nnn):
    if nnn == 1:
        title = "静夜思"
        poem = ["床前明月光，", "疑是地上霜。", "举头望明月，", "低头思故乡。"]
        haiku = "古池や 蛙飛びこむ 水の音"
    else:
        # second instance: Meng Haoran's 春晓 (the docx-001 second poem) replaces Li Bai,
        # and Bashō's frog haiku is replaced by the 五月雨を river haiku — six CJK strings,
        # none shared with 001 except the shared style machinery.
        title = "春晓"
        poem = ["春眠不觉晓，", "处处闻啼鸟。", "夜来风雨声，", "花落知多少。"]
        haiku = "五月雨を あつめて早し 最上川"

    def cjk_p(t: str) -> str:
        return f'<text:p text:style-name="GP CJK">{escape(t)}</text:p>'

    content = _odp_page(
        "page1",
        _odp_textbox("Title1", "title", "dp1", cjk_p(title))
        + _odp_textbox("Body1", "body", "gp1", "".join(cjk_p(t) for t in poem + [haiku])),
    )
    ref = "\n".join([title] + poem + [haiku])
    path = write_odf(
        ctx, "odp", content,
        {"styles.xml": _odp_named_styles_xml(_CJK_STYLE)},
        body="presentation",
    )
    sx = zipfile.ZipFile(path).read("styles.xml").decode("utf-8")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert "<office:presentation>" in cx and "<office:text>" not in cx
    assert cx.count("<draw:page ") == 1 and cx.count("<draw:frame ") == 2
    assert 'style:font-name-asian="Noto Serif CJK SC"' in sx
    assert cx.count('text:style-name="GP CJK"') == 6
    for text in [title] + poem + [haiku]:
        assert f'<text:p text:style-name="GP CJK">{escape(text)}</text:p>' in cx, (
            f"CJK line lost: {text!r}"
        )
    if nnn == 2:
        assert "静夜思" not in cx and "古池や" not in cx, "001 poem leaked into 002"
    notes = (
        "real CJK slide: title 静夜思 (Li Bai, Quiet Night Thought) in the "
        "presentation:title frame, the four verse lines 床前明月光，/ 疑是地上霜。/ "
        "举头望明月，/ 低头思故乡。 plus Bashō's frog haiku 古池や 蛙飛びこむ 水の音 in "
        "the presentation:body frame, exact strings byte-for-byte (same codepoints as "
        "the odt/ods/pptx cjk fixtures). Every text:p carries the named paragraph style "
        '"GP CJK" (shared with the odt builder) whose text-properties set '
        'style:font-name-asian="Noto Serif CJK SC" plus fo:language="zh" '
        'fo:country="CN" — the asian font-name slot, not the latin fo:font-name. Honest '
        "caveats: one shared style tags zh/CN over all six lines, so kana substitution "
        "is renderer-font dependent (Noto CJK covers both scripts when installed); "
        "LibreOffice was not run here. Reference text is the six exact inserted strings; "
        "the per-char CJK tokenization rule applies downstream; tofu from missing CJK "
        "fonts at render is a fidelity finding, not a generator bug. The normative "
        "counters read draw:frame=2."
    )
    if nnn == 2:
        notes = (
            "real CJK slide, second instance — a DIFFERENT poem pair so the two instances "
            "are not string-copies: title 春晓 plus Meng Haoran's four lines 春眠不觉晓，/ "
            "处处闻啼鸟。/ 夜来风雨声，/ 花落知多少。 in the body frame plus Bashō's "
            "五月雨を あつめて早し 最上川 (the frog haiku and 静夜思 of 001 are asserted "
            "absent). Same GP CJK named style (style:font-name-asian=\"Noto Serif CJK SC\" "
            "+ zh/CN) over all six text:p; same honest caveats (one zh/CN tag over the kana "
            "line, LO not run, tofu is a fidelity finding); counters stay draw:frame=2 and "
            "the six-string C9 §10.6 per-char rule applies."
        )
    return BuildResult(path=path, title=f"CJK text odp {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("rtl-text", "odp")
def rtl_text_odp(ctx, nnn):
    title = f"rtl-text odp {nnn:03d}"
    if nnn == 1:
        arabic = "هذا اختبار للنص العربي في ملف الثبوت."
        items = [
            "בדיקה עברית ראשונה של יישור מימין לשמאל",
            "רשימת תבליטים בעברית עם ",
            "שורה שלישית לשמירת היישור לאורך הרשימה",
        ]
        ltr = "hello stichos"
        tail = " בתוכה"
    else:
        # second instance: different Arabic sentence, different Hebrew bullet set and a
        # different Latin island — same flag/style machinery, zero shared content strings.
        arabic = "نص عربي ثان لاختبار المحاذاة في العرض التقديمي."
        items = [
            "שורה ראשונה ברשימה העברית השנייה",
            "תבליט עם הרצאה לטינית ",
            "שורה חותמת ששומרת על היישור",
        ]
        ltr = "rendering probe 002"
        tail = " באמצע"
    rtl_p = '<text:p text:style-name="GP RTL">'
    item_xml = [
        f"<text:list-item>{rtl_p}{escape(t)}</text:p></text:list-item>"
        for t in (items[0], items[2])
    ]
    item_xml.insert(
        1,
        f"<text:list-item>{rtl_p}{escape(items[1])}"
        f'<text:span text:style-name="GP LTR">{ltr}</text:span>{escape(tail)}</text:p>'
        "</text:list-item>",
    )
    inner = (
        f"{rtl_p}{escape(arabic)}</text:p>"
        + "<text:list>"
        + "".join(item_xml)
        + "</text:list>"
    )
    content = _odp_page(
        "page1",
        _odp_textbox("Title1", "title", "dp1", _odp_paras([title]))
        + _odp_textbox("Body1", "body", "gp1", inner),
    )
    ref = "\n".join([title, arabic, items[0], items[1] + ltr + tail, items[2]])
    path = write_odf(
        ctx, "odp", content,
        {"styles.xml": _odp_named_styles_xml(_RTL_STYLES)},
        body="presentation",
    )
    sx = zipfile.ZipFile(path).read("styles.xml").decode("utf-8")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert "<office:presentation>" in cx and "<office:text>" not in cx
    assert cx.count("<draw:page ") == 1 and cx.count("<draw:frame ") == 2
    assert 'style:name="GP RTL" style:family="paragraph"' in sx
    assert 'fo:text-align="end"' in sx
    assert '<style:text-properties fo:direction="rtl"/>' in sx
    assert '<style:text-properties fo:direction="ltr"/>' in sx
    assert cx.count('text:style-name="GP RTL"') == 4
    assert cx.count('<text:span text:style-name="GP LTR">') == 1
    assert cx.count("<text:list-item>") == 3
    assert escape(arabic) in cx
    if nnn == 2:
        assert "hello stichos" not in cx, "001 LTR island leaked into 002"
        assert escape("هذا اختبار للنص العربي في ملف الثبوت.") not in cx, "001 Arabic leaked"
    notes = (
        "real RTL slide: the Arabic sentence "
        "هذا اختبار للنص العربي في ملف الثبوت. plus three Hebrew bullets "
        "(strings shared with the odt rtl-text fixtures) ride the named paragraph style "
        "GP RTL (style:paragraph-properties fo:text-align=\"end\" + "
        "style:text-properties fo:direction=\"rtl\"); the second bullet embeds an LTR "
        "run via text:span with the GP LTR character style (fo:direction=\"ltr\") "
        "holding \"hello stichos\"; the Hebrew list is a real text:list > "
        "text:list-item chain inside the presentation:body frame, the plain deck title "
        "sits in the title frame. Honest caveats: ODF has no dedicated paragraph-bidi "
        "attribute, direction rides on fo:direction and end-alignment on "
        "fo:text-align=\"end\" (mirror logic differs left-to-right), and bidi reordering "
        "is performed by the renderer's bidi engine, not stored as layout — LibreOffice "
        "was not run here. Reference text is the exact inserted strings in logical "
        "order. The normative counters read draw:frame=2."
    )
    if nnn == 2:
        notes = (
            "real RTL slide, second instance — all content strings differ from 001 (the 001 "
            "Arabic sentence and the «hello stichos» island are asserted absent): a "
            "second Arabic sentence plus three DIFFERENT Hebrew bullets on the same GP RTL "
            "named style (fo:text-align=\"end\" + fo:direction=\"rtl\"), with the middle "
            "bullet embedding the LTR island «rendering probe 002» in a GP LTR "
            "text:span. Same text:list machinery, same GP RTL×4 / span×1 / list-item×3 "
            "asserts, same honest caveats (direction via fo:direction only, bidi is "
            "renderer-side, LO not run); counters stay draw:frame=2."
        )
    return BuildResult(path=path, title=f"RTL text odp {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("custom-styles", "odp")
def custom_styles_odp(ctx, nnn):
    deck_title = f"custom-styles odp {nnn:03d}"
    if nnn == 1:
        named = (
            '<style:style style:name="GP Body" style:family="paragraph">'
            '<style:text-properties fo:font-size="18pt"/></style:style>'
            '<style:style style:name="GP Head" style:family="paragraph"'
            ' style:parent-style-name="GP Body">'
            '<style:text-properties fo:font-weight="bold" fo:font-size="28pt"/>'
            "</style:style>"
            '<style:style style:name="GP Char" style:family="character">'
            '<style:text-properties fo:font-style="italic"/></style:style>'
        )
        head = "Body heading rendered through GP Head."
        body1 = "First body line rendered through GP Body."
        pre, span, post = "Second body line through GP Body with ", "italic emphasis", " inside."
    else:
        # second instance: a THREE-level paragraph chain (GP Body2 <- GP Mid2 <- GP Head2,
        # 001 has only one parent hop) plus a SECOND character style, and the body uses the
        # middle style directly — inheritance-depth probing instead of 001's flat chain.
        named = (
            '<style:style style:name="GP Body2" style:family="paragraph">'
            '<style:text-properties fo:font-size="16pt"/></style:style>'
            '<style:style style:name="GP Mid2" style:family="paragraph"'
            ' style:parent-style-name="GP Body2">'
            '<style:text-properties fo:font-size="20pt" fo:font-style="italic"/>'
            "</style:style>"
            '<style:style style:name="GP Head2" style:family="paragraph"'
            ' style:parent-style-name="GP Mid2">'
            '<style:text-properties fo:font-weight="bold" fo:font-size="30pt"/>'
            "</style:style>"
            '<style:style style:name="GP Char2" style:family="character">'
            '<style:text-properties fo:font-weight="bold"/>'
            "</style:style>"
        )
        head = "Heading three levels deep in the GP Head2 chain."
        body1 = "Middle style applied directly — italic 20pt inherited from GP Mid2."
        pre, span, post = "Body line on GP Body2 carrying a ", "bold character span", " via GP Char2."
    head_style, body_style, char_style = (
        ("GP Head", "GP Body", "GP Char") if nnn == 1 else ("GP Head2", "GP Body2", "GP Char2")
    )
    body_inner = (
        f'<text:p text:style-name="{head_style}">{escape(head)}</text:p>'
        + f'<text:p text:style-name="{body_style}">{escape(body1)}</text:p>'
        + f'<text:p text:style-name="{body_style}">{escape(pre)}'
        f'<text:span text:style-name="{char_style}">{escape(span)}</text:span>{escape(post)}</text:p>'
    )
    content = _odp_page(
        "page1",
        _odp_textbox("Title1", "title", "dp1", _odp_paras([deck_title]))
        + _odp_textbox("Body1", "body", "gp1", body_inner),
    )
    ref = "\n".join([deck_title, head, body1, pre + span + post])
    path = write_odf(
        ctx, "odp", content, {"styles.xml": _odp_named_styles_xml(named)},
        body="presentation",
    )
    sx = zipfile.ZipFile(path).read("styles.xml").decode("utf-8")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert f'style:name="{body_style}" style:family="paragraph"' in sx
    assert (
        f'style:name="{head_style}" style:family="paragraph" style:parent-style-name="' in sx
    )
    assert f'style:name="{char_style}" style:family="character"' in sx
    assert cx.count(f'text:p text:style-name="{head_style}"') == 1
    assert cx.count(f'text:p text:style-name="{body_style}"') == 2
    assert f'<text:span text:style-name="{char_style}">' in cx
    if nnn == 2:
        assert 'style:name="GP Mid2" style:family="paragraph" style:parent-style-name="GP Body2"' in sx
        assert 'style:name="GP Head2" style:family="paragraph" style:parent-style-name="GP Mid2"' in sx
        assert 'style:name="GP Body"' not in sx, "001 style name leaked into 002"
    notes = (
        'real ODP named styles: styles.xml office:styles holds paragraph styles "GP Body" '
        '(style:family="paragraph", fo:font-size 18pt) and "GP Head" (family paragraph, '
        'style:parent-style-name="GP Body", bold 28pt — a one-level parent chain), plus a '
        'CHARACTER-family style GP Char (fo:font-style italic). The presentation:body frame '
        "applies them from content.xml: two GP Body paragraphs (one carrying a GP Char "
        "text:span) and one GP Head paragraph, all via text:p text:style-name (named, not "
        "automatic, styles); the deck title frame stays on the named presentation style dp1. "
        "Honest caveat: style rendering (inheritance, span italics) is LibreOffice policy and "
        "LO was not run here. None of the five normative ODF counters match style elements, so "
        "the only nonzero counter is draw:frame=2; reference text is every visible paragraph "
        "string (style names themselves are not visible text)."
    )
    if nnn == 2:
        notes = (
            "named styles, second instance — the dimension probed is inheritance DEPTH: "
            "styles.xml carries a THREE-level paragraph chain GP Body2 (16pt) <- GP Mid2 "
            "(20pt italic) <- GP Head2 (bold 30pt) where 001 has a single parent hop, plus a "
            "second character style GP Char2 (fo:font-weight bold; 001's GP Char is italic), "
            "and the body exercises the MIDDLE style's inheritance explicitly. GP Body/GP "
            "Head/GP Char (001 names) are asserted absent, all four visible strings differ. "
            "Same honest caveats (rendering policy LO-side, LO not run); no normative counter "
            "matches style elements, draw:frame=2 stands; reference text = every visible "
            "paragraph string."
        )
    return BuildResult(path=path, title=f"Custom styles odp {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("numbering-restarts", "odp")
def numbering_restarts_odp(ctx, nnn):
    deck_title = f"numbering-restarts odp {nnn:03d}"
    if nnn == 1:
        named = (
            '<text:list-style style:name="L1">' + _lvl_number(1, "1", "1") + "</text:list-style>"
        )
        alpha = [f"Alpha item {i}" for i in range(1, 6)]
        bravo = [f"Bravo item {i}" for i in range(1, 6)]
        inner = (
            _num_list("".join(_num_item(t) for t in alpha), style="L1")
            + _num_list("".join(_num_item(t) for t in bravo), style="L1", restart=True)
        )
    else:
        # second instance: THREE lists on TWO different list styles (001: two lists, one
        # style) — Charlie restarts on the LETTERED style L2, so restart semantics meet
        # num-format diversity; item counts are uneven (4+3+3 vs 5+5).
        named = (
            '<text:list-style style:name="L1">' + _lvl_number(1, "1", "1") + "</text:list-style>"
            '<text:list-style style:name="L2">' + _lvl_number(1, "A", "1") + "</text:list-style>"
        )
        alpha = [f"Alpha item {i}" for i in range(1, 5)]
        bravo = [f"Bravo item {i}" for i in range(1, 4)]
        charlie = [f"Charlie item {i}" for i in range(1, 4)]
        inner = (
            _num_list("".join(_num_item(t) for t in alpha), style="L1")
            + _num_list("".join(_num_item(t) for t in bravo), style="L1", restart=True)
            + _num_list("".join(_num_item(t) for t in charlie), style="L2", restart=True)
        )
    content = _odp_page(
        "page1",
        _odp_textbox("Title1", "title", "dp1", _odp_paras([deck_title]))
        + _odp_textbox("Body1", "body", "gp1", inner),
    )
    ref = "\n".join([deck_title] + alpha + bravo + (charlie if nnn == 2 else []))
    path = write_odf(
        ctx, "odp", content, {"styles.xml": _odp_named_styles_xml(named)},
        body="presentation",
    )
    sx = zipfile.ZipFile(path).read("styles.xml").decode("utf-8")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert '<text:list-style style:name="L1">' in sx
    assert 'text:num-format="1"' in sx and 'text:start-value="1"' in sx
    assert cx.count("<text:list-item>") == 10
    if nnn == 1:
        assert cx.count('<text:list text:style-name="L1"') == 2
        assert cx.count('text:continue-numbering="false"') == 1
    else:
        assert '<text:list-style style:name="L2">' in sx and 'text:num-format="A"' in sx
        assert cx.count('<text:list text:style-name="L1"') == 2
        assert cx.count('<text:list text:style-name="L2"') == 1
        assert cx.count('text:continue-numbering="false"') == 2
        assert "Charlie item 3" in cx
    notes = (
        'real ODP list numbering: text:list-style "L1" in styles.xml office:styles (level 1 '
        'text:list-level-style-number, text:num-format="1", text:start-value="1"), used by '
        "two text:list of 5 text:list-item > text:p inside the presentation:body frame; the "
        'second list carries text:continue-numbering="false" so its counter restarts at 1 '
        "(same list machinery as the odt numbering-restarts fixture, placed on a slide). ODF "
        "restart semantics stated honestly: text:continue-numbering DEFAULTS to false, so a "
        "sibling text:list sharing a list-style does not continue the previous counter anyway "
        '— the explicit text:continue-numbering="false" pins it (continuation would require '
        '"true"). Reference text is the 10 exact item strings plus the deck title; the number '
        "markers are render-derived, never in the file as text. No normative counter matches "
        "list elements, so the only nonzero counter is draw:frame=2."
    )
    if nnn == 2:
        notes = (
            "ODP list numbering, second instance: THREE lists on TWO list styles instead of "
            "001's two-on-one — Alpha (4 items) on L1 with NO restart attribute, Bravo (3) "
            "on L1 with text:continue-numbering=\"false\", Charlie (3) on a SECOND style L2 "
            "(text:num-format=\"A\", lettered markers, 001 only ever shows decimal) also "
            "restart-pinned, so restart flags ×2 (001 ×1) and marker formats ×2 meet on one "
            "slide; uneven sizes 4+3+3 (001: 5+5). Same honest default-false restart "
            "semantics as 001, same draw:frame=2 (no counter matches list elements); "
            "reference text is the 10 item strings + title, markers stay render-derived."
        )
    return BuildResult(path=path, title=f"Numbering restarts odp {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("comments-annotations", "odp")
def comments_annotations_odp(ctx, nnn):
    deck_title = f"comments-annotations odp {nnn:03d}"
    n = 3 if nnn == 1 else 4
    if nnn == 1:
        author_cycle = ["bot-a", "bot-b"]
    else:
        # second instance: FOUR annotations (001: three) cycled over THREE authors so a
        # consumer that keys notes by author sees a different partition.
        author_cycle = ["bot-b", "bot-c", "bot-a"]
    annos, claims, comment_texts = [], [], []
    for i in range(1, n + 1):
        creator = author_cycle[i % len(author_cycle)]
        if nnn == 1:
            ctext = f"Comment {i}: reviewer note on slide claim {i}."
            claims.append(f"Claim {i} opens anchor-text-{i} closes claim {i}.")
            lead, trail = f"Slide claim {i}: ", f"end note {i}."
        else:
            ctext = f"Second-pass note {i}: verify wording of statement {i}."
            claims.append(f"Statement {i} reviewed by {creator} on this slide.")
            lead, trail = f"Reviewed statement {i}: ", f"review tail {i}."
        comment_texts.append(ctext)
        annos.append(
            f"<text:p>{lead}"
            f'<office:annotation office:name="anno{i}">'
            f"<dc:creator>{creator}</dc:creator>"
            f"<dc:date>{_FIXED_META_STAMP}</dc:date>"
            f"<text:p>{escape(ctext)}</text:p>"
            f"</office:annotation>"
            f"{trail}</text:p>"
        )
    content = _odp_page(
        "page1",
        _odp_textbox("Title1", "title", "dp1", _odp_paras([deck_title]))
        + _odp_textbox("Body1", "body", "gp1", _odp_paras(claims))
        # Impress has no slide-comment model: inline office:annotation inside a
        # slide's draw:text-box makes LO refuse to load the package (measured
        # 2026-09-05: "source file could not be loaded"; stripping the
        # annotations loads). Annotations therefore live in the page's
        # draw:notes (speaker notes), which LO loads and which is the closest
        # real Impress surface for reviewer comments.
        + '<draw:notes presentation:class="notes">'
        + "".join(a for a in annos)
        + "</draw:notes>",
    )
    ref = "\n".join([deck_title] + claims)
    path = write_odf(ctx, "odp", content, {"styles.xml": _odp_styles_xml()},
                     body="presentation")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert "<office:presentation>" in cx and "<office:text>" not in cx
    assert cx.count("<draw:page ") == 1 and cx.count("<draw:frame ") == 2
    assert cx.count("<office:annotation ") == n
    assert "<draw:notes" in cx and cx.index("<office:annotation ") > cx.index("<draw:notes")
    for i in range(1, n + 1):
        assert cx.count(f'<office:annotation office:name="anno{i}">') == 1
    assert "office:annotation-end" not in cx
    if nnn == 2:
        assert cx.count("<dc:creator>bot-c</dc:creator>") == 2, "author cycle wrong"
        assert "reviewer note on slide claim" not in cx, "001 comment wording leaked"
    notes = (
        f"reviewer comments as {n} office:annotation elements (office:name anno1..anno3, "
        "alternating dc:creator bot-a/bot-b, fixed dc:date 2026-01-01T00:00:00) anchored "
        "inside the page's draw:notes (speaker-notes) paragraphs — NOT inside slide frames: "
        "Impress has no slide-comment model and inline annotation in a slide text box makes "
        "LibreOffice refuse the package (measured 2026-09-05: 'source file could not be "
        "loaded'; same file without annotations loads). The normative office:annotation "
        "counter scans content.xml and reads 3 next to draw:frame=2. Reference text is deck "
        "title + claim paragraphs only: default PDF export renders slides, not notes, so "
        "comment strings are expected NOT to appear in the render (F1 must not demand them)."
    )
    if nnn == 2:
        notes = (
            "reviewer comments, second instance: FOUR office:annotation elements (001: three) "
            "named anno1..anno4 with fixed dc:date, cycled over THREE dc:creators "
            "(bot-b/bot-c/bot-a; 001 alternates two) so per-author note grouping has to "
            "re-shard, all still anchored inside the page's draw:notes paragraphs — the "
            "annotation-lesson from 001 holds: inline annotation in a slide text box makes "
            "LibreOffice refuse the package (measured 2026-09-05), notes placement is asserted "
            "(first annotation appears after <draw:notes>). Normative office:annotation "
            "counter reads 4 next to draw:frame=2. Reference text is deck title + the four "
            "statement paragraphs only; comment strings must NOT be demanded in the render."
        )
    return BuildResult(path=path, title=f"Comments and annotations odp {nnn:03d}",
                       reference_text=ref, notes=notes)


def _settings_xml_odp(protect: str = "false") -> str:
    return (
        _XML_DECL
        + '<office:document-settings office:version="1.2"'
        ' xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"'
        + " "
        + _CONFIG_NS
        + ">"
        "<office:settings>"
        '<config:config-item-set config:name="OpenOffice.org/Impress">'
        f'<config:config-item config:name="ProtectSections" config:type="boolean">{protect}'
        "</config:config-item>"
        "</config:config-item-set>"
        "</office:settings>"
        "</office:document-settings>"
    )


_SETTINGS_XML_ODP = _settings_xml_odp()


@builder("protected-readonly", "odp")
def protected_readonly_odp(ctx, nnn):
    deck_title = f"protected-readonly odp {nnn:03d}"
    if nnn == 1:
        body = "This deck carries a settings.xml protection hint, not a real readonly flag."
        settings = _SETTINGS_XML_ODP
    else:
        # second instance: the same config item flipped to TRUE (001: false) — a consumer
        # that parses the boolean must behave differently here — over different slide copy.
        body = "Flip test: the same Impress item now reads true — parse the value, not the name."
        settings = _settings_xml_odp("true")
    content = _odp_page(
        "page1",
        _odp_textbox("Title1", "title", "dp1", _odp_paras([deck_title]))
        + _odp_textbox("Body1", "body", "gp1", _odp_paras([body])),
    )
    ref = "\n".join([deck_title, body])
    path = write_odf(
        ctx, "odp", content,
        {"styles.xml": _odp_styles_xml(), "settings.xml": settings},
        body="presentation",
    )
    names = zipfile.ZipFile(path).namelist()
    assert "settings.xml" in names
    mf = zipfile.ZipFile(path).read("META-INF/manifest.xml").decode("utf-8")
    assert (
        '<manifest:file-entry manifest:full-path="settings.xml" '
        'manifest:media-type="text/xml"/>' in mf
    )
    sx = zipfile.ZipFile(path).read("settings.xml").decode("utf-8")
    assert '<config:config-item-set config:name="OpenOffice.org/Impress">' in sx
    assert '<config:config-item config:name="ProtectSections" config:type="boolean">' in sx
    if nnn == 2:
        assert 'config:type="boolean">true</config:config-item>' in sx, "flip to true lost"
    notes = (
        "ODF has no first-class readonly flag — settings.xml heuristic, honest stub (same "
        "approach as protected-readonly-odt-001, Impress flavor). Real ODF document "
        "protection lives in settings.xml (office:document-settings, "
        "config:config-item-set), and application-level items there are feature toggles, not "
        "a document-wide read-only bit; no password digest is emitted anywhere. This fixture "
        "adds a well-formed settings.xml part (media type text/xml, manifest entry) with an "
        "OpenOffice.org/Impress config-item-set carrying a ProtectSections boolean false — a "
        "protection-flavored marker a renderer may legitimately ignore, and LO's exact "
        "tolerance of this item name under the Impress set is UNVERIFIED (LibreOffice is not "
        "run in this environment). Reference text covers only the two visible slide strings; "
        "no normative ODF counter matches settings/config elements, so the only nonzero "
        "counter is draw:frame=2."
    )
    if nnn == 2:
        notes = (
            "ODF has no first-class readonly flag — same settings.xml heuristic as 001, but "
            "the PROTECTSECTIONS BOOLEAN IS FLIPPED TO TRUE (001 ships false): a consumer "
            "that only checks 'does the item exist' cannot tell the two decks apart, one "
            "that parses the boolean can — asserted 'boolean\">true<'. Same honest caveats "
            "(feature toggle, not a read-only bit; no password digest; LO tolerance of the "
            "item name under the Impress set UNVERIFIED, LO not run here). Different slide "
            "copy; reference text is the two visible strings; the only nonzero normative "
            "counter stays draw:frame=2."
        )
    return BuildResult(path=path, title=f"Protected readonly odp {nnn:03d}",
                       reference_text=ref, notes=notes)


try:  # share the NATO 12-cycle constants with the pptx long-deck builder
    from .pptx import _LD_PHRASES
except ImportError:  # fallback duplicate: same 12 phrases as long-deck-300 pptx
    _LD_PHRASES = (
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot",
        "golf", "hotel", "india", "juliet", "kilo", "lima",
    )


@builder("long-deck-300", "odp")
def long_deck_300_odp(ctx, nnn):
    import time

    t0 = time.monotonic()
    phase = 0 if nnn == 1 else 6
    pages, ref_lines = [], []
    for i in range(1, 301):
        title = f"Slide {i}"
        bullets = [_LD_PHRASES[(3 * (i - 1) + k + phase) % 12] for k in range(3)]
        pages.append(
            _odp_page(
                f"page{i}",
                _odp_textbox(f"Title{i}", "title", "dp1", _odp_paras([title]))
                + _odp_textbox(f"Body{i}", "body", "gp1", _odp_paras(bullets)),
            )
        )
        ref_lines.append(title)
        ref_lines.extend(bullets)
    ref = "\n".join(ref_lines)
    assert len(ref_lines) == 1200
    content = "".join(pages)
    path = write_odf(ctx, "odp", content, {"styles.xml": _odp_styles_xml()},
                     body="presentation")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    assert "<office:presentation>" in cx and "<office:text>" not in cx
    assert cx.count("<draw:page ") == 300
    assert cx.count("<draw:frame ") == 600
    assert "<text:p>Slide 1</text:p>" in cx and "<text:p>Slide 300</text:p>" in cx
    for p in _LD_PHRASES:
        assert cx.count(f"<text:p>{p}</text:p>") == 75
    if nnn == 2:
        # phase shift must actually rotate the bullets: slide 1's body carries the phrases
        # starting at index 6 ('golf/hotel/india'), and none of 001's slide-1 openers.
        p1 = cx[cx.index('draw:name="page1"'): cx.index('draw:name="page2"')]
        assert "<text:p>golf</text:p>" in p1 and "<text:p>india</text:p>" in p1
        assert "<text:p>alpha</text:p>" not in p1, "phase offset not applied to slide 1"
    normalize_zip(path)  # measure the FINAL (deterministic, STORED) container
    size = path.stat().st_size
    assert size < 12 * 1024 * 1024, f"blob {size} bytes >= 12 MB"
    elapsed = time.monotonic() - t0
    assert elapsed < 60, f"generation took {elapsed:.1f} s >= 60 s"
    notes = (
        "300-slide pagination stress, odp counterpart of long-deck-300 pptx: 300 draw:page "
        "elements (10in x 7.5in, master-page mp1), each with a presentation:title frame "
        '(named style dp1) holding the exact line "Slide {i}" and a presentation:body frame '
        "(named style gp1) holding three bullets from the fixed 12-phrase NATO cycle "
        "(_LD_PHRASES imported from the pptx builder — imported, not re-spelled), so the "
        "phrase cycle is index-aligned with the pptx deck. Slide geometry is constant "
        "(dp1/gp1 slots); slide count is the only variable. Bullets are stored as plain "
        "text:p — no bullet glyph is in the file, bullet markers are renderer/style "
        "territory, and LibreOffice was not run here. reference_text is the exact "
        "title+bullets of every slide in order (1200 lines). The normative counters read "
        f"draw:frame=600; the raw ~240 KB content.xml is carried uncompressed in the "
        f"normalized STORED container (cross-env byte determinism, INV-4): saved blob is "
        f"{size} bytes, far under the 12 MB cap; generation (build+asserts) is asserted "
        "< 60 s at build time (no clock value recorded here — machine-dependent)."
    )
    if nnn == 2:
        notes = (
            "300-slide stress, second instance — the phase knob: bullets come from the SAME "
            "_LD_PHRASES 12-cycle but ROTATED BY +6 (slide 1 opens 'golf/hotel/india' where "
            "001 opens 'alpha/bravo/charlie' — asserted), so per-slide text differs from 001 "
            "while the per-phrase count stays exactly 75 (asserted) and the deck remains "
            "index-comparable across instances; titles stay 'Slide {i}' so the delta is "
            "isolated to body text. Same 300 draw:page / 600 draw:frame geometry, same "
            "plain-text-p bullet honesty, same <12 MB and <60 s budgets asserted here, and "
            "reference_text is the rotated 1200 lines."
        )
    return BuildResult(path=path, title=f"Long deck 300 odp {nnn:03d}",
                       reference_text=ref, notes=notes)


@builder("unicode-edge", "odp")
def unicode_edge_odp(ctx, nnn):
    strings = [
        _UE_LIGATURE,
        _UE_ZWJ_FAMILY,
        _UE_COMBINING,
        _UE_NBSP,
        _UE_SOFT_HYPHEN,
        _UE_MATH_SANS,
        _UE_MATH_DIGITS,
        _UE_VS_PAIR,
        _UE_BIDI_SANDWICH,
        _UE_ZWNBSP,
    ]
    title = "unicode edge odp" if nnn == 1 else "unicode edge odp B"

    def p(t: str) -> str:
        return f"<text:p>{escape(t)}</text:p>"

    if nnn == 1:
        content = _odp_page(
            "page1",
            _odp_textbox("Title1", "title", "dp1", p(title))
            + _odp_textbox("Body1", "body", "gp1", "".join(p(t) for t in strings)),
        )
    else:
        # second instance: the ten edge strings are SPLIT ACROSS TWO SLIDES (001: one), in
        # two halves interleaved so a page-count or per-page text-extraction bug shows up;
        # different deck title.
        content = (
            _odp_page(
                "page1",
                _odp_textbox("Title1", "title", "dp1", p(title))
                + _odp_textbox("Body1", "body", "gp1", "".join(p(t) for t in strings[::2])),
            )
            + _odp_page(
                "page2",
                _odp_textbox("Body2", "body", "gp1", "".join(p(t) for t in strings[1::2])),
            )
        )
    path = write_odf(ctx, "odp", content, body="presentation")
    cx = zipfile.ZipFile(path).read("content.xml").decode("utf-8")
    for t in strings:
        assert escape(t) in cx, f"unicode string lost: {t!r}"
    assert cx.count("<draw:page ") == (1 if nnn == 1 else 2)
    assert cx.count("<draw:frame ") == (2 if nnn == 1 else 3)
    ref = "\n".join([title] + strings)
    notes = (
        "one text:p per unicode-edge string (same _UE_* codepoints as docx/odt/ods "
        "unicode-edge fixtures): ligature glyphs, ZWJ family emoji, combining stacks, "
        "NBSP sentence, soft hyphens, math alphanumerics, VS15/VS16 heart pair, "
        "LRM/RLM bidi sandwich, leading U+FEFF. Tofu/substitution under LibreOffice is "
        "a REAL fidelity finding for the baseline report, not a generator bug."
    )
    if nnn == 2:
        notes = (
            "the ten _UE_* edge strings spread over TWO slides by interleaving (even indices "
            "on page 1, odd on page 2 — 001 stacks all ten on one slide), under the deck "
            "title 'unicode edge odp B': same codepoint coverage as 001, different page "
            "geometry, so per-page extraction and any page-splitting/merging bug in a "
            "consumer is probed while the corpus still holds the same codepoint inventory "
            "twice at different granularities. Normative counters read draw:frame=3 across "
            "2 draw:pages. Tofu under LibreOffice stays a REAL fidelity finding."
        )
    return BuildResult(path=path, title=f"Unicode edge odp {nnn:03d}",
                       reference_text=ref, notes=notes)
