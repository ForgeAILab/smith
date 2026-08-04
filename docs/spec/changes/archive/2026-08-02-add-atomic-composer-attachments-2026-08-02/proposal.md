---
created_at: 2026-08-02T18:42:57Z
updated_at: 2026-08-02T19:06:30Z
---

## Why

Smith already collapses large text pastes and clipboard images behind compact
composer labels, but those labels remain ordinary strings in the editing
buffer. Cursor movement and deletion therefore step through every visible
character even though each label represents one indivisible piece of input.

## What Changes

- Treat every registered `[Pasted text #N +L lines]` and `[Image #N W×H]`
  composer placeholder as one logical editing unit while continuing to render
  its complete label.
- Make `Left` and `Right` cross a registered placeholder in one key press, and
  make `Backspace` or `Delete` remove the entire placeholder when invoked from
  the adjacent boundary.
- Keep ordinary Unicode text character-addressable and keep manually typed
  paths or placeholder-shaped text ordinary; only a label backed by registered
  paste or clipboard-image material receives atomic behavior.
- Keep pasted-text labels only while input is editable or pending. Once the
  message is committed, render the exact original pasted text in the user
  transcript instead of `[Pasted text #N +L lines]`.
- Preserve the image distinction: a real clipboard image remains image content
  ordered by its placeholder, and its `[Image #N W×H]` label remains visible
  after send; typed paths and lookalikes remain raw text.
- Add focused composer, reducer, rendering, and submission regression tests,
  and document the keyboard contract in `DESIGN.md`.

## Impact

- Affected specs: `client-interaction`
- Affected code: `crates/smith-tui/src/composer.rs`,
  `crates/smith-tui/src/app/input.rs`,
  `crates/smith-tui/src/app/pending_input.rs`, composer rendering/tests, and
  `DESIGN.md`
- Integrates with the completed composer-history and pending-input work by
  preserving compact labels before commitment and cloned out-of-band material,
  then using a separate expanded user-transcript projection at commitment.
- No provider protocol, persistence schema, image encoding, file-reference,
  paste threshold, or clipboard shortcut changes are introduced.
