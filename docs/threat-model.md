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
| Prompt injection via document text | Hostile document content | Taint marking (Phase 2) | Deferred |
| LLM prompt injection via rendered images | Rendered content | N/A for M1 (read-only) | Out of scope |

## Security Review Checklist
- [x] No external command execution for document processing
- [x] No macro/script execution paths
- [x] All input validated against size/structure limits
- [x] Fuzz corpus tested (P1-53)
- [x] No shipped runtime dependencies (D4)
- [x] Zero-C dependency policy maintained
- [x] No telemetry or network calls
