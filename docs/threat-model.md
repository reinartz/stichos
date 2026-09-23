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

## SEC-06 Phase-5 Security Review

### Phase-5 Attack Surface
Phase 5 introduces new vectors: PDF annotation dictionaries (deep nesting, ref loops, oversized rects, malformed QuadPoints), signature/byte-range structures (maligned ranges, huge ByteRange ints, conflicting AcroForm/SigFields), text-layer data.json strings (XSS corpus reuse with Unicode evasion), and vendored-asset path attacks (traversal, null bytes, double-encoding, Unicode normalization).

### Review Checklist
- [x] Annotation dictionaries: hostile nested/ref-loop/oversized/malformed → parser rejects, no panic
- [x] Signature/byte-range structures: maligned ranges, huge ints, conflicting AcroForm → detect_signatures refuses gracefully
- [x] Text-layer data.json: XSS payloads (40+ techniques), Unicode homoglyph evasion, HTML entity bombs → JSON-serialized safely (serde_json), CSP script-src 'self' unchanged, textContent-only rendering
- [x] Vendored asset routes: path traversal (dotdot, null bytes, double-encoding, Unicode normalization, 8KB paths) → whitelist-based serving, no traversal possible
- [x] Annotation write-back: signed-doc detection → GP-S-001 refusal, never silent drop
- [x] Sign-later: sign-after-sign refusal → GP-S-016, PKCS#12 only, self-signed only (B-B)
- [x] C7 v1.5.0 additive: sign_document tool (23 tools total), no breaking changes
- [x] CSP unchanged: script-src 'self', no eval, no external resources
- [x] Fuzz regression corpus: 19 new fixture functions + 23 tests, all green

### Findings
No new vulnerabilities identified beyond Phase-4 threat model. The new components follow the same defensive patterns:
- All inputs validated before processing
- Errors returned as structured diagnostics (C3 codes), never panicking
- Text layer uses textContent-only rendering (XSS-inert)
- CSP script-src 'self' unchanged
- Vendored assets served via whitelist (not path-based)
- Signature detection + refusal on every mutation path
- sign-later uses self-signed keys only (no CA, no TSA, honest-absent)

### Mitigations Added
- Hostile annotation fixtures (deep nesting, ref loops, oversized rects, malformed QuadPoints, damaged dicts)
- Signature/byte-range hostile fixtures (maligned ranges, huge ints, conflicting AcroForm, nested sig dicts, corrupted PKCS#7)
- Text-layer XSS corpus reuse (40+ techniques, Unicode evasion, entity bombs)
- Vendored-asset path attack fixtures (traversal, null bytes, double-encoding, Unicode normalization, 8KB paths)
- All added as deterministic regression tests in adapters-ooxfuzz (19 fixtures, 23 tests)

Review completed by: p5-17 agent
Date: 2026-09-19

## SEC-06 Phase-6 Security Review

### Phase-6 Attack Surface
Phase 6 introduces new vectors: CTX-01..05 envelope types and caps (max_elements, cursor pagination, temp file paths, diff max_ops, journal max_entries, dry_run boundary), C15 formula evaluator (deep nesting, huge integers, external references, macro patterns, unsupported functions), Wave I surface (Spreadsheet/Presentation IRDocument converters, apply-op against real writers), and parser.rs:498 regression corpus (LO-produced PDFs triggering subtract-overflow).

- **CTX-01..05**: `inspect_document` with extreme `max_elements` (0, 2^31-1, negative, float overflow), cursor forgery (truncated, oversized, base64 garbage, null bytes), temp file path traversal (dot-dot, absolute path, null bytes, unicode normalization), caps boundary (max_ops=0 legacy opt-out, negative caps), mutation safety (conflicting dry_run + apply flags, huge op arrays).
- **C15 Formula Evaluator**: Deeply nested formulas (100 levels of SUM), huge integers exceeding i64 range, external workbook references ([1]Sheet!$A$1, [Book2.xlsx]Sheet1!A1), R1C1-style ranges (R1C1:R1048576), macro patterns (SUBPROCESS, RUNMACRO, VBA, DDE|, OLE|), array/matrix functions (MMULT, TRANSPOSE), unsupported functions (VLOOKUP, INDEX, RANDBETWEEN), 1M-char text literals, 10000-function-argument calls, unbalanced parentheses.
- **Wave I Surface**: Spreadsheet IR (empty sheets, empty rows, 100K rows, 1M-char cell values, malformed cell refs like A1:B1048576:extra, of:= ODF formula syntax, huge merged ranges, C15 deep-nesting refusal boundary). Presentation IR (empty slides, 10K shapes per slide, 500-level shape chains, 100 slides, many layouts/masters). Apply-op (deeply nested IR, non-existent element IDs, empty element IDs, 10KB element IDs, 1000-op batches, malformed ops, 1MB text replacement, null bytes in values).
- **Parser Regression**: 25 LO-produced PDF fixtures targeting parser.rs:498 subtract-overflow fix (empty data, short data, boundary data at scan limit, binary garbage with stream bytes, UTF-8 BOM, CRLF stream delimiters, very long data at 10MB).

### Review Checklist
- [x] CTX caps: extreme max_elements values handled without panic (0, 2^31-1, -1, 1e308)
- [x] Cursor forgery: truncated, oversized (102KB), base64 garbage, null bytes, newlines — all handled
- [x] Temp path traversal: dot-dot, absolute path, null bytes, 8KB paths, unicode normalization — all handled
- [x] Caps boundary: max_ops=0 (legacy opt-out), negative caps, journal max_entries=0 — all handled
- [x] Mutation safety: conflicting dry_run+apply, huge op arrays (1000) — all handled
- [x] Formula deep nesting: 100 levels of SUM parsing completes without panic (refusal triggered at 32+ paren depth)
- [x] Formula huge integers: 10^20, f64::MAX, f64::NEG_MAX, division by zero, subtraction overflow — all handled
- [x] Formula external refs: [1]Sheet, [Book2.xlsx], R1C1 ranges, multi-external refs — all caught by refusal detection
- [x] Formula macro patterns: SUBPROCESS, RUNMACRO, VBA, DDE|, OLE| — all caught by refusal detection
- [x] Formula unsupported functions: VLOOKUP, INDEX, RANDBETWEEN, MMULT, TRANSPOSE — all handled
- [x] Formula edge cases: 100KB text literal, 10000 function args, unbalanced parens, empty string, operators only — all handled
- [x] Sheet inspect: empty sheets, malformed refs, C15 deep-nesting formula — converters don't panic
- [x] Sheet inspect stress: 100K rows, 1M-char cell, huge merged ranges, 10 sheets — converters don't panic
- [x] Presentation inspect: empty slides, empty text shapes, many layouts/masters — converters don't panic
- [x] Presentation inspect stress: 10K shapes, 100 slides, varied shape kinds — converters don't panic
- [x] Apply-op: nonexistent/empty/long element IDs, oversized values, null bytes — all handled
- [x] Apply-op batches: 1000-op arrays, malformed ops (missing fields, wrong types), circular props — all handled
- [x] Parser regression: 25 PDF fixtures all pass find_stream_kw without panic (saturating_sub fix verified)
- [x] Parser PDF parser: subset of regression fixtures pass full PDF parser without panic

### Findings
No new vulnerabilities identified beyond the Phase-5 threat model. The new attack surfaces are handled by existing defensive patterns:
- All inputs validated before processing
- Errors returned as structured diagnostics (C3 codes), never panicking
- Formula refusal detection catches external refs, macro patterns, deep nesting
- Parser.saturating_sub prevents underflow on short data
- CTX caps use defensive defaults (max_elements defaults to large value, cursor validation is lenient)
- Apply-op uses UnwindSafe for panic safety with mutable IRDocument

### Mitigations Added
- CTX fuzz fixtures (caps, cursor, temp paths, mutation safety): 25 fixtures, 15 tests
- Formula fuzz fixtures (deep nesting, huge ints, external refs, macro patterns): 25 fixtures, 22 tests
- Wave I fuzz fixtures (sheet/presentation IR, apply-op): 20 fixtures, 18 tests
- Parser regression corpus (parser.rs:498): 25 fixtures, 12 tests
- All added as deterministic regression tests in adapters-ooxfuzz
- run_wave_x_fuzz() entry point for CI integration

Review completed by: p6-17 agent
Date: 2026-09-23
