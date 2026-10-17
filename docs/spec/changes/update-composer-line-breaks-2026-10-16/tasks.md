---
created_at: 2026-10-17T00:10:06Z
updated_at: 2026-10-17T02:02:09Z
completed_at: 2026-10-17T02:02:09Z
---

## 1. Implementation
- [x] 1.1 `terminal.rs`: query keyboard-enhancement support once at the
      first `enter()`, caching it. Push `DISAMBIGUATE_ESCAPE_CODES` after
      the alternate screen. Pop it first in `leave_screen`, guarded by a
      swap-once flag, so `leave`, `restore_best_effort`, the panic hook,
      the exit-deadline backstop and the stop-signal path all pop exactly
      once.
- [x] 1.2 Map Ctrl+[, Ctrl+I and Ctrl+M back to Esc, Tab and Enter in the
      shared terminal event reader.
- [x] 1.3 Ctrl+J inserts a newline in the composer. Shift+Enter and
      Alt+Enter insert one in the command palette.
- [x] 1.4 Add a `Ctrl+J` keymap row (help and shortcuts) and the
      `ctrl+j for newline` hint for idle non-empty drafts.
- [x] 1.5 Scroll the composer so the cursor row stays visible past eight
      rows.
- [x] 1.6 Update the help fixtures and `DESIGN.md`.

## 2. Tests
- [x] 2.1 smith-tui: Ctrl+J, Shift+Enter, Alt+Enter and `\` then Enter
      insert newlines. A multi-line draft submits intact. Palette newline
      keys do not run the command.
- [x] 2.2 smith-tui: the bindings crossterm reports under disambiguation
      (Esc, Ctrl+C, Ctrl+D, Tab, BackTab with SHIFT, Ctrl+letters with
      lock state, Alt+B/F, paste) behave as in legacy mode. The whole
      keymap table also runs with disambiguated chords.
- [x] 2.3 smith-tui: the hint row and the tall-draft viewport.
- [x] 2.4 smith-cli: exact enter and leave bytes with and without
      enhancement. The pop comes before `?1049l`. A second leave writes no
      pop. The legacy aliases map back.
- [x] 2.5 PTY: a terminal that answers the query as kitty does gets
      exactly one push after `?1049h` and one pop before `?1049l` on
      `/quit`. A CSI-u Shift+Enter does not submit. A terminal that
      answers only device attributes gets neither. The `expect` harness
      now answers the query the way a real terminal does. Before that,
      every PTY test waited out crossterm's 2 s timeout, which cut each
      test's 10 s budget and failed 1 run in 20 under load.

## 3. Verification
- [x] 3.1 Focused tests, clippy, fmt, PTY suite, full gate.
      (2026-10-16:
      - Gate green: 2,981 passed, 0 failed. fmt, clippy, clippy 1.88,
        `--no-default-features`, smoke and deny all clean, with no fixture
        drift.
      - After the harness answered the query, the 19-test PTY suite
        passed 20 runs out of 20, at about 10 s a run instead of about
        27 s. stable and 1.88 clippy on smith-cli are clean.)
- [x] 3.2 Live tmux check: a multi-line draft from Ctrl+J and from CSI-u
      Shift+Enter, then quit, and confirm the shell's key mode is intact.
      (2026-10-16, debug build, real `HOME`, tmux 3.7b:
      - Ctrl+J, CSI-u Shift+Enter, Alt+Enter (`M-Enter`) and `\` then
        Enter built one five-line draft, with the hint
        `ctrl+j for newline`.
      - Past eight rows the composer showed the last eight, and Up
        scrolled back to the first line without recalling history.
      - Esc cleared the draft and `/quit` exited. The shell then reported
        `icanon isig echo icrnl` and took input normally.
      - tmux does not answer `CSI ? u`, even with `extended-keys always`,
        so Smith pushed nothing there.
      - Under a pty harness that answers the query as kitty does, with
        the real config: query, `?1049h`, push `>1u`, pop `<1u`,
        `?1049l`, in that order, once each, exit 0, and no resume hint.
      - A fake-provider run submitted `please echo these lines` /
        `  indented second line` / `third line` with the newlines and
        indentation intact.)
