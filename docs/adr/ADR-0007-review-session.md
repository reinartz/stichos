# ADR-0007: Review Session Model

**Status**: proposed
**Date**: 2026-09-17
**Decisions**: D22 (review-overlay medium) is consumed here.

## Context

Phase 4 (M4) ships the "human half" of the review loop. The agent writes tracked revisions
(D7); a human must see proposals, decide, and have the decision applied losslessly.
A review session provides the scoped context for a single human reviewer to examine,
accept/reject, and comment on revisions in a document version.

## Decision

A review session is a server-side construct that:
1. Is scoped to a single document version (identified by content-addressed hash of the document bytes).
2. Has a cryptographically random token (≥128 bits entropy) for authentication on every route.
3. Stores a snapshot of the revision list at session creation time (so the human sees what was
   there when the session started).
4. Maintains an audit trail of all actions (accept, reject, comment) performed within the session.
5. Is invalidated (HTTP 410 Gone) if the document version hash changes mid-session (optimistic
   concurrency — the document was edited while the human was reviewing).
6. Is a local-only server (binds to 127.0.0.1 by default).

### Token generation

Tokens are generated as `stichos-review-<64 hex chars>` (128 bits of CSPRNG entropy),
using `rand::thread_rng()` for hex generation. This provides strong collision resistance
while remaining URL-safe and copyable.

### Session lifecycle

1. `create` — `stichos review <path>` creates a session. Refuses if the document has uncommitted
   edits (dirty doc check). Prints URL + token. Optionally opens browser (`--no-browser` flag).
2. `get` — `GET /review/<token>/data.json` returns the review data (revisions + bboxes + taint).
3. `list` — `GET /review/<token>/` (viewer HTML) shows the revision overlay.
4. `close` — `stichos review close <token>` terminates the session.
5. `invalidate` — if the document version hash changes, all active sessions are invalidated.

### Security model

- **localhost bind default**: Server binds to `127.0.0.1` only (not 0.0.0.0).
- **Token required on every route**: Every GET/POST to review routes requires `?token=<token>`.
- **Version-hash invalidation**: If the document changes, sessions return 410 Gone.
- **No ambient start**: Sessions are explicitly created and must be explicitly closed.
- **CSP header**: Strict Content-Security-Policy on all review routes (no eval, no external resources).
- **HTML-escaped document text**: All document text in the viewer is HTML-escaped before rendering.
- **Path traversal protection**: All file paths are validated to prevent directory traversal.
- **MIME type enforcement**: Content-Type headers correctly match the response body.

### Data model

```rust
pub struct ReviewSession {
    pub token: String,
    pub doc_path: String,
    pub doc_version_hash: String, // SHA-256 of document bytes at session creation
    pub created_at: SystemTime,
    pub expires_at: Option<SystemTime>,
    pub revision_snapshot: Vec<serde_json::Value>, // snapshot of revisions at creation
    pub audit_trail: Vec<AuditEntry>,
    pub comment_drafts: Vec<serde_json::Value>,
}

pub struct AuditEntry {
    pub timestamp: SystemTime,
    pub action: String, // "create", "accept", "reject", "comment", "close"
    pub details: serde_json::Value,
}
```

### Review data schema

`GET /review/<token>/data.json` returns:
```json
{
  "session": { "token": "...", "doc_path": "...", "created_at": "..." },
  "document_version": "<sha256 hex>",
  "revisions": [
    {
      "id": "id-abc123",
      "kind": "insert",
      "author": "agent",
      "date": "2026-01-01T00:00:00Z",
      "anchor_element_id": "id-def456",
      "page_hint": 1,
      "content": "inserted text",
      "bbox": [100, 200, 300, 220],
      "status": "pending", // pending | accepted | rejected
      "taint": false
    }
  ],
  "comments": [],
  "pages": [
    {"number": 1, "has_pages": false}
  ]
}
```

## Consequences

- The review server is a single-request-per-connection HTTP server (no persistent connections needed).
- Session state is in-memory only (no persistence — sessions are short-lived).
- The server runs until explicitly closed or the session expires.
- The review verb starts a temporary server, prints the URL, and blocks.
- A separate `stichos review close` command can be used to terminate early.
