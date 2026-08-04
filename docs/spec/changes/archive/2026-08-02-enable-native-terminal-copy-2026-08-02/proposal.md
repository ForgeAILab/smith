---
created_at: 2026-08-02T17:41:10Z
updated_at: 2026-08-02T17:56:23Z
---

## Why

Smith enables terminal-wide mouse reporting even though mouse input is
optional. The terminal therefore sends drag gestures to Smith instead of
performing native text selection, so users cannot reliably copy transcript,
status, picker, or setup text with their terminal's ordinary selection flow.

## What Changes

- Stop enabling terminal-wide mouse reporting on Smith terminal surfaces so
  the terminal retains native drag selection and copy behavior.
- Remove Smith's optional click-to-position composer handling and wheel-driven
  transcript scrolling; the existing keyboard paths remain authoritative.
- Keep bracketed paste, raw mode, the alternate screen, keyboard navigation,
  and terminal-native copy shortcuts unchanged.
- Document native terminal selection as the default pointer contract and cover
  terminal entry/restore behavior with focused tests.

## Impact

- Affected specs: `client-surfaces`
- Affected design: `DESIGN.md` keyboard and pointer interaction contract
- Affected code: `crates/smith-cli/src/terminal.rs`,
  `crates/smith-cli/src/tui_driver.rs`, `crates/smith-tui/src/app/input.rs`,
  pointer-layout state, and focused terminal/TUI tests
- Active-change coordination: `update-smith-interaction-model-2026-07-27`
  remains authoritative for the composer-only focus and global keyboard
  transcript navigation contracts. This change adds the orthogonal native
  terminal-selection behavior and does not alter command or profile routing.

## Approval Boundary

Approval authorizes copy-first terminal input ownership: Smith will not enable
global mouse reporting and will give up its optional composer click and wheel
handlers. It does not authorize an application-owned selection model, direct
clipboard access, OSC 52 writes, new dependencies, or changes to keyboard and
paste behavior.
