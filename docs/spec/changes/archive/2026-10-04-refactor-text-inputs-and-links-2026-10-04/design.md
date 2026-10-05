## Context

Fields other than the composer push and pop characters on a `String`. The
renderer draws through ratatui's cell buffer, which computes widths from cell
symbols; writing an OSC 8 sequence into a symbol naively makes the diff think
the cell is wide and shifts the row.

## Decisions

- **Line input.** A `LineInput { text, cursor }` with methods for each
  editing key and paste, plus a `masked` presentation flag; the composer
  delegates its per-line edits to the same functions. One shared key-mapping
  function turns a `KeyEvent` into a line-edit, so every field gets the same
  bindings.
- **Detection.** OSC 8 is on when `TERM_PROGRAM` is `iTerm.app`, `WezTerm`,
  `ghostty`, or `vscode`; `TERM` is `xterm-kitty` or `xterm-ghostty`;
  `VTE_VERSION` ≥ 5000; or `WT_SESSION` is set. It is off when `TMUX` or
  `STY` is set (multiplexers that strip or mangle OSC 8 by default), and off
  otherwise. Detection lives where the theme is built from the environment.
  `NO_COLOR`/`--no-color` do not disable it; tests build themes without it.
- **Emission.** Follow ratatui's documented hyperlink technique: render the
  label normally, then rewrite the covered cells so the opening sequence rides
  on the first cell's symbol and the closing sequence on the last, without
  changing any cell's display width (split into width-1 symbols and skip
  flags as needed). The selection reader strips escape sequences when
  collecting glyphs, and fixtures render with hyperlinks off.

## Risks / Trade-offs

- A terminal that claims support but mishandles OSC 8 shows stray text; the
  allow-list is deliberately short. No configuration switch is added in this
  change.
- Hyperlinks are written for the transcript only, not pickers or setup.
