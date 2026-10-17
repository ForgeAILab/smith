---
created_at: 2026-10-17T06:23:25Z
updated_at: 2026-10-17T06:56:42Z
completed_at: 2026-10-17T06:56:42Z
---

## 1. Capture
- [x] 1.1 Capture the affected windows before the change in tmux with a
      scratch HOME: setup welcome, a credential method list, a key field,
      the review, Change default with an installed-agent profile configured,
      `/model` with a windows-only model, and `/context` on a model with
      several windows (`docs/qa/model-window-followups-2026-10-17/before-*.txt`).

## 2. Named-window models
- [x] 2.1 Failing test first: a windows-only active model whose windows omit
      `max_input_tokens`, and a second windows-only model that is not active,
      are both missing from the inventory and from `/model`.
- [x] 2.2 The inventory previews each windows-only model through its
      selected or default window and derives a missing input limit as
      context minus output (`smith-config/src/inventory.rs`).

## 3. `/context` picker
- [x] 3.1 `ResourceTarget::ContextWindow`: title, entries, opening on the
      current window, Enter applying `SelectionCommand::ContextWindow`.
- [x] 3.2 Bare `/context` keeps the report and also opens the picker when
      idle with more than one window; `/context NAME` and `default` unchanged.

## 4. Setup agent rows
- [x] 4.1 Setup's default-model rows filter provider-qualified agent pairs and
      append the `/model` agent rows, keeping availability and marking the
      active agent current (`smith-cli/src/resources.rs`,
      `setup/choices.rs`).
- [x] 4.2 An agent row carries the current default's provider, or the first
      configured provider when that is unavailable, so Change default writes
      a provider and the `cli/<kind>/<model>` id.

## 5. Bottom-anchored pre-session screens
- [x] 5.1 The shared standalone layout anchors title, content, and controls
      to the bottom of the screen; the picker heading and setup's field
      cursor follow it (`picker/rendering.rs`).
- [x] 5.2 Regenerate and review the standalone screen fixtures.

## 6. Tests and verification
- [x] 6.1 Tests for 2–5: inventory and `/model` rows, the `/context` picker
      reducer, setup agent rows and the written pair, bottom placement at
      44 and 100 columns.
- [x] 6.2 Capture the same windows after the change
      (`docs/qa/model-window-followups-2026-10-17/after-*.txt`).
- [x] 6.3 Targeted tests, clippy, fmt, and the no-default-features build.
