---
created_at: 2026-10-17T00:10:06Z
updated_at: 2026-10-17T00:10:06Z
completed_at:
---

## 1. Implementation
- [ ] 1.1 `terminal.rs`: query keyboard-enhancement support once at the
      first `enter()`, caching it. Push `DISAMBIGUATE_ESCAPE_CODES` after
      the alternate screen. Pop it first in `leave_screen`, guarded by a
      swap-once flag, so `leave`, `restore_best_effort`, the panic hook,
      the exit-deadline backstop and the stop-signal path all pop exactly
      once.
- [ ] 1.2 Map Ctrl+[, Ctrl+I and Ctrl+M back to Esc, Tab and Enter in the
      shared terminal event reader.
- [ ] 1.3 Ctrl+J inserts a newline in the composer. Shift+Enter and
      Alt+Enter insert one in the command palette.
- [ ] 1.4 Add a `Ctrl+J` keymap row (help and shortcuts) and the
      `ctrl+j for newline` hint for idle non-empty drafts.
- [ ] 1.5 Scroll the composer so the cursor row stays visible past eight
      rows.
- [ ] 1.6 Update the help fixtures and `DESIGN.md`.

## 2. Tests
- [ ] 2.1 smith-tui: Ctrl+J, Shift+Enter, Alt+Enter and `\` then Enter
      insert newlines. A multi-line draft submits intact. Palette newline
      keys do not run the command.
- [ ] 2.2 smith-tui: the bindings crossterm reports under disambiguation
      (Esc, Ctrl+C, Ctrl+D, Tab, BackTab with SHIFT, Ctrl+letters with
      lock state, Alt+B/F, paste) behave as in legacy mode.
- [ ] 2.3 smith-tui: the hint row and the tall-draft viewport.
- [ ] 2.4 smith-cli: exact enter and leave bytes with and without
      enhancement. The pop comes before `?1049l`. A second leave writes no
      pop. The legacy aliases map back.

## 3. Verification
- [ ] 3.1 Focused tests, clippy, fmt, PTY suite, full gate.
- [ ] 3.2 Live tmux check: a multi-line draft from Ctrl+J and from CSI-u
      Shift+Enter, then quit, and confirm the shell's key mode is intact.
