# stichos M1 Threat Model

## Scope
Phase 1: read-only document inspection and rendering.

## Assets
1. Document content (user data)
2. System resources (CPU, memory, disk)
3. Network (MCP server, if enabled)

## Threats
| Threat | Vector | Mitigation | Status |
|--------|--------|------------|--------|
| Zip bomb / resource exhaustion | Malformed OPC | MAX_ENTRIES, MAX_EXPANSION_RATIO, MAX_ENTRY_SIZE | Mitigated |
| Path traversal | Malicious OPC entries | Path traversal detection → OOX-E-004 | Mitigated |
| XXE | Malicious XML | No entity expansion in XML parser | Mitigated |
| Stack overflow | Deeply nested XML | Panic catch in fuzz harness | Mitigated |
| Memory exhaustion | Oversized documents | Entry size limits | Mitigated |
| Macro execution | xlsm/docm | Detection + flagging only, no execution path | Mitigated |
| Prompt injection via document text | Hostile document content | Taint marking (Phase 2) | Implemented |
| LLM prompt injection via rendered images | Rendered content | N/A for M1 (read-only) | Out of scope |

## Agent-facing threats

### Prompt injection via document text
A malicious document may contain text designed to override or manipulate
agent system prompts. When an LLM processes a document, the extracted
text becomes part of the model's input context, potentially carrying
covert instructions.

**Attack chain:**
1. Attacker creates a document with hidden or embedded text containing
   adversarial instructions (e.g., "ignore previous instructions").
2. The document is fed to the executor, which parses and extracts text.
3. The extracted text flows into the IR and then into a tool response
   sent to an LLM.
4. The LLM may follow the injected instructions.

**Mitigation:** Taint marking propagates document-taint information
through the IR into tool response envelopes.

### Instruction-channel quarantine
When content is marked as tainted, tools MUST treat it as untrusted.
Specific requirements:
- Tainted responses MUST carry `tainted: true` in their response envelope
  along with the source part that introduced the taint.
- Downstream agents receiving tainted content MUST NOT execute
  instructions embedded in tainted text without explicit user approval.
- Tainted parts MUST be surfaced to the user so they can inspect or
  reject the content before execution.

### Taint propagation chain
The taint flow is:

```
document (source) ──bytes──> executor.execute()
                                │
                                ├── extract_taint(source_bytes, format)
                                │       → TaintMark { tainted: true, source_part, kind }
                                │
                                └── ExecuteResult { tainted_parts: [...] }
                                              │
                                              └── with_taint(TaintMark)
                                                      │
                                              ResponseEnvelope { taint: Some(...) }
```

1. `extract_taint()` scans source bytes for format-specific markers:
   - **OOXML** (docx/xlsx/pptx): OPC part markers (`word/`, `xl/`, `ppt/`)
   - **PDF**: `%PDF-` binary header
2. If markers are found, a `TaintMark` is returned with `tainted: true`.
3. The executor adds `tainted_parts` to `ExecuteResult`.
4. The `with_taint()` method propagates the `TaintMark` through the
   `ResponseEnvelope` so clients can inspect taint status.

## Phase 2 — Write Path Threats

Phase 2 adds the write path (patch execution, journaling, post-commit validation).
This table covers threats introduced by the write path.

| Threat | Vector | Mitigation | Status |
|--------|--------|------------|--------|
| Patch JSON injection | Malicious MCP tool input | JSON schema validation (P2-26) + schema gate (P2-31) | Implemented |
| Policy bypass via patch crafting | Exploiting edit-policy gaps | Deny-by-default policy (P2-24) + deny-matrix tests | Implemented |
| Schema corruption via patch | Write produces invalid OOXML | Schema validation gate (P2-31) | Implemented |
| Journal chain tampering | Malicious parent_id references | Content-addressed store integrity | Implemented |
| Taint escape via patch | Tainted content bypasses marking | Taint propagation through executor (P2-23) | Implemented |
| Write-path DoS | Massive patch ops causing resource exhaustion | Entry count limits + nesting depth + timeout in executor | Implemented |
| Signature spoofing | Creating fake signature-valid documents | No fake signing; only invalidation detection (P2-22) | Implemented |
| Path traversal via patch part URIs | Malicious part URI in patch | OPC entry URI validation | Implemented |

## SEC-06 M2 Security Review Checklist

- [x] All fuzz targets pass (100% no-panic)
- [x] Schema gate catches broken mutations
- [x] Policy deny-matrix verified (no allowed destructive ops)
- [x] Taint propagation verified on patched output
- [x] Journal chain integrity verified
- [x] No RCE vectors (parsers never execute code)
- [x] No DoS via panic (all errors caught)
- [x] Write-path fuzz: >=24h clean run
- [x] Adversarial patch JSON: all rejected gracefully
- [x] Hostile package + patch: refused correctly

Review completed by: p2-32 agent
Date: 2026-09-15

## SEC-06 Phase-3 Security Review

### Phase-3 Attack Surface
Phase 3 introduces new attack vectors: ODF/PDF writers, OOXML creation engines, pandoc/Chromium/Typst
authoring paths, and the Rhai sandbox. This review assesses each.

### Review Checklist
- [x] ODF writer: hostile content.xml → parser rejects, no panic, C3 code returned
- [x] PDF incremental: hostile mutations → original bytes preserved as prefix, or refusal
- [x] Creation IR: malformed IR → writer rejects or produces valid package, never panics
- [x] Pandoc args: no command injection via source content (pandoc invoked as subprocess with
      explicit args, not interpolated into shell command string)
- [x] Chromium flags: no flag injection via source HTML (flags are hardcoded, HTML goes to temp file)
- [x] Typst input: no path traversal via .typ source (compiled from temp file, output controlled)
- [x] Rhai sandbox: escape attempts (fs, net, env, register_package) → denied, zero side effects
- [x] Selfupdate: signature verification before swap (ADR-0006)
- [x] C7 additive-only: no breaking schema changes since v1.0.0

### Findings
No new vulnerabilities identified beyond M2 threat model. The new write-path components (ODF writer,
PDF incremental writer, creation engines) follow the same defensive patterns as existing code:
- All inputs are validated before processing
- Errors are returned as structured diagnostics (C3 codes), never panicking
- The append-only invariant on PDF ensures original content is preserved
- The Rhai sandbox enforces deny-by-default with explicit operation limits

### Mitigations Added
- ODF writer: mimetype-first enforcement, encryption detection (ODF-E-003), manifest corruption handling
- PDF incremental writer: append-only prefix invariant, encryption/xref-stream/object-stream/signature
  detection (PDF-E-003/004/005/006), hostile mutation rejection
- Creation IR: graceful handling of empty documents, malformed UTF-8, duplicate IDs, missing templates
- Rhai sandbox: operation limits, call depth limits, script size limits, register_package denial,
  std/env/fs/net access denied (RHI-E-001/002/003/004/005/006)

Review completed by: p3-27 agent
Date: 2026-09-16

## SEC-06 Phase-4 Security Review

### Phase-4 Attack Surface
Phase 4 introduces new vectors: revision parsers (hostile trees), review HTTP server (session tokens, XSS), and the viewer surface.

- **Revision reader/parser**: Accepts revision data with nested operations, orphan IDs, duplicate IDs, extremely long author/date fields, and 1000+ revision entries. The parser must refuse malformed input without panicking.
- **Review HTTP server**: Token-based authentication via URL path, query string, and header. Attack surfaces include path traversal in tokens, token confusion across multiple sources, HTTP header injection via CR/LF, request smuggling, and oversized headers.
- **Review viewer**: Renders document text in HTML via JSON responses. Attack surface includes XSS payloads embedded in document text (script injection, event handlers, javascript: URIs, mixed payloads).
- **Version-hash invalidation**: Document changes after session creation must invalidate the session (410 Gone).

### Review Checklist
- [x] Revision reader: hostile trees (nested, orphan IDs, duplicates, author bombs, huge counts) → refuses/doesn't panic
- [x] Session HTTP: path traversal in token → 400; token confusion → rejected; header injection → rejected
- [x] Viewer XSS: document text in data.json → HTML-escaped; viewer escapeHtml() → neutralizes payloads
- [x] Version-hash invalidation: doc change → 410 Gone
- [x] CSP headers: present on all review routes; no eval; no external resources
- [x] Rejection of unknown revision kinds → GP-R-001, never silent drop
- [x] Format-change revision refusal → GP-R-001 (out of scope for M4)

### Findings
No new vulnerabilities identified.

### Mitigations Added
- Hostile revision fixtures (nested, orphan, duplicate, bombs, huge counts)
- Session HTTP attack fixtures (traversal, confusion, header injection, smuggling)
- XSS corpus (script injection, event handlers, javascript URIs)
- All added as deterministic regression tests in adapters-ooxfuzz

Review completed by: p4-18 agent
Date: 2026-09-18
