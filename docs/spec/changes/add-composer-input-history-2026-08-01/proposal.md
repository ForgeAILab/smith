---
created_at: 2026-08-01T22:42:33Z
updated_at: 2026-08-01T23:00:46Z
---

## Why

Smith can already recover drafts cleared by the first `Ctrl+C`, but recall is
limited to interrupted drafts. Successfully submitted prompts and commands are
not available through the composer, and there is no local reverse-history
search. This makes repeated or lightly edited terminal input unnecessarily
expensive.

## What Changes

- Replace interrupted-draft-only recall with one bounded, process-local
  composer history containing accepted submissions and non-blank drafts
  cleared by the first `Ctrl+C`.
- Make `Up` and `Down` browse the shared history while preserving the draft
  that was present before navigation and restoring it after the newest entry.
- Add a local `Ctrl+R` reverse search over the same history, with incremental
  matching, repeated-`Ctrl+R` cycling, explicit accept, and lossless cancel.
- Keep overlays authoritative for their existing arrow-key behavior, retain
  the double-`Ctrl+C` exit contract, and prevent history navigation or search
  from creating provider requests or canonical conversation entries.
- Document the new controls and cover Unicode, multi-line input, duplicate
  suppression, capacity, validation failures, and overlay precedence.

## Impact

- Affected specs: `client-interaction`
- Affected code: `crates/smith-tui/src/composer.rs`,
  `crates/smith-tui/src/app.rs`, `crates/smith-tui/src/render.rs`, local help,
  TUI tests, and `DESIGN.md`
- Builds on the completed `add-agent-first-workflow-ux` interrupted-draft
  recall contract; it does not change canonical session persistence or model
  history.
