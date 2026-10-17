---
created_at: 2026-10-17T00:11:27Z
updated_at: 2026-10-17T00:11:27Z
completed_at:
---

## 1. Capture
- [x] 1.1 Capture every model-related window before the change in tmux with
      a scratch HOME: `/model`, `/effort`, `/think`, `/advisor`, `/provider`,
      `/profile`, `/context`, `smith setup` → Change default, first run, and
      `/connect google` (`docs/qa/model-window-2026-10-16/before-*.txt`).

## 2. Implementation
- [x] 2.1 Compact picker drawing under a flow's own title, with a validation
      error kept above the choices, plus a bottom-anchored standalone variant
      (`picker/rendering.rs`).
- [x] 2.2 Setup's default-model step draws, measures, and stacks its controls
      through that path, embedded and standalone (`setup/rendering.rs`,
      `Screen::stacked_controls`, `draw_with_screen`).
- [x] 2.3 The default-model step opens on the current model unless the flow
      remembered a choice (`setup/flow.rs`).
- [x] 2.4 Setup model rows reuse the `/model` row builder; catalog rows use
      the same shape with the connecting provider's name
      (`smith-cli/src/resources.rs`, `setup/choices.rs`, `setup.rs`).
- [x] 2.5 Model, provider, profile, thinking, effort, and advisor pickers
      open on the current choice (`app/resources.rs`).

## 3. Tests
- [x] 3.1 Standalone default-model choice: bottom anchoring, five rows,
      position, selected detail, controls last, at 44/80/100 columns.
- [x] 3.2 Embedded default-model choice renders the same rows, position,
      identity row, and controls as `/model` at 44 and 100 columns.
- [x] 3.3 Preselection and remembered pick on Back; validation error stays
      visible in the compact pane.
- [x] 3.4 Runtime pickers open on the current choice; connect and resume do
      not.
- [x] 3.5 Setup and catalog rows match the `/model` row builder; setup keeps
      every listed pair selectable.

## 4. Verification
- [x] 4.1 Capture the same windows after the change
      (`docs/qa/model-window-2026-10-16/after-*.txt`).
- [ ] 4.2 Focused tests, clippy, fmt, and the full gate.
