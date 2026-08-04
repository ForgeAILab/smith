---
created_at: 2026-07-27T06:38:48Z
updated_at: 2026-07-27T16:28:54Z
---

## Why

Smith currently opens informational command results such as `/status`,
`/agent`, and `/diff` in a blocking viewer that must be dismissed. This
interrupts the conversational flow and hides the surrounding context, unlike
the direct transcript output used by Codex.

## What Changes

- Render read-only local command results as attributed transcript blocks
  instead of viewer modals.
- Keep the composer active after local output and preserve normal transcript
  scrolling/follow behavior.
- Keep command completion, approvals, provider-spend confirmation, `/undo`,
  and `/revert` confirmation modal because they require selection or an
  explicit safety decision.
- Bound local output and render empty, unavailable, binary, oversized, and
  error results inline without sending them to the provider or adding them to
  canonical model conversation history.
- Remove the informational viewer overlay once no command depends on it.

## Impact

- Affected specs: `client-surfaces`
- Affected code: `DESIGN.md`, `crates/smith-tui/src/app.rs`,
  `crates/smith-tui/src/transcript.rs`, `crates/smith-tui/src/render.rs`,
  `crates/smith-cli/src/main.rs`
