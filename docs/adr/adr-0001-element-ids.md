# ADR-0001: Content-anchored element IDs

| Field | Value |
|---|---|
| Status | accepted |
| Date | 2026-09-09 |
| Decision | Use content-anchored SHA-256 derived IDs for all IR elements |

## Context

stichos needs stable element IDs so a 40-turn agent conversation doesn't lose "that third table".
Random UUIDs change between sessions, making cross-session references impossible.

## Forces

- **F1**: IDs must be deterministic (same input -> same ID)
- **F2**: IDs must be short enough for LLM context (<=32 chars)
- **F3**: ID collisions must be practically impossible
- **F4**: Edit-stability (IDs survive edits) is deferred to Phase 2
- **F5**: Read-stability (IDs survive sessions) is the M1 requirement

## Considered Options

### Option A: Content-anchored SHA-256 (chosen)
- Deterministic, collision-resistant, short prefix
- No central registry needed
- Edit-stability requires re-derivation (Phase 2)

### Option B: Path-based sequential numbering
- Simple but fragile under edits
- Breaks with reordering

### Option C: UUIDv7 (time-ordered)
- Monotonic but not content-anchored
- Not deterministic across sessions

## Decision

Use content-anchored SHA-256 derived IDs (Option A).

ID format: `id-<first 12 hex chars of sha256(kind + "/" + path + "/" + content_hash)>`

This satisfies F1-F3. Edit-stability (F5) is deferred.

## Consequences

- IR elements need content accessible for ID derivation
- Readers must produce elements in a deterministic order
- Element IDs are opaque to agents but stable for reference
- Phase 2 will add edit-stability on top (P4/C2)
