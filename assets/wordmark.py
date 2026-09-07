#!/usr/bin/env python3
"""stichos wordmark + lockup generator.

Lays out the lowercase wordmark, measures the tittle of the `i` with real font metrics (PIL),
knocks it out of the type, and sets a vermilion proofreader's tick in its place — the checked
dot. `stichos` has no descenders, so the old "tick under the y" construction is gone; the
tittle slot is the only place the tick can sit without crossing a letter.

Usage:  python3 wordmark.py
Then convert to outlines so the SVG ships with no font dependency:
  for f in wordmark wordmark-mono wordmark-inverse wordmark-tagline lockup-h lockup-v; do
    inkscape $f.svg --export-text-to-path --export-plain-svg --export-filename=$f-outline.svg; done
"""
import subprocess, sys
from PIL import ImageFont

TEXT = "stichos"
TAGLINE = "PROOF BEFORE PUBLISH"
INK, PAPER, VERMILION, SLATE = "#101A2E", "#FBF7EE", "#E2483A", "#93A0B8"
SIZE = 220


def resolve(family, style="Regular"):
    try:
        out = subprocess.run(["fc-match", "-f", "%{file}|%{family}", f"{family}:style={style}"],
                             capture_output=True, text=True, timeout=10).stdout.strip()
        path, fam = out.split("|", 1)
        return path, fam.split(",")[0], ImageFont.truetype(path, SIZE)
    except Exception:
        return None


def pick_font():
    for fam, style in [("Inter", "Medium"), ("Inter", "Regular"), ("Noto Sans", "Medium"),
                       ("Adwaita Sans", "Medium"), ("Source Sans 3", "Regular"),
                       ("Liberation Sans", "Regular"), ("DejaVu Sans", "Book")]:
        r = resolve(fam, style)
        if r:
            return r
    sys.exit("no usable grotesque found")


def svg_inner(path):
    """element body of an .svg file, so it can be embedded in a <g>."""
    t = open(path).read()
    t = t[t.index(">", t.index("<svg")) + 1:]
    return t[:t.rindex("</svg>")]


def tittle_box(font):
    """bbox of the `i` tittle (first ink block from the top of the glyph), baseline-relative."""
    from PIL import Image, ImageDraw
    l, t, r, b = font.getbbox("i", anchor="ls")
    pad = 4
    img = Image.new("L", (r - l + 2 * pad, b - t + 2 * pad), 0)
    ImageDraw.Draw(img).text((pad - l, pad - t), "i", font=font, fill=255, anchor="ls")
    ox, oy = pad - l, pad - t  # glyph origin in image coords; baseline at y=oy
    w, h = img.size
    px = img.load()
    rows = [any(px[x, y] > 10 for x in range(w)) for y in range(h)]
    y0 = rows.index(True)
    y1 = y0
    while y1 + 1 < h and rows[y1 + 1]:
        y1 += 1
    xs = [x for y in range(y0, y1 + 1) for x in range(w) if px[x, y] > 10]
    return min(xs) - ox, y0 - oy, max(xs) - ox, y1 - oy


def main():
    path, family, font = pick_font()
    asc, desc = font.getmetrics()
    total = font.getlength(TEXT)
    cap = -font.getbbox(TEXT, anchor="ls")[1]
    tl, tt, tr, tb = tittle_box(font)
    tw, th = tr - tl, tb - tt
    tcx, tcy = (tl + tr) / 2, (tt + tb) / 2

    # --- tick-in-tittle geometry (text-origin coords; word() translates onto the baseline) ---
    Wt, Ht = tw * 2.25, th * 1.5
    sw = round(tw * 0.55, 1)
    ux = [(0.04, 0.34), (0.36, 0.88), (0.96, 0.06)]
    pts = [(tcx - Wt / 2 + u * Wt, tcy - Ht / 2 + v * Ht) for u, v in ux]
    tick_d = f"M {pts[0][0]:.1f} {pts[0][1]:.1f} L {pts[1][0]:.1f} {pts[1][1]:.1f} L {pts[2][0]:.1f} {pts[2][1]:.1f}"

    pad_x, pad_y = round(SIZE * 0.22), round(SIZE * 0.30)
    W, H = round(total + 2 * pad_x), round(asc + desc + 2 * pad_y)
    baseline = pad_y + asc

    def doc(body, w=W, h=H, bg=None):
        bg = f'<rect width="{w}" height="{h}" fill="{bg}"/>\n  ' if bg else ""
        return (f'<?xml version="1.0" encoding="UTF-8"?>\n'
                f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}">\n'
                f'  {bg}{body}\n</svg>\n')

    def word(ink, acc, x_base=0.0, mono=False):
        ox = pad_x + x_base
        kt = (f'<rect x="{ox + tl - 2:.1f}" y="{baseline + tt - 2:.1f}" width="{tw + 4:.1f}" '
              f'height="{th + 4:.1f}" fill="#000"/>')
        halo = ""
        if mono:
            halo = (f'\n    <path transform="translate({ox:.1f},{baseline:.1f})" d="{tick_d}" fill="none" '
                    f'stroke="#000" stroke-width="{round(sw * 2.0, 1)}" stroke-linecap="round" '
                    f'stroke-linejoin="round"/>')
        return (f'<mask id="kt" maskUnits="userSpaceOnUse" x="-{W}" y="-{H}" width="{3 * W}" height="{3 * H}">\n'
                f'    <rect x="-{W}" y="-{H}" width="{3 * W}" height="{3 * H}" fill="#fff"/>\n    {kt}{halo}\n  </mask>\n  '
                f'<text x="{ox:.1f}" y="{baseline}" font-family="{family}" font-size="{SIZE}" '
                f'font-weight="500" fill="{ink}" mask="url(#kt)">{TEXT}</text>\n  '
                f'<path transform="translate({ox:.1f},{baseline:.1f})" d="{tick_d}" fill="none" '
                f'stroke="{acc}" stroke-width="{sw}" stroke-linecap="round" stroke-linejoin="round"/>')

    open("wordmark.svg", "w").write(doc(word(INK, VERMILION)))
    # monochrome: the tick is the same colour as the type, so knock a halo gap around it out
    # of the neighbouring letterforms.
    open("wordmark-mono.svg", "w").write(doc(word("currentColor", "currentColor", mono=True)))
    open("wordmark-inverse.svg", "w").write(doc(word(PAPER, VERMILION), bg=INK))

    # --- tagline lockup ---
    ts = round(SIZE * 0.165)
    rule_y, text_y = H + 54, H + 62
    body = (word(INK, VERMILION) + "\n  "
            + f'<path d="M {pad_x} {rule_y} H {pad_x + 44}" stroke="{VERMILION}" stroke-width="6" stroke-linecap="round"/>\n  '
            + f'<text x="{pad_x + 66}" y="{text_y}" font-family="{family}" font-size="{ts}" '
              f'letter-spacing="{ts*0.24:.1f}" fill="{SLATE}">{TAGLINE}</text>')
    open("wordmark-tagline.svg", "w").write(doc(body, h=text_y + pad_y * 0.5))

    # --- lockups embed the full-tier sheet mark (512x512 art) ---
    mark_art = svg_inner("icon-app.svg")

    def mark_group(size, x, y):
        s = size / 512.0
        return f'<g transform="translate({x:.1f},{y:.1f}) scale({s:.4f})">{mark_art}</g>'

    # vertical: mark above wordmark
    m = 150
    LW = max(W, m + 60)
    body = (mark_group(m, (LW - m) / 2, 0) + "\n  "
            + word(INK, VERMILION, x_base=(LW - total) / 2 - pad_x))
    open("lockup-v.svg", "w").write(doc(body, w=LW, h=m + 24 + baseline + pad_y * 0.6))

    # horizontal: mark left, wordmark right, cap-height centred
    mh = round(asc + desc) * 0.92
    gap = round(SIZE * 0.24)
    HW = mh + gap + total + 2 * pad_x
    my = (baseline - cap / 2) - mh / 2
    body = (mark_group(mh, pad_x, my) + "\n  " + word(INK, VERMILION, x_base=mh + gap))
    open("lockup-h.svg", "w").write(doc(body, w=round(HW)))

    print(f"font   : {family} ({path})")
    print(f"word   : '{TEXT}' advance={total:.1f} cap={cap:.1f}")
    print(f"tittle : x {tl:.1f}..{tr:.1f}  y {tt:.1f}..{tb:.1f}  tick box {Wt:.1f}x{Ht:.1f} stroke={sw}")
    print(f"canvas : {W}x{H}   lockup-v {LW}px   lockup-h {round(HW)}px")
    print("wrote: wordmark{,-mono,-inverse,-tagline}.svg  lockup-{v,h}.svg")


if __name__ == "__main__":
    main()
