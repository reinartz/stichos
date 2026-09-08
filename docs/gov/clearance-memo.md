# stichos — name clearance memo (desk research, GOV-01 research support) — 2026-09-03

**Scope:** Desk research supporting counsel on the name `stichos`. **This is not a legal opinion** and not a clearance search. Verification levels: **measured** (command run, status recorded), **researched** (web page consulted), **unverified** (could not check; method stated).

## 1. Trademark office queries attempted

| Office | Endpoint tried | Result | Level |
|---|---|---|---|
| EUIPO eSearch | `curl https://euipo.europa.eu/eSearch/` | 200; JS single-page app (hash-routed UI, no server-side query params) | measured block → unverified data |
| WIPO Global Brand DB | `curl https://branddb.wipo.int/` | 200; JS SPA, no queryable no-JS endpoint | measured block → unverified data |
| USPTO (TESS successor) | `curl https://tmsearch.uspto.gov/search/search-information` → 200 (Angular SPA); `POST https://tmsearch.uspto.gov/api-v1-0-0/tmsearch` → **405** (S3 MethodNotAllowed) | not scriptable without browser/API key | measured block → unverified data |
| TMview (EUIPO aggregate incl. US) | `GET`+`POST https://www.tmdn.org/tmview/api/search/results?...` → **connection reset by peer** (non-browser TLS blocked) | — | measured block → unverified data |

**Manual steps for counsel (browser, none executed by this tool):**
1. EUIPO eSearch — "Trade marks", basic search `stichos`, then broad `galley proof`; filter Nice classes **9** (software) and **42** (SaaS/software services); check status (filed/registered/opposition).
2. WIPO Global Brand Database — same terms, all offices, classes **9, 42**; also phonetic/similar search `galley*proof`.
3. USPTO tmsearch — Exact dictionary search `stichos` and wordmark contains `galley proof`, International Classes **009, 042**; check live/dead + owner.
4. TMview `https://www.tmdn.org/tmview/` — one cross-office sweep, classes 9+42.

## 2. "galley proof" as a descriptive printing term

- `galley proof` (and closed form `stichos`) is the standard printing-industry term for a proof pulled from a galley before final publication. Merriam-Webster lists **stichos** as a dictionary entry word — **researched** via DDG snippet (`www.merriam-webster.com/dictionary/stichos`); direct fetch returned **403** Cloudflare challenge (measured block).
- Relevance: for publishing/proofing services the term trends **descriptive**; for document-rendering software it is more plausibly **suggestive** (metaphorical extension). A registered-mark examiner may cite the industry meaning for related services.

## 3. Existing commercial use of the name (web + GitHub)

| Source | What | Status | Collision relevance |
|---|---|---|---|
| `https://stichos.org/` | **"stichos — Production Proofing & Reference Gate for Scholarly Journals"** — converts accepted Word manuscripts into HTML, JATS XML, journal-styled PDF. RDAP (measured, 200): domain registered **2026-07-06**, Cloudflare registrar, expires 2027-07-06 | **measured (200)** | **HIGH — same conceptual space:** document rendering + proofing software |
| `https://stichos.levships.com/` | "stichos — Proof every expert book before it ships." Authoring SaaS, paid plans $249/$499/$79mo, © 2026 "stichos" (appears to be a Levships product/flagship demo) | measured (200) | MEDIUM-HIGH — commercial software using the name |
| `https://ppchem.com/category/stichos/` | "stichos" column category, PPCHEM AG (chemical process engineering reviews) | measured (200) | LOW — unrelated goods/services |
| GitHub repo search `https://api.github.com/search/repositories?q=stichos` | **total_count: 0** | measured (200) | none — no repo named `stichos` |
| `github.com/clauseggers/BX90000FontPresenterSuperSuite` file `src/stichos/stichos.ts` | code-level use of the term inside a font-presenter project | **unverified** (DDG snippet only; `api.github.com/search/code` → 401 without auth) | LOW-MED — context suggests descriptive use |

## 4. Conclusion

**Distinctiveness assessment.** `stichos` is a closed compound of a descriptive printing term. For **software that renders/proofs office documents (classes 9, 42)** it reads as **suggestive** (metaphor from print production), but distinctiveness is **thin**: the term is a dictionary headword, the trade usage "galley proof" is common, and at least one third party (stichos.org) already applies it to manuscript-proofing software. Expect weaker protection than an arbitrary mark; classic-descriptive refusals possible only for proofing-services claims.

**Risk flags.**
1. **stichos.org (live product, registered 2026-07-06)** — direct concept-space collision (journal production proofing/rendering). Prior-in-time user in an adjacent field; not visible in registry-only checks done 2026-08-30. *[Update 2026-09-08, measured RDAP 404: the registration has lapsed — domain free again. The levships product's use history (adoption date, whether truly abandoned) still requires counsel analysis before any TM filing.]*
2. levships "stichos" SaaS — commercial software use of the name, possibly a demo; nature of the offering needs investigation.
3. `stichos.com` registered since 2016, parked at NameBright (see registration-checklist) — domain-side squat risk, not necessarily a mark.

**A real clearance search must cover:** classes **9** and **42** (also consider 41 for publishing services) in EUIPO, USPTO, WIPO MM designations, UKIPO, and national DE registers; TMview phonetic/similar sweeps; US common-law and domain/company-registry sweep (state SOS, OpenCorporates); investigation of the stichos.org operator (adoption date, territory, goods overlap); GitHub/npm/crates.io as evidence of prior software use. **No filing until stichos.org is analyzed.**

> **Outcome 2026-09-08:** the project owner decided **not to file any trademark** — this is an open-source
> project; squat-proofing is done through registries + the GitHub repo (GOV-02), not registration
> (`project_concept.md` D21). The counsel-gate in this memo is retired; the analysis above stands as
> reference should D21's revisit trigger ever fire (commercialization or a confusingly-identical
> commercial competitor).
