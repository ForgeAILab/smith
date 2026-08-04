---
created_at: 2026-07-27T04:52:54Z
updated_at: 2026-07-27T05:50:19Z
---

## Why

Smith's current TUI exposes internal regions and focus state that do not help
the primary coding loop, while the product has no coherent way to inspect,
review, undo, or selectively revert workspace changes. The desired experience
is a single conversational surface with command-driven control and explicit,
safe recovery.

## What Changes

- **BREAKING** Replace composer/transcript/inbox focus cycling with a
  composer-first interaction model. Transcript scrolling is global,
  background activity renders inline, and `Tab` is reserved for command-menu
  completion rather than region navigation.
- Make slash commands the canonical control surface. Typing `/` opens a
  filterable command menu; `Ctrl+P` opens the same registry instead of a
  separate palette implementation.
- Keep the initial command set intentionally small: existing session and model
  controls plus `/status`, `/diff`, `/review`, `/undo`, `/revert`, and
  `/agent`.
- Add Git-aware change inspection and recovery. `/diff` inspects the current
  checkout, `/review` starts a read-only review, `/undo` reverses the last
  fully attributable Smith turn, and `/revert` lets the user select an exact
  file or hunk from the current diff.
- Require previews, explicit confirmation, post-image conflict checks,
  recoverable handling of untracked files, and journaled outcomes. Smith never
  uses broad reset or checkout operations to implement recovery.
- Update `DESIGN.md` before implementation so the simplified focus, command
  menu, inline activity, diff/review view, and recovery confirmations become
  the visual and interaction contract.

## Impact

- Affected specs: `client-surfaces`, new `change-review`
- Affected code: `DESIGN.md`, `crates/smith-tui`, `crates/smith-cli`,
  `crates/smith-runtime`, session journal/change attribution, Git inspection
  and reverse-patch support
- Active-change coordination: this change depends on
  `add-smith-agent-harness-2026-07-23` and
  `add-smith-slash-commands-2026-07-26`. Once approved, its single-focus and
  command-menu requirements supersede the older visible region-focus and
  separate-palette details; the underlying bounded safe-boundary inbox remains
  an internal delivery mechanism.
