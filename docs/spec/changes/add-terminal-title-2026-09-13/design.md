# Design: Dynamic terminal window title for the interactive TUI

## Mechanism

The window/tab title is set by writing an OSC 0 sequence to the real stdout:

```
ESC ] 0 ; <sanitized title> BEL      (bytes: 0x1b 5d 30 3b ... 07)
```

Choices, following the Codex implementation this change ports
(`codex-rs/tui/src/terminal_title.rs`):

- **BEL terminator, not ST.** Some terminal integrations expose the ST
  terminator (`ESC \`) in process decorations; BEL avoids the artifact while
  being universally accepted.
- **Direct byte write + flush, no crossterm command.** Crossterm's built-in
  `SetTitle` uses ST, and a custom `Command` adds no value over writing the
  bytes ourselves. The write is interleaved with ratatui's backend on the
  same stdout; OSC title sequences are order-insensitive with respect to
  frame content, which is why Codex writes them the same way.
- **`stdout().is_terminal()` guard.** When stdout is piped (tests,
  `smith -p`, CI), `set_terminal_title` reports Applied without writing and
  `clear_terminal_title` is a no-op.

## Sanitization

Title inputs are untrusted-derived: the project root path name and the
model/provider ids come from configuration and the filesystem. Before the
payload enters the OSC sequence:

1. Drop control characters (they could terminate or reshape the sequence).
2. Drop Trojan-Source bidi controls and invisible formatting codepoints
   (`U+00AD`, `U+061C`, `U+200B`–`U+200F`, `U+202A`–`U+202E`, `U+2060`–`U+206F`,
   variation selectors, `U+FEFF`, `U+FFF9`–`U+FFFB`, `U+1BCA0`–`U+1BCA3`,
   `U+E0100`–`U+E01EF`) so a title cannot render misleadingly relative to its
   bytes.
3. Collapse whitespace runs to a single space; strip leading/trailing space.
4. Truncate to 240 visible chars (terminal truncation headroom).

If nothing visible remains, `set_terminal_title` returns
`NoVisibleContent` and the driver clears the managed title — the same policy
Codex chose: an honest empty rather than a stale wrong title.

## Title content

`title_from_status(&Status) -> String` joins available segments with ` · `:

- fixed prefix `smith`
- `Status::project` (already the compact project display name)
- `Status::model`
- an activity label while `Status::activity` is `Working`, `Interrupting`, or
  `ParkedAwaitingChild` (rendered as the existing status vocabulary; no
  label while `Idle`, nothing new to say after `Ended`)

Missing/empty segments are skipped rather than rendered as blanks. The format
is fixed in this change; making segments configurable is deliberately future
work.

## Refresh and teardown policy

- The driver recomputes `title_from_status` on the existing paths that
  already observe state (event batches that update the app, and the tick
  branch) — no new timers or wakeups.
- A last-written cache dedupes: the OSC write happens only when the rendered
  text differs from the last write (or the last action was a clear). This
  keeps the per-tick cost one string comparison.
- After `run_tui` returns and before `terminal.restore()`, the driver issues
  one clear (empty OSC 0) if it ever wrote a title. Smith does not attempt to
  restore the pre-Smith title: reading it back is not portable; the shell's
  prompt hook reasserts its own title on the next prompt, as it does for
  every terminal application that manages its title.

## Test seams

- `sanitize` and `title_from_status` are pure functions with unit tests,
  including the control-character/bidi cases and the 240-char bound.
- The OSC write path writes to `impl std::io::Write`, so tests assert exact
  bytes (`\x1b]0;…\x07`) against a buffer; only the production wrapper binds
  real stdout behind the `is_terminal` guard.
- Driver-level behavior (dedupe, single clear on exit, untouched on
  headless) is asserted with the existing end-to-end/kit tests where the
  event loop is drivable.
