# C14 — Context Budget & Response Size Management

| | |
|---|---|
| Owner | API (tool surface + MCP transport) |
| Status | frozen — M6 (C7 v1.6.0) |
| Version | 1.0.0 |
| Related | C7 (Tool Schema Contract), C8 (Payload Budget) |

## 1. Problem Statement

MCP tool responses consume LLM context in two ways:

1. **Schema bloat** — Tool definitions (parameters, descriptions, output schemas) loaded into
   the LLM's context window upfront. The GitHub MCP server (94 tools) alone uses ~17,600 tokens.
2. **Response bloat** — Tool outputs flowing back through the context. A single HRIS API call
   can return hundreds of thousands of characters of raw JSON.

For the stichos `office-docs` tools this manifests as:

- **`inspect_document`** — A 50-page DOCX with 10,000+ elements returns massive JSON of element
  IDs, coordinates, and styles that can exceed 50,000 tokens.
- **`render_document`** — Rendering pages at 150 DPI produces base64-encoded images (100KB+ each)
  that dominate the response.
- **`diff_documents`** — Comparing two 100-page documents can produce hundreds of diff operations.
- **`inspect_region`** — High-zoom crops with full coordinate records are expensive.
- **`annotate_pages`** — Returns marked+clean image pairs per page.
- **`apply_patch` / `edit_*`** with `dry_run: true` — Returns full IR diff + provenance +
  fidelity budget.

**Goal**: Default responses must fit within a single LLM reasoning turn (~4,000–8,000 output
tokens). Users who need full detail must opt in explicitly.

## 2. Design Principles

| Principle | Rationale |
|---|---|
| **Default to summary** | The common path (LLM reasoning) gets the small response. Full detail is opt-in. |
| **URL references over inline data** | Images and large binaries go to temp files; responses carry paths/URLs. |
| **Explicit opt-in for cost** | Every parameter that increases response size is explicit, never default. |
| **Backward compatible** | No removal of existing parameters; new optional parameters use sensible defaults. |
| **Budget metadata on every response** | Each response includes `__budget: { tokens_est, elements_count }` so the LLM
  can decide whether to continue or request more. |

## 3. Tool-by-Tool Specification

### 3.1 `inspect_document`

#### Current (v1.0.0)
```
Args: path, format
Returns: { structure, ids, coordinates, fidelity_budget, diagnostics }
```

#### Changes (v1.6.0 additive)

**New parameters:**
```
detail: enum<"summary" | "elements" | "all">, default: "summary"
max_elements: number, default: 100
```

**Return schema (detail="summary" — default):**
```json
{
  "summary": {
    "pageCount": 50,
    "wordCount": 25000,
    "elementCount": 10234,
    "hasImages": true,
    "hasTables": true,
    "hasComments": false,
    "hasTrackedRevisions": false
  },
  "pages": [
    { "page": 1, "elementCount": 45, "hasImages": false },
    { "page": 2, "elementCount": 52, "hasImages": true }
  ],
  "elementIds": ["para_1", "para_2", "para_3", ...],
  "__budget": {
    "tokens_est": 150,
    "elements_count": 100
  },
  "diagnostics": [],
  "rendered_by": "stichos/0.1.0"
}
```

**Return schema (detail="elements"):**
```json
{
  "summary": { ... },
  "elements": [
    {
      "id": "para_1",
      "kind": "paragraph",
      "text": "First paragraph...",
      "page": 1,
      "bbox": [50, 100, 400, 20]
    },
    ...
  ],
  "__budget": {
    "tokens_est": 5000,
    "elements_count": 100
  },
  "hasMore": true,
  "diagnostics": [],
  "rendered_by": "stichos/0.1.0"
}
```

**Return schema (detail="all"):**
```
Same as current v1.0.0 response (full coordinates, styles, fidelity budget per element).
```

**Behavior:**
- `max_elements` caps the number of elements returned (default: 100 for "elements",
  0 = unlimited for "all").
- When `hasMore: true`, the LLM knows it can request a second page via `offset` parameter
  (see §3.7 Pagination pattern).
- `elementIds` is always returned in "summary" mode (compact list of IDs only).

### 3.2 `render_document`

#### Current (v1.0.0)
```
Args: path, dpi (default 150), page_range (optional)
Returns: { images: [block], manifest, diagnostics }
```

#### Changes (v1.6.0 additive)

**New parameters:**
```
output: enum<"base64" | "urls" | "both">, default: "urls"
max_pages: number, default: 3
```

**Return schema (output="urls" — default):**
```json
{
  "pages": [
    {
      "page": 1,
      "width": 1700,
      "height": 2200,
      "url": "/tmp/stichos_render_abc123_page1.png",
      "expiresAt": "2026-09-19T15:00:00Z",
      "fileSize": 245760
    },
    {
      "page": 2,
      "width": 1700,
      "height": 2200,
      "url": "/tmp/stichos_render_abc123_page2.png",
      "expiresAt": "2026-09-19T15:00:00Z",
      "fileSize": 238592
    }
  ],
  "totalPages": 12,
  "renderedPages": 2,
  "dpi": 150,
  "hint": "Use page_range to render additional pages. Set output='base64' for inline images.",
  "__budget": {
    "tokens_est": 0,
    "image_count": 2
  },
  "diagnostics": [],
  "rendered_by": "stichos/0.1.0"
}
```

**Return schema (output="base64"):**
```
Same as current v1.0.0: { images: [base64 blocks], manifest, ... }
```

**Return schema (output="both"):**
```
Both URLs and inline base64 for the same pages.
```

**Behavior:**
- `max_pages` prevents rendering the entire document by accident (default: 3).
- `output="urls"` is the new default — images are written to temp files with expiration.
- Temp files are cleaned up after 1 hour or on server restart.
- The LLM sees paths/URLs, not base64 — no context token cost for images.
- If the client needs inline images, it explicitly sets `output="base64"`.

### 3.3 `inspect_region`

#### Current (v1.0.0)
```
Args: path, page, bbox_pt, zoom (default 2.0)
Returns: { image, coord_record, visual_tokens }
```

#### Changes (v1.6.0 additive)

**New parameters:**
```
output: enum<"base64" | "urls">, default: "urls"
```

**Return schema (output="urls" — default):**
```json
{
  "url": "/tmp/stichos_region_xyz789.png",
  "expiresAt": "2026-09-19T15:00:00Z",
  "width": 850,
  "height": 1100,
  "coord_record": {
    "page": 3,
    "bbox": [100, 200, 300, 150],
    "zoom": 2.0
  },
  "visual_tokens": 1520,
  "__budget": {
    "tokens_est": 0,
    "image_count": 1
  },
  "diagnostics": [],
  "rendered_by": "stichos/0.1.0"
}
```

**Behavior:**
- `output="base64"` is available for clients that need inline images.
- `visual_tokens` is always included (compact token cost for the LLM to reason about image size).

### 3.4 `diff_documents`

#### Current (v1.0.0)
```
Args: path_a, path_b, format, max_ops (optional)
Returns: { ir_diff, xml_diffs, diagnostics }
```

#### Changes (v1.6.0 additive)

**Modified `max_ops` semantics:**
- Current: `"Maximum number of diff operations to return (default unlimited)"`
- New: `max_ops: number, default: 20`

**Return schema (with default max_ops=20):**
```json
{
  "summary": {
    "pagesAdded": 2,
    "pagesRemoved": 1,
    "elementsModified": 45,
    "elementsAdded": 12,
    "elementsDeleted": 8,
    "totalOperations": 523
  },
  "operations": [
    { "op": "replace", "path": "/body/para[3]", "oldText": "foo", "newText": "bar" },
    ...
  ],
  "hasMore": true,
  "totalOperations": 523,
  "__budget": {
    "tokens_est": 1200,
    "operations_returned": 20,
    "operations_total": 523
  },
  "diagnostics": [],
  "rendered_by": "stichos/0.1.0"
}
```

**Return schema (max_ops not set or very large):**
```
All operations returned (current behavior).
```

**Behavior:**
- Default cap of 20 operations prevents context blowup from large diffs.
- `summary` field gives the LLM the high-level picture without reading all operations.
- `hasMore: true` signals that more operations exist.
- `totalOperations` in `summary` and `__budget` helps the LLM decide if it needs more.

### 3.5 `apply_patch` (dry_run)

#### Current (v1.0.0)
```
Args: path, ops[], dry_run, format, session_id, direct, author
Returns: full IR diff + provenance + fidelity budget
```

#### Changes (v1.6.0 additive)

**New behavior for `dry_run: true`:**

**Return schema (dry_run=true — default path for LLM safety):**
```json
{
  "preview": {
    "estimatedTokenChange": "+450",
    "elementsAffected": 3,
    "diffSummary": "Changed 2 paragraphs, added 1 table row",
    "operations": [
      { "op": "replace", "path": "/body/para[3]", "oldText": "...", "newText": "..." },
      { "op": "replace", "path": "/body/para[5]", "oldText": "...", "newText": "..." },
      { "op": "insert", "path": "/body/table[1]/row[2]", "kind": "tr" }
    ]
  },
  "fidelity_budget": {
    "preHash": "sha256:abc...",
    "postHash": "sha256:def...",
    "structuralChange": "minor"
  },
  "__budget": {
    "tokens_est": 300,
    "operations_count": 3
  },
  "diagnostics": [],
  "rendered_by": "stichos/0.1.0"
}
```

**Return schema (dry_run=false — actual application):**
```
Current behavior unchanged: returns the full document with edits applied.
```

**Behavior:**
- When `dry_run: true`, return only the preview (limited to 20 operations, capped).
- `estimatedTokenChange` gives the LLM a quick sense of cost before committing.
- `diffSummary` is a human-readable one-liner for the LLM to include in its reasoning.
- `fidelity_budget` is always included (existing field).
- Full diff is available via `includeFullDiff: true` parameter.

### 3.6 `edit_text` / `edit_cell` / `edit_structure`

#### Changes (v1.6.0 additive)

**New parameters:**
```
dry_run: boolean, default: true
```

**Return schema (dry_run=true — default):**
```json
{
  "preview": {
    "estimatedTokenChange": "+50",
    "elementId": "para_3",
    "oldText": "foo bar baz",
    "newText": "foo baz",
    "operations": [
      { "op": "replace", "path": "/body/para[3]" }
    ]
  },
  "fidelity_budget": { ... },
  "__budget": { "tokens_est": 50 },
  "diagnostics": [],
  "rendered_by": "stichos/0.1.0"
}
```

**Behavior:**
- `dry_run` defaults to `true` for all edit tools — the LLM sees a preview, not the full
  modified document.
- To apply the change, the LLM must explicitly set `dry_run: false`.
- This prevents the LLM from accidentally receiving a 50-page document in its context.

### 3.7 `annotate_pages`

#### Current (v1.1.0)
```
Args: path, dpi, format
Returns: { images: [block], legend }
```

#### Changes (v1.6.0 additive)

**New parameters:**
```
output: enum<"base64" | "urls" | "both">, default: "urls"
max_pages: number, default: 5
```

**Return schema (output="urls" — default):**
```json
{
  "marked": [
    { "page": 1, "url": "/tmp/stichos_annot_m1_p1.png", "fileSize": 245760 },
    { "page": 2, "url": "/tmp/stichos_annot_m1_p2.png", "fileSize": 238592 }
  ],
  "clean": [
    { "page": 1, "url": "/tmp/stichos_annot_c1_p1.png", "fileSize": 241000 },
    { "page": 2, "url": "/tmp/stichos_annot_c1_p2.png", "fileSize": 234000 }
  ],
  "legend": "Red boxes indicate lint issues.",
  "__budget": {
    "tokens_est": 0,
    "image_count": 4
  },
  "diagnostics": [],
  "rendered_by": "stichos/0.1.0"
}
```

### 3.8 `journal`

#### Current (v1.1.0)
```
Args: path, session_id, format
Returns: array of journal entries
```

#### Changes (v1.6.0 additive)

**New parameters:**
```
max_entries: number, default: 10
```

**Behavior:**
- Default cap of 10 entries prevents blowup on documents with many journal entries.
- Each entry includes `id`, `author`, `date`, `kind` (short description).
- Full entry details available via `includeFullEntry: true` per entry.

## 4. Common Response Envelope

Every tool response MUST include:

```json
{
  "..." : "...",           // Tool-specific fields
  "__budget": {
    "tokens_est": number,  // Estimated token cost of this response
    "elements_count": number,  // Element/data count
    "image_count": number    // Number of images (0 if none)
  },
  "diagnostics": [],        // C3-coded diagnostics
  "rendered_by": "string"   // Engine ID + version
}
```

The `__budget` field is reserved (starts with `__`) so it won't collide with future tool fields.

## 5. Pagination Pattern (§3.7 referenced above)

For tools that can return large result sets, support cursor-based pagination:

```json
{
  "elements": [ ... ],
  "hasMore": true,
  "nextCursor": "eyJwYWdlIjogMn0=",
  "totalElements": 10234
}
```

And the corresponding request parameter:

```
cursor: string (optional) — Opaque cursor from a previous response
```

This follows the MCP pagination spec:
<https://modelcontextprotocol.io/specification/2025-03-26/server/utilities/pagination>

## 6. Schema Versioning

| Tool | Current Version | New Version | Change Type |
|---|---|---|---|
| `inspect_document` | 1.0.0 | 1.6.0 | Additive (new optional params) |
| `render_document` | 1.0.0 | 1.6.0 | Additive (new optional params) |
| `inspect_region` | 1.0.0 | 1.6.0 | Additive (new optional params) |
| `diff_documents` | 1.0.0 | 1.6.0 | Additive (new default) |
| `apply_patch` | 1.1.0 | 1.6.0 | Additive (new default behavior for dry_run) |
| `edit_text` | 1.1.0 | 1.6.0 | Additive (new default for dry_run) |
| `edit_cell` | 1.1.0 | 1.6.0 | Additive (new default for dry_run) |
| `edit_structure` | 1.1.0 | 1.6.0 | Additive (new default for dry_run) |
| `annotate_pages` | 1.1.0 | 1.6.0 | Additive (new optional params) |
| `journal` | 1.1.0 | 1.6.0 | Additive (new optional param) |

All changes are additive — no existing parameters are removed or retyped.
Clients using the old parameters continue to work with the same behavior.

## 7. Implementation Checklist

### 7.1 `crates/mcp/src/spec.rs`

- [x] Update `ParamDef` to support `enum` types (or use `String` with enum documentation)
- [x] Add new parameters to each tool spec:
  - `inspect_document`: `detail`, `max_elements`
  - `render_document`: `output`, `max_pages`
  - `inspect_region`: `output`
  - `diff_documents`: (update description for new default `max_ops`)
  - `apply_patch`: (update description for new `dry_run` default)
  - `edit_text`, `edit_cell`, `edit_structure`: (update `dry_run` default description)
  - `annotate_pages`: `output`, `max_pages`
  - `journal`: `max_entries`
- [x] Add `schema_version: "1.6.0"` to all modified tools

### 7.2 `crates/cli/src/` (tool handlers)

- [x] `inspect_document` handler:
  - [x] Parse `detail` parameter
  - [x] Implement summary mode (page stats, element counts, IDs only)
  - [x] Implement elements mode (elements up to `max_elements`)
  - [x] Implement `hasMore` / `nextCursor` support
  - [x] Add `__budget` to response
- [x] `render_document` handler:
  - [x] Parse `output` parameter
  - [x] Write images to temp files when `output="urls"`
  - [x] Return URL metadata instead of base64
  - [x] Implement `max_pages` cap
  - [x] Add temp file cleanup (1-hour TTL)
  - [x] Add `__budget` to response
- [x] `inspect_region` handler:
  - [x] Parse `output` parameter
  - [x] Return URL for images when `output="urls"`
  - [x] Add `__budget` to response
- [x] `diff_documents` handler:
  - [x] Change `max_ops` default from unlimited to 20
  - [x] Add `summary` field to response
  - [x] Add `hasMore` / `nextCursor` support
  - [x] Add `__budget` to response
- [x] `apply_patch` handler:
  - [x] Change `dry_run` default from `false` to `true` (for LLM safety)
  - [x] Limit preview operations to 20
  - [x] Add `estimatedTokenChange` to preview
  - [x] Add `diffSummary` (human-readable one-liner) to preview
  - [x] Add `__budget` to response
- [x] `edit_text`, `edit_cell`, `edit_structure` handlers:
  - [x] Change `dry_run` default from `false` to `true` (for LLM safety)
  - [x] Return preview structure (as in `apply_patch`)
  - [x] Add `__budget` to response
- [x] `annotate_pages` handler:
  - [x] Parse `output` parameter
  - [x] Write images to temp files when `output="urls"`
  - [x] Return URL metadata
  - [x] Implement `max_pages` cap
  - [x] Add `__budget` to response
- [x] `journal` handler:
  - [x] Parse `max_entries` parameter
  - [x] Default to 10 entries
  - [x] Add `__budget` to response

### 7.3 `docs/contracts/C7.md`

- [x] Add v1.6.0 section with updated tool matrix
- [x] Document new parameters and return schemas
- [x] Note behavioral changes for `dry_run` defaults

### 7.4 Tests

- [x] Unit tests for each tool with new parameters
- [x] Integration tests verifying:
  - Summary mode returns <500 tokens
  - URL mode returns no base64
  - `max_ops` cap works correctly
  - `dry_run` default prevents full document return
  - Temp file cleanup runs correctly
- [x] Contract compliance tests (C7 v1.6.0)

### 7.5 Documentation

- [x] Update `CHANGELOG.md`
- [x] Update `README.md` if needed
- [x] Update phase status files (`phase5-status.json` or next phase)

## 8. Migration Notes

### For CLI Users
- No breaking changes. `dry_run` flags are optional and default to the same values
  as before when explicitly set.
- The new defaults only affect behavior when parameters are omitted.
- CLI users who want the old behavior (full document returned) can set `--dry-run=false`
  or the JSON-RPC equivalent.

### For MCP Clients
- Clients will see the new `__budget` field on every response.
- Clients that parse response structures should ignore unknown fields (forward compatible).
- Clients that rely on base64 images in `render_document` or `annotate_pages` must
  explicitly set `output="base64"`.

### For Automation / CI
- Scripts that parse tool responses should handle the new `__budget` and `summary` fields.
- Scripts that expect base64 in `render_document` responses must be updated to handle
  URL references or set `output="base64"`.

## 9. Open Questions

| Question | Options | Status |
|---|---|---|
| What should `dry_run` default be for `apply_patch`? | `true` (safer) vs `false` (current) | [resolved: true for safety] |
| What temp file TTL is appropriate? | 30 min, 1 hour, 24 hours | [resolved: 1 hour] |
| Should `inspect_region` default to `urls`? | Yes (saves ~100KB per call) | [resolved: urls] |
| Should `render_document` return page count hint? | `"hint": "Use page_range to render more"` | [resolved: included] |
| How to handle `max_pages` overflow? | Return error vs return partial | [resolved: return partial with hasMore] |

## 10. References

- MCP Pagination Spec: <https://modelcontextprotocol.io/specification/2025-03-26/server/utilities/pagination>
- Dual-Response Pattern (arXiv:2510.05968): <https://arxiv.org/html/2510.05968v1>
- Client Best Practices (Progressive Discovery & Code Mode):
  <https://modelcontextprotocol.io/docs/2026-07-28/develop/clients/client-best-practices>
- StackOne Token Optimization: <https://www.stackone.com/blog/mcp-token-optimization/>
- Response Filtering with jq (Speakeasy): <https://www.speakeasy.com/mcp/tool-design/response-filtering-jq/>
- C7 — Tool Schema Contract: [`C7.md`](./C7.md)
- C8 — Payload Budget: (referenced in C7)
