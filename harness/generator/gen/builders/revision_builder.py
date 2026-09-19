# SPDX-License-Identifier: MIT OR Apache-2.0
"""Build docx fixtures with tracked changes for lossless proof suite.

Provides focused helpers for creating docx fixtures that exercise the
revision accept/reject pipeline. Unlike the general tracked-changes
builder (docx.py), these produce minimal, targeted fixtures for the
lossless proof suite: single-insert, single-delete, mixed, and
multi-author revisions.
"""

from ..core import BuildResult, builder, normalize_zip, pin_docx_props, zip_read, zip_write


_TC_DATE = "2026-01-01T00:00:00Z"


def _ins_wrap(xml: str, token: str, author: str, rid: int) -> str:
    """Wrap a plain run containing *token* with w:ins markup."""
    import re
    run = f"<w:r><w:t>{token}</w:t></w:r>"
    assert run in xml, f"token run not found: {token!r}"
    ins = (
        f'<w:ins w:id="{rid}" w:author="{author}" w:date="{_TC_DATE}">'
        + run
        + "</w:ins>"
    )
    return xml.replace(run, ins, 1)


def _del_wrap(xml: str, token: str, author: str, rid: int) -> str:
    """Wrap a plain run containing *token* with w:del markup (text->delText)."""
    import re
    run = f"<w:r><w:t>{token}</w:t></w:r>"
    assert run in xml, f"token run not found: {token!r}"
    space = f" xml:space=\"preserve\"" if "xml:space" in run else ""
    del_n = (
        f'<w:del w:id="{rid}" w:author="{author}" w:date="{_TC_DATE}">'
        f"<w:r><w:delText{space}>{token}</w:delText></w:r></w:del>"
    )
    return xml.replace(run, del_n, 1)


@builder("revision-insert", "docx")
def revision_insert(ctx, nnn):
    """Document with one tracked insertion."""
    from docx import Document

    doc = Document()
    p = doc.add_paragraph()
    p.add_run("Base text with ")
    p.add_run("inserted fragment")
    p.add_run(" and ending here.")
    ref = "Base text with inserted fragment and ending here."
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)

    # Inject tracked change: wrap "inserted fragment" in w:ins
    entries = zip_read(path)
    xml = entries["word/document.xml"].decode("utf-8")
    xml = _ins_wrap(xml, "inserted fragment", "agent", 9001)
    entries["word/document.xml"] = xml.encode("utf-8")
    zip_write(path, entries)
    normalize_zip(path)

    notes = (
        "Single tracked insertion via w:ins; author=agent, fixed date. "
        "Reference text includes the inserted content."
    )
    return BuildResult(path=path, title="Revision insert docx 001",
                       reference_text=ref, notes=notes)


@builder("revision-delete", "docx")
def revision_delete(ctx, nnn):
    """Document with one tracked deletion."""
    from docx import Document

    doc = Document()
    p = doc.add_paragraph()
    p.add_run("Keep this ")
    p.add_run("deleted fragment")
    p.add_run(" and tail.")
    ref = "Keep this  and tail."
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)

    # Inject tracked change: wrap "deleted fragment" in w:del
    entries = zip_read(path)
    xml = entries["word/document.xml"].decode("utf-8")
    xml = _del_wrap(xml, "deleted fragment", "agent", 9001)
    entries["word/document.xml"] = xml.encode("utf-8")
    zip_write(path, entries)
    normalize_zip(path)

    notes = (
        "Single tracked deletion via w:del (text converted to w:delText); "
        "author=agent, fixed date. Reference text excludes deleted content."
    )
    return BuildResult(path=path, title="Revision delete docx 001",
                       reference_text=ref, notes=notes)


@builder("revision-mixed", "docx")
def revision_mixed(ctx, nnn):
    """Document with mixed insertions and deletions."""
    from docx import Document

    doc = Document()
    p = doc.add_paragraph()
    p.add_run("Base ")
    p.add_run("alpha-insert")
    p.add_run(" stable ")
    p.add_run("alpha-del")
    p.add_run(" tail ")
    p.add_run("beta-insert")
    p.add_run(" end.")
    ref = (
        "Base alpha-insert stable tail beta-insert end."
    )
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)

    entries = zip_read(path)
    xml = entries["word/document.xml"].decode("utf-8")
    rid = 9001
    xml = _ins_wrap(xml, "alpha-insert", "bot-a", rid); rid += 1
    xml = _del_wrap(xml, "alpha-del", "bot-a", rid); rid += 1
    xml = _ins_wrap(xml, "beta-insert", "bot-b", rid); rid += 1
    entries["word/document.xml"] = xml.encode("utf-8")
    zip_write(path, entries)
    normalize_zip(path)

    notes = (
        "Three tracked changes: two inserts (bot-a, bot-b) and one delete (bot-a). "
        "Reference text includes inserted content, excludes deleted content."
    )
    return BuildResult(path=path, title="Revision mixed docx 001",
                       reference_text=ref, notes=notes)


@builder("revision-multi-author", "docx")
def revision_multi_author(ctx, nnn):
    """Document with revisions by multiple authors."""
    from docx import Document

    doc = Document()
    # Paragraph 1: Alice inserts, Bob deletes
    p1 = doc.add_paragraph()
    p1.add_run("Alice added: ")
    p1.add_run("alice-insert")
    p1.add_run(" Bob removed: ")
    p1.add_run("bob-delete")
    p1.add_run(" done.")
    # Paragraph 2: Charlie inserts
    p2 = doc.add_paragraph()
    p2.add_run("Section: ")
    p2.add_run("charlie-insert")
    p2.add_run(" more.")
    ref = (
        "Alice added: alice-insert Bob removed:  done.\n"
        "Section: charlie-insert more."
    )
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)

    entries = zip_read(path)
    xml = entries["word/document.xml"].decode("utf-8")
    rid = 9001
    xml = _ins_wrap(xml, "alice-insert", "alice", rid); rid += 1
    xml = _del_wrap(xml, "bob-delete", "bob", rid); rid += 1
    xml = _ins_wrap(xml, "charlie-insert", "charlie", rid); rid += 1
    entries["word/document.xml"] = xml.encode("utf-8")
    zip_write(path, entries)
    normalize_zip(path)

    notes = (
        "Three tracked changes by three authors (alice, bob, charlie); "
        "two inserts and one delete. Tests multi-author authorship tracking."
    )
    return BuildResult(path=path, title="Revision multi-author docx 001",
                       reference_text=ref, notes=notes)


@builder("revision-table-row", "docx")
def revision_table_row(ctx, nnn):
    """Document with a tracked table-row insertion."""
    from docx import Document

    doc = Document()
    doc.add_paragraph("Table with tracked row insertion.")
    t = doc.add_table(rows=2, cols=2)
    t.style = "Table Grid"
    t.cell(0, 0).text = "header-a"
    t.cell(0, 1).text = "header-b"
    t.cell(1, 0).text = "data-a"
    t.cell(1, 1).text = "data-b"
    ref = "Table with tracked row insertion.\nheader-a header-b\ndata-a data-b"
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)

    # Add a second data row, then mark it as a tracked insertion via w:trPr/w:ins
    entries = zip_read(path)
    xml = entries["word/document.xml"].decode("utf-8")
    # Find the closing </w:tr> of the second data row and inject after it
    # Look for the closing </w:tbl> and insert before it
    table_close = "</w:tbl>"
    new_row = (
        '<w:tr><w:trPr><w:ins w:id="9001" w:author="agent" w:date="'
        + _TC_DATE + '"/></w:trPr>'
        '<w:tc><w:p><w:r><w:t>ins-row-a</w:t></w:r></w:p></w:tc>'
        '<w:tc><w:p><w:r><w:t>ins-row-b</w:t></w:r></w:p></w:tc></w:tr>'
    )
    xml = xml.replace(table_close, new_row + table_close, 1)
    entries["word/document.xml"] = xml.encode("utf-8")
    zip_write(path, entries)
    normalize_zip(path)

    notes = (
        "Tracked table-row insertion via w:trPr/w:ins on a second data row; "
        "author=agent, fixed date."
    )
    return BuildResult(path=path, title="Revision table-row docx 001",
                       reference_text=ref, notes=notes)


@builder("revision-empty-body", "docx")
def revision_empty_body(ctx, nnn):
    """Document that is just a tracked change in an otherwise empty body."""
    from docx import Document

    doc = Document()
    p = doc.add_paragraph()
    p.add_run("inserted-only")
    ref = "inserted-only"
    pin_docx_props(doc)
    path = ctx.tmp / "fixture.docx"
    doc.save(path)

    entries = zip_read(path)
    xml = entries["word/document.xml"].decode("utf-8")
    xml = _ins_wrap(xml, "inserted-only", "agent", 9001)
    entries["word/document.xml"] = xml.encode("utf-8")
    zip_write(path, entries)
    normalize_zip(path)

    notes = (
        "Minimal fixture: entire body content is a single tracked insertion. "
        "Accepting yields the text; rejecting yields an empty document."
    )
    return BuildResult(path=path, title="Revision empty-body docx 001",
                       reference_text=ref, notes=notes)
