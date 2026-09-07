# Engine-upgrade gate (HAR-08)

**Scope:** the pinned harness image `stichos-harness` — LibreOffice build + poppler + fixed font
set (D14/D15, `docker/harness/`). The single source of truth for what is pinned is
`docker/harness/pin.txt`; prose evidence lives in `docker/harness/README.md`.
**Depends:** HAR-07 (nightly is the enforcement point, §7). **Freezes:** M0 procedure; thresholds are
deliberately NOT invented here (C9 §8 precedent — no made-up numbers).

## 1. The one rule (INV-4)

> **An engine upgrade through this gate is the ONLY admissible cause for a mass golden change.
> A golden drift on an UNCHANGED pin is a bug** — never re-approve it; open an issue.

INV-4 makes this mechanically checkable: the render-cache key is
`sha256(doc)+engine+version+fontset_hash+dpi` (plan §3.4), and the golden store is keyed by
`engine-key = soffice-<lo_version>-<lo_build>-<fontset_hash[:16]>` (`harness/render/`).
Same pin ⇒ same hashes, always (`golden.py verify`). Different pin ⇒ different engine-key directory ⇒
**old goldens are never overwritten** — which is what makes the upgrade diff possible and rollback free.

## 2. Triggers

| Trigger | Kind | Detection |
|---|---|---|
| New LO **patch** release inside the pinned series (26.2.5 → 26.2.6) | routine | §3 watch |
| New LO **series** becomes TDF stable (26.2 → 26.8) | scheduled | §3 watch |
| Pinned LO branch leaves TDF's supported branches (EOL) | **mandatory**, D15 window shift | §3 watch + D-log note (§4 step 8) |
| Harness base-image refresh (new Ubuntu base digest) — carries **poppler** (`pdftoppm` is part of the render pipeline) and font packages | routine or security | maintainer-initiated; `fontset_hash` will move |
| Security advisory (LO or poppler CVE) | fast-track | distro/TDF advisories; §6 SLA |

A `fontset_hash` change **without** an LO change (Ubuntu archive moved font packages — measured risk,
`docker/harness/README.md` "Deviations") is still a full gate event: the hash is golden identity.
Pin an apt snapshot if goldens must ever be re-harvested bit-identically much later.

## 3. Watching for new releases

Two sources, one mechanical, one human:

1. **TDF download-tree listing** (canonical, machine-readable):
   `https://download.documentfoundation.org/libreoffice/stable/` — an Apache index of `X.Y.Z/` dirs.
   Measured 2026-09-05: `25.8.7/`, `26.2.5/`, `26.2.6/` (dated 2026-09-03), `26.8.0/` (2026-08-26).
   Check-script sketch (`tools/watch_lo_release.py`, weekly CI job or manual; NOT shipped at M0):

   ```python
   # GET https://download.documentfoundation.org/libreoffice/stable/
   vers = re.findall(r'href="(\d+\.\d+\.\d+)/"', html)               # version dirs
   pin  = parse_keyfile("docker/harness/pin.txt")                     # lo_version = 26.2.5
   new  = sorted(vers, key=version_key)
   same_series = [v for v in new if v.startswith(pin["lo_version"].rsplit(".", 1)[0] + ".") and
                  version_key(v) > version_key(pin["lo_version"])]
   # same_series -> PATCH bump candidate ; versions in a newer series -> SERIES bump candidate
   # for the top candidate: GET ${tar_url}.sha256 -> pre-fill the new LO_SHA256 ARG
   ```

2. **Release-announce pages** (human context + supported-branch labels):
   `https://www.libreoffice.org/download/release-notes/` (measured 2026-09-05: labels exactly two
   supported branches — 26.8.0 "Latest Branch", 26.2.6 "Previous Branch"; older versions moved to
   `downloadarchive.documentfoundation.org/libreoffice/old`) plus `https://blog.documentfoundation.org/`.
   Note: the fetched page dated 26.2.6 "September 24, 2026" while the download tree already carried a
   `26.2.6/` dir dated 2026-09-03 — observed discrepancy, unexplained (unverified); rely on the tree
   for availability, the page for support status.

**Current state (measured 2026-09-05):** pin is `26.2.5`, TDF supports 26.8.x + 26.2.x → the gate has
immediate work: first application = patch bump **26.2.6** (dry-run of this procedure), then **26.8.0**
(series bump, expect a large golden diff) as a scheduled upgrade.

## 4. Procedure

0. Open an upgrade record: branch `upgrade/lo-<new>` + report file `docs/harness/upgrade-reports/<new-pin>.md`.
1. **Bump the ARGs** in `docker/harness/Dockerfile`: `LO_VERSION` + `LO_SHA256` (from the upstream
   `.sha256`, per the URL pattern already in the Dockerfile header); `BASE_REF` too if the base moves
   (poppler/font consequence, §2). Never edit goldens or `pin.txt` yet.
2. **Build the new tag:** `bash docker/harness/build.sh` → `stichos-harness:<new>` (the build
   double-verifies the bundle: pinned ARG + upstream `.sha256`, `sha256sum -c`).
3. **Record `fontset_hash`:** `bash tools/fontset_hash.sh` → candidate value + `soffice --version` +
   `dpkg-query poppler-utils`. Enter all three in the upgrade record. A changed hash is expected only
   for base/font moves; a changed hash for a pure LO bump is itself a finding.
4. **Full-corpus golden harvest under the new pin** (NOT the 25-id nightly sample):
   `GP_IMAGE=stichos-harness:<new> python3 harness/render/golden.py render` (127 × 150 dpi,
   `-gray -png`, C9 §4 inputs). Per INV-4 the store writes to a NEW engine-key dir
   (`soffice-<new>-<hash16>/`); the old engine-key dir is untouched — old goldens survive and stay
   queryable for this diff (C9 §2, `harness/render/README.md`).
5. **Stability pass** (full corpus, not the sample):
   `GP_IMAGE=... golden.py stability --repeats 2` — PASS iff page-hash sequences identical per
   fixture. Any instability at a NEW pin = blocker (investigate before review); approval is only
   possible for `ok` fixtures (`approve` enforces this).
6. **Diff report vs the old pin** — the review input (sketch `harness/render/upgrade_diff.py`,
   machine-readable JSON + human tables, attached to the upgrade report):
   - **Page-count deltas** (C9 §5): new golden page count vs old approved count (approved measured
     value, never a guess). Report per fixture + histogram; every nonzero Δ needs a reason.
     (Baseline reference, report-v1: Δ=0 on 127/127 at 26.2.5.)
   - **SSIM distribution** — only where page counts match, via `harness/metrics`
     `ssim_pages(new_staging_dir, old_approved_pages_dir, dpi=150)`; the old approved rasters are
     materialized from the content-addressed store (`record.json` page hashes →
     `goldens/objects/<sha[:2]>/<sha>.png`). Report per-fixture `min`/`mean` + corpus p05/p50/p95 +
     counts below owner-set review lines (owner sets them per upgrade — no invented constants), and
     honor the returned flags (`size_mismatch`, `missing`).
   - **Retention diffs** (C9 §6): rerun the LO round-trip feature-count map under the new image
     (`harness/baseline/baseline.py run` from a CLEAN CSV, as nightly does) and diff the PASS set vs
     `baseline/report-v1.md` (111/127 at 26.2.5). A NEW failure is a regression and blocks by default;
     a FIXED failure is noted. Known-mechanism classes (odp `draw:frame` re-wrap, odp annotation
     drop, OLE frame loss) must not get worse without explicit owner sign-off.
   - **F1 deltas** (C9 §7): `text_f1(pdftotext(new golden), corpus/reference/<id>.txt)` vs report-v1
     per fixture. Gate by category floors, not corpus-wide means — report-v1 proved pdftotext F1 is
     unusable for odp (one-char-per-line golden artifact) and unreliable for RTL; those categories
     are review-only here.
7. **Human review checklist** (owner + 1 reviewer, per C9 §2; every box cites evidence in the report):
   - [ ] every golden change attributable to a changelog entry or a classified mechanism (rendering
         improvement / representation churn / true regression)
   - [ ] no unexplained page-count Δ
   - [ ] no retention regression vs old pin (new `count_after != count_before` classes)
   - [ ] no F1 regression below category floors outside the known-unreliable (odp/RTL pdftotext) set
   - [ ] full-corpus stability `ok` at the new pin
   - [ ] SSIM distribution reviewed against owner-set lines; worst pages eyeballed (staging dirs)
   - [ ] known artifacts (e.g. odp text-layer, RTL) carried over, not silently "fixed" by re-approval
   - [ ] approvals written via `golden.py approve --reviewer NAME` (human gate; never inside a pipeline)
8. **Pin bump commit** (single commit): `docker/harness/Dockerfile` ARGs + `docker/harness/pin.txt`
   (`image_tag`, `base_digest`, `lo_version`, `lo_build`, `poppler_version`, `fontset_hash`).
   Nightly (HAR-07) reads `pin.txt` as the single source of truth and starts gating the new values
   immediately. PR description links the upgrade report.
9. **D-log note if the LO N-1 support window shifted** (D15): when the bump was forced by TDF
   retiring the pinned branch, or when it changes which versions the product claims to support
   (OPS-03 matrix), the **owner** files an entry in `project_concept.md` §16 (subagents never edit it,
   phase0 rule).

## 5. Rollback rule

Rollback is **free by construction**: old image tags are immutable and stay pull-able, goldens are
keyed per pin (INV-4 engine-key dirs are never overwritten, approvals are append-only), so reverting
= `git revert` of the pin-bump commit (pin.txt + Dockerfile in one commit by §4.8). No golden
migration, no re-harvest, no data loss. The old pin remains usable as long as its tag is pull-able;
keep old tags forever for diff/rollback, garbage-collect object storage only outside the N/N-1 window.

## 6. Owners + SLA sketch (to be confirmed by the owner)

| Role | Who | SLA (sketch) |
|---|---|---|
| Watch | weekly CI job (or E4 on-call until it exists) | detect within 1 week of TDF publish |
| Gate run + diff report | HAR owner (E4 engines/ops, plan §6.2) | report drafted ≤ 1 week after trigger |
| Review + approvals | project owner + 1 reviewer | verdict ≤ 1 week after report |
| Security fast-track (CVE) | HAR owner, owner for approval | 48 h triage → report; expedited review; pin LO patch release within 1 week |
| D-log entry (window shift) | project owner only | same commit series as the pin bump |

## 7. Enforcement point — HAR-07 nightly

`.github/workflows/nightly.yml` is what makes this procedure more than prose:
it builds the image, then hard-gates `soffice --version == pin.lo_build` and
`fontset_hash == pin.fontset_hash` **against `docker/harness/pin.txt`**. Consequences:

- drift at an unchanged pin ⇒ nightly red ⇒ bug per §1 (the workflow error string says exactly that);
- the upgrade gate PR is the only change touching `docker/harness/**` + `pin.txt` (PR builds otherwise
  reuse the published image);
- after a pin bump the nightly sample (25 ids, stability ×2) starts exercising the new engine-key;
  the full-corpus re-approval is a one-time §4.4–4.7 event, the nightly keeps the new pin honest.

## Findings log

- Measured 2026-09-05 (webfetch): `download.documentfoundation.org/libreoffice/stable/` lists
  25.8.7 / 26.2.5 / **26.2.6** (2026-09-03) / **26.8.0** (2026-08-26); release-notes page labels two
  supported branches (26.8.0 Latest, 26.2.6 Previous). Gate is live-worthy the day it lands.
- Measured (repo, SA-3/5/6/8): engine-key scheme, store layout, nightly pin checks,
  `fontset_hash=sha256:6f26ae4f…` reproducible across runs — `docker/harness/README.md`,
  `harness/render/README.md`, `.github/workflows/nightly.yml`.
- Unverified: LO date discrepancy above; future TDF release-rhythm changes. Poppler: no independent
  watch yet — rides the base-digest decision (§2).
