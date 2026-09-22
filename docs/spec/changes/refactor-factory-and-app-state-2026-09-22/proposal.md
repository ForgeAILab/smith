---
created_at: 2026-09-22T20:50:03Z
updated_at: 2026-09-22T22:08:21Z
---

# Proposal: Refactor factory and TUI application state

## Why

`smith-runtime/src/factory.rs` has 3,381 production lines, while its seven
private stage modules contain only 86 lines combined and mostly forward back
to the parent. `smith-tui/src/app/state.rs` has 1,895 lines and combines the
`App` state definition with child-session, provider-progress, and presentation
methods. These concentrated responsibilities make independent changes harder
to review.

## What Changes

- Move complete factory responsibilities into the existing `factory/` child
  modules, especially provider preparation/construction and authority policy.
  Keep the public factory types and ordered `build` entry point at their
  existing paths.
- Move cohesive `App` method groups into private `app/` child modules, starting
  with child-session presentation and inspection. Keep the `App` type and its
  public fields/method paths stable.
- Remove forwarding-only stage wrappers once their implementation is owned by
  the appropriate module. Preserve runtime behavior, validation order, event
  order, output, and redaction.

## Impact

- Affected spec: code-organization
- Affected code: `smith-runtime/src/factory.rs` and `factory/*`,
  `smith-tui/src/app/state.rs` and `app/*`
- Dependencies: apply after context-window and image-generation work that
  edits these files, so the refactor moves their final implementation.
- No public API, configuration, dependency, or product behavior change.
