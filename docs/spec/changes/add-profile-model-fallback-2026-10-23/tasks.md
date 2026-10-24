---
created_at: 2026-10-23T09:29:57Z
updated_at: 2026-10-23T13:32:37Z
completed_at: 2026-10-23T13:32:37Z
---

## 1. Implementation

- [x] 1.1 Add the shared Agent Runtime route-selection/replanning seam, first-response deadline, attempt cancellation fence, and typed fallback events; cover them with deterministic provider-loop and Smith consumer-conformance tests.
- [x] 1.2 Enable a git-ignored local runtime override for Smith integration; finalize the immutable dependency pin in 1.7 after Smith compatibility is verified.
- [x] 1.3 Add typed profile fallback configuration, inheritance/replacement semantics, candidate overrides, source provenance, exact-model override behavior, and preflight validation.
- [x] 1.4 Resolve candidate provider/model plans through the shared Smith factory, retain profile authority, and integrate reactive quota handling with existing credential-pool cooldowns.
- [x] 1.5 Wire request-scoped fallback for main and child turns, current model reporting, history-safe continuation, cache invalidation, and per-attempt usage/pricing attribution.
- [x] 1.6 Project fallback reasons and deadlines consistently into TUI notices/status, headless events, and replay; update configuration and profile guides with supported syntax.
- [x] 1.7 Pin the tested immutable runtime revision, remove the development override, and verify the committed dependency resolves without sibling checkouts.

## 2. Validation

- [x] 2.1 Cover absent configuration, inheritance, empty-list opt-out, invalid/duplicate candidates, per-model overrides, missing credential sources, and CLI/session exact-model overrides.
- [x] 2.2 Use a controlled clock to cover timeout before stream creation, accepted empty stream, heartbeat-only stream, retry/backoff within the same timer, per-candidate timeout overrides, and exhaustion of all candidates.
- [x] 2.3 Prove that meaningful text/reasoning/tool progress disarms the startup timer, late events cannot enter the next attempt, unsafe hosted-tool requests cannot be replayed, and user cancellation/turn deadlines never trigger fallback.
- [x] 2.4 Cover quota and transient failures, credential-rotation precedence, smaller model windows, modality/tool incompatibility, preserved tool results, read-only authority, cache partition changes, and unknown usage on cancelled attempts.
- [x] 2.5 Compare canonical main/child and TUI/headless event behavior; verify old snapshots/events and per-binding cost rollups remain readable and correct.
- [x] 2.6 Run targeted Rust tests, formatting and Clippy, the shared Smith consumer-conformance gate, and strict proposal validation; record evidence before declaring implementation complete.
