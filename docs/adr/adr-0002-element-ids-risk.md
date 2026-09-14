# ADR-0002: Element ID risk register

| Risk | Likelihood | Impact | Mitigation | Status |
|---|---|---|---|---|
| SHA-256 prefix collision at 12 hex chars | 10^-13 per 1000 IDs | Low | Use 16 chars if corpus exceeds 10k elements | monitored |
| Content hash not stable across IR versions | Medium | High | Pin IR content serialization (C1) | addressed by C1 freeze |
| Path ordering depends on reader implementation | Medium | Medium | Readers must produce deterministic order | C9 manifest assertions |
| IDs grow too long for LLM context | Low | Low | 24-char format fits 8k token window | verified |
