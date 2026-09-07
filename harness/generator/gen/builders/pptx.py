"""pptx category builders (python-pptx + raw XML injection where needed)."""

import re

from ..core import BuildResult, builder, pin_pptx_props, zip_read, zip_write

_CMT_NS = "http://schemas.microsoft.com/office/powerpoint/2012/main"
_CMT_CT = "application/vnd.openxmlformats-officedocument.presentationml.comments+xml"
_CMT_AUTHORS_CT = (
    "application/vnd.openxmlformats-officedocument.presentationml.commentsAuthors+xml"
)
_CMT_RELTYPE = "http://schemas.microsoft.com/office/2018/10/relationships/comments"
_CMT_AUTHORS_RELTYPE = (
    "http://schemas.microsoft.com/office/2018/10/relationships/commentsAuthors"
)


def _blank_slide(prs):
    return prs.slides.add_slide(prs.slide_layouts[6])


def _rel_next_id(rels: str) -> int:
    used = [int(m) for m in re.findall(r'Id="rId(\d+)"', rels)]
    return max(used) + 1 if used else 1


@builder("merged-cells", "pptx")
def merged_cells(ctx, nnn):
    from pptx import Presentation
    from pptx.util import Inches

    prs = Presentation()
    slide = _blank_slide(prs)
    if nnn == 1:
        table = slide.shapes.add_table(3, 3, Inches(1), Inches(1), Inches(4), Inches(2)).table
        for r in range(3):
            for c in range(3):
                table.cell(r, c).text = f"R{r}C{c}"
        table.cell(0, 0).merge(table.cell(0, 1))  # horizontal: row 0, cols 0..1
        table.cell(1, 2).merge(table.cell(2, 2))  # vertical: col 2, rows 1..2
        ref = "\n".join(f"R{r}C{c}" for r in range(3) for c in range(3))
        notes = (
            "real python-pptx cell.merge on a 3x3 table: horizontal merge (0,0)+(0,1) emits "
            "gridSpan=2 with an hMerge=1 continuation cell, vertical merge (1,2)+(2,2) emits "
            "rowSpan=2 with a vMerge=1 continuation cell (both asserted in slide1.xml). Unlike "
            "xlsx, merging does NOT drop the covered cells' text — all nine a:t strings stay in "
            "the XML and the renderer only displays the origin cells (seven visible: R0C0, R0C2, "
            "R1C0, R1C1, R1C2, R2C0, R2C1; R0C1 and R2C2 are merge-hidden). reference_text lists "
            "all nine inserted strings; text extractors must decide which convention to follow "
            "and say so."
        )
        pin_pptx_props(prs)
        path = ctx.tmp / "fixture.pptx"
        prs.save(path)

        xml = zip_read(path)["ppt/slides/slide1.xml"].decode("utf-8")
        assert 'gridSpan="2"' in xml and 'hMerge="1"' in xml, "horizontal merge lost"
        assert 'rowSpan="2"' in xml and 'vMerge="1"' in xml, "vertical merge lost"
        for r in range(3):
            for c in range(3):
                assert f"R{r}C{c}" in xml, f"cell text lost: R{r}C{c}"
    else:
        # second instance, different shape of nastiness: a 4x3 table whose merges are a 2x2
        # BLOCK (both gridSpan and rowSpan on one origin) plus a full-width bottom-row merge
        # (gridSpan=3, two hMerge continuations) — no plain single-axis merge anywhere.
        table = slide.shapes.add_table(4, 3, Inches(1), Inches(1), Inches(6), Inches(2.6)).table
        for r in range(4):
            for c in range(3):
                table.cell(r, c).text = f"T{r}{c}"
        table.cell(0, 0).merge(table.cell(1, 1))  # block: rows 0..1, cols 0..1
        table.cell(3, 0).merge(table.cell(3, 2))  # horizontal: bottom row, all three cols
        ref = "\n".join(f"T{r}{c}" for r in range(4) for c in range(3))
        notes = (
            "real python-pptx cell.merge on a 4x3 table, deliberately unlike 001's two "
            "single-axis merges: a 2x2 BLOCK merge (0,0)+(1,1) puts gridSpan=2 AND rowSpan=2 on "
            "one origin (covered cells carry the hMerge+vMerge combination), and a bottom-row "
            "merge (3,0)+(3,2) emits gridSpan=3 with two hMerge=1 continuation cells — "
            "assertions count gridSpan=2 x2 / gridSpan=3 x1 / rowSpan=2 x2 / hMerge x4 / "
            "vMerge x2 in slide1.xml. As in 001, covered cells keep their a:t text (all twelve "
            "T{r}{c} strings stay in the XML, seven cells visible), so extraction convention "
            "is again the probe. reference_text lists all twelve inserted strings."
        )
        pin_pptx_props(prs)
        path = ctx.tmp / "fixture.pptx"
        prs.save(path)

        xml = zip_read(path)["ppt/slides/slide1.xml"].decode("utf-8")
        assert xml.count('gridSpan="2"') == 2 and xml.count('gridSpan="3"') == 1, "grid lost"
        assert xml.count('rowSpan="2"') == 2, "rowSpan lost"
        assert xml.count('hMerge="1"') == 4 and xml.count('vMerge="1"') == 2, "merge flags lost"
        assert '<a:tc rowSpan="2" gridSpan="2">' in xml, "block-merge origin lost"
        for r in range(4):
            for c in range(3):
                assert f"T{r}{c}" in xml, f"cell text lost: T{r}{c}"
    return BuildResult(
        path=path, title=f"Merged cells table pptx {nnn:03d}", reference_text=ref, notes=notes
    )


@builder("image-textwrap", "pptx")
def image_textwrap(ctx, nnn):
    from PIL import Image
    from pptx import Presentation
    from pptx.util import Inches, Pt

    prs = Presentation()
    slide = _blank_slide(prs)
    if nnn == 1:
        png = ctx.tmp / "block.png"
        Image.new("RGB", (64, 48), (16, 26, 46)).save(png, "PNG")
        slide.shapes.add_picture(str(png), Inches(1), Inches(1), Inches(4), Inches(2.2))
        lines = [
            f"image-textwrap pptx {nnn:03d}",
            "This text frame overlaps the opaque picture block from the lower-right corner:",
            "z-order decides visibility, not wrap — the frame added last paints on top.",
        ]
        tb = slide.shapes.add_textbox(Inches(2.2), Inches(1.8), Inches(4.2), Inches(2.4))
    else:
        # second instance: mirrored overlap — picture on the RIGHT, ink block swapped for a
        # slate one of a different size, text frame enters from the UPPER-LEFT, four lines
        # instead of three, and the picture gets an explicit alt-text name.
        png = ctx.tmp / "block-b.png"
        Image.new("RGB", (96, 72), (147, 160, 184)).save(png, "PNG")
        pic = slide.shapes.add_picture(str(png), Inches(4.6), Inches(2.4), Inches(3.6), Inches(2.6))
        pic._element.nvPicPr.cNvPr.set("descr", "slate overlap block")
        lines = [
            f"image-textwrap pptx {nnn:03d}",
            "Mirror layout: the opaque slate block sits right, the frame enters upper-left.",
            "The frame overlaps the picture's upper-left corner region:",
            "z-order decides visibility, not wrap — the frame added last paints on top.",
        ]
        tb = slide.shapes.add_textbox(Inches(1.0), Inches(1.2), Inches(4.6), Inches(2.2))
    tf = tb.text_frame
    tf.word_wrap = True
    tf.paragraphs[0].text = lines[0]
    for line in lines[1:]:
        p = tf.add_paragraph()
        p.text = line
        p.font.size = Pt(14)
    ref = "\n".join(lines)
    notes = (
        "honest category divergence: PPTX has NO text-wrap concept (wrap is a wordprocessing "
        "feature — nothing like w:wrap in DrawingML), so this is a z-order overlap test, not a "
        "text-wrap test. An opaque solid-ink 64x48 PNG (deterministic PIL bytes) is placed "
        "first, then a word-wrapped text frame overlapping its lower-right corner is added "
        "second — later shapes sit above earlier ones in the shape tree, so the frame paints "
        "on top of the picture (asserted: p:pic precedes the p:sp in slide1.xml). The picture "
        "carries no text; reference_text is the three frame lines."
    )
    if nnn == 2:
        notes = (
            "honest category divergence (same call as 001): PPTX has NO text-wrap concept, so "
            "this is a z-order overlap test. 002 mirrors 001's geometry instead of repeating "
            "it: an opaque SLATE 96x72 PNG (deterministic PIL bytes, different size and fill "
            "from 001's ink 64x48) is placed at the RIGHT (4.6in, 2.4in, 3.6in x 2.6in) with a "
            "pic:descr alt-text string, and a four-line word-wrapped frame added SECOND enters "
            "from the upper-left (1.0in, 1.2in), overlapping the picture's upper-left corner — "
            "the opposite corner from 001. Later shape still paints on top (asserted: p:pic "
            "precedes the p:sp). The picture carries no text; reference_text is the four frame "
            "lines (three in 001)."
        )
    pin_pptx_props(prs)
    path = ctx.tmp / "fixture.pptx"
    prs.save(path)

    xml = zip_read(path)["ppt/slides/slide1.xml"].decode("utf-8")
    assert "<p:pic>" in xml and "<p:sp>" in xml
    assert xml.index("<p:pic>") < xml.index("<p:sp>"), "z-order: picture not below textbox"
    if nnn == 2:
        assert 'descr="slate overlap block"' in xml, "alt-text lost"
    return BuildResult(
        path=path, title=f"Image overlap pptx {nnn:03d}", reference_text=ref, notes=notes
    )


@builder("custom-styles", "pptx")
def custom_styles(ctx, nnn):
    from pptx import Presentation
    from pptx.dml.color import RGBColor
    from pptx.util import Inches, Pt

    prs = Presentation()
    slide = _blank_slide(prs)
    tb = slide.shapes.add_textbox(Inches(1), Inches(1), Inches(8), Inches(4))
    tf = tb.text_frame
    tf.word_wrap = True
    if nnn == 1:
        spec = [
            (f"custom-styles pptx {nnn:03d}", 32, True, False, RGBColor(0x10, 0x1A, 0x2E)),
            ("Bold ink headline run at 28pt.", 28, True, False, RGBColor(0x10, 0x1A, 0x2E)),
            ("Italic slate body run at 18pt.", 18, False, True, RGBColor(0x93, 0xA0, 0xB8)),
            ("Regular vermilion caption at 12pt.", 12, False, False, RGBColor(0xE2, 0x48, 0x3A)),
        ]
        notes = (
            "PPTX has no named-paragraph-style API in python-pptx (style defs live in the theme/"
            "slideLayout chain, not as named per-paragraph styles like docx w:styles), so every "
            "variation here is DIRECT formatting: four paragraphs whose runs carry literal a:rPr "
            "size (32/28/18/12pt), b/i flags and srgbClr fills (ink 101A2E, slate 93A0B8, "
            "vermilion E2483A) — asserted in slide1.xml. Nothing references a custom layout or "
            "master placeholder; a style-table reader finds only the default python-pptx theme "
            "parts and must fall back to run-level properties."
        )
    else:
        # second instance: five paragraphs instead of four, a different size ladder
        # (40/24/24/14/10 — note the REPEATED 24pt step, which a size-histogram reader must
        # not collapse), the b/i flags in a different order, and vermilion promoted from the
        # caption (001) to the headline (002) while ink/slate stay on the other runs.
        spec = [
            (f"custom-styles pptx {nnn:03d}", 40, False, False, RGBColor(0x10, 0x1A, 0x2E)),
            ("Vermilion italic headline run at 24pt.", 24, False, True, RGBColor(0xE2, 0x48, 0x3A)),
            ("Bold ink sub-headline run, same 24pt size.", 24, True, False, RGBColor(0x10, 0x1A, 0x2E)),
            ("Italic slate body run at 14pt.", 14, False, True, RGBColor(0x93, 0xA0, 0xB8)),
            ("Regular ink footer run at 10pt.", 10, False, False, RGBColor(0x10, 0x1A, 0x2E)),
        ]
        notes = (
            "PPTX has no named-paragraph-style API in python-pptx (same divergence as 001: "
            "everything is DIRECT run formatting). 002 differs from 001 in shape, not just "
            "words: FIVE paragraphs (001 has four), a 40/24/24/14/10pt ladder whose REPEATED "
            "24pt step defeats size-histogram collapsing, b/i flags in a different order "
            "(title plain, headline italic, sub-head bold), and vermilion E2483A promoted from "
            "001's 12pt caption to the 24pt headline while ink 101A2E/slate 93A0B8 stay on the "
            "rest — asserted in slide1.xml (sz=4000/2400/1400/1000, E2483A on the headline "
            "run). A style-table reader still finds only the default theme parts."
        )
    for i, (text, size, bold, italic, rgb) in enumerate(spec):
        p = tf.paragraphs[0] if i == 0 else tf.add_paragraph()
        run = p.add_run()
        run.text = text
        run.font.size = Pt(size)
        run.font.bold = bold
        run.font.italic = italic
        run.font.color.rgb = rgb
    ref = "\n".join(text for text, *_ in spec)
    pin_pptx_props(prs)
    path = ctx.tmp / "fixture.pptx"
    prs.save(path)

    xml = zip_read(path)["ppt/slides/slide1.xml"].decode("utf-8")
    if nnn == 1:
        assert 'sz="3200"' in xml and 'sz="1200"' in xml
        assert 'b="1"' in xml and 'i="1"' in xml
        assert 'val="E2483A"' in xml and 'val="101A2E"' in xml
    else:
        assert 'sz="4000"' in xml and 'sz="2400"' in xml and 'sz="1400"' in xml and 'sz="1000"' in xml
        assert xml.count('sz="2400"') == 2, "repeated-24pt ladder lost"
        assert 'b="1"' in xml and 'i="1"' in xml
        assert 'val="E2483A"' in xml and 'val="93A0B8"' in xml
    return BuildResult(
        path=path, title=f"Custom styles pptx {nnn:03d}", reference_text=ref, notes=notes
    )


@builder("numbering-restarts", "pptx")
def numbering_restarts(ctx, nnn):
    from pptx import Presentation
    from pptx.oxml import parse_xml
    from pptx.oxml.ns import nsdecls
    from pptx.util import Inches

    prs = Presentation()
    slide = _blank_slide(prs)
    if nnn == 1:
        frame_a = [
            f"numbering-restarts pptx {nnn:03d}",
            "bulleted frame line two",
            "bulleted frame line three",
        ]
        frame_b = [
            "first numbered item",
            "second numbered item",
            "third numbered item",
        ]

        tb_a = slide.shapes.add_textbox(Inches(0.5), Inches(0.5), Inches(5.5), Inches(3))
        tf_a = tb_a.text_frame
        tf_a.word_wrap = True
        tf_a.paragraphs[0].text = frame_a[0]
        for line in frame_a[1:]:
            p = tf_a.add_paragraph()
            p.text = line
        for p in tf_a.paragraphs:
            pPr = p._p.get_or_add_pPr()
            pPr.append(parse_xml(f'<a:buFont {nsdecls("a")} typeface="Arial"/>'))
            pPr.append(parse_xml(f'<a:buChar {nsdecls("a")} char="&#8226;"/>'))

        tb_b = slide.shapes.add_textbox(Inches(6.2), Inches(0.5), Inches(5.5), Inches(3))
        tf_b = tb_b.text_frame
        tf_b.word_wrap = True
        tf_b.paragraphs[0].text = frame_b[0]
        for line in frame_b[1:]:
            p = tf_b.add_paragraph()
            p.text = line
        for p in tf_b.paragraphs:
            pPr = p._p.get_or_add_pPr()
            pPr.append(parse_xml(f'<a:buAutoNum {nsdecls("a")} type="arabicPeriod"/>'))

        ref = "\n".join(frame_a + frame_b)
        notes = (
            "two text frames on one slide with bullets injected as raw XML (python-pptx has no "
            "bullet API): frame A carries a:buFont Arial + a:buChar '&#8226;' on all three "
            "paragraphs, frame B carries a:buAutoNum type=arabicPeriod on its three paragraphs. "
            "The numbering restart between frames is IMPLICIT: DrawingML autonumbering state is "
            "per text-body, so frame B starts at '1.' with no restart flag to set — unlike docx "
            "numbering-restarts there is no numId/level to continue or reset, and no cross-frame "
            "continue mode exists. Bullet glyphs and numbers are formatting, not text: "
            "reference_text lists the six paragraph strings only."
        )
    else:
        # second instance: the roles SWAP (001: A=bullets, B=auto-numbers; 002: A=auto-numbers
        # with a different autonum format, B=bullets with a different glyph), and the item
        # counts differ (4+3 vs 3+3) so the bullet/number histograms change too.
        frame_a = [
            f"numbering-restarts pptx {nnn:03d}",
            "numbered frame item two",
            "numbered frame item three",
            "numbered frame item four",
        ]
        frame_b = [
            "dash-bulleted item one",
            "dash-bulleted item two",
            "dash-bulleted item three",
        ]

        tb_a = slide.shapes.add_textbox(Inches(0.5), Inches(0.5), Inches(5.5), Inches(3.2))
        tf_a = tb_a.text_frame
        tf_a.word_wrap = True
        tf_a.paragraphs[0].text = frame_a[0]
        for line in frame_a[1:]:
            p = tf_a.add_paragraph()
            p.text = line
        for p in tf_a.paragraphs:
            pPr = p._p.get_or_add_pPr()
            pPr.append(parse_xml(f'<a:buAutoNum {nsdecls("a")} type="arabicParenR"/>'))

        tb_b = slide.shapes.add_textbox(Inches(6.2), Inches(0.5), Inches(5.5), Inches(3))
        tf_b = tb_b.text_frame
        tf_b.word_wrap = True
        tf_b.paragraphs[0].text = frame_b[0]
        for line in frame_b[1:]:
            p = tf_b.add_paragraph()
            p.text = line
        for p in tf_b.paragraphs:
            pPr = p._p.get_or_add_pPr()
            pPr.append(parse_xml(f'<a:buFont {nsdecls("a")} typeface="Arial"/>'))
            pPr.append(parse_xml(f'<a:buChar {nsdecls("a")} char="&#8211;"/>'))

        ref = "\n".join(frame_a + frame_b)
        notes = (
            "two text frames, roles SWAPPED versus 001 (001: A=buChar bullets, "
            "B=buAutoNum): here frame A (FOUR paragraphs, 001 has three) carries a:buAutoNum "
            "type=arabicParenR — a different autonum format, '1)' not '1.' — and frame B "
            "(three) carries a:buFont Arial + a:buChar '&#8211;' (en-dash glyph, not the "
            "bullet U+2022 of 001), raw-XML injected as in 001. The per-text-body restart "
            "semantics are the point and are unchanged: frame A starts at '1)' with no flag to "
            "set, no cross-frame continue mode exists in DrawingML. reference_text lists the "
            "seven paragraph strings (six in 001); markers stay render-side."
        )
    pin_pptx_props(prs)
    path = ctx.tmp / "fixture.pptx"
    prs.save(path)

    xml = zip_read(path)["ppt/slides/slide1.xml"].decode("utf-8")
    if nnn == 1:
        assert xml.count("<a:buChar") == 3, "buChar bullets lost"
        assert xml.count('<a:buAutoNum') == 3, "buAutoNum lost"
        assert 'type="arabicPeriod"' in xml
    else:
        assert xml.count("<a:buChar") == 3, "buChar bullets lost"
        assert xml.count('<a:buAutoNum') == 4, "buAutoNum lost"
        assert 'type="arabicParenR"' in xml and 'type="arabicPeriod"' not in xml
        assert xml.count("&#8211;") == 3 or "–" in xml, "en-dash glyph lost"
    return BuildResult(
        path=path, title=f"Numbering restarts pptx {nnn:03d}", reference_text=ref, notes=notes
    )


@builder("unicode-edge", "pptx")
def unicode_edge(ctx, nnn):
    from pptx import Presentation
    from pptx.util import Inches

    from .docx import (
        _UE_BIDI_SANDWICH,
        _UE_COMBINING,
        _UE_LIGATURE,
        _UE_MATH_DIGITS,
        _UE_MATH_SANS,
        _UE_NBSP,
        _UE_SOFT_HYPHEN,
        _UE_VS_PAIR,
        _UE_ZWNBSP,
        _UE_ZWJ_FAMILY,
    )

    if nnn == 1:
        lines = [
            f"unicode-edge pptx {nnn:03d}",
            _UE_LIGATURE,
            f"Family emoji (ZWJ sequence): {_UE_ZWJ_FAMILY} end",
            _UE_COMBINING,
            _UE_NBSP,
            _UE_SOFT_HYPHEN,
        ]
        notes = (
            "the same six edge strings as docx unicode-edge docx-001 (constants imported from the "
            "docx module, so the two fixtures are codepoint-identical by construction) spread as "
            "six paragraphs in one text frame: precomposed ligature glyphs U+FB01..U+FB04, the "
            "ZWJ family emoji sequence (U+1F468/469/467/466 joined by U+200D), decomposed "
            "combining stacks (e+U+0301, a+U+0302+U+0301), NBSP-separated sentence, soft hyphens "
            "U+00AD inside one long word — stored byte-exact in the a:t runs, no normalization by "
            "the generator. LibreOffice font coverage varies per glyph class: tofu .notdef boxes "
            "are a REAL fidelity finding, not a generator bug."
        )
    else:
        # second instance: the SECOND edge family (the codepoints docx unicode-edge 002
        # exercises) — BMP ligatures are swapped for astral-plane math alphanumerics, a
        # VS15/VS16 presentation-selector pair, an LRM/RLM bidi sandwich and a leading U+FEFF;
        # six paragraphs again but a disjoint codepoint set from 001.
        lines = [
            f"unicode-edge pptx {nnn:03d}",
            _UE_MATH_SANS,
            _UE_MATH_DIGITS,
            _UE_VS_PAIR,
            _UE_BIDI_SANDWICH,
            _UE_ZWNBSP,
        ]
        notes = (
            "second instance exercises a DIFFERENT edge family than 001 (001 = BMP ligatures/"
            "ZWJ/combining/NBSP/soft-hyphen, shared with docx-001): here the docx-002 family "
            "constants — mathematical double-struck letters U+1D54F/U+1D550 (astral plane), "
            "mathematical bold digits U+1D7CF..D7D1 (non-ASCII numerals that break naive "
            "isdigit), U+2764 with VS15 U+FE0E vs VS16 U+FE0F side by side, an LRM U+200E/RLM "
            "U+200F bidi sandwich wrapping Hebrew, and a paragraph whose FIRST character is "
            "U+FEFF — spread as six paragraphs in one text frame, byte-exact in a:t runs. "
            "Tofu/substitution under LibreOffice remains a REAL fidelity finding."
        )
    prs = Presentation()
    slide = _blank_slide(prs)
    tb = slide.shapes.add_textbox(Inches(0.5), Inches(0.5), Inches(9), Inches(5))
    tf = tb.text_frame
    tf.word_wrap = True
    tf.paragraphs[0].text = lines[0]
    for line in lines[1:]:
        p = tf.add_paragraph()
        p.text = line
    ref = "\n".join(lines)
    pin_pptx_props(prs)
    path = ctx.tmp / "fixture.pptx"
    prs.save(path)

    xml = zip_read(path)["ppt/slides/slide1.xml"].decode("utf-8")
    if nnn == 1:
        assert "\ufb01" in xml and "\ufb02" in xml and "\ufb03" in xml and "\ufb04" in xml
        assert xml.count("\u200d") == 3
        assert "\u0301" in xml and "\u0302" in xml
        assert "\u00a0" in xml and "\u00ad" in xml
    else:
        assert "\U0001d54f" in xml and "\U0001d7cf" in xml
        assert xml.count("\ufe0e") == 1 and xml.count("\ufe0f") == 1
        assert xml.count("\u200e") == 2 and xml.count("\u200f") == 2
        assert "\ufeff" in xml
        assert "\ufb01" not in xml and "\u200d" not in xml, "001 family leaked into 002"
    return BuildResult(
        path=path, title=f"Unicode edge pptx {nnn:03d}", reference_text=ref, notes=notes
    )


@builder("comments-annotations", "pptx")
def comments_annotations(ctx, nnn):
    from pptx import Presentation
    from pptx.util import Inches

    prs = Presentation()
    slide = _blank_slide(prs)
    if nnn == 1:
        lines = [
            f"comments-annotations pptx {nnn:03d}",
            "first commented line",
            "second commented line",
        ]
        texts = [
            "Comment one on the first commented line.",
            "Comment two on the second commented line.",
            "Comment three on the whole slide body.",
        ]
    else:
        # second instance: FOUR comments (001: three) spread over TWO authors (001: one),
        # different body lines and comment wording, ref ids keyed to all four lines.
        lines = [
            f"comments-annotations pptx {nnn:03d}",
            "reviewed line alpha",
            "reviewed line beta",
            "reviewed line gamma",
        ]
        texts = [
            "Alpha note: wording checked against the spec.",
            "Beta note: number here is provisional.",
            "Gamma note: please re-render at 150 dpi.",
            "Deck-wide note from the second reviewer.",
        ]
    tb = slide.shapes.add_textbox(Inches(0.5), Inches(0.5), Inches(8), Inches(3))
    tf = tb.text_frame
    tf.word_wrap = True
    tf.paragraphs[0].text = lines[0]
    for line in lines[1:]:
        p = tf.add_paragraph()
        p.text = line
    ref = "\n".join(lines + texts)
    notes = (
        "python-pptx has NO comments API, so the comment layer is injected post-save by a zip "
        "rewrite: ppt/comments/authors1.xml (one p15:cavAuthor 'stichos-bot') and "
        "ppt/comments/comments1.xml (three p15:cmt entries, authorId=0, fixed dateTime "
        "2026-01-01T00:00:00Z, no wall-clock), plus the relationship chain from "
        "ppt/_rels/presentation.xml.rels (MS 2018/10 comments + commentsAuthors relationship "
        "types) and Content_Types overrides for both parts. This is the MINIMAL viable shape: "
        "synthetic parts modeled on PowerPoint's modern p15 cavAuthors/cmts layout — not "
        "validated against a schema, and the slide carries NO cmAuthorId/run-level anchors, so "
        "renderers may show nothing or drop the parts; tolerance varies by consumer and is "
        "part of what the fixture probes. Comment texts are part of the reference text."
    )
    if nnn == 2:
        notes = (
            "second instance deepens the author dimension instead of repeating 001: "
            "authors1.xml carries TWO p15:cavAuthor entries (id 0 'bot-a', id 1 'bot-b'; 001 "
            "has the single 'stichos-bot'), comments1.xml carries FOUR p15:cmt entries "
            "(001: three) alternating authorId 0/1/0/1, all with the fixed dateTime "
            "2026-01-01T00:00:00Z, ref ids keyed slide1-line1..4 over a four-line slide body. "
            "Same injection machinery as 001 (zip rewrite + MS 2018/10 relationship types + "
            "Content_Types overrides), same honest caveats: synthetic minimal-shape parts, no "
            "schema validation, no run-level anchors — renderers may show nothing or drop "
            "them. Comment texts are part of the reference text."
        )
    pin_pptx_props(prs)
    path = ctx.tmp / "fixture.pptx"
    prs.save(path)

    if nnn == 1:
        authors = (
            '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
            f'<p15:cavAuthors xmlns:p15="{_CMT_NS}">'
            '<p15:cavAuthor id="0" name="stichos-bot" initials="GP" lastIdx="3"/>'
            "</p15:cavAuthors>"
        )
        cmt_authors = [0, 0, 0]
    else:
        authors = (
            '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
            f'<p15:cavAuthors xmlns:p15="{_CMT_NS}">'
            '<p15:cavAuthor id="0" name="bot-a" initials="BA" lastIdx="2"/>'
            '<p15:cavAuthor id="1" name="bot-b" initials="BB" lastIdx="2"/>'
            "</p15:cavAuthors>"
        )
        cmt_authors = [0, 1, 0, 1]
    cmts = [
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        f'<p15:cmts xmlns:p15="{_CMT_NS}"'
        ' xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">'
    ]
    for i, (text, author_id) in enumerate(zip(texts, cmt_authors), start=1):
        cmts.append(
            f'<p15:cmt authorId="{author_id}" ref="slide1-para{i}" seq="{i}" '
            'dateTime="2026-01-01T00:00:00Z">'
            f"<p:text>{text}</p:text></p15:cmt>"
        )
    cmts.append("</p15:cmts>")

    entries = zip_read(path)
    rels = entries["ppt/_rels/presentation.xml.rels"].decode("utf-8")
    rid = _rel_next_id(rels)
    rels = rels.replace(
        "</Relationships>",
        f'<Relationship Id="rId{rid}" Type="{_CMT_RELTYPE}" Target="comments/comments1.xml"/>'
        f'<Relationship Id="rId{rid + 1}" Type="{_CMT_AUTHORS_RELTYPE}" '
        'Target="comments/authors1.xml"/></Relationships>',
        1,
    )
    entries["ppt/_rels/presentation.xml.rels"] = rels.encode("utf-8")
    entries["ppt/comments/comments1.xml"] = "".join(cmts).encode("utf-8")
    entries["ppt/comments/authors1.xml"] = authors.encode("utf-8")

    ct = entries["[Content_Types].xml"].decode("utf-8")
    assert "/ppt/comments/" not in ct
    ct = ct.replace(
        "</Types>",
        f'<Override PartName="/ppt/comments/comments1.xml" ContentType="{_CMT_CT}"/>'
        f'<Override PartName="/ppt/comments/authors1.xml" ContentType="{_CMT_AUTHORS_CT}"/>'
        "</Types>",
        1,
    )
    entries["[Content_Types].xml"] = ct.encode("utf-8")
    zip_write(path, entries)

    import xml.etree.ElementTree as ET

    entries = zip_read(path)
    cmt_xml = entries["ppt/comments/comments1.xml"].decode("utf-8")
    ET.fromstring(cmt_xml)
    ET.fromstring(entries["ppt/comments/authors1.xml"].decode("utf-8"))
    assert cmt_xml.count("<p15:cmt ") == len(texts), "comment records lost"
    if nnn == 2:
        assert cmt_xml.count('authorId="1"') == 2, "second author not referenced"
        auth_xml = entries["ppt/comments/authors1.xml"].decode("utf-8")
        assert auth_xml.count("<p15:cavAuthor ") == 2, "second author record lost"
    return BuildResult(
        path=path, title=f"Slide comments pptx {nnn:03d}", reference_text=ref, notes=notes
    )


from .docx import (  # noqa: E402
    _A_NS,
    _ARABIC_ALPHABET,
    _ARABIC_FOLK,
    _CJK_LINES_001,
    _CJK_LINES_002,
    _DGM_NS,
    _DIAG_DATA_CT,
    _DIAG_DATA_RELTYPE,
    _HEBREW_LINES_002,
    _OLE_CT,
    _OLE_PNG_B64,
    _OLE_RELTYPE,
    _SA_DIAGRAM_URI,
    _SA_GROUP_URI,
    _SA_SHAPE_URI,
    _ole_cfb_stub,
    _sa_wsp,
    _sc_scan_png,
)

_PPT_DIAGRAM_DATA_PART = "ppt/diagrams/data1.xml"
_R_NS_P = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"


def _pgf(gf_id, name, uri, inner, x, y, cx, cy):
    from pptx.oxml.ns import nsdecls

    return (
        f'<p:graphicFrame {nsdecls("p")}'
        f' xmlns:a="{_A_NS}" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"'
        ' xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"'
        ' xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"'
        f' xmlns:dgm="{_DGM_NS}" xmlns:r="{_R_NS_P}">'
        f'<p:nvGraphicFramePr><p:cNvPr id="{gf_id}" name="{name}"/>'
        "<p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>"
        f'<p:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></p:xfrm>'
        f'<a:graphic><a:graphicData uri="{uri}">{inner}</a:graphicData></a:graphic>'
        "</p:graphicFrame>"
    )


def _pgf_group(gf_id, name, shapes, x, y, cx, cy):
    inner = (
        '<wpg:wgp><wpg:cNvPr id="100" name="Slide shape group"/>'
        '<wpg:cNvGrpSpPr/><wpg:grpSpPr><a:xfrm>'
        f'<a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/><a:chExt cx="{cx}" cy="{cy}"/>'
        f"</a:xfrm></wpg:grpSpPr>{shapes}</wpg:wgp>"
    )
    return _pgf(gf_id, name, _SA_GROUP_URI, inner, x, y, cx, cy)


def _pptx_diag_data_xml():
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        f'<dgm:dataRoot xmlns:dgm="{_DGM_NS}" xmlns:a="{_A_NS}" '
        'modelId="00000000-0000-0000-0000-000000000002">'
        "<dgm:ptLst>"
        '<dgm:pt modelId="0" type="doc"><dgm:prstGeom prst="rect"/></dgm:pt>'
        '<dgm:pt modelId="1"><dgm:prstGeom prst="rect"/><dgm:text>'
        '<a:bodyPr/><a:p><a:r><a:t>Diagram node: synthetic pptx alpha</a:t></a:r></a:p>'
        "</dgm:text></dgm:pt>"
        '<dgm:pt modelId="2"><dgm:prstGeom prst="rect"/><dgm:text>'
        '<a:bodyPr/><a:p><a:r><a:t>Diagram node: synthetic pptx beta</a:t></a:r></a:p>'
        "</dgm:text></dgm:pt>"
        "</dgm:ptLst><dgm:cxnLst/><dgm:bg/><dgm:whole/><dgm:extLst/></dgm:dataRoot>"
    )


@builder("smartart-drawing", "pptx")
def smartart_drawing(ctx, nnn):
    from pptx import Presentation
    from pptx.oxml import parse_xml
    from pptx.util import Inches

    prs = Presentation()
    slide = _blank_slide(prs)
    head = f"smartart-drawing pptx {nnn:03d}"
    desc = "Two DrawingML shapes stand in for a diagram below."
    tb = slide.shapes.add_textbox(Inches(0.5), Inches(0.3), Inches(9), Inches(1))
    tf = tb.text_frame
    tf.word_wrap = True
    tf.paragraphs[0].text = head
    p = tf.add_paragraph()
    p.text = desc

    if nnn == 1:
        t1 = "Diagram stand-in: intake step"
        t2 = "Diagram stand-in: output check"
        slide.shapes._spTree.append(
            parse_xml(
                _pgf(
                    10, "SmartArt stand-in rect", _SA_SHAPE_URI,
                    _sa_wsp(1, "Node rect", "rect", t1, 0, 0, 2200000, 900000, "93A0B8"),
                    914400, 1828800, 2200000, 900000,
                )
            )
        )
        slide.shapes._spTree.append(
            parse_xml(
                _pgf(
                    11, "SmartArt stand-in ellipse", _SA_SHAPE_URI,
                    _sa_wsp(2, "Node ellipse", "ellipse", t2, 0, 0, 2200000, 900000, "F4F1EA"),
                    3504400, 1828800, 2200000, 900000,
                )
            )
        )
        labels = [t1, t2]
        notes = (
            "python-pptx cannot emit diagrams: raw p:graphicFrame elements with wps:wsp shapes "
            "(rect + ellipse, wps:txbx/w:txbxContent text, same wps layout the docx builder uses) "
            "appended to the slide spTree via parse_xml — the pptx-scale mirror of docx "
            "smartart-drawing 001. LibreOffice renders shape txbx text into the PDF text layer, "
            "so the labels are reference text. DrawingML shapes only — no diagram parts, not "
            "MS-generated SmartArt."
        )
    else:
        t1 = "Group node: plan"
        t2 = "Group node: review"
        t3 = "Group node: ship"
        shapes = (
            _sa_wsp(1, "Group ellipse a", "ellipse", t1, 0, 0, 1400000, 900000, "93A0B8")
            + _sa_wsp(2, "Group rect b", "rect", t2, 1600000, 0, 1400000, 900000, "F4F1EA")
            + _sa_wsp(3, "Group ellipse c", "ellipse", t3, 3200000, 0, 1400000, 900000, "E2483A")
        )
        slide.shapes._spTree.append(
            parse_xml(_pgf_group(12, "SmartArt stand-in group", shapes, 914400, 1828800, 4600000, 900000))
        )
        diag = '<dgm:relIds><dgm:dm r:id="__RID_DM__"/></dgm:relIds>'
        slide.shapes._spTree.append(
            parse_xml(
                _pgf(
                    13, "Synthetic diagram data ref", _SA_DIAGRAM_URI, diag,
                    914400, 3200000, 4600000, 900000,
                )
            )
        )
        labels = [t1, t2, t3]
        notes = (
            "python-pptx cannot emit diagrams: one p:graphicFrame carrying a wpg:wgp group of "
            "three wps:wsp txbx-text shapes, PLUS a second p:graphicFrame whose a:graphicData "
            "(uri drawingml/2006/diagram) carries dgm:relIds to a synthetic diagram part: "
            "ppt/diagrams/data1.xml (minimal dgm:dataRoot with a ptLst of two text nodes — no "
            "layout/colors/quickStyle parts), wired by a diagramData relationship in "
            "ppt/slides/_rels/slide1.xml.rels and a Content_Types override. Synthetic, not "
            "MS SmartArt — no MS tool produced these parts; LibreOffice may ignore the diagram "
            "reference entirely, so the dgm node strings stay OUT of reference text while the "
            "three group shape labels (real DrawingML) are reference text."
        )

    ref = "\n".join([head, desc] + labels)
    pin_pptx_props(prs)
    path = ctx.tmp / "fixture.pptx"
    prs.save(path)

    entries = zip_read(path)
    xml = entries["ppt/slides/slide1.xml"].decode("utf-8")
    if nnn == 2:
        rels = entries["ppt/slides/_rels/slide1.xml.rels"].decode("utf-8")
        rid = _rel_next_id(rels)
        assert "__RID_DM__" in xml
        xml = xml.replace("__RID_DM__", f"rId{rid}")
        entries["ppt/slides/slide1.xml"] = xml.encode("utf-8")
        rels = rels.replace(
            "</Relationships>",
            f'<Relationship Id="rId{rid}" Type="{_DIAG_DATA_RELTYPE}" '
            'Target="../diagrams/data1.xml"/></Relationships>',
            1,
        )
        entries["ppt/slides/_rels/slide1.xml.rels"] = rels.encode("utf-8")
        entries[_PPT_DIAGRAM_DATA_PART] = _pptx_diag_data_xml().encode("utf-8")
        ct = entries["[Content_Types].xml"].decode("utf-8")
        assert "/ppt/diagrams/" not in ct
        ct = ct.replace(
            "</Types>",
            f'<Override PartName="/ppt/diagrams/data1.xml" ContentType="{_DIAG_DATA_CT}"/></Types>',
            1,
        )
        entries["[Content_Types].xml"] = ct.encode("utf-8")
    zip_write(path, entries)

    import xml.etree.ElementTree as ET

    entries = zip_read(path)
    xml = entries["ppt/slides/slide1.xml"].decode("utf-8")
    ET.fromstring(xml)
    assert xml.count("<p:graphicFrame") == (2 if nnn == 1 else 2), "graphicFrame count lost"
    if nnn == 1:
        assert xml.count("<wps:wsp>") == 2 and t1 in xml and t2 in xml
        assert "<wpg:wgp>" not in xml
    else:
        assert "<wpg:wgp>" in xml and xml.count("<wps:wsp>") == 3
        assert "<dgm:relIds>" in xml and "__RID_DM__" not in xml
        ET.fromstring(entries[_PPT_DIAGRAM_DATA_PART].decode("utf-8"))
        rels = entries["ppt/slides/_rels/slide1.xml.rels"].decode("utf-8")
        assert _DIAG_DATA_RELTYPE in rels
        assert "/ppt/diagrams/data1.xml" in entries["[Content_Types].xml"].decode("utf-8")
    return BuildResult(
        path=path, title=f"SmartArt-like drawing pptx {nnn:03d}", reference_text=ref, notes=notes
    )


@builder("embedded-ole", "pptx")
def embedded_ole(ctx, nnn):
    import base64

    from pptx import Presentation
    from pptx.oxml import parse_xml
    from pptx.util import Inches

    prs = Presentation()
    slide = _blank_slide(prs)
    if nnn == 1:
        lines = [
            f"embedded-ole pptx {nnn:03d}",
            "This slide carries an embedded OLE object (mc:AlternateContent with v:shape o:ole).",
            "Body line after the embedded object.",
        ]
        blob, progid, shape_id, obj_id = _ole_cfb_stub(), "Package", "PowerPoint Object 1", "_1770000001"
        blob_magic = bytes.fromhex("D0CF11E0A1B11AE1")
    else:
        # second instance: the docx pattern of varying the PAYLOAD — 001 ships the CFB stub,
        # 002 ships the 1x1 PNG byte stub under the same oleObject part (the intentional
        # type/content mismatch), a different ProgID, and different body wording.
        lines = [
            f"embedded-ole pptx {nnn:03d}",
            "Second deck, same plumbing, different payload: the .bin part is PNG bytes here.",
            "The v:shape still declares Type=Embed — the mismatch is the probe.",
        ]
        blob, progid = base64.b64decode(_OLE_PNG_B64), "Paint.Picture"
        shape_id, obj_id = "PowerPoint Object 2", "_1770000002"
        blob_magic = b"\x89PNG\r\n\x1a\n"
    tb = slide.shapes.add_textbox(Inches(0.5), Inches(0.3), Inches(9), Inches(1.8))
    tf = tb.text_frame
    tf.word_wrap = True
    tf.paragraphs[0].text = lines[0]
    for line in lines[1:]:
        p = tf.add_paragraph()
        p.text = line

    ac = (
        '<mc:AlternateContent '
        'xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" '
        'xmlns:v="urn:schemas-microsoft-com:vml" '
        'xmlns:o="urn:schemas-microsoft-com:office:office">'
        '<mc:Choice Requires="v">'
        f'<v:shape id="{shape_id}" type="#_x0000_t75" '
        'style="left:108pt;top:216pt;width:144pt;height:108pt" o:ole="__OLE_RID__">'
        f'<o:OLEObject Type="Embed" ProgID="{progid}" DrawAspect="Icon" '
        f'ObjectID="{obj_id}" ShapeID="{shape_id}"/>'
        "</v:shape></mc:Choice><mc:Fallback/></mc:AlternateContent>"
    )
    slide.shapes._spTree.append(parse_xml(ac))
    ref = "\n".join(lines)
    pin_pptx_props(prs)
    path = ctx.tmp / "fixture.pptx"
    prs.save(path)

    entries = zip_read(path)
    xml = entries["ppt/slides/slide1.xml"].decode("utf-8")
    rels = entries["ppt/slides/_rels/slide1.xml.rels"].decode("utf-8")
    rid = _rel_next_id(rels)
    assert "__OLE_RID__" in xml
    xml = xml.replace("__OLE_RID__", f"rId{rid}")
    xml = xml.replace(
        "<p:sld ",
        '<p:sld xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" '
        'mc:Ignorable="v o" ',
        1,
    )
    entries["ppt/slides/slide1.xml"] = xml.encode("utf-8")
    rels = rels.replace(
        "</Relationships>",
        f'<Relationship Id="rId{rid}" Type="{_OLE_RELTYPE}" Target="../embeddings/object1.bin"/>'
        "</Relationships>",
        1,
    )
    entries["ppt/slides/_rels/slide1.xml.rels"] = rels.encode("utf-8")
    entries["ppt/embeddings/object1.bin"] = blob
    ct = entries["[Content_Types].xml"].decode("utf-8")
    assert "/ppt/embeddings/" not in ct
    ct = ct.replace(
        "</Types>",
        f'<Override PartName="/ppt/embeddings/object1.bin" ContentType="{_OLE_CT}"/></Types>',
        1,
    )
    entries["[Content_Types].xml"] = ct.encode("utf-8")
    zip_write(path, entries)

    import xml.etree.ElementTree as ET

    entries = zip_read(path)
    xml = entries["ppt/slides/slide1.xml"].decode("utf-8")
    ET.fromstring(xml)
    assert 'mc:Ignorable="v o"' in xml and 'o:ole="rId' in xml and "__OLE_RID__" not in xml
    assert _OLE_RELTYPE in entries["ppt/slides/_rels/slide1.xml.rels"].decode("utf-8")
    assert f'ProgID="{progid}"' in xml
    blob_out = entries["ppt/embeddings/object1.bin"]
    assert blob_out[:8] == blob_magic, "embed payload magic lost"
    stub_note = (
        "ppt/embeddings/object1.bin is the hand-built minimal CFB stub imported from the docx "
        "builder (D0CF11E0A1B11AE1 magic, v3 header + Root Entry dir sector + FAT sector, no "
        "payload streams — olefile is read-only)"
        if nnn == 1
        else "ppt/embeddings/object1.bin carries a 1x1 PNG byte stub under the oleObject part — "
        "the same honest type/content mismatch the docx counterpart uses in its 001 (declared "
        "oleObject content type, PNG magic inside), ProgID Paint.Picture, and the v:shape "
        "declares Type=Embed regardless: a consumer that sniffs magic vs one that trusts the "
        "declared type will disagree, which is exactly what this instance probes"
    )
    notes = (
        "real package-level OLE plumbing modeled on how PowerPoint writes legacy embeds: "
        + stub_note + ", declared with the oleObject relationship "
        "type in ppt/slides/_rels/slide1.xml.rels and a Content_Types override. The slide "
        "carries an mc:AlternateContent whose mc:Choice Requires=\"v\" holds a v:shape with "
        "o:ole=rIdN + o:OLEObject child (Type=Embed, DrawAspect=Icon) and an empty "
        "mc:Fallback; the p:sld root gains xmlns:mc + mc:Ignorable=\"v o\" via zip rewrite. "
        "Synthetic layout, not schema-validated: LibreOffice may show an icon, render nothing, "
        "or drop the parts — tolerance varies by consumer and is what the fixture probes. The "
        "embed carries no text truth, so reference_text covers only the three body lines."
    )
    return BuildResult(
        path=path, title=f"Embedded OLE object pptx {nnn:03d}", reference_text=ref, notes=notes
    )


@builder("scanned-ocr", "pptx")
def scanned_ocr(ctx, nnn):
    from pptx import Presentation
    from pptx.util import Inches

    png = ctx.tmp / "scan-slide.png"
    _sc_scan_png(png, ctx.rng.randrange(1 << 32))

    prs = Presentation()
    slide = _blank_slide(prs)
    slide.shapes.add_picture(str(png), 0, 0, prs.slide_width, prs.slide_height)
    if nnn == 1:
        captions = ["Slide 1 — scanned fixture"]
    else:
        # second instance: different caption wording AND a second caption line (001 has one),
        # plus the raster is mirrored horizontally — a different pixel stream on top of the
        # per-fixture noise seed.
        from PIL import Image, ImageOps

        with Image.open(png) as im:
            ImageOps.mirror(im).save(png, "PNG")
        captions = [
            "Slide 1 — scanned fixture, second pass",
            "raster-only body: the truth here is the noise, not the bars",
        ]
    tb = slide.shapes.add_textbox(Inches(0.5), Inches(6.9), Inches(9), Inches(0.5))
    tb.text_frame.word_wrap = True
    tb.text_frame.paragraphs[0].text = captions[0]
    for extra in captions[1:]:
        tb.text_frame.add_paragraph().text = extra

    ref = "\n".join(captions)
    pin_pptx_props(prs)
    path = ctx.tmp / "fixture.pptx"
    prs.save(path)

    import xml.etree.ElementTree as ET

    xml = zip_read(path)["ppt/slides/slide1.xml"].decode("utf-8")
    ET.fromstring(xml)
    assert "<p:pic>" in xml, "full-slide picture lost"
    notes = (
        "scanned-slide emulation on the docx scanned-ocr pattern: the same synthetic raster "
        "generator (_sc_scan_png imported from the docx builder — off-white paper gray, gray "
        "pseudo-text word bars, seeded per-pixel noise from the fixture-seeded ctx.rng) is "
        "embedded via add_picture stretched to the FULL slide (slide_width x slide_height), "
        "with one caption text box near the bottom. The raster has NO text layer (bars, not "
        "glyphs), so there is no OCR truth inside the picture: reference_text is the caption "
        "ONLY — the honest near-None case."
    )
    if nnn == 2:
        notes = (
            "scanned-slide emulation, second instance: differs from 001 in three visible ways "
            "— the raster is the shared _sc_scan_png recipe MIRRORED horizontally on top of the "
            "per-fixture ctx.rng noise seed (different pixel stream, same bar texture), the "
            "caption text box carries TWO lines (001: one) with different wording, and the "
            "reference text therefore lists both caption strings. Still the honest near-None "
            "case: the picture has no text layer, the caption is the only text truth."
        )
    return BuildResult(
        path=path, title=f"Scanned slide pptx {nnn:03d}", reference_text=ref, notes=notes
    )


@builder("cjk-text", "pptx")
def cjk_text(ctx, nnn):
    from pptx import Presentation
    from pptx.oxml import parse_xml
    from pptx.oxml.ns import nsdecls
    from pptx.util import Inches

    if nnn == 1:
        title = "静夜思"
        poem = _CJK_LINES_001[:4]  # Li Bai, same codepoints as docx cjk-text 001
        haiku = _CJK_LINES_002[0]  # Bashō, same codepoints as docx cjk-text 002
        extra: list[tuple[str, str]] = []
    else:
        # second instance: a DIFFERENT poem pair and a third script — Meng Haoran's 春晓
        # (the lines docx cjk-text 001 shows, never used on a slide before) as the bulleted
        # body, Bashō's 1680 frog-haiku replaced by the 五月雨を river haiku, plus the Korean
        # 애국가 line with a KR font hint — seven CJK runs across three locales, five in 001.
        title = "春晓"
        poem = _CJK_LINES_001[4:]
        haiku = _CJK_LINES_002[1]
        extra = [(_CJK_LINES_002[3], "Noto Serif CJK KR")]

    prs = Presentation()
    slide = _blank_slide(prs)
    tb = slide.shapes.add_textbox(Inches(1), Inches(0.6), Inches(8), Inches(6))
    tf = tb.text_frame
    tf.word_wrap = True

    def add_line(text, ea_font, bullet):
        p = tf.paragraphs[0] if not tf.paragraphs[0].runs and not tf.paragraphs[0].text else tf.add_paragraph()
        if bullet:
            pPr = p._p.get_or_add_pPr()
            pPr.append(parse_xml(f'<a:buFont {nsdecls("a")} typeface="Arial"/>'))
            pPr.append(parse_xml(f'<a:buChar {nsdecls("a")} char="&#8226;"/>'))
        run = p.add_run()
        run.text = text
        run._r.get_or_add_rPr().append(
            parse_xml(f'<a:ea {nsdecls("a")} typeface="{ea_font}"/>')
        )

    add_line(title, "Noto Serif CJK SC", False)
    for line in poem:
        add_line(line, "Noto Serif CJK SC", True)
    add_line(haiku, "Noto Serif CJK JP", False)
    for text, font in extra:
        add_line(text, font, False)

    all_lines = [title] + poem + [haiku] + [t for t, _ in extra]
    ref = "\n".join(all_lines)
    pin_pptx_props(prs)
    path = ctx.tmp / "fixture.pptx"
    prs.save(path)

    xml = zip_read(path)["ppt/slides/slide1.xml"].decode("utf-8")
    for text in all_lines:
        assert f"<a:t>{text}</a:t>" in xml, f"cjk line missing: {text!r}"
    if nnn == 1:
        assert xml.count('typeface="Noto Serif CJK') == 6
        assert xml.count("<a:buChar") == 4
    else:
        assert xml.count('typeface="Noto Serif CJK') == 7
        assert xml.count('typeface="Noto Serif CJK KR"') == 1
        assert xml.count("<a:buChar") == 4
        assert "<a:t>静夜思</a:t>" not in xml, "001 title leaked into 002"
    notes = (
        "real public-domain CJK text, codepoint-identical to the docx fixtures by import: "
        "title 静夜思 plus the four lines of Li Bai's 静夜思 (床前明月光…低头思故乡, "
        "_CJK_LINES_001[:4]) as buChar-bulleted paragraphs, plus Bashō's frog haiku "
        "(古池や 蛙飛びこむ 水の音, _CJK_LINES_002[0]) with an inter-phrase-spaced exact "
        "string; every run carries an explicit East Asian font hint as raw <a:ea typeface=...> "
        "in a:rPr (SC for the Chinese lines, JP for the haiku — python-pptx font.name writes "
        "a:latin only). reference_text is the six exact inserted strings; the C9 §10.6 CJK "
        "per-char tokenization rule applies downstream; tofu from missing CJK fonts at render "
        "is a fidelity finding, not a generator bug."
    )
    if nnn == 2:
        notes = (
            "second instance, different poem pair and a third script: title 春晓 plus Meng "
            "Haoran's four lines (春眠不觉晓…花落知多少, _CJK_LINES_001[4:], the docx-001 "
            "second poem, unused on slides until now) buChar-bulleted with an SC hint, "
            "Bashō's 五月雨を あつめて早し 最上川 (_CJK_LINES_002[1]) with the JP hint, and "
            "the Korean 애국가 line 동해 물과 백두산이 마르고 닳도록 (_CJK_LINES_002[3]) with "
            "an KR hint — SEVEN runs across THREE locales (001: six runs, two locales), all "
            "with raw <a:ea typeface=...> slots; 静夜思 is asserted ABSENT so the two "
            "instances cannot silently collapse to the same poem. reference_text is the seven "
            "exact inserted strings; CJK per-char tokenization (C9 §10.6) applies; tofu is a "
            "fidelity finding."
        )
    return BuildResult(
        path=path, title=f"CJK text pptx {nnn:03d}", reference_text=ref, notes=notes
    )


@builder("rtl-text", "pptx")
def rtl_text(ctx, nnn):
    from pptx import Presentation
    from pptx.oxml import parse_xml
    from pptx.oxml.ns import nsdecls
    from pptx.util import Inches

    prs = Presentation()
    slide = _blank_slide(prs)
    tb = slide.shapes.add_textbox(Inches(0.5), Inches(0.5), Inches(9), Inches(6))
    tf = tb.text_frame
    tf.word_wrap = True

    if nnn == 1:
        arabic = _ARABIC_ALPHABET  # same string as docx rtl-text 001
        hebrew = _HEBREW_LINES_002[:3]  # same strings as docx rtl-text 002
        note_runs = [
            ("הערה: Latin LTR run ", False),
            ("stichos", True),
            (" inside RTL paragraph", False),
        ]
    else:
        # second instance: same flag machinery, different strings — the Arabic ALPHABET demo
        # is replaced by the folk-opening sentence (used in docx rtl-text 001), the Hebrew
        # bullets take the LAST three lines of the shared pool (lines 2-4, not 1-3), and the
        # mixed paragraph swaps both the Hebrew context and the Latin island text.
        arabic = _ARABIC_FOLK
        hebrew = _HEBREW_LINES_002[1:]
        note_runs = [
            ("מסמך בעריכה: ", False),
            ("HAR-02 gate", True),
            (" — הערה מעורבת", False),
        ]
    note_text = "".join(t for t, _ in note_runs)

    def rtl_para(runs, bullet, cs_font):
        p = tf.paragraphs[0] if not tf.paragraphs[0].runs and not tf.paragraphs[0].text else tf.add_paragraph()
        pPr = p._p.get_or_add_pPr()
        pPr.set("algn", "r")
        pPr.set("rtl", "1")
        if bullet:
            pPr.append(parse_xml(f'<a:buFont {nsdecls("a")} typeface="Arial"/>'))
            pPr.append(parse_xml(f'<a:buChar {nsdecls("a")} char="&#8226;"/>'))
        for text, is_ltr in runs:
            run = p.add_run()
            run.text = text
            rPr = run._r.get_or_add_rPr()
            if is_ltr:
                rPr.set("rtl", "0")
            else:
                rPr.append(parse_xml(f'<a:cs {nsdecls("a")} typeface="{cs_font}"/>'))

    rtl_para([(arabic, False)], bullet=False, cs_font="Noto Naskh Arabic")
    for line in hebrew:
        rtl_para([(line, False)], bullet=True, cs_font="Noto Sans Hebrew")
    rtl_para(note_runs, bullet=False, cs_font="Noto Sans Hebrew")

    ref = "\n".join([arabic] + hebrew + [note_text])
    pin_pptx_props(prs)
    path = ctx.tmp / "fixture.pptx"
    prs.save(path)

    xml = zip_read(path)["ppt/slides/slide1.xml"].decode("utf-8")
    assert xml.count('rtl="1"') == 5, "paragraph-level rtl flags lost"
    assert xml.count('algn="r"') == 5, "right-align flags lost"
    assert xml.count('rtl="0"') == 1, "LTR run flag lost"
    assert xml.count("<a:buChar") == 3
    assert xml.count('typeface="Noto Naskh Arabic"') == 1
    assert xml.count('typeface="Noto Sans Hebrew"') == 5
    if nnn == 2:
        assert _ARABIC_ALPHABET not in xml, "001 alphabet demo leaked into 002"
        assert "stichos" not in xml, "001 latin island leaked into 002"
    notes = (
        "real RTL slide content in DrawingML: one right-aligned Arabic paragraph (the 28-letter "
        "alphabet string shared with docx rtl-text 001) with a:pPr algn=\"r\" + rtl=\"1\", three "
        "buChar-bulleted Hebrew paragraphs (strings shared with docx rtl-text 002, same "
        "rtl/algn flags), and a mixed-direction note paragraph whose embedded Latin run "
        "«stichos» is the only run marked a:rPr rtl=\"0\" — the LTR island inside the RTL "
        "context. Hebrew runs carry a:cs typeface=\"Noto Sans Hebrew\" (python-pptx has no bidi "
        "API; flags set on raw oxml attributes). reference_text is the exact inserted strings "
        "in logical order; bidi reordering is a render-time concern."
    )
    if nnn == 2:
        notes = (
            "second instance, same flag machinery (a:pPr algn=\"r\" + rtl=\"1\" on all five "
            "paragraphs, one a:rPr rtl=\"0\" island), different STRINGS: the Arabic paragraph "
            "is the folk opening «كان الزمان زمان» (the alphabet demo of 001 is asserted "
            "absent), the three buChar Hebrew bullets take lines 2-4 of the shared pool "
            "(001 takes 1-3), and the mixed note paragraph embeds the Latin island "
            "«HAR-02 gate» between Hebrew runs (001's «stichos» is asserted absent). cs "
            "hints unchanged (Naskh for Arabic, Noto Sans Hebrew ×5). reference_text is the "
            "exact inserted strings in logical order."
        )
    return BuildResult(
        path=path, title=f"RTL text pptx {nnn:03d}", reference_text=ref, notes=notes
    )


@builder("protected-readonly", "pptx")
def protected_readonly(ctx, nnn):
    import xml.etree.ElementTree as ET

    from pptx import Presentation
    from pptx.util import Inches

    prs = Presentation()
    if nnn == 1:
        lines = [
            f"protected-readonly pptx {nnn:03d}",
            "This presentation carries a legacy modifyVerifier in ppt/presentation.xml.",
            "Editors should treat the deck as final.",
        ]
        slide = _blank_slide(prs)
        tb = slide.shapes.add_textbox(Inches(0.5), Inches(0.5), Inches(9), Inches(3))
        tf = tb.text_frame
        tf.word_wrap = True
        tf.paragraphs[0].text = lines[0]
        for line in lines[1:]:
            p = tf.add_paragraph()
            p.text = line
    else:
        # second instance: the deck is TWO slides (001: one) with different wording — a
        # slide-per-content layout exercises "does a consumer see the flag at all, or only
        # on the first slide it touches", which a single-slide deck cannot probe.
        slide_lines = [
            [f"protected-readonly pptx {nnn:03d}",
             "The modifyVerifier flag lives once, in presentation.xml, not per slide."],
            ["Second slide: consumers must read the package, not just slide one."],
        ]
        for slide, paras in zip((_blank_slide(prs), _blank_slide(prs)), slide_lines):
            tb = slide.shapes.add_textbox(Inches(0.5), Inches(0.5), Inches(9), Inches(3))
            tf = tb.text_frame
            tf.word_wrap = True
            tf.paragraphs[0].text = paras[0]
            for line in paras[1:]:
                p = tf.add_paragraph()
                p.text = line
        lines = [line for paras in slide_lines for line in paras]
    ref = "\n".join(lines)
    pin_pptx_props(prs)
    path = ctx.tmp / "fixture.pptx"
    prs.save(path)

    entries = zip_read(path)
    pres = entries["ppt/presentation.xml"].decode("utf-8")
    assert "<p:modifyVerifier" not in pres and "</p:presentation>" in pres
    verifier = '<p:modifyVerifier covered="1"/>'
    pres = pres.replace("</p:presentation>", verifier + "</p:presentation>", 1)
    entries["ppt/presentation.xml"] = pres.encode("utf-8")
    zip_write(path, entries)

    entries = zip_read(path)
    pres = entries["ppt/presentation.xml"].decode("utf-8")
    ET.fromstring(pres)
    assert '<p:modifyVerifier covered="1"/>' in pres
    assert pres.index("<p:defaultTextStyle") < pres.index("<p:modifyVerifier"), (
        "modifyVerifier must follow defaultTextStyle per CT_Presentation order"
    )
    if nnn == 2:
        assert len([n for n in entries if re.fullmatch(r"ppt/slides/slide\d+\.xml", n)]) == 2, (
            "second slide lost"
        )
    notes = (
        "the only real modify-protection knob in presentationML is CT_Presentation/modifyVerifier "
        "(PowerPoint's legacy 'modify password' element) — injected via zip rewrite at its schema "
        "position, after defaultTextStyle, as <p:modifyVerifier covered=\"1\"/> with NO password/"
        "salt/spinCount/cryptAlgorithm attributes: a password-less modify-reminder flag, not a "
        "real protection hash. There is no clean package-level read-only-recommended flag a "
        "generator can set (docProps appMetadata is not writable here; <p15:persistence> is a "
        "co-authoring marker, not protection), so this is the honest closest feature — legacy "
        "modifyVerifier; modern PowerPoint may ignore it and LibreOffice likely does. Body text "
        "is plain python-pptx content and is the whole reference_text."
    )
    if nnn == 2:
        notes = (
            "same honest closest-feature as 001 (package-level modifyVerifier covered=\"1\", "
            "no password attributes, injected after defaultTextStyle per CT_Presentation "
            "order) — but a TWO-slide deck (asserted: two slide parts) with different body "
            "wording: the flag exists ONCE per package, so the probe is whether a consumer "
            "reads protection at package level or only notices slide one. Same caveats as "
            "001: legacy knob, likely ignored by modern PowerPoint and LibreOffice. Body text "
            "of both slides is the whole reference_text."
        )
    return BuildResult(
        path=path, title=f"Protected read-only pptx {nnn:03d}", reference_text=ref, notes=notes
    )


_LD_PHRASES = (
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot",
    "golf", "hotel", "india", "juliet", "kilo", "lima",
)


@builder("long-deck-300", "pptx")
def long_deck_300(ctx, nnn):
    from pptx import Presentation
    from pptx.oxml import parse_xml
    from pptx.oxml.ns import nsdecls
    from pptx.util import Inches

    prs = Presentation()
    ref_lines = []
    for i in range(1, 301):
        slide = _blank_slide(prs)
        title = f"Slide {i}"
        bullets = [_LD_PHRASES[(3 * (i - 1) + k) % 12] for k in range(3)]
        if nnn == 2 and i % 2 == 0:
            pos = (Inches(1.2), Inches(2.2))
        else:
            pos = (Inches(0.6), Inches(0.6))
        tb = slide.shapes.add_textbox(pos[0], pos[1], Inches(8), Inches(2))
        tf = tb.text_frame
        tf.word_wrap = True
        tf.paragraphs[0].text = title
        for b in bullets:
            p = tf.add_paragraph()
            p.text = b
            pPr = p._p.get_or_add_pPr()
            pPr.append(parse_xml(f'<a:buFont {nsdecls("a")} typeface="Arial"/>'))
            pPr.append(parse_xml(f'<a:buChar {nsdecls("a")} char="&#8226;"/>'))
        ref_lines.append(title)
        ref_lines.extend(bullets)
    ref = "\n".join(ref_lines)

    notes = (
        "300-slide pagination/theme stress; deck size is the feature. Every slide reuses the "
        "same blank layout (slide_layouts[6]) with one fixed textbox position at (0.6in, 0.6in) "
        "— theme/layout resolution is constant and slide count is the only variable; each slide "
        "carries a 'Slide {i}' title line plus three buChar bullets drawn from a fixed "
        "12-phrase NATO cycle, so per-slide content stays tiny. reference_text is the exact "
        "title+bullets of every slide in order (1200 lines)."
    )
    if nnn == 2:
        notes = (
            "300-slide pagination/theme stress; deck size is the feature. Same single blank "
            "layout (slide_layouts[6]) on all 300 slides, but the textbox position ALTERNATES "
            "between two placements — odd slides at (0.6in, 0.6in), even slides at "
            "(1.2in, 2.2in) — so geometry varies while the layout chain stays constant; each "
            "slide carries a 'Slide {i}' title line plus three buChar bullets drawn from a "
            "fixed 12-phrase NATO cycle. reference_text is the exact title+bullets of every "
            "slide in order (1200 lines)."
        )
    pin_pptx_props(prs)
    path = ctx.tmp / "fixture.pptx"
    prs.save(path)

    import xml.etree.ElementTree as ET

    entries = zip_read(path)
    slide_names = [
        n for n in entries
        if re.fullmatch(r"ppt/slides/slide\d+\.xml", n)
    ]
    assert len(slide_names) == 300, f"slide count lost: {len(slide_names)}"
    xml1 = entries["ppt/slides/slide1.xml"].decode("utf-8")
    ET.fromstring(xml1)
    assert "<a:t>Slide 1</a:t>" in xml1
    assert xml1.count("<a:buChar") == 3
    assert path.stat().st_size < 12 * 1024 * 1024, "blob exceeds 12 MB"
    if nnn == 2:
        xml2 = entries["ppt/slides/slide2.xml"].decode("utf-8")
        y1 = re.search(r"<a:off x=\"(\d+)\" y=\"(\d+)\"/>", xml1).group(2)
        y2 = re.search(r"<a:off x=\"(\d+)\" y=\"(\d+)\"/>", xml2).group(2)
        assert y1 != y2, "two-position alternation lost"
    return BuildResult(
        path=path, title=f"Long deck 300 pptx {nnn:03d}", reference_text=ref, notes=notes
    )
