---
created_at: 2026-10-05T01:34:57Z
updated_at: 2026-10-05T02:58:07Z
completed_at: 2026-10-05T02:58:07Z
---

Approved 2026-10-04 ("i think lets fix everything then push").

## 1. One line input

- [x] 1.1 `LineInput` in `smith-tui` with every line-editing key, paste, and
  masking; the composer's line operations use it.
- [x] 1.2 Picker filters, setup fields (plain and masked), and history search
  use it, with a visible cursor.
- [x] 1.3 One table of line-editing cases driven through all five fields.

## 2. Links

- [x] 2.1 OSC 8 capability detection where the theme is built.
- [x] 2.2 Transcript links and bare URLs carry OSC 8 when supported, with no
  change to cell widths or layout; selection/copy strips sequences.
- [x] 2.3 Tests: width-identical rendering with and without links; copy text;
  detection matrix.

## 3. Verification

- [x] 3.1 Gate; fixtures unchanged by links; live check in a supporting
  terminal (cmux/ghostty) and inside tmux.

Live 2026-10-04 (debug build under `expect`, 100x32): with
`TERM_PROGRAM=ghostty` and no multiplexer, a GLM answer `[the docs](https://example.com/docs)`
wrote the label with OSC 8 around each glyph (`ESC]8;;https://example.com/docs ESC\`);
with `TMUX` set, no OSC 8 sequence was written and the label rendered the same.
