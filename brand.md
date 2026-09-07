# stichos — brand & identity rules

> Identity for the **stichos** project: a local agent that analyses, edits and creates office
> documents and returns *visual proof* of the result. Tagline: **proof before publish**.

## 1. Concept

The mark is **a sheet of set lines carrying a proofreader's tick**. *stichos* is Greek for
**line, row** — the unit of stichometry, the old craft of counting lines of text — and the three
slate bars in the mark are exactly that: three stichoi. "Document" is read instantly; the tick
plus printer's vermilion says *proofed*, not *written*. That distinction is the product.

Three tiers, because one drawing cannot survive 1024px → 16px:

| Tier | File | Contents | Minimum size |
|---|---|---|---|
| **Full** | `assets/icon-app.svg` | ink squircle, offset *verso* sheet, three typeset bars, vermilion tick | 64px |
| **Tight** | `assets/icon-tight.svg` | larger sheet, no bars, fat tick, all geometry on a 32-unit grid (= 1px at 16px) | 16px |
| **Glyph** | `assets/icon-glyph.svg` | monochrome outline, `currentColor`, no squircle | 20px |
| **Hero** | `assets/icon-hero.svg` | full mark + crop marks + registration target + type grid | 512px (marketing only) |

Rules of use:
- ≥64px → **Full**. 16–48px → **Tight**. Inline UI/terminal/docs → **Glyph**.
- Never use Full below 64px; never use Tight above ~160px (it reads as a checkbox).
- Never render Glyph as a filled sheet with a knocked-out tick at ≤16px (tested: it fills in).

## 2. Colour

| Token | Hex | Use |
|---|---|---|
| `ink` | `#101A2E` | squircle ground, primary type |
| `paper` | `#FBF7EE` | sheets, type on dark |
| `slate` | `#93A0B8` | typeset bars, metadata, tagline |
| `vermilion` | `#E2483A` | **accent only**: the tick, one rule, one link underline |
| `vermilion-deep` | `#C0392B` | hover/active |
| `paper-22` | `#FBF7EE` @ 22 % | verso hint |
| `surface` | `#F4F1EA` | page background, screenshots |

Constraints:
- **One** vermilion element per composition. Never fill a shape with it, never gradient it.
- Vermilion on `ink` or `paper` only. Never vermilion text at body sizes.
- `ink` on `paper`, or `paper` on `ink`. No greys between them.
- Deliberately **not** Office blue (#0F6CBD/#2B579A) or LibreOffice green — do not reintroduce.
- WCAG: `ink` on `paper` = 15.9:1 ✅; `slate` on `paper` = 3.4:1 → large text / non-essential only.

## 3. Typography

| Role | Face | Weight | Fallbacks |
|---|---|---|---|
| Wordmark, headings | **Inter** | 500 (wordmark), 600 (headings) | `Noto Sans, system-ui, sans-serif` |
| UI, body | Inter | 400/500 | same |
| Code, CLI, terminal banners | **JetBrains Mono** | 400/500 | `ui-monospace, monospace` |
| Tagline | Inter | 500, uppercase, +0.24 em tracking, `slate` | — |

- Wordmark is **always lowercase**: `stichos`. No capitals, no camel case, never `Stichos`.
- Wordmark is set at weight 500; do not bold it for emphasis — use `ink` vs `paper` instead.
- Inter is OFL-licensed and free to bundle. If the repo must stay dependency-free, ship the
  outline SVGs (`*-outline.svg`) — text is already converted to paths.

## 4. Wordmark construction

Laid out programmatically (`assets/wordmark.py`) from real font metrics:

```
TEXT      "stichos"        SIZE 220 px     advance 759.5 px
'i' tittle  x 13.0 → 42.0    y -167.0 → -139.0  (baseline-relative; cap 167.0)
tick      replaces the knocked-out tittle: a compact check centred on the tittle slot,
          short arm from upper-left, vertex low-left, long arm rising right
stroke    16.0 px (0.55 × tittle width), round caps and joins
canvas    855 × 400
```

Variants: `wordmark.svg` (ink + checked `i` in vermilion) · `wordmark-inverse.svg` (paper on ink) ·
`wordmark-mono.svg` (single `currentColor`; the mask knocks out the tittle slot and keeps a halo
gap between tick and type — a same-colour tick without the gap reads as a broken `i`) ·
`wordmark-tagline.svg` (rule + tagline).

The tick lives in the tittle because `stichos` has no descenders — the tittle slot is the only
place a tick can sit without crossing a letter (tested: zero glyph collision at 1× and at the
mono halo width).

Lockups: `lockup-h.svg` (mark left of wordmark, cap-height centred) and `lockup-v.svg`
(mark above wordmark). Both embed the **Full** tier art and both carry the checked `i`.

## 5. Clear space & sizing

- Clear space = **20 %** of the mark's height on all sides; never crowd it, never let the tick
  touch a frame edge.
- Minimum sizes: Full 64px · Tight 16px · Glyph 20px · wordmark 96px wide · lockups 24px mark.
- Maximum: the mark must not exceed 60 % of the containing tile's height.
- Never rotate, outline, bevel, shadow, or add a keyline to the squircle.

## 6. Application surfaces

| Surface | Asset | Notes |
|---|---|---|
| macOS `.icns` | `icon-app.svg` @ 16…1024 | `iconutil -c icns stichos.iconset` (run on macOS) |
| Windows `.ico` | per-size sources, not resampled | `stichos-*.png` 16/24/32/48/64/128/256 → `stichos.ico` |
| DEB/RPM | `dist/icons/stichos-512.png` → `hicolor/512x512/apps/stichos.png` | ship `stichos.desktop` with `Icon=stichos` |
| Homebrew / pkg | iconset + squircle PNG | notarised bundle |
| winget / MSI | `.ico` | Authenticode-signed with timestamp |
| README / docs header | `lockup-h.svg` | on `surface` background |
| Terminal banner / ASCII | `icon-glyph.svg` | or the text form `stichos` |
| Favicon | `favicon-32.png` + `icon-tight.svg` | 16px-safe |

## 7. Voice

- Say **proof**, **render**, **fidelity budget**, **renderer provenance** — the vocabulary is the
  product model; keep it consistent in docs, CLI output and API field names.
- Never claim a document "looks fine" — report the checks that ran and what they found.
- Plain verbs for CLI: `inspect`, `render`, `diff`, `apply`, `doctor`.

## 8. File inventory

```
assets/
  icon-app.svg          icon-tight.svg      icon-glyph.svg      icon-hero.svg
  wordmark.py           wordmark.svg        wordmark-mono.svg   wordmark-inverse.svg
  wordmark-tagline.svg  lockup-h.svg        lockup-v.svg
  *-outline.svg         (text converted to paths; ships without fonts)
  build-icons.sh        (regenerates every raster size, .ico, iconset, favicons)
  dist/icons/           generated raster + .ico + .iconset + SVG copies
  out/                  previews used during design review (regenerable, not shipped)
```

## 9. Open items

- Trademark clearance (classes 9 & 42) still pending — registry availability is not legal clearance.
- `stichos.com` is registered; `.dev` is free. Decide before publishing a website.
- Decide whether the terminal banner gets an ASCII rendition of the tick or text only.
