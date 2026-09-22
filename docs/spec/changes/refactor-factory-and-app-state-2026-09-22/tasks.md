---
created_at: 2026-09-22T20:50:03Z
updated_at: 2026-09-22T22:08:21Z
completed_at: 2026-09-22T22:08:21Z
---

# Tasks: Refactor factory and TUI application state

## 1. Baseline

- [x] 1.1 Wait for the context-window and image-generation implementations to
      finish, then record the actual factory and App ownership boundaries.

## 2. Factory

- [x] 2.1 Move provider resolution, credential preparation, adapter construction,
      and context policy into real private stage modules.
- [x] 2.2 Move authority and approval implementation into its private stage
      module; keep `build`'s validation and assembly order unchanged.
- [x] 2.3 Remove forwarding-only wrappers and preserve public factory paths
      without widening items beyond the parent module.

## 3. TUI state

- [x] 3.1 Move cohesive child-session presentation and inspection methods out
      of `app/state.rs`, preserving `App` fields and method signatures.
- [x] 3.2 Move other cohesive method groups where this reduces responsibility
      mixing without altering reducer, input, or rendering behavior.

## 4. Verification

- [x] 4.1 Validate this change's delta spec strictly and run focused build,
      behavior, and formatting checks for the moved code.
- [x] 4.2 Review the final diff for forwarding wrappers, accidental visibility
      expansion, semantic edits, and untouched pre-existing work.
