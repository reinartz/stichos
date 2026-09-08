"""docx category builders (python-docx + raw XML injection where needed)."""

from ..core import BuildResult, builder, normalize_zip, pin_docx_props, png_rgb_bytes


@builder("merged-cells", "docx")
def merged_cells(ctx, nnn):
    from docx import Document

    doc = Document()
    doc.add_paragraph(f"merged-cells docx {nnn:03d}")
    if nnn == 1:
        t = doc.add_table(rows=3, cols=4)
        t.style = "Table Grid"
        for r in range(3):
            for c in range(4):
                t.cell(r, c).text = f"R{r}C{c}"
        t.cell(0, 0).merge(t.cell(0, 2))  # horizontal -> gridSpan
        t.cell(1, 3).merge(t.cell(2, 3))  # vertical   -> vMerge
        p2 = doc.add_paragraph("after table")
        ref = "merged-cells docx 001\nR0C0 R0C1 R0C2 R0C3\nR1C0 R1C1 R1C2 R1C3\nR2C0 R2C1 R2C2 R2C3\nafter table"
        notes = "real h-merge (gridSpan) + v-merge (vMerge continuation); python-docx merge."
    else:
        t1 = doc.add_table(rows=2, cols=3)
        t1.style = "Table Grid"
        for r in range(2):
            for c in range(3):
                t1.cell(r, c).text = f"A{r}{c}"
        t1.cell(0, 0).merge(t1.cell(1, 1))  # block merge 2x2
        doc.add_paragraph("between")
        t2 = doc.add_table(rows=4, cols=2)
        t2.style = "Table Grid"
        for r in range(4):
            for c in range(2):
                t2.cell(r, c).text = f"B{r}{c}"
        t2.cell(0, 0).merge(t2.cell(0, 1))
        t2.cell(2, 0).merge(t2.cell(3, 1))
        ref = (
            "merged-cells docx 002\nA00 A01 A02\nA10 A11 A12\nbetween\n"
            "B00 B01\nB10 B11\nB20 B21\nB30 B31"
        )
        notes = "block 2x2 merge + two h-merges; merged region text may repeat across span in extraction."
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)
    return BuildResult(path=path, title=f"Merged cells grid {nnn}", reference_text=ref, notes=notes)


@builder("protected-readonly", "docx")
def protected_readonly(ctx, nnn):
    from docx import Document

    from ..core import zip_read, zip_write

    doc = Document()
    doc.add_paragraph(f"protected-readonly docx {nnn:03d}")
    doc.add_paragraph("This document sets read-only enforcement in word/settings.xml.")
    doc.add_paragraph("Editors should treat the body as final.")
    ref = (
        f"protected-readonly docx {nnn:03d}\n"
        "This document sets read-only enforcement in word/settings.xml.\n"
        "Editors should treat the body as final."
    )
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)
    entries = zip_read(path)
    settings = entries["word/settings.xml"].decode("utf-8")
    prot = '<w:documentProtection w:edit="readOnly" w:enforcement="1"/>'
    if prot not in settings:
        anchor = "<w:defaultTabStop"
        assert anchor in settings, "settings.xml anchor missing"
        settings = settings.replace(anchor, prot + anchor, 1)  # schema order: protection before defaultTabStop
        entries["word/settings.xml"] = settings.encode("utf-8")
    zip_write(path, entries)
    notes = (
        "real w:documentProtection (w:edit=\"readOnly\" w:enforcement=\"1\") injected into "
        "word/settings.xml via gen.core zip_read/zip_write; body text is plain python-docx content."
    )
    return BuildResult(path=path, title=f"Protected read-only docx {nnn:03d}", reference_text=ref, notes=notes)


@builder("toc-fields", "docx")
def toc_fields(ctx, nnn):
    from docx import Document
    from docx.oxml import parse_xml
    from docx.oxml.ns import nsdecls

    doc = Document()
    doc.add_paragraph(f"toc-fields docx {nnn:03d}")
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
    for level, text in headings:
        doc.add_heading(text, level=level)
    placeholder = "Contents placeholder — field not evaluated"
    fld = parse_xml(
        r'<w:fldSimple %s w:instr=" TOC \o &quot;1-3&quot; \h \z \u ">'
        r"<w:r><w:t>%s</w:t></w:r>"
        r"</w:fldSimple>" % (nsdecls("w"), placeholder)
    )
    doc.add_paragraph()._p.append(fld)
    ref = (
        f"toc-fields docx {nnn:03d}\n"
        + "\n".join(text for _, text in headings)
        + f"\n{placeholder}"
    )
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)
    notes = (
        "real w:fldSimple TOC field (instr \" TOC \\o \"1-3\" \\h \\z \\u \") injected via raw "
        "python-docx oxml parse_xml; placeholder run only — no cached field results and no "
        "w:fldChar/instrText begin-separate-end sequence (python-docx does not evaluate fields)."
    )
    return BuildResult(path=path, title=f"TOC field docx {nnn:03d}", reference_text=ref, notes=notes)


_TC_DATE = "2026-01-01T00:00:00Z"


def _tc_run_pat(token: str):
    import re

    return re.compile(r'<w:r><w:t( xml:space="preserve")?>' + re.escape(token) + r"</w:t></w:r>")


def _tc_wrap_ins(xml: str, token: str, author: str, rid: int) -> str:
    m = _tc_run_pat(token).search(xml)
    assert m, f"ins token run not found: {token!r}"
    repl = (
        f'<w:ins w:id="{rid}" w:author="{author}" w:date="{_TC_DATE}">' + m.group(0) + "</w:ins>"
    )
    return xml[: m.start()] + repl + xml[m.end() :]


def _tc_wrap_del(xml: str, token: str, author: str, rid: int) -> str:
    m = _tc_run_pat(token).search(xml)
    assert m, f"del token run not found: {token!r}"
    space = m.group(1) or ""
    repl = (
        f'<w:del w:id="{rid}" w:author="{author}" w:date="{_TC_DATE}">'
        f"<w:r><w:delText{space}>{token}</w:delText></w:r></w:del>"
    )
    return xml[: m.start()] + repl + xml[m.end() :]


def _tc_track_row(xml: str, row_token: str, author: str, rid: int) -> str:
    i = xml.find(row_token)
    assert i >= 0, f"row token not found: {row_token!r}"
    tr = xml.rfind("<w:tr>", 0, i)
    assert tr >= 0, "enclosing <w:tr> not found"
    open_end = xml.find(">", tr) + 1
    return (
        xml[:open_end]
        + f'<w:trPr><w:ins w:id="{rid}" w:author="{author}" w:date="{_TC_DATE}"/></w:trPr>'
        + xml[open_end:]
    )


def _tc_para(p, parts):
    for text in parts:
        p.add_run(text)


@builder("tracked-changes", "docx")
def tracked_changes(ctx, nnn):
    from docx import Document

    from ..core import zip_read, zip_write

    doc = Document()
    doc.add_paragraph(f"tracked-changes docx {nnn:03d}")

    if nnn == 3:
        ins_tok, del_tok = "inserted fragment", "deleted fragment"
        _tc_para(
            doc.add_paragraph(),
            ["Base text with ", ins_tok, " and ", del_tok, " ending here."],
        )
        t = doc.add_table(rows=2, cols=2)
        t.style = "Table Grid"
        t.cell(0, 0).text = "header-a"
        t.cell(0, 1).text = "header-b"
        t.cell(1, 0).text = "ins-row-a"
        t.cell(1, 1).text = "ins-row-b"
        ref = (
            "tracked-changes docx 003\n"
            "Base text with inserted fragment and ending here.\n"
            "header-a header-b\n"
            "ins-row-a ins-row-b"
        )
        notes = (
            "real w:ins/w:del tracked revisions (w:delText for the deleted run) wrapped around "
            "python-docx runs via zip_read/zip_write, plus a tracked table-row insertion marked "
            "by w:trPr/w:ins on the second row; all w:date fixed to 2026-01-01T00:00:00Z. "
            "Reference text keeps ins text and the inserted row, excludes delText."
        )
    elif nnn == 2:
        _tc_para(
            doc.add_paragraph(),
            ["Section alpha revised: ", "alpha-note-inserted", " stable ", "alpha-note-deleted", " tail."],
        )
        _tc_para(
            doc.add_paragraph(),
            ["Section beta revised: ", "beta-note-inserted", " stable ", "beta-note-deleted", " tail."],
        )
        ref = (
            "tracked-changes docx 002\n"
            "Section alpha revised: alpha-note-inserted stable tail.\n"
            "Section beta revised: beta-note-inserted stable tail."
        )
        notes = (
            "real w:ins/w:del tracked revisions with two distinct w:author values "
            "(bot-a, bot-b); del text carried as w:delText and excluded from reference text. "
            "Runs wrapped via zip_read/zip_write post-processing; w:date fixed."
        )
    else:
        _tc_para(
            doc.add_paragraph(),
            ["Base text with ", "inserted fragment", " and ", "deleted fragment", " ending here."],
        )
        ref = "tracked-changes docx 001\nBase text with inserted fragment and ending here."
        notes = (
            "real w:ins run and w:del run (text converted to w:delText) wrapped around "
            "python-docx runs via zip_read/zip_write; single author bot-a, w:date fixed. "
            "Reference text includes ins text, excludes del text."
        )
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)

    entries = zip_read(path)
    xml = entries["word/document.xml"].decode("utf-8")
    rid = 9001
    if nnn == 2:
        xml = _tc_wrap_ins(xml, "alpha-note-inserted", "bot-a", rid); rid += 1
        xml = _tc_wrap_del(xml, "alpha-note-deleted", "bot-a", rid); rid += 1
        xml = _tc_wrap_ins(xml, "beta-note-inserted", "bot-b", rid); rid += 1
        xml = _tc_wrap_del(xml, "beta-note-deleted", "bot-b", rid); rid += 1
        assert xml.count("<w:ins ") == 2 and xml.count("<w:del ") == 2
    else:
        xml = _tc_wrap_ins(xml, "inserted fragment", "bot-a", rid); rid += 1
        xml = _tc_wrap_del(xml, "deleted fragment", "bot-a", rid); rid += 1
        assert xml.count("<w:ins ") == 1 and xml.count("<w:del ") == 1
        if nnn == 3:
            xml = _tc_track_row(xml, "ins-row-a", "bot-a", rid)
            assert "<w:trPr><w:ins " in xml
    entries["word/document.xml"] = xml.encode("utf-8")
    zip_write(path, entries)
    return BuildResult(path=path, title=f"Tracked changes docx {nnn:03d}", reference_text=ref, notes=notes)


_COMMENTS_CT = "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"
_COMMENTS_RELTYPE = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments"


def _cm_author(i: int) -> str:
    return "bot-a" if i % 2 == 0 else "bot-b"


def _cm_comments_xml(n: int) -> str:
    body = []
    for i in range(n):
        a = _cm_author(i)
        body.append(
            f'<w:comment w:id="{i}" w:author="{a}" w:initials="{a[-1].upper()}" '
            f'w:date="{_TC_DATE}">'
            f"<w:p><w:r><w:t>Comment {i}: reviewer note on claim {i}.</w:t></w:r></w:p>"
            f"</w:comment>"
        )
    return (
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n"
        '<w:comments xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
        + "".join(body)
        + "</w:comments>"
    )


@builder("comments-annotations", "docx")
def comments_annotations(ctx, nnn):
    import re

    from docx import Document

    from ..core import zip_read, zip_write

    n = 3 if nnn == 1 else 5
    doc = Document()
    doc.add_paragraph(f"comments-annotations docx {nnn:03d}")
    for i in range(n):
        p = doc.add_paragraph()
        p.add_run(f"Claim {i} opens ")
        p.add_run(f"anchor-text-{i}")
        p.add_run(f" closes claim {i}.")
    comment_texts = [f"Comment {i}: reviewer note on claim {i}." for i in range(n)]
    ref = (
        f"comments-annotations docx {nnn:03d}\n"
        + "\n".join(
            f"Claim {i} opens anchor-text-{i} closes claim {i}." for i in range(n)
        )
        + "\n"
        + "\n".join(comment_texts)
    )
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)

    entries = zip_read(path)
    xml = entries["word/document.xml"].decode("utf-8")
    for i in range(n):
        token = f"anchor-text-{i}"
        run = f"<w:r><w:t>{token}</w:t></w:r>"
        assert xml.count(run) == 1, f"anchor run not unique: {token}"
        anchored = (
            f'<w:commentRangeStart w:id="{i}"/>'
            + run
            + f'<w:commentRangeEnd w:id="{i}"/>'
            + f'<w:r><w:commentReference w:id="{i}"/></w:r>'
        )
        xml = xml.replace(run, anchored, 1)
    entries["word/document.xml"] = xml.encode("utf-8")

    entries["word/comments.xml"] = _cm_comments_xml(n).encode("utf-8")

    ct = entries["[Content_Types].xml"].decode("utf-8")
    assert "/word/comments.xml" not in ct
    override = f'<Override PartName="/word/comments.xml" ContentType="{_COMMENTS_CT}"/>'
    ct = ct.replace("</Types>", override + "</Types>", 1)
    entries["[Content_Types].xml"] = ct.encode("utf-8")

    rels = entries["word/_rels/document.xml.rels"].decode("utf-8")
    used = [int(m) for m in re.findall(r'Id="rId(\d+)"', rels)]
    next_id = max(used) + 1 if used else 1
    rel = (
        f'<Relationship Id="rId{next_id}" Type="{_COMMENTS_RELTYPE}" Target="comments.xml"/>'
    )
    rels = rels.replace("</Relationships>", rel + "</Relationships>", 1)
    entries["word/_rels/document.xml.rels"] = rels.encode("utf-8")

    assert xml.count("<w:commentRangeStart ") == n and xml.count("<w:commentReference ") == n
    zip_write(path, entries)
    notes = (
        f"real comments support: word/comments.xml part with {n} w:comment elements (ids 0..{n - 1}, "
        "authors bot-a/bot-b, fixed w:date), [Content_Types].xml override and "
        "document.xml.rels relationship added, w:commentRangeStart/w:commentRangeEnd + "
        "w:commentReference wrapped around anchor runs in the body via zip_read/zip_write; "
        "comment texts are also part of the reference text."
    )
    return BuildResult(path=path, title=f"Comments docx {nnn:03d}", reference_text=ref, notes=notes)


def _hf_field_runs(par, instr: str, cached: str) -> None:
    from docx.oxml import parse_xml
    from docx.oxml.ns import nsdecls

    ns = nsdecls("w")
    seq = [
        f'<w:r {ns}><w:fldChar w:fldCharType="begin"/></w:r>',
        f'<w:r {ns}><w:instrText xml:space="preserve"> {instr} </w:instrText></w:r>',
        f'<w:r {ns}><w:fldChar w:fldCharType="separate"/></w:r>',
        f'<w:r {ns}><w:t>{cached}</w:t></w:r>',
        f'<w:r {ns}><w:fldChar w:fldCharType="end"/></w:r>',
    ]
    for raw in seq:
        par._p.append(parse_xml(raw))


def _hf_page_field(par) -> None:
    from docx.oxml import parse_xml
    from docx.oxml.ns import nsdecls

    par._p.append(parse_xml(f'<w:r {nsdecls("w")}><w:t xml:space="preserve">Page </w:t></w:r>'))
    _hf_field_runs(par, "PAGE", "1")
    par._p.append(parse_xml(f'<w:r {nsdecls("w")}><w:t xml:space="preserve"> of </w:t></w:r>'))
    _hf_field_runs(par, "NUMPAGES", "1")


def _hf_date_field(par) -> None:
    from docx.oxml import parse_xml
    from docx.oxml.ns import nsdecls

    par._p.append(parse_xml(f'<w:r {nsdecls("w")}><w:t xml:space="preserve">Rendered </w:t></w:r>'))
    _hf_field_runs(par, 'DATE \\@ "yyyy-MM-dd"', "2026-01-01")


@builder("headers-fields", "docx")
def headers_fields(ctx, nnn):
    from docx import Document

    filler = "Body paragraph {i:02d} of 60: fixed filler sentence for pagination stress. " * 3
    body = [filler.format(i=i) for i in range(1, 61)]

    doc = Document()
    doc.add_paragraph(f"headers-fields docx {nnn:03d}")
    for text in body:
        doc.add_paragraph(text)

    section = doc.sections[0]
    _hf_page_field(section.header.paragraphs[0])
    if nnn == 2:
        _hf_date_field(section.footer.paragraphs[0])

    ref = f"headers-fields docx {nnn:03d}\n" + "\n".join(body)
    notes = (
        "real complex fields (fldChar begin / w:instrText / separate / cached run / end): "
        "PAGE + NUMPAGES fields with cached results in the first-page header"
        + (
            "; footer carries a DATE field (instr 'DATE \\@ \"yyyy-MM-dd\"') with fixed cached "
            "value 2026-01-01"
            if nnn == 2
            else ""
        )
        + ". Body has 60 fixed-text paragraphs so LibreOffice paginates past two pages; "
        "header/footer strings are excluded from reference text because renderers re-evaluate "
        "fields per page. Fields live in header/footer parts, so w:instrText in "
        "word/document.xml stays 0."
    )
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)
    return BuildResult(path=path, title=f"Headers and fields docx {nnn:03d}", reference_text=ref, notes=notes)


_XML_DECL = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
_W_NS = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"

_FN_CT = "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"
_EN_CT = "application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml"
_FN_RELTYPE = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes"
_EN_RELTYPE = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes"


def _note_separators(kind: str) -> str:
    return (
        f'<w:{kind} w:type="separator" w:id="-1">'
        '<w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>'
        "<w:r><w:separator/></w:r></w:p>"
        f"</w:{kind}>"
        f'<w:{kind} w:type="continuationSeparator" w:id="0">'
        '<w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>'
        "<w:r><w:continuationSeparator/></w:r></w:p>"
        f"</w:{kind}>"
    )


def _fn_part(ids, texts):
    notes = "".join(
        f'<w:footnote w:id="{nid}"><w:p><w:r><w:footnoteRef/></w:r>'
        f'<w:r><w:t xml:space="preserve"> {txt}</w:t></w:r></w:p></w:footnote>'
        for nid, txt in zip(ids, texts)
    )
    return (
        _XML_DECL
        + f'<w:footnotes xmlns:w="{_W_NS}">'
        + _note_separators("footnote")
        + notes
        + "</w:footnotes>"
    )


def _en_part(ids, texts):
    notes = "".join(
        f'<w:endnote w:id="{nid}"><w:p><w:r><w:endnoteRef/></w:r>'
        f'<w:r><w:t xml:space="preserve"> {txt}</w:t></w:r></w:p></w:endnote>'
        for nid, txt in zip(ids, texts)
    )
    return (
        _XML_DECL
        + f'<w:endnotes xmlns:w="{_W_NS}">'
        + _note_separators("endnote")
        + notes
        + "</w:endnotes>"
    )


def _rel_next_id(rels: str) -> int:
    import re

    used = [int(m) for m in re.findall(r'Id="rId(\d+)"', rels)]
    return max(used) + 1 if used else 1


@builder("footnotes-endnotes", "docx")
def footnotes_endnotes(ctx, nnn):
    from docx import Document

    from ..core import zip_read, zip_write

    n_fn = 3 if nnn == 1 else 1
    n_en = 2 if nnn == 1 else 1
    Greek = ["alpha", "beta", "gamma"]
    fn_texts = [
        f"Footnote {i} text: supporting note for {Greek[i - 1]}." for i in range(1, n_fn + 1)
    ]
    en_texts = [
        f"Endnote {i} text: endmatter for {Greek[i - 1]}." for i in range(1, n_en + 1)
    ]
    fn_ids = [2 + i for i in range(n_fn)]
    en_ids = [2 + i for i in range(n_en)]

    doc = Document()
    doc.add_paragraph(f"footnotes-endnotes docx {nnn:03d}")
    body_lines = []
    for i in range(1, n_fn + 1):
        line = f"Body claim {i} cites footnote of type {i}."
        p = doc.add_paragraph()
        p.add_run(f"Body claim {i} cites footnote")
        p.add_run(f"fn-anchor-{i}")
        p.add_run(f" of type {i}.")
        body_lines.append(line)
    for i in range(1, n_en + 1):
        line = f"Body claim E{i} cites endnote of type {i}."
        p = doc.add_paragraph()
        p.add_run(f"Body claim E{i} cites endnote")
        p.add_run(f"en-anchor-{i}")
        p.add_run(f" of type {i}.")
        body_lines.append(line)

    ref = (
        f"footnotes-endnotes docx {nnn:03d}\n"
        + "\n".join(body_lines)
        + "\n"
        + "\n".join(fn_texts + en_texts)
    )
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)

    entries = zip_read(path)
    xml = entries["word/document.xml"].decode("utf-8")
    for i, nid in zip(range(1, n_fn + 1), fn_ids):
        run = f"<w:r><w:t>fn-anchor-{i}</w:t></w:r>"
        assert xml.count(run) == 1, f"fn anchor not unique: {i}"
        xml = xml.replace(run, f'<w:r><w:footnoteReference w:id="{nid}"/></w:r>', 1)
    for i, nid in zip(range(1, n_en + 1), en_ids):
        run = f"<w:r><w:t>en-anchor-{i}</w:t></w:r>"
        assert xml.count(run) == 1, f"en anchor not unique: {i}"
        xml = xml.replace(run, f'<w:r><w:endnoteReference w:id="{nid}"/></w:r>', 1)
    entries["word/document.xml"] = xml.encode("utf-8")

    entries["word/footnotes.xml"] = _fn_part(fn_ids, fn_texts).encode("utf-8")
    entries["word/endnotes.xml"] = _en_part(en_ids, en_texts).encode("utf-8")

    ct = entries["[Content_Types].xml"].decode("utf-8")
    assert "/word/footnotes.xml" not in ct and "/word/endnotes.xml" not in ct
    ct = ct.replace(
        "</Types>",
        f'<Override PartName="/word/footnotes.xml" ContentType="{_FN_CT}"/>'
        f'<Override PartName="/word/endnotes.xml" ContentType="{_EN_CT}"/></Types>',
        1,
    )
    entries["[Content_Types].xml"] = ct.encode("utf-8")

    rels = entries["word/_rels/document.xml.rels"].decode("utf-8")
    next_id = _rel_next_id(rels)
    rels = rels.replace(
        "</Relationships>",
        f'<Relationship Id="rId{next_id}" Type="{_FN_RELTYPE}" Target="footnotes.xml"/>'
        f'<Relationship Id="rId{next_id + 1}" Type="{_EN_RELTYPE}" Target="endnotes.xml"/>'
        "</Relationships>",
        1,
    )
    entries["word/_rels/document.xml.rels"] = rels.encode("utf-8")

    assert xml.count("<w:footnoteReference ") == n_fn
    assert xml.count("<w:endnoteReference ") == n_en
    zip_write(path, entries)
    notes = (
        f"real footnotes/endnotes support: word/footnotes.xml + word/endnotes.xml added with the "
        f"ECMA-376 separator convention inside the parts (w:footnote/w:endnote ids -1 "
        f"(w:separator) and 0 (w:continuationSeparator)) plus real notes ids 2..{max(fn_ids[-1], en_ids[-1])}; "
        "settings.xml w:footnotePr/w:endnotePr intentionally omitted (minimum viable per ECMA: "
        "separators live in the notes parts). [Content_Types].xml overrides and document.xml.rels "
        "footnotes/endnotes relationships added; body references via w:footnoteReference/"
        "w:endnoteReference runs wrapped onto anchor runs via zip_read/zip_write. LibreOffice "
        "renders footnote and endnote text into the PDF text layer, so the note strings are part "
        "of the reference text."
    )
    return BuildResult(
        path=path, title=f"Footnotes and endnotes docx {nnn:03d}", reference_text=ref, notes=notes
    )


_WP_NS = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
_A_NS = "http://schemas.openxmlformats.org/drawingml/2006/main"
_WPG_NS = "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"
_WPS_NS = "http://schemas.microsoft.com/office/word/2010/wordprocessingShape"
_DGM_NS = "http://schemas.openxmlformats.org/drawingml/2006/diagram"
_R_NS = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
_SA_SHAPE_URI = "http://schemas.microsoft.com/office/word/2010/wordprocessingShape"
_SA_GROUP_URI = "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"
_SA_DIAGRAM_URI = "http://schemas.openxmlformats.org/drawingml/2006/diagram"
_DIAG_DATA_CT = "application/vnd.openxmlformats-officedocument.drawingml.diagramData+xml"
_DIAG_LAYOUT_CT = "application/vnd.openxmlformats-officedocument.drawingml.diagramLayout+xml"
_DIAG_DATA_RELTYPE = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramData"
_DIAG_LAYOUT_RELTYPE = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramLayout"


def _sa_nsdecls() -> str:
    from docx.oxml.ns import nsdecls

    return (
        nsdecls("w")
        + f' xmlns:wp="{_WP_NS}" xmlns:a="{_A_NS}" xmlns:wpg="{_WPG_NS}"'
        + f' xmlns:wps="{_WPS_NS}" xmlns:dgm="{_DGM_NS}" xmlns:r="{_R_NS}"'
    )


def _sa_wsp(sid, name, prst, text, x, y, cx, cy, fill) -> str:
    return (
        f'<wps:wsp><wps:cNvPr id="{sid}" name="{name}"/><wps:cNvSpPr/>'
        f'<wps:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>'
        f'<a:prstGeom prst="{prst}"><a:avLst/></a:prstGeom>'
        f'<a:solidFill><a:srgbClr val="{fill}"/></a:solidFill>'
        '<a:ln><a:solidFill><a:srgbClr val="101A2E"/></a:solidFill></a:ln></wps:spPr>'
        '<wps:txbx><w:txbxContent><w:p><w:r>'
        f'<w:t>{text}</w:t></w:r></w:p></w:txbxContent></wps:txbx></wps:wsp>'
    )


def _sa_inline(docpr_id, uri, inner, cx, cy) -> str:
    return (
        f'<w:r {_sa_nsdecls()}><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0">'
        f'<wp:extent cx="{cx}" cy="{cy}"/><wp:effectExtent l="0" t="0" r="0" b="0"/>'
        f'<wp:docPr id="{docpr_id}" name="Drawing {docpr_id}"/>'
        f'<a:graphic><a:graphicData uri="{uri}">{inner}</a:graphicData></a:graphic>'
        "</wp:inline></w:drawing></w:r>"
    )


def _sa_group(docpr_id, shapes, cx, cy) -> str:
    inner = (
        '<wpg:wgp><wpg:cNvPr id="100" name="Shape group"/>'
        '<wpg:cNvGrpSpPr/><wpg:grpSpPr><a:xfrm>'
        f'<a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/><a:chExt cx="{cx}" cy="{cy}"/>'
        f"</a:xfrm></wpg:grpSpPr>{shapes}</wpg:wgp>"
    )
    return _sa_inline(docpr_id, _SA_GROUP_URI, inner, cx, cy)


def _sa_data1_xml() -> str:
    return (
        _XML_DECL
        + f'<dgm:dataRoot xmlns:dgm="{_DGM_NS}" xmlns:a="{_A_NS}" '
        'modelId="00000000-0000-0000-0000-000000000001">'
        '<dgm:ptLst>'
        '<dgm:pt modelId="0" type="doc"><dgm:prstGeom prst="rect"/></dgm:pt>'
        '<dgm:pt modelId="1"><dgm:prstGeom prst="rect"/><dgm:text>'
        '<a:bodyPr/><a:p><a:r><a:t>Diagram node: synthetic alpha</a:t></a:r></a:p>'
        "</dgm:text></dgm:pt>"
        '<dgm:pt modelId="2"><dgm:prstGeom prst="rect"/><dgm:text>'
        '<a:bodyPr/><a:p><a:r><a:t>Diagram node: synthetic beta</a:t></a:r></a:p>'
        "</dgm:text></dgm:pt>"
        "</dgm:ptLst><dgm:cxnLst/><dgm:bg/><dgm:whole/><dgm:extLst/></dgm:dataRoot>"
    )


def _sa_layout1_xml() -> str:
    return (
        _XML_DECL
        + f'<dgm:layoutDefinition xmlns:dgm="{_DGM_NS}">'
        '<dgm:type guid="urn:microsoft.com/office/officeart/2005/8/layout/default"/>'
        "<dgm:varLst/><dgm:extLst/></dgm:layoutDefinition>"
    )


@builder("smartart-drawing", "docx")
def smartart_drawing(ctx, nnn):
    from docx import Document
    from docx.oxml import parse_xml

    from ..core import zip_read, zip_write

    doc = Document()
    doc.add_paragraph(f"smartart-drawing docx {nnn:03d}")
    doc.add_paragraph("Two DrawingML shapes stand in for a diagram below.")
    labels = []
    if nnn == 1:
        t1 = "SmartArt node: intake process"
        t2 = "SmartArt node: output check"
        doc.add_paragraph()._p.append(
            parse_xml(_sa_inline(1, _SA_SHAPE_URI, _sa_wsp(1, "Node rect", "rect", t1, 0, 0, 2200000, 900000, "93A0B8"), 2200000, 900000))
        )
        doc.add_paragraph()._p.append(
            parse_xml(_sa_inline(2, _SA_SHAPE_URI, _sa_wsp(2, "Node ellipse", "ellipse", t2, 0, 0, 2200000, 900000, "F4F1EA"), 2200000, 900000))
        )
        labels = [t1, t2]
        notes = (
            "python-docx cannot emit diagrams: raw w:drawing with two separate inline wps:wsp "
            "shapes (rect + ellipse) carrying wps:txbx text, injected via parse_xml. LibreOffice "
            "renders shape txbx text into the PDF text layer, so the labels are reference text. "
            "DrawingML shapes only — no diagram parts, not MS-generated SmartArt."
        )
    else:
        t1 = "Group node: plan"
        t2 = "Group node: review"
        shapes = (
            _sa_wsp(1, "Group ellipse a", "ellipse", t1, 0, 0, 2000000, 1000000, "93A0B8")
            + _sa_wsp(2, "Group ellipse b", "ellipse", t2, 2200000, 0, 2000000, 1000000, "F4F1EA")
        )
        doc.add_paragraph()._p.append(parse_xml(_sa_group(3, shapes, 4400000, 1000000)))
        labels = [t1, t2]
        notes = (
            "python-docx cannot emit diagrams: raw w:drawing with one inline wpg:wgp group "
            "containing two wps:wsp ellipse shapes carrying wps:txbx text, injected via parse_xml. "
            "LibreOffice renders shape txbx text into the PDF text layer, so the labels are "
            "reference text. DrawingML shape group only — no diagram parts in 00{0}, not "
            "MS-generated SmartArt.".format(nnn)
        )
        if nnn == 3:
            doc.add_paragraph("A synthetic diagramData reference follows.")
            diag = (
                '<dgm:relIds><dgm:dm r:id="__RID_DM__"/><dgm:lo r:id="__RID_LO__"/></dgm:relIds>'
            )
            doc.add_paragraph()._p.append(
                parse_xml(_sa_inline(4, _SA_DIAGRAM_URI, diag, 4400000, 1000000))
            )
            notes = (
                "python-docx cannot emit diagrams: w:drawing with a wpg:wgp group of two "
                "wps:txbx text shapes, PLUS a second w:drawing whose a:graphicData (uri "
                "drawingml/2006/diagram) carries dgm:relIds to synthetic diagram parts: "
                "word/diagrams/data1.xml (dgm:dataRoot with ptLst nodes) + word/diagrams/"
                "layout1.xml (minimal dgm:layoutDefinition), with diagramData/diagramLayout "
                "relationships in document.xml.rels and Content_Types overrides added. "
                "Synthetic diagram parts, not MS-generated SmartArt (no MS produced these); "
                "LibreOffice may ignore them entirely, so the diagram node strings are kept OUT "
                "of the reference text; the two shape txbx labels are real DrawingML and ARE "
                "reference text."
            )

    extra = "A synthetic diagramData reference follows.\n" if nnn == 3 else ""
    ref = (
        f"smartart-drawing docx {nnn:03d}\n"
        "Two DrawingML shapes stand in for a diagram below.\n"
        + extra
        + "\n".join(labels)
    )
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)

    entries = zip_read(path)
    if nnn == 3:
        xml = entries["word/document.xml"].decode("utf-8")
        rels = entries["word/_rels/document.xml.rels"].decode("utf-8")
        next_id = _rel_next_id(rels)
        rid_dm, rid_lo = next_id, next_id + 1
        xml = xml.replace("__RID_DM__", f"rId{rid_dm}").replace("__RID_LO__", f"rId{rid_lo}")
        entries["word/document.xml"] = xml.encode("utf-8")
        rels = rels.replace(
            "</Relationships>",
            f'<Relationship Id="rId{rid_dm}" Type="{_DIAG_DATA_RELTYPE}" Target="diagrams/data1.xml"/>'
            f'<Relationship Id="rId{rid_lo}" Type="{_DIAG_LAYOUT_RELTYPE}" Target="diagrams/layout1.xml"/>'
            "</Relationships>",
            1,
        )
        entries["word/_rels/document.xml.rels"] = rels.encode("utf-8")
        entries["word/diagrams/data1.xml"] = _sa_data1_xml().encode("utf-8")
        entries["word/diagrams/layout1.xml"] = _sa_layout1_xml().encode("utf-8")
        ct = entries["[Content_Types].xml"].decode("utf-8")
        assert "/word/diagrams/" not in ct
        ct = ct.replace(
            "</Types>",
            f'<Override PartName="/word/diagrams/data1.xml" ContentType="{_DIAG_DATA_CT}"/>'
            f'<Override PartName="/word/diagrams/layout1.xml" ContentType="{_DIAG_LAYOUT_CT}"/></Types>',
            1,
        )
        entries["[Content_Types].xml"] = ct.encode("utf-8")
    zip_write(path, entries)
    return BuildResult(
        path=path, title=f"SmartArt-like drawing docx {nnn:03d}", reference_text=ref, notes=notes
    )


_OLE_CT = "application/vnd.openxmlformats-officedocument.oleObject"
_OLE_RELTYPE = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/oleObject"
_OLE_PNG_B64 = (
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
)


def _ole_cfb_stub() -> bytes:
    """Hand-built minimal CFB (v3): header + Root Entry dir sector + FAT sector."""
    import struct

    endofchain, fatsect, freesect = 0xFFFFFFFE, 0xFFFFFFFD, 0xFFFFFFFF
    hdr = bytearray(512)
    hdr[0:8] = bytes.fromhex("D0CF11E0A1B11AE1")
    struct.pack_into("<HH", hdr, 24, 0x003E, 0x0003)  # minor, major version
    struct.pack_into("<H", hdr, 28, 0xFFFE)  # little-endian byte order
    struct.pack_into("<HH", hdr, 30, 9, 6)  # 512-byte sectors, 64-byte mini sectors
    struct.pack_into("<I", hdr, 40, 0)  # num dir sectors (must be 0 for v3)
    struct.pack_into("<I", hdr, 44, 1)  # num FAT sectors
    struct.pack_into("<I", hdr, 48, 0)  # first dir sector = sector 0
    struct.pack_into("<I", hdr, 52, 0)  # transaction signature
    struct.pack_into("<I", hdr, 56, 4096)  # mini stream cutoff
    struct.pack_into("<I", hdr, 60, endofchain)  # first mini FAT sector
    struct.pack_into("<I", hdr, 64, 0)  # num mini FAT sectors
    struct.pack_into("<I", hdr, 68, endofchain)  # first DIFAT sector
    struct.pack_into("<I", hdr, 72, 0)  # num DIFAT sectors
    struct.pack_into("<I", hdr, 76, 1)  # DIFAT[0] = FAT lives in sector 1
    for i in range(1, 109):
        struct.pack_into("<I", hdr, 76 + 4 * i, freesect)

    d = bytearray(512)
    name = "Root Entry".encode("utf-16-le")
    d[0 : len(name)] = name
    struct.pack_into("<H", d, 64, len(name) + 2)
    d[66] = 0x05  # storage type: root
    d[67] = 0x01  # color: black
    for off in (68, 72, 76):  # left / right / child = NOSTREAM
        struct.pack_into("<I", d, off, 0xFFFFFFFF)
    struct.pack_into("<I", d, 116, endofchain)  # start sector
    struct.pack_into("<Q", d, 120, 0)  # stream size

    fat = bytearray(512)
    struct.pack_into("<I", fat, 0, endofchain)  # sector 0 = dir chain end
    struct.pack_into("<I", fat, 4, fatsect)  # sector 1 = the FAT itself
    for i in range(2, 128):
        struct.pack_into("<I", fat, 4 * i, freesect)
    return bytes(hdr + d + fat)


@builder("embedded-ole", "docx")
def embedded_ole(ctx, nnn):
    import base64

    from docx import Document
    from docx.oxml import parse_xml
    from docx.oxml.ns import nsdecls

    from ..core import zip_read, zip_write

    png = base64.b64decode(_OLE_PNG_B64)
    if nnn == 1:
        blob, progid = png, "Package"
    else:
        blob, progid = _ole_cfb_stub(), "Composite.Document"

    doc = Document()
    doc.add_paragraph(f"embedded-ole docx {nnn:03d}")
    doc.add_paragraph("This paragraph precedes an embedded OLE object (legacy w:pict with o:ole).")
    pict = parse_xml(
        f'<w:r {nsdecls("w")} xmlns:v="urn:schemas-microsoft-com:vml" '
        'xmlns:o="urn:schemas-microsoft-com:office:office">'
        '<w:pict><v:shape id="OLEObject1" type="#_x0000_t75" '
        'style="width:72pt;height:72pt" o:ole="__OLE_RID__">'
        f'<o:OLEObject Type="Embed" ProgID="{progid}" DrawAspect="Icon" '
        'ObjectID="_1770000001" ShapeID="OLEObject1"/>'
        "</v:shape></w:pict></w:r>"
    )
    doc.add_paragraph()._p.append(pict)
    doc.add_paragraph("Body line after the embedded object.")

    ref = (
        f"embedded-ole docx {nnn:03d}\n"
        "This paragraph precedes an embedded OLE object (legacy w:pict with o:ole).\n"
        "Body line after the embedded object."
    )
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)

    entries = zip_read(path)
    xml = entries["word/document.xml"].decode("utf-8")
    rels = entries["word/_rels/document.xml.rels"].decode("utf-8")
    rid = _rel_next_id(rels)
    assert "__OLE_RID__" in xml
    xml = xml.replace("__OLE_RID__", f"rId{rid}")
    entries["word/document.xml"] = xml.encode("utf-8")
    rels = rels.replace(
        "</Relationships>",
        f'<Relationship Id="rId{rid}" Type="{_OLE_RELTYPE}" Target="embeddings/object1.bin"/>'
        "</Relationships>",
        1,
    )
    entries["word/_rels/document.xml.rels"] = rels.encode("utf-8")
    entries["word/embeddings/object1.bin"] = blob

    ct = entries["[Content_Types].xml"].decode("utf-8")
    assert "/word/embeddings/" not in ct
    ct = ct.replace(
        "</Types>",
        f'<Override PartName="/word/embeddings/object1.bin" ContentType="{_OLE_CT}"/></Types>',
        1,
    )
    entries["[Content_Types].xml"] = ct.encode("utf-8")
    zip_write(path, entries)

    stub = (
        "the bin part is a 1x1 PNG byte stub declared with the oleObject relationship type and "
        "oleObject content type — the type/content mismatch is intentional: an honest stub, "
        "no CFB structure inside"
        if nnn == 1
        else "the bin part is a hand-built minimal compound file (D0CF11E0A1B11AE1 magic, v3, "
        "512-byte sectors: header + Root Entry directory sector + one FAT sector, no payload "
        "streams) because olefile is read-only; a real CFB container, but an OLE stub with no "
        "server payload"
    )
    notes = (
        f"real package-level embedding plumbing, stub payload: word/embeddings/object1.bin added "
        f"({stub}); body carries a legacy w:pict with v:shape o:ole=rId{rid} plus an o:OLEObject "
        "child (Type=Embed, DrawAspect=Icon) — no w:drawing used, so the w:drawing metric stays 0; "
        "document.xml.rels oleObject relationship + Content_Types oleObject override added via "
        "zip_read/zip_write. LibreOffice may show an icon or render nothing for the stub; the "
        "embed carries no text truth, so reference_text covers only the body text."
    )
    return BuildResult(
        path=path, title=f"Embedded OLE object docx {nnn:03d}", reference_text=ref, notes=notes
    )


_SC_PAPER = (238, 236, 230)
_SC_INK = (110, 108, 104)


def _sc_scan_png(path, seed):
    import numpy as np
    from PIL import Image, ImageDraw

    w, h = 300, 360
    img = Image.new("RGB", (w, h), _SC_PAPER)
    draw = ImageDraw.Draw(img)
    y = 18
    while y < h - 20:
        x = 16
        while x < w - 24:
            word = 18 + (y * 31 + x * 7) % 34
            draw.rectangle([x, y, min(x + word, w - 16), y + 5], fill=_SC_INK)
            x += word + 7
        y += 14
    rng = np.random.default_rng(seed)
    arr = np.asarray(img).astype(np.int16)
    arr = np.clip(arr + rng.integers(-6, 7, size=arr.shape, dtype=np.int16), 0, 255)
    path.write_bytes(png_rgb_bytes(Image.fromarray(arr.astype("uint8"), "RGB")))


@builder("scanned-ocr", "docx")
def scanned_ocr(ctx, nnn):
    from docx import Document
    from docx.shared import Inches

    png = ctx.tmp / "scan-page.png"
    _sc_scan_png(png, ctx.rng.randrange(1 << 32))

    doc = Document()
    doc.add_picture(str(png), width=Inches(6.5))
    caption = "Figure 1 — scanned page fixture"
    doc.add_paragraph(caption)

    ref = caption
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)
    notes = (
        "scanned-page emulation: a 300px-wide synthetic raster (off-white paper gray, gray "
        "pseudo-text word bars, seeded per-pixel noise) rendered with PIL and embedded as a "
        "real PNG media part via python-docx add_picture at 6.5in full-page width, with one "
        "caption line below. The image has NO text layer (bars, not glyphs), so there is no "
        "OCR truth inside the picture: reference_text is the caption ONLY — the honest "
        "near-None case; the noise seed derives from the fixture-seeded ctx.rng."
    )
    return BuildResult(path=path, title=f"Scanned page docx {nnn:03d}", reference_text=ref, notes=notes)


_ITW_LINE = (
    "Wrap line {i:02d}: fixed filler sentence that flows around the picture edge and keeps "
    "the wrap boundary exercised across several lines. "
)


def _itw_png(path, w, h):
    from PIL import Image, ImageDraw

    img = Image.new("RGB", (w, h), (244, 241, 234))
    draw = ImageDraw.Draw(img)
    draw.rectangle([4, 4, w - 5, h - 5], outline=(147, 160, 184), width=3)
    draw.line([4, 4, w - 5, h - 5], fill=(147, 160, 184), width=2)
    draw.line([4, h - 5, w - 5, 4], fill=(147, 160, 184), width=2)
    path.write_bytes(png_rgb_bytes(img))


@builder("image-textwrap", "docx")
def image_textwrap(ctx, nnn):
    import re

    from docx import Document
    from docx.shared import Inches

    from ..core import zip_read, zip_write

    doc = Document()
    if nnn == 1:
        png = ctx.tmp / "inline.png"
        _itw_png(png, 200, 150)
        doc.add_paragraph(f"image-textwrap docx {nnn:03d}")
        p = doc.add_paragraph()
        p.add_run("Before picture: ")
        p.add_run().add_picture(str(png), width=Inches(2))
        p.add_run(" after picture.")
        lines = [_ITW_LINE.format(i=i) for i in range(1, 5)]
        for line in lines:
            doc.add_paragraph(line)
        ref = (
            f"image-textwrap docx 001\n"
            "Before picture:  after picture.\n"
            + "\n".join(line.strip() for line in lines)
        )
        notes = (
            "real inline picture: run.add_picture emits w:drawing/wp:inline with a real PNG "
            "media part + document.xml.rels image relationship; surrounding text runs and "
            "paragraphs provide the inline flow. Inline-only (no floating anchor) in 001."
        )
    else:
        png = ctx.tmp / "float.png"
        _itw_png(png, 200, 150)
        doc.add_paragraph(f"image-textwrap docx {nnn:03d}")
        doc.add_paragraph("A floating square-wrapped picture follows in this paragraph.")
        p = doc.add_paragraph()
        p.add_run().add_picture(str(png), width=Inches(2), height=Inches(1.5))
        lines = [_ITW_LINE.format(i=i) for i in range(1, 9)]
        for line in lines:
            doc.add_paragraph(line)
        ref = (
            f"image-textwrap docx 002\n"
            "A floating square-wrapped picture follows in this paragraph.\n"
            + "\n".join(line.strip() for line in lines)
        )
        notes = (
            "real floating picture with square wrap: python-docx has no float API, so "
            "add_picture creates the image part/rels/inline drawing first, then "
            "word/document.xml is rewritten via zip_read/zip_write to upgrade wp:inline to "
            "wp:anchor (distT/B/L/R, simplePos, relativeHeight, behindDoc, locked, "
            "layoutInCell, allowOverlap, positionH/positionV) with wp:wrapSquare "
            "wrapSide=\"both\" and wp:extent 2in x 1.5in (1828800 x 1371600 EMU); the "
            "a:graphic/pic:pic payload and media rel are the real python-docx ones."
        )

    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)

    if nnn == 2:
        entries = zip_read(path)
        xml = entries["word/document.xml"].decode("utf-8")
        m = re.search(r"<wp:inline\b[^>]*>(.*?)</wp:inline>", xml, re.S)
        assert m, "inline drawing not found for anchor upgrade"
        inner = m.group(1)
        ex = re.search(r"<wp:extent\b[^>]*/>", inner)
        assert ex, "wp:extent not found inside inline drawing"
        inner = (
            inner[: ex.end()]
            + '<wp:effectExtent l="0" t="0" r="0" b="0"/>'
            + '<wp:wrapSquare wrapSide="both"/>'
            + inner[ex.end() :]
        )
        open_tag = m.group(0)[: m.group(0).index(">")]
        ns = "".join(f" {d}" for d in re.findall(r'xmlns:[\w.-]+="[^"]*"', open_tag))
        anchor = (
            f"<wp:anchor{ns} "
            'distT="0" distB="0" distL="114300" distR="114300" simplePos="0" '
            'relativeHeight="251659264" behindDoc="0" locked="0" layoutInCell="1" '
            'allowOverlap="1">'
            '<wp:simplePos x="0" y="0"/>'
            '<wp:positionH relativeFrom="column"><wp:posOffset>182880</wp:posOffset></wp:positionH>'
            '<wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV>'
            + inner
            + "</wp:anchor>"
        )
        xml = xml[: m.start()] + anchor + xml[m.end() :]
        assert "<wp:inline" not in xml and xml.count("<wp:anchor ") == 1
        assert '<wp:wrapSquare wrapSide="both"/>' in xml
        assert 'cx="1828800" cy="1371600"' in xml
        entries["word/document.xml"] = xml.encode("utf-8")
        zip_write(path, entries)
    return BuildResult(
        path=path, title=f"Image text wrap docx {nnn:03d}", reference_text=ref, notes=notes
    )


_LT_WORDS = (
    "alpha", "bravo", "charlie", "delta", "echo",
    "foxtrot", "golf", "hotel", "india", "juliet",
)


@builder("long-tables", "docx")
def long_tables(ctx, nnn):
    from docx import Document

    doc = Document()
    intro = f"long-tables docx {nnn:03d}: intro paragraph before the 70-row grid."
    outro = "Outro paragraph after the 70-row grid; pagination should exceed two pages."
    doc.add_paragraph(intro)

    rows = 71 if nnn == 2 else 70
    t = doc.add_table(rows=rows, cols=4)
    t.style = "Table Grid"
    row_texts = []
    data0 = 0
    if nnn == 2:
        head = t.cell(0, 0).merge(t.cell(0, 3))  # horizontal -> gridSpan on every covered cell
        head.text = "Header merged across all four columns"
        row_texts.append("Header merged across all four columns")
        data0 = 1
    for r in range(70):
        cells = []
        for c in range(4):
            txt = f"r{r}c{c}-{_LT_WORDS[(r + c) % len(_LT_WORDS)]}"
            t.cell(data0 + r, c).text = txt
            cells.append(txt)
        row_texts.append(" ".join(cells))
    doc.add_paragraph(outro)

    ref = "\n".join([intro] + row_texts + [outro])
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)
    if nnn == 2:
        notes = (
            "real table stress: one Table Grid table, 70 data rows x 4 cols plus a merged "
            "header row (python-docx merge across the row emits w:gridSpan on the surviving "
            "cell), intro + outro paragraphs around it so LibreOffice paginates past two "
            "pages; every cell carries r{r}c{c}-word text and all cell strings are in the "
            "reference text."
        )
    else:
        notes = (
            "real table stress: one plain Table Grid table with 70 rows x 4 cols, every cell "
            "carrying r{r}c{c}-word text, intro + outro paragraphs around it so LibreOffice "
            "paginates past two pages; all cell strings are in the reference text."
        )
    return BuildResult(path=path, title=f"Long tables docx {nnn:03d}", reference_text=ref, notes=notes)


def _cs_style_block(styles_xml: str, style_id: str) -> str:
    import re

    m = re.search(
        rf'<w:style [^>]*w:styleId="{style_id}">.*?</w:style>', styles_xml, re.S
    )
    assert m, f"style {style_id} missing from styles.xml"
    return m.group(0)


@builder("custom-styles", "docx")
def custom_styles(ctx, nnn):
    from docx import Document
    from docx.enum.style import WD_STYLE_TYPE
    from docx.shared import Pt, RGBColor

    from ..core import zip_read

    doc = Document()
    if nnn == 1:
        body = doc.styles.add_style("GP Body", WD_STYLE_TYPE.PARAGRAPH)
        body.base_style = doc.styles["Normal"]
        body.font.size = Pt(11)
        head = doc.styles.add_style("GP Head", WD_STYLE_TYPE.PARAGRAPH)
        head.base_style = doc.styles["Heading 1"]
        head.font.color.rgb = RGBColor(0x6B, 0x3F, 0xA0)
        texts = [
            "custom-styles docx 001",
            "Body paragraph one rendered through GP Body.",
            "Body paragraph two rendered through GP Body.",
        ]
        doc.add_paragraph(texts[0], style="GP Head")
        for text in texts[1:]:
            doc.add_paragraph(text, style="GP Body")
        ref = "\n".join(texts)
        notes = (
            "real custom paragraph styles defined in word/styles.xml via "
            "doc.styles.add_style(builtin=False): GP Body basedOn Normal (w:basedOn "
            "w:val=\"Normal\", size override) and GP Head basedOn Heading 1 "
            "(w:basedOn w:val=\"Heading1\", own w:color 6B3FA0); both styles are applied to "
            "body paragraphs via w:pPr/w:pStyle. styles.xml content asserted after save."
        )

        pin_docx_props(doc)
        path = ctx.tmp / "fixture.docx"
        doc.save(path)
        sx = zip_read(path)["word/styles.xml"].decode("utf-8")
        b = _cs_style_block(sx, "GPBody")
        h = _cs_style_block(sx, "GPHead")
        assert '<w:name w:val="GP Body"/>' in b and '<w:basedOn w:val="Normal"/>' in b
        assert '<w:name w:val="GP Head"/>' in h
        assert '<w:basedOn w:val="Heading1"/>' in h
        assert '<w:color w:val="6B3FA0"/>' in h
    else:
        base = doc.styles.add_style("GP Base", WD_STYLE_TYPE.PARAGRAPH)
        base.base_style = doc.styles["Normal"]
        base.font.size = Pt(12)
        mid = doc.styles.add_style("GP Mid", WD_STYLE_TYPE.PARAGRAPH)
        mid.base_style = base
        mid.font.italic = True
        leaf = doc.styles.add_style("GP Leaf", WD_STYLE_TYPE.PARAGRAPH)
        leaf.base_style = mid
        leaf.font.color.rgb = RGBColor(0x0B, 0x66, 0x55)
        char = doc.styles.add_style("GP Char", WD_STYLE_TYPE.CHARACTER)
        char.font.underline = True

        doc.add_paragraph("custom-styles docx 002", style="GP Leaf")
        p = doc.add_paragraph("GP Mid chain paragraph, italic and 12pt through inheritance.", style="GP Mid")
        p2 = doc.add_paragraph(style="GP Leaf")
        p2.add_run("Leaf chain paragraph with ")
        run = p2.add_run("underlined character style")
        run.style = doc.styles["GP Char"]
        p2.add_run(" inside.")
        ref = (
            "custom-styles docx 002\n"
            "GP Mid chain paragraph, italic and 12pt through inheritance.\n"
            "Leaf chain paragraph with underlined character style inside."
        )
        notes = (
            "real 3-deep basedOn chain in word/styles.xml: GP Base basedOn Normal -> GP Mid "
            "basedOn GP Base -> GP Leaf basedOn GP Mid (size 12pt + italic + w:color 0B6655 "
            "resolve by inheritance), plus a custom CHARACTER style GP Char with w:u "
            "w:val=\"single\" applied to one run via w:rStyle; all applied to body "
            "paragraphs/runs. styles.xml chain and underline asserted after save."
        )

        pin_docx_props(doc)
        path = ctx.tmp / "fixture.docx"
        doc.save(path)
        sx = zip_read(path)["word/styles.xml"].decode("utf-8")
        b = _cs_style_block(sx, "GPBase")
        m = _cs_style_block(sx, "GPMid")
        l = _cs_style_block(sx, "GPLeaf")
        c = _cs_style_block(sx, "GPChar")
        assert '<w:basedOn w:val="Normal"/>' in b
        assert '<w:basedOn w:val="GPBase"/>' in m
        assert '<w:basedOn w:val="GPMid"/>' in l
        assert 'w:type="character"' in c and '<w:u w:val="single"/>' in c
    return BuildResult(
        path=path, title=f"Custom styles docx {nnn:03d}", reference_text=ref, notes=notes
    )


_CJK_LINES_001 = [
    "床前明月光",
    "疑是地上霜",
    "举头望明月",
    "低头思故乡",
    "春眠不觉晓",
    "处处闻啼鸟",
    "夜来风雨声",
    "花落知多少",
]

_CJK_LINES_002 = [
    "古池や 蛙飛びこむ 水の音",
    "五月雨を あつめて早し 最上川",
    "閑さや 岩にしみ入る 蝉の声",
    "동해 물과 백두산이 마르고 닳도록",
]


def _cjk_paragraph(doc, text, eastasia_font):
    from docx.oxml import parse_xml
    from docx.oxml.ns import nsdecls

    p = doc.add_paragraph()
    r = p.add_run(text)
    r._r.insert(
        0,
        parse_xml(
            f'<w:rPr {nsdecls("w")}><w:rFonts w:eastAsia="{eastasia_font}"/></w:rPr>'
        ),
    )
    return p


@builder("cjk-text", "docx")
def cjk_text(ctx, nnn):
    from docx import Document

    doc = Document()
    doc.add_paragraph(f"cjk-text docx {nnn:03d}")
    if nnn == 1:
        lines = _CJK_LINES_001
        for text in lines:
            _cjk_paragraph(doc, text, "Noto Serif CJK SC")
        notes = (
            "real public-domain Chinese tang poetry: Li Bai 静夜思 (床前明月光…故乡) and "
            "Meng Haoran 春晓 (春眠不觉晓…多少), 4 lines each, one paragraph per line; every "
            "run carries an explicit eastAsia font hint w:rFonts w:eastAsia=\"Noto Serif CJK SC\" "
            "injected as raw w:rPr via parse_xml (python-docx font.name only writes "
            "ascii/hAnsi). Reference text is the exact inserted strings; downstream F1 applies "
            "the C9 §10.6 CJK per-char tokenization rule."
        )
    else:
        lines = _CJK_LINES_002
        for text in lines[:3]:
            _cjk_paragraph(doc, text, "Noto Serif CJK JP")
        _cjk_paragraph(doc, lines[3], "Noto Serif CJK KR")
        notes = (
            "real public-domain East Asian text: three Bashō haiku (古池や 蛙飛びこむ 水の音 / "
            "五月雨を あつめて早し 最上川 / 閑さや 岩にしみ入る 蝉の声) with eastAsia hint "
            "\"Noto Serif CJK JP\", plus the first line of the Korean 애국가 (동해 물과 "
            "백두산이 마르고 닳도록, public domain) with eastAsia hint \"Noto Serif CJK KR\"; "
            "eastAsia rFonts injected as raw w:rPr via parse_xml. Reference text is the exact "
            "inserted strings including the inter-phrase spaces; CJK per-char tokenization "
            "(C9 §10.6) applies downstream."
        )
    ref = f"cjk-text docx {nnn:03d}\n" + "\n".join(lines)
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)

    from ..core import zip_read

    xml = zip_read(path)["word/document.xml"].decode("utf-8")
    for text in lines:
        assert f"<w:t>{text}</w:t>" in xml, f"cjk line missing: {text!r}"
    assert xml.count('w:eastAsia="Noto Serif CJK') == len(lines)
    return BuildResult(
        path=path, title=f"CJK text docx {nnn:03d}", reference_text=ref, notes=notes
    )


_ARABIC_ALPHABET = "ا ب ت ث ج ح خ د ذ ر ز س ش ص ض ط ظ ع غ ف ق ك ل م ن ه و ي"
_ARABIC_FOLK = "كان الزمان زمان"

_HEBREW_LINES_002 = [
    "עברית שלום",
    "המדען הקטן קרא ספר גדול",
    "הילדים שיחקו בפארק בבוקר",
    "השמש זרחה מעל ההרים",
]


def _rtl_paragraph(doc, runs, cs_font):
    from docx.oxml import parse_xml
    from docx.oxml.ns import nsdecls

    p = doc.add_paragraph()
    p._p.insert(
        0,
        parse_xml(f'<w:pPr {nsdecls("w")}><w:bidi/><w:jc w:val="right"/></w:pPr>'),
    )
    for text, is_ltr in runs:
        r = p.add_run(text)
        if is_ltr:
            rPr = parse_xml(f'<w:rPr {nsdecls("w")}><w:rtl w:val="0"/></w:rPr>')
        else:
            rPr = parse_xml(
                f'<w:rPr {nsdecls("w")}><w:rFonts w:cs="{cs_font}"/></w:rPr>'
            )
        r._r.insert(0, rPr)
    return p


@builder("rtl-text", "docx")
def rtl_text(ctx, nnn):
    from docx import Document

    doc = Document()
    doc.add_paragraph(f"rtl-text docx {nnn:03d}")
    if nnn == 1:
        para_texts = [_ARABIC_ALPHABET, _ARABIC_FOLK, _ARABIC_FOLK, _ARABIC_FOLK]
        for text in para_texts:
            _rtl_paragraph(doc, [(text, False)], "Noto Naskh Arabic")
        ref = "rtl-text docx 001\n" + "\n".join(para_texts)
        notes = (
            "real RTL paragraphs: Arabic 28-letter alphabet demo plus the traditional "
            f"public-domain folk opening «{_ARABIC_FOLK}» (×3 paragraphs, neutral, "
            "bismillah-free); every paragraph carries w:bidi + w:jc w:val=\"right\" in w:pPr "
            "and w:rFonts w:cs=\"Noto Naskh Arabic\" in run w:rPr (raw oxml, python-docx has "
            "no bidi API). Reference text is the exact inserted strings in logical order."
        )
    else:
        for text in _HEBREW_LINES_002:
            _rtl_paragraph(doc, [(text, False)], "Noto Sans Hebrew")
        mixed = [("המועדון החדש נקרא ", False), ("stichos", True), (" בימי שישי", False)]
        _rtl_paragraph(doc, mixed, "Noto Sans Hebrew")
        mixed_ref = "".join(t for t, _ in mixed)
        ref = "rtl-text docx 002\n" + "\n".join(_HEBREW_LINES_002) + "\n" + mixed_ref
        notes = (
            "real RTL Hebrew paragraphs (w:bidi + w:jc right in w:pPr, w:rFonts "
            "w:cs=\"Noto Sans Hebrew\" per run): greeting עברית שלום plus three simple "
            "unencumbered sentences (המדען הקטן קרא ספר גדול …), then one mixed-direction "
            "test paragraph where the embedded Latin run «stichos» is marked "
            "w:rtl w:val=\"0\" inside the RTL paragraph. Reference text is the exact inserted "
            "strings in logical order; bidi reordering is a render-time concern."
        )
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)

    from ..core import zip_read

    xml = zip_read(path)["word/document.xml"].decode("utf-8")
    nrtl = 4 if nnn == 1 else 5
    assert xml.count("<w:bidi/>") == nrtl
    if nnn == 1:
        assert xml.count('w:cs="Noto Naskh Arabic"') == nrtl
    else:
        assert xml.count('w:cs="Noto Sans Hebrew"') == nrtl + 1
        assert xml.count('<w:rtl w:val="0"/>') == 1
    return BuildResult(
        path=path, title=f"RTL text docx {nnn:03d}", reference_text=ref, notes=notes
    )


def _num_lvl(ilvl, fmt, lvl_text, left):
    return (
        f'<w:lvl w:ilvl="{ilvl}"><w:start w:val="1"/>'
        f'<w:numFmt w:val="{fmt}"/><w:lvlText w:val="{lvl_text}"/>'
        f'<w:lvlJc w:val="left"/>'
        f'<w:pPr><w:ind w:left="{left}" w:hanging="360"/></w:pPr></w:lvl>'
    )


def _numbering_xml(levels, override_ilvls):
    abstract = (
        '<w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="hybridMultilevel"/>'
        + "".join(levels)
        + "</w:abstractNum>"
    )
    num1 = '<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>'
    overrides = "".join(
        f'<w:lvlOverride w:ilvl="{i}"><w:startOverride w:val="1"/></w:lvlOverride>'
        for i in override_ilvls
    )
    num2 = f'<w:num w:numId="2"><w:abstractNumId w:val="0"/>{overrides}</w:num>'
    return (
        _XML_DECL
        + f'<w:numbering xmlns:w="{_W_NS}">'
        + abstract
        + num1
        + num2
        + "</w:numbering>"
    )


def _numbered_paragraph(doc, text, num_id, ilvl):
    from docx.oxml import parse_xml
    from docx.oxml.ns import nsdecls

    p = doc.add_paragraph(text)
    p._p.get_or_add_pPr().append(
        parse_xml(
            f'<w:numPr {nsdecls("w")}><w:ilvl w:val="{ilvl}"/>'
            f'<w:numId w:val="{num_id}"/></w:numPr>'
        )
    )
    return p


@builder("numbering-restarts", "docx")
def numbering_restarts(ctx, nnn):
    from docx import Document

    from ..core import zip_read, zip_write

    doc = Document()
    doc.add_paragraph(f"numbering-restarts docx {nnn:03d}")
    if nnn == 1:
        levels = [
            _num_lvl(0, "decimal", "%1.", 720),
            _num_lvl(1, "lowerLetter", "%2.", 1440),
        ]
        override_ilvls = [0]
        pattern = [0, 0, 1, 0, 0]
        items = []
        for num_id, label in ((1, "Alpha"), (2, "Bravo")):
            for i, ilvl in enumerate(pattern, start=1):
                text = f"{label} item {i}"
                _numbered_paragraph(doc, text, num_id, ilvl)
                items.append(text)
        ref = f"numbering-restarts docx 001\n" + "\n".join(items)
        notes = (
            "real numbering part, raw XML: word/numbering.xml replaced wholesale with one "
            "abstractNum (lvl 0 decimal \"%1.\", lvl 1 lowerLetter \"%2.\") and two w:num "
            "instances — numId 1 plain, numId 2 with w:lvlOverride w:ilvl=\"0\" "
            "w:startOverride=1 — so the second 5-item list restarts its decimal counter at 1; "
            "body paragraphs reference the lists via w:pPr/w:numPr (w:ilvl + w:numId), items "
            "3 of each list at ilvl 1 to exercise lowerLetter. The template's own numbering "
            "part was dropped: its abstractNums were only referenced by unapplied built-in "
            "list styles. Reference text is the exact item strings only — number markers are "
            "render-derived, not inserted text."
        )
    else:
        levels = [
            _num_lvl(0, "lowerLetter", "%1.", 720),
            _num_lvl(1, "lowerRoman", "%2.", 1440),
            _num_lvl(2, "decimal", "%3.", 2160),
        ]
        override_ilvls = [0, 1, 2]
        tree_levels = [0, 1, 2, 1, 0, 1, 2, 0]
        items = []
        for i, ilvl in enumerate(tree_levels, start=1):
            text = f"Tree item {i}"
            _numbered_paragraph(doc, text, 1, ilvl)
            items.append(text)
        for i, ilvl in enumerate([0, 1, 0], start=1):
            text = f"Restart item {i}"
            _numbered_paragraph(doc, text, 2, ilvl)
            items.append(text)
        ref = f"numbering-restarts docx 002\n" + "\n".join(items)
        notes = (
            "real numbering part, raw XML: word/numbering.xml replaced with one abstractNum "
            "declaring three levels — lvl 0 lowerLetter \"%1.\", lvl 1 lowerRoman \"%2.\", "
            "lvl 2 decimal \"%3.\" (a/i/1) — and two w:num instances: numId 1 plain over an "
            "8-item tree list, numId 2 with w:lvlOverride w:startOverride=1 on all three "
            "levels so the second list restarts at a./i./1; body paragraphs carry "
            "w:pPr/w:numPr with w:ilvl 0..2. Template numbering part dropped (its abstractNums "
            "served only unapplied built-in list styles). Reference text is the exact item "
            "strings; list markers are render-derived."
        )
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)

    entries = zip_read(path)
    entries["word/numbering.xml"] = _numbering_xml(levels, override_ilvls).encode("utf-8")
    zip_write(path, entries)

    out = zip_read(path)
    num_xml = out["word/numbering.xml"].decode("utf-8")
    doc_xml = out["word/document.xml"].decode("utf-8")
    assert num_xml.count("<w:lvlOverride ") == len(override_ilvls)
    assert '<w:startOverride w:val="1"/>' in num_xml
    assert doc_xml.count("<w:numPr>") == len(items)
    assert '<w:numId w:val="2"/>' in doc_xml
    return BuildResult(
        path=path, title=f"Numbering restarts docx {nnn:03d}", reference_text=ref, notes=notes
    )


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


@builder("unicode-edge", "docx")
def unicode_edge(ctx, nnn):
    from docx import Document

    doc = Document()
    if nnn == 1:
        lines = [
            f"unicode-edge docx {nnn:03d}",
            _UE_LIGATURE,
            f"Family emoji (ZWJ sequence): {_UE_ZWJ_FAMILY} end",
            _UE_COMBINING,
            _UE_NBSP,
            _UE_SOFT_HYPHEN,
        ]
        notes = (
            "real codepoint edge content: precomposed ligature GLYPHS in the text stream "
            "(e\ufb03ciently o\ufb04ine = U+FB03/U+FB04, plus U+FB01/U+FB02 in '\ufb01le'/'\ufb02ag') — "
            "actual characters, NOT a w:ligatures rPr typographic setting; ZWJ family emoji "
            "U+1F468 U+200D U+1F469 U+200D U+1F467 U+200D U+1F466 (4-codepoint sequence); "
            "decomposed combining stacks (e+U+0301, a+U+0302+U+0301); U+00A0 non-breaking "
            "spaces between every word of one sentence; soft hyphens U+00AD inside one long "
            "word. Reference text is the byte-exact inserted codepoints. LibreOffice font "
            "coverage varies per glyph class: missing glyphs rendering as tofu .notdef boxes "
            "is a REAL fidelity finding, not a generator bug."
        )
    else:
        lines = [
            f"unicode-edge docx {nnn:03d}",
            _UE_MATH_SANS,
            _UE_MATH_DIGITS,
            _UE_VS_PAIR,
            _UE_BIDI_SANDWICH,
            _UE_ZWNBSP,
        ]
        notes = (
            "real codepoint edge content: mathematical double-struck (sans) letters "
            "U+1D54F/U+1D550 outside the BMP; mathematical bold digit numerals "
            "U+1D7CF/U+1D7D0/U+1D7D1 (non-ASCII digit forms that break naive isdigit/"
            "number extraction); U+2764 heart paired with VS15 U+FE0E (text) vs VS16 U+FE0F "
            "(emoji) — same base codepoint, different presentation; a bidi sandwich of LRM "
            "U+200E + RLM U+200F-wrapped Hebrew inside an LTR paragraph; and U+FEFF "
            "zero-width no-break space as the FIRST character of a paragraph. Reference text "
            "is the byte-exact inserted codepoints. LibreOffice font coverage varies per glyph "
            "class: missing glyphs rendering as tofu .notdef boxes is a REAL fidelity "
            "finding, not a generator bug."
        )
    ref = "\n".join(lines)
    for text in lines:
        doc.add_paragraph(text)
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)

    from ..core import zip_read

    xml = zip_read(path)["word/document.xml"].decode("utf-8")
    for text in lines:
        assert f">{text}</w:t>" in xml, f"unicode-edge line missing: {text!r}"
    if nnn == 1:
        assert "\ufb01" in xml and "\ufb02" in xml and "\ufb03" in xml and "\ufb04" in xml
        assert xml.count("\u200D") == 3
        assert "\u0301" in xml and "\u0302" in xml
        assert "\u00a0" in xml and "\u00ad" in xml
    else:
        assert "\U0001d54f" in xml and "\U0001d7cf" in xml
        assert xml.count("\ufe0e") == 1 and xml.count("\ufe0f") == 1
        assert xml.count("\u200e") == 2 and xml.count("\u200f") == 2
        assert "<w:t>\ufeff" in xml
    return BuildResult(
        path=path, title=f"Unicode edge cases docx {nnn:03d}", reference_text=ref, notes=notes
    )


@builder("huge-cell-text", "docx")
def huge_cell_text(ctx, nnn):
    from docx import Document

    unit = "lorem ipsum "
    big = unit * 5000  # 60,000 chars in one paragraph
    cell_big = unit * 834  # 10,008 chars in one table cell

    doc = Document()
    head = f"huge-cell-text docx {nnn:03d}"
    doc.add_paragraph(head)
    doc.add_paragraph(big)

    t = doc.add_table(rows=3, cols=2)
    t.style = "Table Grid"
    rows_text = []
    for r in range(3):
        cells = []
        for c in range(2):
            txt = cell_big if (r, c) == (1, 1) else f"r{r}c{c}"
            t.cell(r, c).text = txt
            cells.append(txt)
        rows_text.append(" ".join(cells))

    ref = "\n".join([head, big] + rows_text)
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)

    from ..core import zip_read

    xml = zip_read(path)["word/document.xml"].decode("utf-8")
    assert f">{big}</w:t>" in xml, "60k paragraph not written as one w:t"
    assert f">{cell_big}</w:t>" in xml, "10k cell text not written as one w:t"
    assert xml.count(f">{big}</w:t>") == 1 and xml.count(f">{cell_big}</w:t>") == 1
    normalize_zip(path)  # measure the FINAL container (pre-normalize zip bytes are
    # environment-dependent: library deflate streams vary with the zlib build)
    size = path.stat().st_size
    assert size < 10 * 1024 * 1024, f"blob {size} bytes >= 10 MB"

    notes = (
        "real huge single-string text (not an image or overflow hack): one paragraph holding "
        "the deterministic 60,000-char string 'lorem ipsum ' * 5000 in a single w:t run, plus "
        "a 3x2 Table Grid table whose cell(1,1) holds 'lorem ipsum ' * 834 (10,008 chars); "
        "other cells are short r{r}c{c} markers. Reference text is the exact same strings, "
        "byte-for-byte as inserted. The repetitive lorem would compress hard, but the "
        "normalized container is STORED for cross-environment byte determinism (INV-4): "
        f"saved blob is {size} bytes, far under the 10 MB cap. Overflow/wrap/pagination "
        "behavior at render time is the fixture's point."
    )
    return BuildResult(
        path=path, title=f"Huge cell text docx {nnn:03d}", reference_text=ref, notes=notes
    )
