---
created_at: 2026-08-02T17:41:10Z
updated_at: 2026-08-02T17:56:23Z
completed_at: 2026-08-02T17:56:23Z
---

## 1. Contract and Design

- [x] 1.1 Update `DESIGN.md` to make native terminal selection/copy the
  pointer contract and retain keyboard-only required operation.

## 2. Terminal and TUI Input

- [x] 2.1 Stop enabling and disabling terminal mouse reporting while
  preserving raw mode, alternate-screen ownership, and bracketed paste.
- [x] 2.2 Remove host mouse-event routing and TUI click/wheel handlers plus
  pointer-only layout state.

## 3. Verification

- [x] 3.1 Replace obsolete mouse behavior tests with focused contract coverage
  for copy-first terminal mode and unchanged keyboard scrolling/cursor paths.
- [x] 3.2 Run `cargo fmt --check`, focused Smith CLI/TUI tests, Clippy, and the
  workspace test suite.
- [x] 3.3 Manually verify drag selection and terminal copy on stable transcript,
  footer, picker, and setup text in a real supported terminal.
