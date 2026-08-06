---
created_at: 2026-08-05T00:00:00Z
updated_at: 2026-08-05T00:00:00Z
completed_at:
---

## 1. Quiet the root transcript

- [x] 1.1 Keep spawned, resume-started, needs-input, interrupted, completed,
  stopped, and failed as transcript notices; stop printing `is working`,
  `ran <tool>`, and durable `recovered`.
- [x] 1.2 Append every lifecycle event to a bounded per-child log
  (`MAX_CHILD_LOG_LINES`), oldest line first, with no wall-clock text so live
  application and journal replay stay comparable.

## 2. Keyboard selection

- [x] 2.1 `Down` selects the next delegated child once composer history has
  nowhere left to go; `Up` walks back and returns to the root timeline from the
  first child.
- [x] 2.2 Keyboard order follows the panel's live-first order through one
  shared `ChildSummary::is_live` predicate.
- [x] 2.3 `Esc` leaves the inspector before it can mean interrupt.
- [x] 2.4 Reset scroll on every selection change so neither view opens
  part-way up.

## 3. The inspector view

- [x] 3.1 Replace the transcript region with the selected child's heading,
  coordinator card, and log; `Esc` restores the root timeline unchanged.
- [x] 3.2 Replace the identity footer with `↑↓ agents · esc back to main ·
  enter continues <child>` while the view is open.
- [x] 3.3 An ordinary submission while inspecting becomes a follow-up to that
  child through the existing confirmation; `/` and `!` still address the root.
- [x] 3.4 A child with no activity in this process says so rather than
  rendering an empty pane.

## 4. Host-supplied child accounting

- [x] 4.1 Refresh the inspected child's coordinator card on the existing
  poll-on-redraw in the interactive driver.
- [x] 4.2 Discard a card whose child is no longer the one on screen.
- [x] 4.3 `/agent <id>` opens the inspector instead of printing a detail block
  the swapped region would hide.

## 5. Docs and truth specs

- [x] 5.1 Update `DESIGN.md` section 8 and the keyboard table.
- [x] 5.2 Add the `/help` composer line for agent navigation.
- [x] 5.3 Land the `child-agents` and `client-surfaces` deltas.

## 6. Verification

- [x] 6.1 `cargo test -p smith-tui` (322 passed) covering: progress stays out
  of the transcript and in the log, bounded log tail, arrow walk and Esc,
  history keeps the arrows until it runs out, follow-up while inspecting,
  `/status` while inspecting, stale-card rejection, and the render swap.
- [x] 6.2 `cargo clippy --workspace --all-targets` and `cargo fmt --check`.
