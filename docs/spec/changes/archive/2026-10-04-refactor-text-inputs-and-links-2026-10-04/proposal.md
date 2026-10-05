---
created_at: 2026-10-05T01:34:57Z
updated_at: 2026-10-05T02:58:07Z
---

## Why

Two items remain from the 2026-10-02 audit roadmap and were approved to fix
before pushing 0.3.7 ("i think lets fix everything then push", 2026-10-04):

- S5's text inputs: the composer has full line editing, but picker filters,
  setup fields (including masked key fields), and history search are
  push/pop strings. A typo in a filter or a pasted key can only be fixed by
  deleting back to it.
- OSC 8 links, deferred in 0.3.1 and 0.3.5 because the renderer could not
  emit them without bypassing its cell buffer and Smith had no terminal
  capability detection (DESIGN.md: "OSC 8 hyperlinks are emitted only when
  the terminal is known to support them").

## What Changes

- One single-line input value in `smith-tui` owns text, cursor, and the
  line-editing keys; picker filters, setup fields (plain and masked), and
  history search use it, and the composer's line operations are the same
  code applied to its current line.
- Terminal capability detection for OSC 8 from the environment (known
  terminals only; off inside multiplexers that do not pass hyperlinks
  through, and off when unknown).
- Transcript links (Markdown links and bare `http(s)` URLs) carry OSC 8
  hyperlinks when supported, written so cell widths, wrapping, selection,
  copy, and fixtures are unchanged.

## Impact

- Affected specs: client-surfaces, code-organization.
- Affected code: `smith-tui` `composer.rs`, `picker.rs`, `setup.rs`,
  `app/input.rs` (history search), `render/markdown.rs`, `selection.rs`,
  `theme.rs`; `smith-cli` theme construction.
- No configuration or session format changes.
