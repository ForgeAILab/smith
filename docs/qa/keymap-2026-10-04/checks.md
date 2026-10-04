# Key table and interactive loop — checks

`refactor-keymap-and-loop`, branch `refactor/keymap-and-loop`, 2026-10-04.

- Gate with the macOS runner's temp path: fmt, strict Clippy, 2,027
  workspace tests, `cargo deny --locked check all`. Every fixture
  unchanged; `/help` and the shortcuts panel render from the table with
  identical text.
- Ordering tests (`app/tests/key_ordering.rs`) were written against the
  unchanged `reduce_key` and pass.
- The binding check fails when the table disagrees with key handling.
  Moving the paused-output Ctrl+L binding to Ctrl+J failed both the binding
  check and the footer-hint check, naming `Ctrl+L`; giving the
  confirmation's Ctrl+O the wrong effect failed the binding check, naming
  `Ctrl+O`. Both edits were reverted.
- `run_tui` is 78 lines; its `select!` arms, `biased;`, and the
  end-of-loop quit check keep their order. The longest handler is 105
  lines.
- `../grammar-2026-10-03/final_checks.py`: 67/67.

## Cache, 0.3.4 against 0.3.5

`../grammar-2026-10-03/cache_ab.py` ([results](cache-ab.json)). All three
providers matched turn for turn within normal provider variance (xAI turn 2:
2,496 and 2,432 cached tokens). No miss or re-billed tokens on any turn.
