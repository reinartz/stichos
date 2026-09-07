# stichos harness image (HAR-03)

Pinned OCI image for golden harvesting and render loops: exact LibreOffice build +
poppler + fixed font set. Concept basis: **D14** (fonts first-class, fixed set),
**D15** (pinned engines — LO releases change layout), **INV-4** (render-cache key
includes `fontset_hash`, so goldens are bit-reproducible inside this container).

## Usage

```bash
bash docker/harness/build.sh          # build + tag stichos-harness:26.2.5, prints digest
bash tools/fontset_hash.sh            # fontset_hash=sha256:<hex> + lo/poppler versions

# typical harness invocation (non-root user `harvester` is the default)
docker run --rm stichos-harness:26.2.5 bash -c '
  cd /work
  soffice --headless -env:UserInstallation=file:///tmp/u1 --convert-to pdf doc.docx
  pdftoppm -r 150 -png doc.pdf page'
```

`-env:UserInstallation` is required per invocation profile; `harvester` has a
writable HOME (`/home/harvester`) and `/work` as default workdir.

## What is pinned, and why

| Layer | Pin | Why |
|---|---|---|
| Base | `ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517` (24.04, measured pull 2026-09-03) | reproducibility floor |
| LibreOffice | **26.2.5** official TDF stable deb bundle, sha256-pinned + verified against the upstream `.sha256` at build time | D15: LO releases change layout; distro LO varies and is often stale |
| poppler | `poppler-utils` from the pinned Ubuntu base (`24.02.0-1ubuntu9.9`, in build-info) | `pdftoppm`/`pdffonts` are part of the render pipeline |
| Fonts (D14) | `fonts-crosextra-carlito`, `fonts-crosextra-caladea`, `fonts-liberation`, `fonts-noto-core`, `fonts-noto-cjk`, `fonts-noto-extra` | metric-clone + core/CJK/RTL coverage required by the corpus; the set IS the `fontset_hash` |

`/opt/stichos/build-info.json` records `lo_version`, `poppler_version`,
`image_digest_base`, `fonts`, `built_at`. Because `built_at` changes on every
build, the **image is intentionally not byte-reproducible** and build-info is
**excluded from the fontset hash** — the hash is computed from font file bytes only.

## FONTSET_HASH (INV-4)

`tools/fontset_hash.sh` runs inside the container: for every file under
`/usr/share/fonts` (plus `~/.fonts` if present) it emits
`"<relative-path> <sha256-of-file-bytes>"`, sorts the lines, and sha256s the
aggregate. It covers **bytes and paths only — never mtimes, permissions, or
ownership** — so the same image yields the same hash across any number of
`docker run` invocations and hosts.

## Upgrade procedure

Do **not** bump the pin ad hoc. Upgrades go through the engine-upgrade gate
(HAR-08, procedure doc to follow): new LO release → full corpus run + human
review before the pin changes. When a pin changes, `fontset_hash` may change
too; all goldens are keyed by it (INV-4), old goldens stay queryable.

## CI

Nightly (SA-8 workflow): `docker build` → `soffice --version` must equal the
pin → `tools/fontset_hash.sh` must equal the recorded value → golden smoke.
Any drift fails the nightly, not a user. PR builds reuse the published image;
only `docker/harness/**` changes rebuild it.

## Measured (2026-09-03, Fedora 44 host, docker 29.6.2)

- LO version inside: `soffice --version` → `LibreOffice 26.2.5.2 cd7284b4cbbfeb507e630c1aac019f4157393acb`
- Bundle sha256 `2f03bfb2ac9f33ea7c77331b4b7a23300fb0ed7443566046bf8b5bc51c1bed1e` — upstream `.sha256`
  fetched (HTTP 200) and both checks passed inside the build (`sha256sum -c` against pin and against upstream file).
- `fontset_hash=sha256:6f26ae4f9bdea927cffa0984d48adcb3e4b60b3fbe263c6a44440a5628927509`
  (with cjk+extra fonts; supersedes `6e33fa71…` measured before the font-set fix) —
  identical across two separate `docker run` invocations post-rebuild.
  An honest fresh-cache rebuild has not been run; the hash is byte/path-based, not image-byte-based.
- Image id `sha256:ded77b6fce12480f971ee4cb68077fe59c99dc1235c9a6a57e0fc8a391516efb`,
  size 1,345,721,908 bytes (~1.35 GB, uncompressed), after adding fonts-noto-cjk/extra.
- Smoke: UTF-8 txt → `soffice --headless -env:UserInstallation=file:///tmp/u1 --convert-to pdf`
  → `pdftoppm -r 150 -png` → `page-1.png` (1241×1754 px = A4@150dpi); `pdffonts` shows
  `BAAAAA+LiberationMono` and `CAAAAA+DejaVuSansMono` embedded.

## Deviations / notes

- TDF debs install to `/opt/libreoffice26.2` and create **no** `/usr/bin/soffice`
  on a headless base; the image symlinks `/usr/local/bin/soffice` (and `unopkg`).
- The deb bundle does not pull X11/NSS/glib/cairo/cups runtime libs (measured via
  `ldd`); they are installed explicitly in the apt layer.
- uid 1000 is taken by the base image's `ubuntu` user, so `harvester` is **uid 1001**.
- Harmless stderr noise from LO's bundled python: `Could not find platform
  independent libraries <prefix>` — conversions unaffected.
- `fonts-noto-core` alone has no CJK (and Arabic/Hebrew live in `fonts-noto-extra`);
  fixed 2026-09-03 by adding `fonts-noto-cjk` + `fonts-noto-extra` (160 CJK/Arabic/Hebrew
  faces in `fc-list`, measured) so corpus `cjk-text`/`rtl-text` fixtures render for real.
- The apt font packages are pinned only through the base digest; Ubuntu security
  updates to font packages WILL change `fontset_hash` on a fresh-cache rebuild
  after the archive moves. That is INV-4 working as intended (old goldens stay
  under the old hash), but pin an apt snapshot if goldens must be re-harvested
  bit-identically much later.
