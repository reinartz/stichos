# stichos — identity registration checklist (GOV-02 prep) — 2026-09-03

**Scope:** Same-day re-verification of name availability + the action checklist the human executes. All checks below run 2026-09-03 via `curl` (measured, HTTP status recorded). Baseline: concept doc registries check of 2026-08-30.

## 1. Availability re-verified today (measured)

| Channel | Endpoint | HTTP | Verdict |
|---|---|---|---|
| GitHub org/user | `api.github.com/users/stichos` | **404** | **free** |
| crates.io | `crates.io/api/v1/crates/stichos` | **404** (`crate \`stichos\` does not exist`) | **free** |
| npm | `registry.npmjs.org/stichos` | **404** (`Not found`) | **free** |
| PyPI | `pypi.org/pypi/stichos/json` | **404** | **free** |
| Domain `stichos.dev` | `rdap.org/domain/stichos.dev` | **404** (`stichos.dev not found`) | **unregistered** |
| Domain `stichos.com` | `rdap.org/domain/stichos.com` | **200** | **TAKEN** — registered 2016-08-22, expires **2027-08-22**, registrar TurnCommerce, Inc. dba NameBright, NS `nsg1/nsg2.namebrightdns.com` (parked/for-sale pattern) |
| Domain `stichos.org` | `rdap.org/domain/stichos.org` | **200** | **TAKEN (new finding)** — registered **2026-07-06**, Cloudflare, live product "stichos" journal-proofing SaaS (see clearance-memo §3) |
| MCP registry | `registry.modelcontextprotocol.io/v0/servers?search=stichos` | 200, **0 servers** | **free** (note: `q=` param is ignored; `search=` is the filter) |
| MCP GitHub orgs | `api.github.com/search/repositories?q=owner:modelcontextprotocol+stichos` | 200, total_count 0 | free |
| GitHub repo name | `api.github.com/search/repositories?q=stichos` | 200, total_count 0 | free |

**Delta vs 2026-08-30:** all registry slots still free (no change). `.com` still taken at NameBright (no change; expiry 2027-08-22). **New risk:** `.org` was registered 2026-07-06 and now hosts a same-space product — naming strategy (OQ5) must account for it.

## 2. Same-day action checklist (human, in order)

1. **Create private repo `stichos` under the personal account `reinartz`** (owner decision 2026-09-07 — no GitHub org; concept D18). CI push target = `github.com/reinartz/stichos`; verify the repo exists and CI accepts a push. **Note:** GitHub users/orgs share one namespace, so the handle `stichos` itself stays **unclaimed** (404 measured 2026-09-03) — the personal-account route cannot lock it; re-check before any public push and open the org then if still free (or disputed).
2. **Publish defensive crate `stichos` 0.0.1 to crates.io.** Squat-proof even if empty. Placeholder README: *"This crate name is reserved by the stichos project. No functionality is published yet."* Set `publish = false` on real workspace crates later to avoid accidents.
3. **Publish `stichos` 0.0.1 placeholder to npm** (`npm publish` of a stub with `"private": false`, description "Name reserved — stichos"). npm has no reservation mechanism; a published version is the only squat-proof.
4. **Upload `stichos` 0.0.1 placeholder to PyPI** with the same reserved-name README.
5. **Register `stichos.dev`** (confirmed unregistered today; RDAP 404). Recommended primary domain.
6. **Decide on `stichos.com` (OQ5):** parked at NameBright since 2016, expires 2027-08-22. Options: make an offer via NameBright, skip, or monitor. Record the decision in project_concept.md §16 — do not buy without a budget call.
7. **MCP registry stub:** open a tracking issue in `modelcontextprotocol/registry` (or prepare the `server.schema.json` entry PR) for `io.github.reinartz/stichos` (namespace follows the owning GitHub account — D18); registry shows 0 collisions today.
8. **Flag to counsel immediately:** `stichos.org` + levships "stichos" (see `clearance-memo.md` §3) — do not file trademarks until analyzed.

**Note:** publish the defensive crates/npm/PyPI placeholders **even if empty** — squat-proofing is the only reason; every registry above is free **as of 2026-09-03** and free names in these registries get squatted quickly.
