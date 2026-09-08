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

**Re-measured 2026-09-08 (curl, same method):** `stichos.org` RDAP **404 = free again** (the 2026-07-06 registration lapsed; levships use-history still for counsel — clearance-memo §3). `stichos.dev` still **404 free**. `api.github.com/users/stichos` now **200** — `Stichos`, personal account created 2018 (renamed into the name; GitHub handles are case-insensitive): the org/handle path is **closed**; `reinartz/stichos` stands (D18).

## 2. Same-day action checklist (human, in order)

1. ✅ **Done 2026-09-07** — private repo created under the personal account `reinartz` (owner decision — no GitHub org; concept D18): `github.com/reinartz/stichos`, first push `b4948f9`. **Note (re-checked 2026-09-08):** the handle `Stichos` is now **taken** (dormant personal account) — the org path is closed; D18 stands permanently.
2. **Publish defensive crate `stichos` 0.0.1 to crates.io.** Squat-proof even if empty. Placeholder README: *"This crate name is reserved by the stichos project. No functionality is published yet."* Set `publish = false` on real workspace crates later to avoid accidents. Commands:
   ```bash
   cargo new /tmp/stichos-reserve --name stichos && cd /tmp/stichos-reserve
   # Cargo.toml: description = "Name reserved by the stichos project - no functionality published yet."
   # README.md: the reserved-name text above
   cargo login <API token from https://crates.io/settings/tokens>   # crates.io login via GitHub is one click
   cargo publish        # 0.0.1; yank later only if ever needed
   ```
3. **Publish `stichos` 0.0.1 placeholder to npm** (`npm publish` of a stub with `"private": false`, description "Name reserved — stichos"). npm has no reservation mechanism; a published version is the only squat-proof. Commands:
   ```bash
   mkdir -p /tmp/npm-stichos && cd /tmp/npm-stichos
   printf '{"name":"stichos","version":"0.0.1","description":"Name reserved - stichos. No functionality published yet.","main":"index.js"}\n' > package.json
   echo 'module.exports = {};' > index.js
   npm adduser        # email-verified account; 2FA/OTP required at publish
   npm publish --access public
   ```
4. **Upload `stichos` 0.0.1 placeholder to PyPI** with the same reserved-name README. Commands:
   ```bash
   mkdir -p /tmp/pypi-stichos && cd /tmp/pypi-stichos
   printf '[project]\nname = "stichos"\nversion = "0.0.1"\ndescription = "Name reserved - stichos. No functionality published yet."\nreadme = "README.md"\nrequires-python = ">=3.9"\n\n[build-system]\nrequires = ["setuptools"]\nbuild-backend = "setuptools.build_meta"\n' > pyproject.toml
   echo "Name reserved by the stichos project. No functionality is published yet." > README.md
   python3 -m pip install --user build twine && python3 -m build
   twine upload dist/*    # API token from https://pypi.org/manage/account/token/ ; PyPI mandates 2FA
   ```
5. **Register `stichos.dev`** (confirmed unregistered today; RDAP 404). Recommended primary domain.
6. ✅ **RESOLVED 2026-09-08 (OQ5 domain part, D20):** `stichos.com` — **skip**; no homepage planned; web identity is `stichos.dev` only. No offer, no purchase, no monitoring.
7. **MCP registry stub:** open a tracking issue in `modelcontextprotocol/registry` (or prepare the `server.schema.json` entry PR) for `io.github.reinartz/stichos` (namespace follows the owning GitHub account — D18); registry shows 0 collisions today.
8. **Flag to counsel (scope updated 2026-09-08):** `stichos.org` RDAP now **404 — domain free again** (registration lapsed), but the levships "stichos" product **use history** (adopted ≤2026-07, possibly abandoned) still needs analysis — do not file trademarks until counsel weighs it (see `clearance-memo.md` §3).

**Note:** publish the defensive crates/npm/PyPI placeholders **even if empty** — squat-proofing is the only reason; every registry above is free **as of 2026-09-03** and free names in these registries get squatted quickly.
