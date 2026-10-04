---
created_at: 2026-10-04T00:00:00Z
updated_at: 2026-10-04T05:50:05Z
---

## Why

Two items from the 2026-10-02 structure audit
(`docs/qa/smith-structure-2026-10-02/report.md`) are still open:

- **S5, keys.** `/help` and the `?` shortcuts panel list keys from a
  hand-written string list (`smith-client` `commands::help_keys`), separate
  from the key handling in `smith-tui` `app/input.rs`. Nothing checks that
  the two agree, so the list can name a key that does nothing, or omit one
  that works. The audit's roadmap asks for "a keymap table that also
  generates help and footer hints".
- **S8, the interactive loop.** `smith-cli` `tui_driver.rs` `run_tui` is
  656 lines: about 55 lines of loop state, then one `select!` with every
  `Action` handled inline. A change to one action means reading the whole
  loop.

## What Changes

- **Decision: the key table describes key handling and is checked against
  it, rather than driving it.** Key handling depends on ordering (Ctrl+C is
  read before any prompt, navigation is exempt from a prompt's quiet window,
  a prompt's guard runs before its keys), which a chord-to-action table
  cannot express without a predicate language. `reduce_key` keeps its
  order. The owner can reverse this decision.
- `smith-client` owns a typed key table: each binding has a key chord (a
  `smith-client` type; no terminal library), the context it applies in
  (idle, working, empty draft, a prompt, …), its effect, and the help text
  shown today. `/help` and the shortcuts panel render from it with
  unchanged text.
- A `smith-tui` test maps each binding's chord to a key event, puts the app
  in the binding's context, and checks that key handling produces the
  binding's effect. A key added to the table without handling, or handled
  with a different effect, fails the test.
- Footer hints stay hand-written condensed text; the keys they name are
  bindings in the table, so the same test covers them.
- Tests are added first for the ordering rules the refactor must keep:
  Ctrl+C twice leaves during an approval and a confirmation; navigation
  works inside an approval's quiet window; Esc closes the shortcuts panel
  without acting, other keys close it and act; Ctrl+O works with a prompt
  open.
- `run_tui` becomes a loop-state struct with one method per event source
  (`select!` arm) and one method for actions. Every `Action` arm moves
  verbatim; the arms keep their order.

## Impact

- Affected specs: code-organization.
- Affected code: `smith-client` `commands.rs` (or a new `keymap.rs`),
  `help_report.rs`; `smith-tui` `render/composer.rs` shortcuts, new tests;
  `smith-cli` `tui_driver.rs`.
- No visible change: every fixture is byte-identical and the terminal checks
  are unchanged.
- The rest of S8 (`handle_local_command`, `runtime_resources`,
  `run_with_io`, `use super::*`, `cli::Selection` mutation) and the rest of
  S5 (text inputs, CLI prompt loops) stay open.
