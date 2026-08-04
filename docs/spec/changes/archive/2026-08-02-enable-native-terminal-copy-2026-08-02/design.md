## Context

Crossterm's mouse capture is terminal-wide. Once enabled, the terminal reports
the initial press and drag sequence to Smith instead of starting its own text
selection. Smith may ignore events outside the composer, but ignoring them
after receipt cannot return the gesture to the terminal.

The existing product contract already makes the keyboard the only required
input. Mouse wheel scrolling and click-to-position are optional conveniences;
native selection and copy apply to every visible region and are more valuable
for a transcript-first coding surface.

## Goals / Non-Goals

### Goals

- Restore ordinary terminal drag selection and copy across every Smith
  terminal surface.
- Preserve all required composer, transcript, picker, modal, and setup
  operations through their existing keyboard paths.
- Keep terminal entry and restoration symmetrical without emitting redundant
  mouse-mode control sequences.

### Non-Goals

- Implement Smith-owned text ranges, selection highlighting, or clipboard
  writes.
- Support region-scoped mouse capture; common terminal protocols do not expose
  such a mode.
- Change bracketed paste, alternate-screen ownership, or keyboard shortcuts.

## Decisions

### Native selection owns the pointer

Smith will not enable mouse reporting during terminal entry. The host event
loop will stop routing mouse events, and TUI state will no longer retain a
composer pointer rectangle solely for click-to-position behavior. Terminal
drag selection and the terminal's normal copy command remain outside Smith's
clipboard and security boundary.

Alternative considered: keep mouse reporting and ignore clicks outside the
composer. Rejected because capture occurs before Smith receives the event, so
the terminal still cannot start native selection.

Alternative considered: implement application-owned selection and copy.
Rejected because wrapped transcript mapping, modal layering, clipboard
transport, remote sessions, and sensitive-text policy make that substantially
larger and less portable than restoring the terminal's native behavior.

## Risks / Trade-offs

- Mouse wheel transcript scrolling and click-to-position stop working inside
  Smith. `PageUp`, `PageDown`, `Home`, `End`, arrow/history controls, and
  keyboard cursor movement remain available.
- Terminal-native selection details vary by terminal emulator, but disabling
  reporting is the portable prerequisite and avoids adding clipboard access.
- A terminal may clear a selection when later frames rewrite selected cells;
  completed or otherwise stable screen content remains directly selectable.

## Migration Plan

- Update `DESIGN.md` before changing behavior.
- Remove mouse capture from shared terminal entry/restore.
- Remove unused mouse event routing and pointer-only TUI state/tests.
- Add focused assertions for emitted terminal modes where practical and run
  the Smith workspace format, Clippy, and test gates.

## Open Questions

None for proposal approval.
