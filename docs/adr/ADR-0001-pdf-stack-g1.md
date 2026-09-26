# ADR-0001 — PDF stack for G1 (OQ1): decision-support draft

| | |
|---|---|
| Status | **accepted** (2026-09-08, project owner) — lopdf + pdf-writer pure-Rust core, qpdf shell-out rung, pdfium deferred to G5 (was: **proposed**, decision-support only, SA-9/PDF-01) |
| Gate | G1 = OQ1 (plan §6.4; concept §18-1): license split pure-Rust vs shell-out vs AGPL-in-process |
| Decide by | **W0** (plan §6.4; §6.3 W0 contains PDF-01) |
| Decision owner | **project owner** (G1 is listed human-bound in `docs/phase0-execution.md`) |
| Blocks | PDF-02 raster path, `cargo deny` allow-list outcome, dual MIT/Apache-2.0 choice (GOV-05) |
| Related | D2, D3, D8, D11; plan §3.3; `deny.toml` (bans pending this ADR); G5 (pdfium, plan §6.4); `baseline/report-v1.md`; HAR-08 |

## 1. Context

`crates/adapters-pdf` needs an object-model reader (PDF-02), an append-only incremental writer
(PDF-03, INV-1), text + word-bbox extraction feeding the layout linter (PDF-04 → LIN-01), forms and
annotations (PDF-05/06), and a raster path feeding the D11 ladder. The stack decision fixes three
things at once:

- **D2** — Rust, six targets, *"C stays out of the binary"*: one static binary cross-compiled to
  6 targets (deny.toml `targets`), DIS-01 runs a CI "no C in tree" check.
- **D8** — zero hard deps: engines are soft `Recommends:` spawned subprocesses, never linked,
  never `Depends:` (plan §3.3 "soft, spawned at runtime" row already lists poppler/gs/mutool).
- **License posture** — dual MIT/Apache-2.0 recommended (concept §18-1, GOV-05); `deny.toml`
  allow-list is MIT/Apache-2.0/BSD/ISC/Unicode-DFS-2016 with `copyleft = "deny"` and
  `deny = [AGPL-3.0*, GPL-3.0*]`, plus deny-by-name bans on `pdfium*`, `mupdf*`, `lokit*`
  held "pending PDF-01/OQ1 ADR" (measured: `deny.toml` at M0).

## 2. Options — with measured license facts

All license rows fetched 2026-09-05 (raw.githubusercontent.com / pdfium.googlesource.com); legal
*interpretation* rows are marked unverified — counsel before relying on them.

| Option | Upstream license (source, fetched 2026-09-05) | D2 (C out of binary) | D8 (no hard deps) | dual MIT/Apache-2.0 posture |
|---|---|---|---|---|
| **A. Pure-Rust in-process**: `lopdf` (reader/object layer) + `pdf-writer` (writer) | `lopdf` = **"MIT"** — measured `license = "MIT"` in `Cargo.toml` @ master (0.44.0) **and** tag v0.34.0 → **MIT-only, NOT MIT/Apache dual** (corrects the brief's assumption; MIT is on the allow-list so the split still works). `pdf-writer` = **"MIT OR Apache-2.0"** — measured `Cargo.toml` @ typst/pdf-writer main (0.15.0) | ✅ pure Rust | ✅ zero runtime deps | ✅ safe (MIT ⊆ allow-list; compatible with both grants) |
| **B. Shell-out `qpdf`** (check/repair/optimize glue, PDF-07) | **Apache-2.0** — measured `LICENSE.txt` @ qpdf/qpdf master | ✅ (subprocess; in-process C++ link would violate D2) | ✅ `Recommends:` only; probe-never-assume (note: SA-3 measured a Fedora 44 host **without** qpdf — capability probes, not assumptions) | ✅ permissive upstream; process boundary keeps it clean (unverified legal nit: Apache NOTICE attribution if redistributed alongside — packaging matter, not license conflict) |
| **C. Shell-out `mutool` (MuPDF)** | **AGPL-3.0** — measured `COPYING` @ ArtifexSoftware/mupdf master | ✅ (subprocess) | ✅ `Recommends:` rung | ⚠️ **AGPL-adjacent**: arm's-length CLI invocation is the common read, but FSF positions tighter couplings as derivative; AGPL §13 bites if stichos is ever offered as a network service around it. **Unverified — counsel question.** In-process link (any `mupdf*` crate): kills the dual-license outright (whole artifact AGPL). Policy: optional rung, user-installed, never bundled/required/advertised as core. |
| **C′. Shell-out Ghostscript** (D11 ladder rung, already probed) | AGPL-3.0-or-later **with commercial dual-license** (Artifex; researched, text not re-fetched here) | ✅ | ✅ | ⚠️ same posture as C — ladder rung only. Baseline raster already runs on poppler `pdftoppm` (report-v1), so gs is redundancy, not dependence. |
| **D. pdfium bundled** (G5 territory, listed for completeness) | BSD-3-clause-style — measured `LICENSE` @ pdfium.googlesource.com/pdfium main (note: the file carries BSD-3-clause text **and** an appended Apache-2.0 text — observed oddity; treat as BSD-ish) | ❌ **C in the binary** — violates D2; needs a D-log reversal (plan §6.4 G5 row) | ⚠️ bundled binary dodges D8 by shipping C, at the cost of the 6-target matrix (DIS-01) | ✅ license-permissive; the problem is D2, not licensing |

## 3. G5 status — pdfium stays gated, and M0 produced NO evidence to open it

G5 trigger (plan §6.4): *"pdfium bundling — only if G1 shows pure-Rust extraction insufficient for
LIN-01"*, decided "after HAR-06 data". **Measured state:** the M0 corpus produced **zero PDF-input
evidence** — `baseline/report-v1.md` feeds office formats through LO/pandoc and only re-rasterizes
**our own golden PDFs** with `pdftoppm`; the `scanned-ocr` fixtures are raster-only by design
(GENERATION_REPORT). No third-party PDF corpus, no bbox-extraction run, nothing bearing on
pure-Rust extraction adequacy. → **G5 remains open (no data, not a "no").**

What measurement would decide it (concrete experiment at PDF-04, before any G5 discussion):

1. Build a PDF-input sub-corpus (manifest-governed, C9; real-world PDFs incl. CJK/RTL text,
   forms, tagged structure — the categories report-v1 showed LO's own PDF text layer mangles).
2. Extract word-level bboxes two ways: own content-stream extractor (PDF-04 basic) vs poppler
   `-bbox` JSON (shell-out rung).
3. Measure: word coverage %, bbox IoU vs ground truth, and downstream LIN-01 decision quality
   (overflow/overlap precision/recall) through each feed.
4. Only if the pure-Rust + poppler-shell-out combination measurably fails LIN-01 inputs does the
   pdfium/D2 reversal question become real — and then it is a D-log entry (D2), not a quiet dependency.

## 4. Consequences per option

| Option | Install UX | D2/D8 compliance | License posture | Dev cost |
|---|---|---|---|---|
| A. lopdf + pdf-writer | best — nothing to install, all 6 targets identical | ✅✅ | ✅ dual-license safe | **high**: own incremental writer (PDF-03), own content-stream extractor incl. word bbox (PDF-04, 4 p-w planned) |
| B. + qpdf shell-out | good on most distros, honest degrade where absent (measured: not on our own Fedora box) — `doctor` reports it (ENG-05) | ✅✅ | ✅ | low (glue + runner, PDF-07 = 0.5 p-w) |
| C/C′. mutool/gs rungs | degrade gracefully; availability varies | ✅✅ (subprocess) | ⚠️ counsel sign-off needed for any hosted/service variant | low |
| D. pdfium bundled | best runtime story (no externals) | ❌ reverses D2, cross-compile matrix + binary size | ✅ license-wise | low at runtime, real cost in DIS-01 + supply chain |

## 5. RECOMMENDATION (non-binding — the owner decides)

1. **Pure-Rust core**: `lopdf` (read/object layer) + `pdf-writer` (write) in `adapters-pdf` — already
   the plan §3.3 "baked in" intent; keep `deny.toml` license allow-list as-is. (If lopdf's MIT-only
   licensing is ever a problem for a downstream consumer, that is a pin-level tradeoff, not a reason
   to touch D2.)
2. **qpdf shell-out as the pragmatic default** for validation/repair/optimize glue (PDF-07 `qpdf --check`
   already planned) — Apache-2.0, D8-clean, `Recommends:` only, probe-never-assume.
3. **mutool and gs stay optional ladder rungs** exactly per D11: user-installed, spawned, never
   bundled, never `Depends:`, not in marketing claims; obtain counsel review of the shell-out posture
   **before** any hosted/AGPL-sensitive deployment story.
4. **Keep every `deny.toml` C-renderer/AGPL ban in force.** The ADR accepting this recommendation
   changes no bans; lifting `pdfium*` requires a separate G5 ADR **plus** a D-log entry against D2.
5. **Defer pdfium to G5**, gated on the §3 experiment — which requires first building the PDF-input
   sub-corpus that M0 deliberately did not contain.

Rationale: keeps dual MIT/Apache-2.0 safe without legal exposure we don't need, keeps D2/D8 intact,
and concentrates dev cost where we must own quality anyway — report-v1 shows renderer text layers
themselves are unreliable (RTL F1(soffice) 0.288 vs pandoc 0.985; odp pdftotext artifact), so a
first-class own extractor is the product's job regardless of what pdfium could do.

## 6. After the decision

Owner flips Status to accepted/rejected (this file), PDF-01 deliverable completes with the matching
`deny.toml` state (bans unchanged under the recommendation), and PDF-02/03/04 proceed on the chosen
core. Nothing here edits `project_concept.md` — no D is reversed by a G1 outcome; only a G5 reversal
would force a §16 entry (plan §9 closing note).

## 7. G5 closure — pdfium adopted (dynamic linking only, 2026-09-26)

G5 was triggered by: *"pdfium bundling — only if G1 shows pure-Rust extraction insufficient for
LIN-01"*.

**Decision: Adopt pdfium-render with dynamic linking only, behind a `pdfium` feature flag.**

- **Binding mode**: `Pdfium::bind_to_system_library()` — probes binary directory → system PATH → fails gracefully. No static bundling, no C in the compiled Rust binary.
- **D2 compliance**: Maintained. The Rust binary contains zero C code. `libpdfium.so` is a runtime shared library (analogous to `libpoppler.so` on systems with poppler installed).
- **D8 compliance**: Maintained. Feature is off by default (`pdfium` not in `default =`). Probed at runtime via `pdfium_available()`. Honest-absent fallback to poppler/gs/mutool/soffice.
- **deny.toml**: `pdfium-render` and `pdfium*` bans removed. `mupdf*` bans remain (AGPL upstream).
- **Implementation**: `crates/engines/src/raster_pdfium.rs` with `rasterize_with_pdfium()` and `pdfium_available()`. Integrated as the first rung in the raster ladder when enabled.
- **D-row**: No reversal needed. Dynamic linking does not reverse D2 ("C stays out of the binary") — the binary is pure Rust, the C library is a soft runtime dependency.
- **Transitive deps**: `pdfium-render` 0.9 → `image` 0.25 (MIT/Apache-2.0, on allow-list). No `-sys` crates.

## Sources (researched 2026-09-05 unless marked measured = local repo/host)

- lopdf `Cargo.toml` (master 0.44.0; tag v0.34.0): `license = "MIT"` — raw.githubusercontent.com/J-F-Liu/lopdf
- pdf-writer `Cargo.toml` (typst main 0.15.0): `"MIT OR Apache-2.0"` — raw.githubusercontent.com/typst/pdf-writer
- qpdf `LICENSE.txt`: Apache-2.0 — raw.githubusercontent.com/qpdf/qpdf
- MuPDF `COPYING`: AGPL-3.0 — raw.githubusercontent.com/ArtifexSoftware/mupdf
- pdfium `LICENSE`: BSD-3-clause text + appended Apache-2.0 text — pdfium.googlesource.com/pdfium
- Ghostscript AGPL/commercial dual — researched (well-known Artifex posture), **not re-fetched here**
- LO 26.8.0/26.2.6 supported-branch labels — libreoffice.org/download/release-notes (also in HAR-08 doc)
- G1/G5/D2/D8/OQ1 — `project_plan.md` §3.3/§6.4, `project_concept.md` §16/§18-1; bans — `deny.toml`;
  render/text-quality evidence — `baseline/report-v1.md`; Fedora-host-missing-qpdf — measured, `docs/phase0-execution.md`
- All **legal interpretation** rows: unverified, not legal advice; counsel is the named escalation.

## Decision (owner, 2026-09-08)

**Accepted as recommended:** lopdf + pdf-writer as the pure-Rust core for `adapters-pdf`
(PDF-02/03), with qpdf as a shell-out validation rung (Apache-2.0, CLI-only — D2/D8 preserved)
and mutool/gs as optional D11 ladder rungs. pdfium stays deferred behind G5, to be decided by
the PDF-04 word-bbox experiment; AGPL-in-process remains banned (deny.toml unchanged).
G1 closed → the dual MIT/Apache-2.0 licensing path is unblocked for GOV-05; OQ1 resolved in
`project_concept.md` §18; recorded as D19 in the §16 decision log.
