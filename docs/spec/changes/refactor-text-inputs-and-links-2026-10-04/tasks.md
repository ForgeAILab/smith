---
created_at: 2026-10-05T01:34:57Z
updated_at: 2026-10-05T01:34:57Z
completed_at:
---

Approved 2026-10-04 ("i think lets fix everything then push").

## 1. One line input

- [ ] 1.1 `LineInput` in `smith-tui` with every line-editing key, paste, and
  masking; the composer's line operations use it.
- [ ] 1.2 Picker filters, setup fields (plain and masked), and history search
  use it, with a visible cursor.
- [ ] 1.3 One table of line-editing cases driven through all five fields.

## 2. Links

- [ ] 2.1 OSC 8 capability detection where the theme is built.
- [ ] 2.2 Transcript links and bare URLs carry OSC 8 when supported, with no
  change to cell widths or layout; selection/copy strips sequences.
- [ ] 2.3 Tests: width-identical rendering with and without links; copy text;
  detection matrix.

## 3. Verification

- [ ] 3.1 Gate; fixtures unchanged by links; live check in a supporting
  terminal (cmux/ghostty) and inside tmux.
