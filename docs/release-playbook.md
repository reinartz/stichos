# Release Playbook

This playbook documents the stichos release process: cutting a release, verifying
artifacts, publishing, updating package managers, and rolling back.

---

## 1. How to Cut a Release

### Step 1: Pre-flight checks

```bash
# Ensure clean tree
git status
# Should be clean (untracked files OK, no modifications)

# Run full test suite
cargo test --workspace

# Run clippy
cargo clippy --workspace --all-targets -- -D warnings

# Run fmt check
cargo fmt --all --check
```

### Step 2: Bump version

```bash
# Dry-run preview (does NOT modify files)
./tools/version_bump.sh 0.2.0
# Review output, then re-run for real

# Apply the bump
./tools/version_bump.sh 0.2.0
```

This updates `Cargo.toml` workspace version, all crate versions (via workspace inheritance),
and appends a changelog entry to `CHANGELOG.md`.

### Step 3: Generate changelog

```bash
./tools/changelog_gen.sh > /tmp/changelog-preview.md
# Review output, merge into CHANGELOG.md if needed
```

### Step 4: Commit and tag

```bash
# Commit the version bump
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "bump version to 0.2.0"
git push origin main

# Create an annotated tag
git tag -a v0.2.0 -m "Release v0.2.0"
git push origin v0.2.0
```

### Step 5: CI builds ×6 targets

Pushing the `v*` tag triggers the Release workflow on GitHub Actions. It:

1. Builds for **6 targets** (matrix in `.github/workflows/release.yml`; matches
   `rust-toolchain.toml` — no `aarch64-pc-windows-msvc` target):
   - `x86_64-unknown-linux-gnu`
   - `aarch64-unknown-linux-gnu`
   - `x86_64-unknown-linux-musl`
   - `aarch64-apple-darwin`
   - `x86_64-apple-darwin`
   - `x86_64-pc-windows-msvc`
2. Creates a **GitHub Release draft** with all artifacts
3. Generates **Winget manifest** and **Homebrew formula** as downloadable artifacts

Watch the workflow at: `Actions` tab → `Release` workflow.

### Step 6: Verify the workflow completed

Check the workflow run finished successfully. The draft release will be at:
`https://github.com/reinartz/stichos/releases`

---

## 2. How to Verify the Release

### Check artifacts

On the GitHub Release draft page, verify all 6 artifacts are present:

| Artifact | Target | Format |
|----------|--------|--------|
| `stichos-x86_64-unknown-linux-gnu.tar.gz` | Linux x86_64 | tar.gz |
| `stichos-aarch64-unknown-linux-gnu.tar.gz` | Linux ARM64 | tar.gz |
| `stichos-x86_64-unknown-linux-musl.tar.gz` | Linux x86_64 (static) | tar.gz |
| `stichos-aarch64-apple-darwin.tar.gz` | macOS ARM64 | tar.gz |
| `stichos-x86_64-apple-darwin.tar.gz` | macOS x86_64 | tar.gz |
| `stichos-x86_64-pc-windows-msvc.zip` | Windows x86_64 | zip |

### Smoke tests

```bash
# Download and extract for your platform
curl -L -o stichos.tar.gz <ARTIFACT_URL>
tar xzf stichos.tar.gz

# Version check
./stichos --version
# Should output: stichos <version>

# Basic CLI test
./stichos --help

# Doctor probe (checks available tools)
./stichos doctor --json
```

### Checksum verification

```bash
# Download the .sha256 file alongside each artifact
sha256sum -c stichos-0.2.0-x86_64-unknown-linux-gnu.tar.gz.sha256
```

---

## 3. How to Publish

The Release workflow creates a **draft** release. The owner reviews before publishing.

### To publish:

1. Go to `https://github.com/reinartz/stichos/releases`
2. Find the draft release (e.g., "v0.2.0")
3. Click "Edit" (pencil icon)
4. Review the release notes
5. **Uncheck** "Set as a draft"
6. Click "Publish release"

This makes the release and all artifacts publicly visible.

### To un-publish (mark back as draft):

1. Click "Edit" on the published release
2. **Check** "Set as a draft"
3. Click "Update release"

---

## 4. How to Update Winget

### Winget submission

After the release is published:

1. Fork [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) if you haven't already.
2. Create the manifest set in the fork at:

```
Path:  manifests/o/OleReinartz/stichos/0.2.0/
Files: version.yaml, installer.yaml, locale.en-US.yaml
```

3. Push a branch in the fork and open a PR to `microsoft/winget-pkgs`.
4. Use this PR template:

```markdown
## New package: OleReinartz.stichos version 0.2.0

- PackageIdentifier: OleReinartz.stichos
- PackageVersion: 0.2.0
- InstallerType: portable (zip)
- Validates: SHA256 check passes

Release: https://github.com/reinartz/stichos/releases/tag/v0.2.0
```

5. The Winget team will validate and merge the PR automatically. The wingetbot runs checks 01–10 (manifest validation, URL/domain validation, catalog content, installer scan). First-time package submissions typically require a maintainer merge after the installer pipeline completes.

### Manual installation (pre-Winget merge)

Users can install via the fork's PR:

```bash
winget install --id OleReinartz.stichos --source winget
```

---

## 5. How to Update Homebrew

After the release is published:

1. Push the formula to the personal tap `reinartz/homebrew-formulae`:

```
Path:  Formula/sticho.rb
```

```bash
# (from stichos repo)
cp dist/homebrew/sticho.rb .  # if not already staged
git add Formula/sticho.rb
git commit -m "chore: release sticho $(grep -m1 'version' Formula/sticho.rb | cut -d\" -f2)"
git push origin main
```

2. Homebrew users install via the tap:
```bash
brew tap reinartz/formulae
brew install reinartz/formulae/sticho
```

3. To submit to Homebrew/homebrew-core for `brew install sticho` (unbrewed tap), file a PR to `Homebrew/homebrew-core` — note that homebrew-core generally accepts source-built formulas only, not prebuilt binaries. The personal tap is the canonical distribution route for binary-only releases.

---

## 6. Rollback Procedure

If a release has issues after publishing:

### Option A: Revert and re-release

1. **Do NOT delete the tag** (preserves audit trail)
2. Fix the issue on `main`
3. Bump to a new version (e.g., `0.2.1`)
4. Tag and release as normal

```bash
# Mark the bad release as draft (hide from users)
# 1. Go to https://github.com/reinartz/stichos/releases/tag/v0.2.0
# 2. Edit → Check "Set as a draft" → Update

# Create a fix and re-release
git checkout main
# ... fix the issue ...
./tools/version_bump.sh 0.2.1
git commit -m "fix: <description>"
git push origin main
git tag -a v0.2.1 -m "Release v0.2.1 (fixes v0.2.0)"
git push origin v0.2.1
```

### Option B: Delete and re-release (only if release has zero downloads)

1. Delete the GitHub Release
2. Delete the git tag:

```bash
git push origin --delete v0.2.0
git tag -d v0.2.0
```

3. Fix, re-bump to `0.2.0` (or `0.2.1`), and re-release

**Warning:** Option B leaves no trace. Prefer Option A for any release that may have been used.

### Option C: Winget/Homebrew rollback

Package managers handle version ordering automatically. Publishing a new version
(`0.2.1`) will supersede `0.2.0` in user's package lists.

For Winget specifically, if you need to force-update users:
- Submit a new manifest with the same version but a fixed installer URL
- Winget will re-download and re-verify

---

## 7. Emergency Contact

| Role | Responsibility |
|------|---------------|
| Release owner | Approves/publishes releases |
| Maintainer | Reviews PRs, handles Winget/Homebrew |
| Security | Handles CVEs, security advisories |

---

## Appendix: Version Bump Script

```bash
# Full usage
./tools/version_bump.sh <new-version>

# Examples
./tools/version_bump.sh 0.2.0          # minor bump
./tools/version_bump.sh 0.2.1          # patch bump
./tools/version_bump.sh 1.0.0-rc.1     # pre-release
```

The script:
- Reads version from `Cargo.toml [workspace.package] version`
- Updates ALL crate Cargo.toml files (via workspace inheritance)
- Appends changelog entry to `CHANGELOG.md`
- Prints diff summary (does NOT commit)

## Appendix: Changelog Generator

```bash
# Since last tag
./tools/changelog_gen.sh

# Since specific date
./tools/changelog_gen.sh --since "2026-01-01"

# Full history (if no tags)
./tools/changelog_gen.sh
```

Groups commits by: CLI, gp-core, adapters-ooxml, adapters-odf, adapters-pdf,
engines, lint, mcp, docs, tools, CI, other.
