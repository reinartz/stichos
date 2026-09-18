# P4-17: Office Add-in Spike Report

**Track:** HUM-03 · **Phase:** 4, Wave H · **Size:** 1 session
**Status:** proposed · **Date:** 2026-09-18

---

## 1. Overview

This spike evaluates building an Office (Word / Excel / PowerPoint) add-in (taskpane) that connects to the stichos review HTTP API for human-in-the-loop revision review. The add-in would render page images with revision boxes directly inside the Office app, replacing the current browser-based review workflow.

**Recommendation: Defer implementation.** The browser-based review (P4-11/P4-12/P4-13) is already shipped, tested, and meets M4 requirements. An Office add-in introduces manifest complexity, cross-platform compatibility concerns, and distribution overhead for a marginal UX improvement over the current workflow.

---

## 2. Current Review Flow (P4-11 / P4-12 / P4-13)

The current review mechanism works as follows:

1. `stichos review <document>` starts an HTTP server bound to `127.0.0.1` on a random port.
2. A review session is created, yielding a **token** (stored in the session store).
3. The browser opens `http://127.0.0.1:<port>/review/<token>/`.
4. The reviewer sees a static HTML page (`assets/reviewer/index.html`) — a single-page app with ~1k LOC of hand-rolled JavaScript, no build step, no external dependencies.
5. The page polls `GET /review/<token>/data.json` every 3 seconds for revision state changes.
6. Accept/reject actions are POSTed to `POST /review/<token>/action` with `{action, revision_id}`.
7. Page images are served via `GET /pages/<N>.png` from the engine cache (currently returns 501).
8. Comments are managed client-side via `add_comment_draft` flow (stored in session).
9. The server enforces **CSP headers**, **X-Frame-Options: DENY**, token-based auth on every route, and path traversal prevention.
10. A **version-hash check** invalidates the session (HTTP 410) if the document changes on disk since the session was created.

The review viewer is a dark-themed single-page app with:
- Sidebar: revision list with status dots and detail panel
- Main area: page images with overlay bounding boxes for each revision
- Keyboard navigation: arrow keys, Enter (accept), Shift+Enter (reject), C (comment), Esc (deselect)

---

## 3. Add-in Architecture Proposal

### 3.1 Manifest Structure

A Word add-in manifest (`manifest.xml`) would contain:

```xml
<OfficeApp>
  <Id>stichos-review-guid</Id>
  <Version>1.0.0</Version>
  <ProviderName>Stichos</ProviderName>
  <DefaultLocale>en-US</DefaultLocale>
  <DisplayName DefaultValue="Stichos Review"/>
  <SupportUrl DefaultValue="https://github.com/stichos/stichos"/>
  <AppDomains>
    <AppDomain>localhost</AppDomain>
  </AppDomains>
  <DefaultSettings>
    <SourceLocation DefaultValue="http://localhost:{port}/review/{token}/"/>
  </DefaultSettings>
  <Permissions>ReadWriteDocument</Permissions>
  <Hosts>
    <Host>Document</Host>
  </Hosts>
</OfficeApp>
```

**Key considerations:**
- `Host: Document` covers Word. Excel uses `<Host>Workbook</Host>`; PowerPoint uses `<Host>Presentation</Host>`. Multi-host manifests require separate manifests per host per Office version.
- `SourceLocation` points to the stichos review HTTP server — the taskpane IS the browser-based viewer, served from the local server.
- `AppDomain: localhost` must be declared for the Office runtime to allow iframe navigation.
- No public URL: this is inherently a **side-loaded** add-in (see §5.1).

**Multi-host scope:** The initial spike targets Word only (DOCX is the primary format for tracked changes). Excel and PowerPoint could be added later but require separate manifests and host-specific APIs.

### 3.2 Taskpane Implementation

The taskpane would not require new JavaScript logic. The existing `assets/reviewer/index.html` (single-page app, no build step, ~1k LOC) can be served directly by the stichos HTTP server and displayed inside the taskpane iframe.

**Architecture:**

```
┌──────────────────────────────────────────────┐
│           Office Application                  │
│  ┌────────────────────────────────────────┐  │
│  │        Office Taskpane (iframe)        │  │
│  │  ┌──────────────────────────────────┐  │  │
│  │  │  assets/reviewer/index.html      │  │  │
│  │  │  (existing viewer, served by     │  │  │
│  │  │   stichos HTTP server)           │  │  │
│  │  └──────────────────────────────────┘  │  │
│  └────────────────────────────────────────┘  │
│  ┌────────────────────────────────────────┐  │
│  │  Document Canvas (native)               │  │
│  └────────────────────────────────────────┘  │
└──────────────────────────────────────────────┘
              │
              ▼
┌──────────────────────────────────────────────┐
│  stichos review HTTP server                  │
│  - GET /review/{token}/ → index.html         │
│  - GET /review/{token}/data.json             │
│  - POST /review/{token}/action               │
│  - GET /pages/{N}.png                        │
└──────────────────────────────────────────────┘
```

**Taskpane-specific changes needed:**
- The Office taskpane iframe has a narrower viewport (~360px) compared to a full browser window. The existing viewer CSS would need media-query adjustments for the narrow taskpane width.
- `X-Frame-Options: DENY` (currently set in `http.rs:363`) **must be removed** for the add-in case, as the taskpane itself IS an iframe. This is a significant security architecture question: should the same server serve both `DENY` (browser mode) and `ALLOW` (taskpane mode)? Options include:
  - Separate HTTP server/port for add-in mode
  - A `?taskpane=1` query parameter that switches the frame policy
  - Using `Content-Security-Policy: frame-ancestors 'self'` (modern alternative to X-Frame-Options) which can be varied per-request

- **Office JS API integration (optional, future):** The taskpane could use the Office JS API to read document content or insert comments directly, avoiding the `add_comment_draft` flow. However, this adds a dependency on the Office JavaScript library and increases the add-in scope.

### 3.3 Authentication

The review token is the authentication mechanism for all review routes. Options for delivering the token to the taskpane:

| Option | Mechanism | Pros | Cons |
|--------|-----------|------|------|
| **Query parameter in manifest** | `SourceLocation` includes `{token}` as a URL parameter | Simple, works at launch | Token visible in manifest file; limited to initial load |
| **Local manifest file** | `stichos review` writes a temporary `.xml` manifest with the token | Token stays local, manifest is machine-scoped | Requires file I/O and cleanup |
| **Post-message protocol** | Taskpane opens a WebSocket or polling endpoint on the stichos server to fetch the token | Token never appears in URLs | Adds complexity; requires new endpoint |
| **Current approach (no change)** | User copies token from CLI output and pastes into browser | Simple, explicit | Fragile UX — doesn't translate to add-in well |

**Recommendation:** Use `?token={token}` appended to the `SourceLocation` in the manifest. The taskpane is ephemeral (tied to the `stichos review` process lifetime), so token exposure is limited. The existing JS viewer already reads the token from the URL path (`assets/reviewer/index.html:114-121`).

### 3.4 API Endpoints Used

| Endpoint | Method | Purpose | Status |
|----------|--------|---------|--------|
| `GET /review/{token}/` | GET | Serves `index.html` (the viewer SPA) | Implemented (P4-12) |
| `GET /review/{token}/data.json` | GET | Revision list, bboxes, comments, page count | Implemented (P4-12) |
| `POST /review/{token}/action` | POST | Accept/reject a revision by ID | Implemented (P4-12) |
| `GET /pages/{N}.png` | GET | Page image raster from engine cache | Stub only (returns 501, wired in P4-13) |
| `POST /review/{token}/comment` | POST | Comment write-back | **Not yet implemented** (planned for P4-14) |

**Gaps:**
1. **Page images:** The `/pages/{N}.png` endpoint returns 501. It is planned for P4-13.
2. **Comment write-back:** No dedicated comment endpoint exists. P4-14 proposes using the `add_comment_draft` tool (via MCP `tools/call`) for comment persistence. The add-in would need a path to this — either a new HTTP endpoint or an MCP proxy.

**No new endpoints are required for the add-in itself.** The existing review API is already well-designed for any HTTP-based client.

---

## 4. Cost/Risk Table

| Factor | Assessment | Details |
|--------|-----------|---------|
| **Development effort** | Medium (2–3 weeks) | Office Add-in Toolkit setup (manifest + taskpane scaffolding), CSS adjustments for narrow viewport, testing across LO and Word, integration testing with stichos HTTP server |
| **Maintenance burden** | Low–Medium | Single taskpane, no native code, follows existing patterns (no build step, hand-rolled JS). Office JS updates could require periodic manifest tweaks |
| **Supply-chain risk** | Low | No external JS dependencies. The existing reviewer is already ≤1k LOC. Office JS is loaded from Microsoft CDN (or can be inlined for air-gapped). D22 philosophy applies: minimal, self-contained |
| **Security** | Low–Medium | Localhost only, token-authenticated, CSP enforced. **Caveat:** removing `X-Frame-Options: DENY` for taskpane mode requires careful design to prevent cross-site taskpane injection. A `frame-ancestors` policy scoped to `localhost` URLs is safer than blanket removal |
| **Cross-platform** | Medium | LibreOffice supports add-ins (since v7.0); Word for Microsoft 365 and Word 2021+ support them. Word 2019 and earlier do not. Excel and PowerPoint add-ins have separate manifest requirements per host |
| **User experience** | Medium | In-app review eliminates browser context switch. However, the taskpane is narrower than a browser window, and the existing viewer is designed for wide displays. Keyboard shortcuts may conflict with Office shortcuts (e.g., Ctrl+B, Ctrl+I). The narrow pane also affects the bbox overlay rendering accuracy |
| **Distribution** | **High (problem)** | Office add-ins require publishing to Microsoft AppSource or side-loading. Side-loading requires manual manifest installation per machine. No AppStore equivalent for LO add-ins. This is the single biggest blocker for adoption |
| **COM dependency** | None | Purely manifest + JS + stichos HTTP server. No COM, no native code, no registry changes |
| **Context switching for dev** | Medium | Office Add-in debugging requires browser DevTools (F12) within the Office host, LO-specific tooling, and manifest validation. Not all environments have Office installed |

---

## 5. Alternatives Considered

### 5.1 Side-loading (current approach for add-in testing)

Users install the add-in via "My Add-ins > Manage Add-ins > Add a custom add-in" with the localhost manifest URL.

**Pros:** No AppSource review, fast iteration during development, token is ephemeral (tied to review session)
**Cons:** Must be re-added on each machine, not user-friendly for distribution, Office 365 tenant policies may restrict side-loading

### 5.2 AppSource Publishing

Publish to Microsoft AppSource for discoverability and one-click install.

**Pros:** Discoverable, one-click install for end users, trusted publisher badge
**Cons:** Review process takes weeks, requires Microsoft partner account, ongoing compliance requirements (privacy statements, data handling reviews), AppSource review cycle would slow down iteration significantly. Not viable for an internal/tooling product

### 5.3 Browser Extension + stichos HTTP Server

Build a Chrome/Firefox extension that enhances the stichos review page (e.g., keyboard shortcuts, split-view with the document).

**Pros:** Easier to distribute than an Office add-in (no manifest XML), simpler debugging (Chrome DevTools), wider browser support
**Cons:** Still a browser-based workflow (context switch from Office), doesn't integrate into the Office application, limited by browser extension permissions (cannot access Office document content directly)

### 5.4 Keep Current Browser-Based Review (Status Quo)

Continue with `stichos review` → browser approach as currently implemented.

**Pros:** Zero new code, already shipped and tested, no distribution overhead, no manifest debugging, works on all platforms, CSP/X-Frame-Options already configured, keyboard shortcuts are explicit and documented
**Cons:** Requires opening a browser, reviewer context switch away from the document, no in-app presence

---

## 6. Recommendation

**Recommendation: Defer implementation.**

### Rationale

1. **Current review flow is functional.** The browser-based review (P4-11/P4-12/P4-13) is already shipped, tested, and meets the M4 exit gate requirements (lossless accept/reject, review sessions). It is a mature, well-tested workflow.

2. **Add-in complexity is non-trivial.** Office add-in manifest debugging, LO vs Word compatibility testing, and distribution overhead introduce significant friction. The taskpane viewport is narrow (~360px), which would require CSS work on the existing viewer. The existing keyboard shortcuts (arrow keys, Enter, Shift+Enter) may conflict with Office shortcuts, and the taskpane is a separate focus context.

3. **Low priority for M4.** The M4 gate focuses on lossless accept/reject and review sessions — both fully achieved via the browser approach. An add-in does not improve the technical correctness of the review process.

4. **Better ROI on other features.** The same effort (2–3 weeks) could deliver ODF revision coverage, PDF review capabilities, fuzz testing improvements, or comment write-back — all of which expand the tool's core functionality rather than the review UX surface.

5. **Localhost limitation remains.** Even with an add-in, the stichos HTTP server must be running locally. The core deployment challenge (local-only server) is unchanged. An add-in does not solve remote review, collaborative review, or cloud deployment.

6. **Distribution is the killer constraint.** Without AppSource (which requires months of process) or a tenant admin deployment channel, the add-in is only installable via side-loading, which itself requires manual steps per machine. This negates much of the UX benefit of having the review pane in-app.

### When to Revisit

- **Trigger 1:** User feedback indicates the browser context switch is a significant usability blocker for the primary reviewer persona.
- **Trigger 2:** A dedicated UX designer is available to invest in the taskpane experience (CSS, layout, keyboard shortcut mapping, Office-style UI).
- **Trigger 3:** Office 365 subscription deployment via tenant admin is available as a distribution channel (bypasses AppSource, enables silent install within an organization).
- **Trigger 4:** A companion tool (e.g., a document viewer that can open side-by-side with the add-in) is built, making the in-app review pane more valuable as part of a larger review workspace.

---

## 7. Decision Row for Owner Board

| Decision | Status | Date | Owner |
|----------|--------|------|-------|
| **D23: Office add-in spike — deferred** | proposed | 2026-09-18 | project owner |

---

## Appendix A: Technical References

- **Office Add-in manifest schema:** [Microsoft Docs — Office App manifest schema](https://learn.microsoft.com/en-us/office/dev/add-ins/reference/xml-manifest/)
- **Taskpane API:** Office JS `Office.context.ui` for messaging between taskpane and host document
- **LibreOffice add-in support:** LO 7.0+ supports Office Add-ins via the `extensibility` module
- **Existing review server:** `crates/mcp/src/http.rs` — routes, token auth, CSP headers
- **Existing reviewer UI:** `assets/reviewer/index.html` — ~1k LOC, no build step, hand-rolled JS

## Appendix B: Security Considerations

1. **X-Frame-Options removal:** The current `X-Frame-Options: DENY` header (`http.rs:363`) must be conditionally disabled for taskpane mode. Recommended approach: use `Content-Security-Policy: frame-ancestors 'self'` (modern, flexible) scoped to `localhost` origins only, rather than removing frame protection entirely.

2. **Token exposure in manifest:** If the manifest is stored on disk (side-loading), the token in `SourceLocation` would be readable. Mitigation: the token is ephemeral (tied to session lifetime) and the review session itself is localhost-only.

3. **CSP in taskpane context:** The existing CSP (`default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:`) is already compatible with the taskpane since all resources are served from localhost.

4. **XSS surface:** The existing viewer already uses DOM text-node escaping (`esc()` function at `index.html:184-188`). The add-in does not expand the XSS attack surface.

---

*This is a design-only spike (no implementation). Follows the convention of spike reports in this project. No external dependencies assumed.*
