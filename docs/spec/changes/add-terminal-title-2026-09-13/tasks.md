# Tasks: Dynamic terminal window title for the interactive TUI

## 1. `terminal_title` module (smith-tui)

- [x] 1.1 Create `crates/smith-tui/src/terminal_title.rs`: OSC 0 + BEL writer
      over `impl Write` with flush, `is_terminal`-guarded stdout wrappers,
      `SetTerminalTitleResult::{Applied, NoVisibleContent}`, and
      `clear_terminal_title`. Export from `lib.rs`.
- [x] 1.2 Implement `sanitize_terminal_title` (control chars, bidi/invisible
      codepoints, whitespace collapsing, 240-char truncation) with unit tests
      covering each rule, the truncation boundary, and the empty result.
- [x] 1.3 Implement pure `title_from_status(&Status) -> String` (segments
      `smith · project · model`, activity label only while a turn is in
      flight, skip-empty segments) with unit tests for idle/working/interrupted
      and a missing provider/model path.

## 2. Driver wiring (smith-cli)

- [x] 2.1 In `tui_driver::run_tui`, recompute the title on event batches and
      the tick branch; write through the module only when the rendered text
      changed (last-written cache).
- [x] 2.2 After `run_tui` returns, before `terminal.restore()`, clear the
      managed title exactly once when one was written; confirm the setup,
      picker, and login surfaces never write a title.
- [x] 2.3 Test at the tracker seam rather than through the event loop:
      `TerminalTitleState` writes only when the rendered title changed,
      clears exactly once, and emits no bytes at all when its target is not
      a terminal. The driver holds one tracker and calls it on the redraw
      path, so the loop itself is left untested.

## 3. Validation

- [x] 3.1 `cargo fmt`, `cargo clippy --workspace` (warnings are errors), and
      workspace tests; focused runs for `smith-tui` and `smith-cli`.
- [ ] 3.2 Live check in a real terminal: two concurrent TUI sessions in
      different projects show distinct, updating titles; title clears on
      exit; `smith -p 'x' | cat` output contains no ESC bytes.
      (Machine-verified half: the headless/setup/picker/login paths contain
      no `terminal_title` reference, and the non-terminal guard is unit
      tested; the two-window visual check still needs a human at a real
      terminal.)

