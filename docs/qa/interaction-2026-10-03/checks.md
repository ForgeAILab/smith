# Confirmations, prompt queue, and typed notices — checks

`refactor-tui-interaction`, branch `refactor/tui-interaction`, 2026-10-03.

- Eleven confirmations (undo, redo, revert, MCP trust, skill trust, review,
  child invocation, follow-up, resume, account switch, quit) now use one
  component. Titles, warnings, bodies, and accept labels render as before
  (local-command fixtures: dialog frames unchanged); bodies scroll to their
  last real line, with a position hint on overflow, including at 40x10.
- Fixture captures of confirmations no longer draw the startup guide behind
  the dialog (an open confirmation counts as pending work) and are shorter;
  the raw captures name each dialog's real title.
- Prompts queue in arrival order; a prompt closes an open picker; a picker
  requested over a prompt is refused with keypress feedback.
- Feedback (refused commands, unchanged selections, clipboard no-ops, no
  foreground shell to background) shows on the left of the hint row with the
  identity kept on the right, and clears on the next key
  (`account-active` fixtures). All other notices render as before.
- The cached-rendering property test from `refactor-transcript-rendering`
  took 326 s in debug; it now covers the same mutation kinds, widths, and
  fold states in 3.3 s. Workspace tests: 2,001 passed in 57 s.
- `../grammar-2026-10-03/final_checks.py`: 67/67.
