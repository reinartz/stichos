# harness/render — golden render pipeline + content-addressed approved store (HAR-05)

Single-file CLI: `python3 golden.py <command>` (run from anywhere; repo root auto-detected,
override with `GP_ROOT`; image override with `GP_IMAGE`, default `stichos-harness:26.2.5`).

Renders every corpus fixture in the pinned container (LibreOffice 26.2.5.2 + poppler 24.02),
content-addresses the outputs, and holds the `candidate → approved` human gate
(C9 §2 `rendered` → `golden-approved`).

## Commands

```
python3 golden.py render   [--id ID] [--category C] [--dpi 150] [--jobs 4]
python3 golden.py stability [--ids a,b] [--category C] [--repeats N] [--jobs 4]
python3 golden.py approve  <id> | --all-staged  [--reviewer NAME]
python3 golden.py list     [--state candidate|approved]
python3 golden.py verify   <id>
```

- **render** — per fixture: container run (`corpus/blobs` mounted READ-ONLY at `/in`, all
  container writes in `/tmp`, results pulled out through an RW mount on the staging dir) →
  `timeout 240 soffice --headless -env:UserInstallation=file:///tmp/u --convert-to pdf` (cwd `/tmp`)
  → `pdftoppm -r <dpi> -gray -png` → PDF + page PNGs land in `goldens/staging/<id>/`.
  sha256 of PDF + each page → `goldens/store/<engine-key>/<fixture-sha>/record.json`
  (state `candidate`) and bytes copied to `goldens/objects/` (dedup via `exists()`).
- **stability** — N fresh re-renders (default 2) per fixture in throwaway containers; PASS iff the
  page-hash sequences are identical. With `--repeats 1` the single fresh render is compared against
  the stored record (used to fold the second full pass into the stability gate). Writes
  `goldens/stability.json` incrementally (crash-safe, resumable per fixture).
- **approve** — promotes `candidate → approved` ONLY if the fixture's stability entry is `ok`;
  appends `{fixture, keys…, reviewer, when}` to `goldens/approvals.json`. Reviewer comes from
  `--reviewer`; the M0 default `"owner-pending"` is allowed but recorded verbatim. Never run this
  as part of a pipeline — it is the human gate.
- **list** — table of records under the current engine-key.
- **verify** — one fresh re-render compared hash-for-hash against the stored record; JSON verdict +
  exit code (0 stable / 1 drift). This is the INV-4 check.

## Layout (`goldens/`, gitignored — regenerable)

```
goldens/env.json                     fontset_hash + engine/poppler versions (measured, cached)
goldens/staging/<id>/                latest staged render: render.pdf + pages/page-*.png
goldens/objects/<sha[:2]>/<sha>.pdf  content-addressed objects (dedup across runs/fixtures)
goldens/objects/<sha[:2]>/<sha>.png
goldens/store/<engine-key>/<fixture-sha>/record.json   {fixture, fixture_sha, engine,
                                       engine_version, fontset_hash, dpi, pdf_sha, pages[],
                                       state, staged_at}
goldens/stability.json               per-fixture stability results + summary
goldens/approvals.json               approval records (append-only)
```

## Engine key & INV-4

`engine-key = "{engine}-{engine_version}-{fontset_hash_hex[:16]}"`
= `soffice-26.2.5-26.2.5.2-6f26ae4f9bdea927` (fontset_hash from `tools/fontset_hash.sh`,
sha256 over sorted `<relpath> <sha256>` of every font file in the image).

INV-4 says the font set is part of the render identity: if the image's fonts change, `fontset_hash`
changes, the engine-key changes, and every fixture needs a fresh golden + approval. Old
engine-key directories stay in the store, so old goldens remain queryable for upgrade diffs
(HAR-08) instead of being overwritten. `verify` proves it: same image ⇒ same page hashes.

## C9 mapping

| C9 | Where implemented |
|---|---|
| §2 `rendered` | `render` writes the candidate record for `(engine, dpi)` in manifest `render{}` |
| §2 `golden-approved` | `approve` writes `(fixture sha256, engine id+version, fontset_hash, dpi)` to the store + approvals.json; golden key = sha256 over the ordered page-hash list ("content hash of the approved raster set") |
| §4 SSIM inputs | rasters are `pdftoppm -r 150 -gray -png`, un-reencoded, 8-bit gray — exactly the §4 normative input for `harness/metrics` |
| §10.8 page order | pages natural-sorted by trailing page number before hashing |

Note (measured): poppler 24.02's bare `-gray` emits **PGM**; `-gray -png` is the documented way to
get the same grayscale raster as PNG ("generate a grayscale … PNG file" in `pdftoppm --help`).

## Measured notes

- PDF **bytes** are not stable across runs (LO embeds CreationDate/DOCID), so `pdf_sha` differs
  run-to-run while page rasters hash identical. Stability/golden identity therefore keys on the
  **page-hash sequence** (C9 §2 golden key), with `pdf_sha` stored as object provenance only.
- Container user is `harvester` uid 1001; host staging dirs are chmod 777 so the RW mount accepts
  writes from any host uid. Blobs are mounted `:ro` — the container cannot mutate the corpus.
- `soffice` prints `Could not find platform independent libraries` (bundled-python noise) on stderr;
  harmless — success is checked by `f.pdf` existing non-empty.
