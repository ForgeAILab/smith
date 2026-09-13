# Proposal: Dynamic terminal window title for the interactive TUI

## Why

Every Smith TUI session today leaves the terminal window/tab title at whatever
the shell last set, so a user running several Smith sessions — different
projects, different models, different branches of work — sees N identical tabs
and must guess which is which. The differentiation data already exists in the
TUI's own status model (`crates/smith-tui/src/status.rs`: project, model,
activity), but nothing in `crates/smith-tui` or `crates/smith-cli` writes a
title; a repository-wide search for `set_title`/OSC title sequences finds no
product code.

The Codex CLI demonstrates the standard solution: write an OSC window-title
sequence (`\x1b]0;<title>\x07`) to stdout while the TUI runs, sanitize the
payload because its inputs are untrusted, dedupe writes, and clear the managed
title on exit (`codex-rs/tui/src/terminal_title.rs`). This change ports that
mechanism — not its configuration surface — to Smith.

## What Changes

- A new `terminal_title` module in `smith-tui` owning the OSC 0 + BEL write
  path: an `is_terminal` guard so piped/headless stdout is never touched,
  sanitization (control characters, Trojan-Source bidi/invisible codepoints,
  whitespace collapsing, a 240-char bound), an explicit
  Applied/NoVisibleContent result, and a clear-on-exit write.
- A pure `title_from_status(&Status) -> String` renderer producing a fixed
  format: `smith · <project> · <model>`, plus a short activity label while a
  turn is in flight. All inputs come from the existing status model; no new
  state is tracked.
- `tui_driver` wiring: recompute the title on the existing event/tick paths,
  write only when the rendered text changed (last-written cache), and clear
  the managed title once after `run_tui` returns, before the terminal is
  restored. Short-lived terminal surfaces (setup, pickers, login) never set a
  title and are unaffected.
- Headless `smith -p` is not wired to the module at all, and the `is_terminal`
  guard is a second defense: machine stdout must stay free of OSC bytes.

Out of scope: configurable title segments (a future `tui.terminal_title`
list), spinner animation in the title, git-branch/session-id segments,
restoring the pre-Smith title (not portable across terminals; the shell's own
prompt hook reasserts it).

## Impact

- Affected specs: `client-surfaces` (one added requirement)
- Affected code: new `crates/smith-tui/src/terminal_title.rs`;
  `crates/smith-cli/src/tui_driver.rs` (refresh + clear wiring); focused unit
  tests in both crates
- No configuration, persistence, wire-protocol, credential, approval, or
  authority changes; no new dependencies (OSC bytes are written directly, the
  `is_terminal` check is `std`)
- Security: title payload derives from untrusted text (project path segments,
  model ids), so sanitization before the OSC write is mandatory and unit-tested

## Approval Boundary

Approval authorizes exactly the module, the fixed-format renderer, and the
driver wiring described above. It does not authorize new configuration keys,
animation timers or periodic wakeups beyond existing ticks, additional title
segments, changes to terminal enter/leave semantics beyond the one clear
write, or any headless-output behavior change.
