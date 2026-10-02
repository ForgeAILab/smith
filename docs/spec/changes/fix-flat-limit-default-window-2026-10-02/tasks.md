---
created_at: 2026-10-02T22:33:07Z
updated_at: 2026-10-02T23:10:00Z
completed_at:
---

# Tasks: Let a flat model limit outrank a built-in default window

## 1. Implementation
- [x] 1.1 Add failing factory tests: a flat limit over a trusted default
      window resolves to the flat limits; an explicit window over the same
      flat limit still fails naming the flat key.
- [x] 1.2 Drop a default window name the flat limit outranks in
      `resolve_context_window_selection`.

## 2. Release
- [x] 2.1 Bump the workspace and lockfile to 0.2.16 and add
      `docs/releases/v0.2.16.md`.
- [x] 2.2 Verify formatting, Clippy, and workspace tests.
- [x] 2.3 Start a release build against the owner's real configuration.
- [x] 2.4 Commit on `release/v0.2.16`.
