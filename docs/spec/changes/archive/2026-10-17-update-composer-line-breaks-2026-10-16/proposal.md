---
created_at: 2026-10-17T00:10:06Z
updated_at: 2026-10-17T20:03:09Z
---

## Why

The owner asked on 2026-10-16: "Can we support line break in the TUI?"

The composer already inserts a newline on Shift+Enter or Alt+Enter, and
`\` then Enter turns the backslash into a newline. In practice:

- Smith never turns on the kitty keyboard protocol, so terminals that
  implement it, such as Ghostty (and cmux, which is built on it) and kitty,
  send Shift+Enter as a plain Enter, and the draft is submitted.
- Alt+Enter works only where Option is set up as Meta.
- No key inserts a newline in every terminal, and nothing on screen says
  which keys do.
- A draft taller than the composer's eight rows hides the cursor: the
  rows past the eighth are cut off instead of scrolled.

## What Changes

- **Keyboard disambiguation while Smith owns the screen.** When the
  terminal answers the kitty keyboard-protocol query, Smith pushes
  `DISAMBIGUATE_ESCAPE_CODES` after it enters the alternate screen. It pops
  the flags before it leaves, on every exit path: normal quit, error,
  panic, the exit-deadline backstop, and a second stop signal. Smith pops
  only what it pushed, and at most once, so the shell is never left in
  kitty mode. The query runs once per process, at the first entry, before
  any input reader exists. Release and repeat events stay off.
- **Legacy aliases keep their meaning.** With disambiguation on, Ctrl+[,
  Ctrl+I and Ctrl+M are reported as their own keys. Smith maps them back to
  Esc, Tab and Enter, as every terminal without the protocol reports them.
- **Ctrl+J inserts a newline in every terminal.** In raw mode, Ctrl+J is a
  line feed that every terminal reports the same way. Shift+Enter,
  Alt+Enter and `\` then Enter keep working. In the command palette,
  Shift+Enter and Alt+Enter insert a newline as they do in the composer
  instead of running the command.
- **Discoverable.** The shortcuts panel and `/help` gain a `Ctrl+J` row.
  While an idle draft is non-empty, the hint row reads
  `ctrl+j for newline`.
- **Tall drafts scroll.** The composer keeps the cursor row visible
  when a draft is taller than its eight rows.

## Impact

- Affected specs: `client-surfaces` (ADDED "The composer inserts line
  breaks in every terminal", ADDED "Keyboard disambiguation never outlives
  the screen")
- Affected code:
  - `crates/smith-cli/src/terminal.rs`: support query, push and pop,
    legacy aliases.
  - `crates/smith-cli/src/screen_runner.rs`: the event reader applies
    the aliases.
  - `crates/smith-tui/src/app/input.rs`: Ctrl+J, and newline keys in
    the palette.
  - `crates/smith-tui/src/render/composer.rs`: hint and draft
    scrolling.
  - `crates/smith-client/src/keymap.rs`: help row.
  - Help fixtures and `DESIGN.md`.
