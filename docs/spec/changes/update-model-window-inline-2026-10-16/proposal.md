---
created_at: 2026-10-17T00:11:27Z
updated_at: 2026-10-17T06:23:25Z
---

## Why

The owner asked on 2026-10-16: "update the model window — it should be the
same as the model picker, just show at the bottom."

Captured frames (`docs/qa/model-window-2026-10-16/before-*.txt`) show one
model window that is not the `/model` picker: setup's "Choose default model"
step.

- Standalone (`smith setup` → Change default) it fills the alternate screen
  from the top. Its rows are `remote/example-model  profiles: dev` instead of
  `example-model  remote · 128k context`, there is no selected-row detail,
  and the cursor starts on the first row instead of the current model.
- Embedded (`/connect google` → catalog model step) it is already above the
  composer, but with a blank row under its title, each row's description
  is the whole limit line cut off with `…`, there is no selected-row detail,
  and its keys share the identity row.

The runtime `/effort`, `/think`, `/advisor`, `/provider`, and `/profile`
pickers already use the `/model` pane, but unlike `/model` they open on
their first row instead of the current choice
(`before-effort-current-high.txt`: `❯ provider default` while `high` is
`✓ current`).

## What Changes

- Setup's default-model step draws through the same compact picker
  renderer `/model` uses: one title row with filter and position, at most
  five rows, and the selected row's facts.
- Its rows come from the same builder as `/model` rows. Catalog rows for
  `/connect` use the same shape: model name, `provider · N context`, then
  limits and `provider/<model-id>` beneath the selected row.
- In a session it sits above the composer with the identity footer above
  its controls row, as `/model` does. Before a session it sits at the
  bottom of the screen with its controls on the last row.
- It opens on the current model until the flow remembers a choice of its
  own, so Back still returns to what the user picked.
- The model, provider, profile, thinking, effort, and advisor pickers open
  on the current choice. Connect, disconnect, resume, account, and
  reference pickers still open on their first row.

Unchanged: what each chooser selects and writes, Escape going back a step in
setup and cancelling elsewhere, and setup's ability to pick any listed pair
as the default (a row `/model` marks unavailable stays selectable in setup,
as before). Setup's other steps keep their top-left placement (superseded
by `update-model-window-followups-2026-10-17`, which puts every pre-session
screen at the bottom).

Considered and left alone: `/context` lists the active model's named context
windows inside its usage report and switches with `/context NAME`. It has
no picker, so giving it one would be a new interaction rather than a change
in presentation. It is flagged for the owner instead. (Superseded: the owner
approved a `/context` picker, specified in
`update-model-window-followups-2026-10-17`.)

## Impact

- Affected specs: `client-surfaces`
- Affected code: `crates/smith-tui/src/picker/rendering.rs` (compact picker
  under a flow title, bottom-anchored standalone variant),
  `crates/smith-tui/src/setup/{rendering,flow}.rs`,
  `crates/smith-tui/src/screen.rs` and `render/layout.rs` (stacked controls
  for embedded screens), `crates/smith-tui/src/app/resources.rs`
  (preselection), `crates/smith-cli/src/resources.rs` (shared model row
  builder), `crates/smith-cli/src/setup/choices.rs` and `setup.rs` (setup and
  catalog rows)
