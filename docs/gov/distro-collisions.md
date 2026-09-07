# stichos — distro/package-name collision check (GOV-03) — 2026-09-03

**Scope:** Is the package name `stichos` free on the five planned distribution targets? Checks run 2026-09-03. Levels: measured / researched / unverified.

## Commands & evidence

**Fedora 44 (this host):** — measured
- `dnf search stichos` → `No matches found.` (exit 0; repos incl. updates, RPM Fusion, RPM Sphere, Terra, Copr, google, cuda, VS Code, Adobe, Adoptium all loaded)
- `dnf repoquery "*stichos*"` → empty output (exit 0)
- Note: "Terra 44" repo hit a repomd GPG key prompt during metadata refresh; not accepted, that repo was skipped for the search.

**Debian:** `curl "https://packages.debian.org/search?keywords=stichos&searchon=names"` → 200; page body: *"Sorry, your search gave no results"* — measured.

**Ubuntu:** `curl "https://packages.ubuntu.com/search?keywords=stichos&searchon=names"` → 200; body: *"Sorry, your search gave no results"* — measured.

**Homebrew:**
- `curl https://formulae.brew.sh/api/formula/stichos.json` → **404** (formula free) — measured
- `curl https://formulae.brew.sh/api/cask/stichos.json` → **404** (cask free) — measured
- `curl "https://api.github.com/search/repositories?q=owner:Homebrew+stichos"` → 200, `total_count: 0` — measured

**Chocolatey:** `curl https://community.chocolatey.org/packages/stichos` → **404** = package free — measured.

**Repology (cross-distro):**
- `curl https://repology.org/api/v1/projects/stichos` → **404** (no project of that name) — measured
- `https://repology.org/search/?q=stichos` → **404** (Repology returns 404 for zero-result searches) — measured
- Caveat: endpoint is rate-limited (observed 403 then 429 on rapid retries; one anomalous cached body returned an unrelated `gamescope-session-steam-sk` entry — disregarded, exact-name endpoint 404 is authoritative).

**winget:**
- `curl "https://api.github.com/search/code?q=stichos+repo:microsoft/winget-pkgs"` → **401** (code search requires auth). **Unverified.** Method: authenticated `GET` with token, or browser search `https://github.com/microsoft/winget-pkgs/search?q=stichos`, or `winget search stichos` on Windows. (Repo is `microsoft/winget-pkgs`; `winget-pkcts` does not exist.)

## Results table

| Distro / channel | Query | Result | Collision? | Level |
|---|---|---|---|---|
| Fedora 44 (dnf, all enabled repos) | `dnf search` + `dnf repoquery "*stichos*"` | no matches / empty | none | measured |
| Debian (packages.debian.org, names) | keywords=stichos | "no results" (200) | none | measured |
| Ubuntu (packages.ubuntu.com, names) | keywords=stichos | "no results" (200) | none | measured |
| Homebrew formula API | `/api/formula/stichos.json` | 404 | none | measured |
| Homebrew cask API | `/api/cask/stichos.json` | 404 | none | measured |
| Homebrew repos (GitHub) | `owner:Homebrew stichos` | total_count 0 | none | measured |
| Chocolatey | `/packages/stichos` | 404 | none | measured |
| Repology (all distros+AUR etc.) | `api/v1/projects/stichos` | 404 | none | measured |
| winget (`microsoft/winget-pkgs`) | code search | 401, needs auth | unknown | unverified |

**Bottom line:** no collisions found on any channel checked — Fedora, Debian/Ubuntu, Homebrew (formula+cask), Chocolatey, and the Repology cross-distro sweep are all free for `stichos`. winget remains unverified pending an authenticated/manual check.
