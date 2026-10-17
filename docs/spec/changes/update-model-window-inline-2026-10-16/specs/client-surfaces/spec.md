## MODIFIED Requirements

### Requirement: Accessible shared resource picker

Every setup/runtime/session chooser SHALL render through one inline list: a
title line, the rows, and one footer, with no frame and no centering. It SHALL
support Up/Down selection, Enter confirmation, Escape to go back or cancel,
scrolling with a position count, active and disabled labels, narrow
terminals, no color, and reduced motion. Inventories (models, providers,
profiles, sessions, connections) SHALL filter on typing; fixed choice lists of
at most nine entries SHALL be numbered and accept the digit instead. Filtering
and selection MUST operate on bounded display metadata and MUST NOT add
picker contents to canonical model history. Every chooser MUST use the same
footer words: `↑↓ choose · enter confirm · esc back`, or `esc cancel` where
there is no earlier step; a numbered list says `↑↓ or 1–N choose`, and an
empty list whose guidance names its key shows no footer.

Runtime pickers SHALL render directly above the fixed composer with at most
five matching rows visible. They MUST preserve the transcript region instead
of drawing a centered modal over it. The model, provider, profile, thinking,
effort, and advisor pickers SHALL open on the current choice, so Enter keeps
it; lists of actions or destinations (connect, disconnect, resume, account,
file and agent references) keep opening on their first row.

Setup's default-model chooser (setup's Change default, and the catalog model
step of `/connect`) SHALL be the `/model` picker in a different place: the
same title row with filter and position, at most five rows in the `/model`
row text (model name, `provider · N context`, `✓ current`), the selected
row's facts beneath it, opening on the current model until the flow
remembers a choice of its own. Inside a session it SHALL sit directly above
the composer with the identity footer above its controls row, as `/model`
does. Before a session exists it SHALL sit at the bottom of the screen with
one blank row and then its single controls row last. Escape still returns to
the previous setup step, and the chooser selects and writes exactly what it
did before.

Other choosers shown before a session exists SHALL draw from the top-left of
the screen, sized to their content.

#### Scenario: Runtime picker preserves the coding surface

- **GIVEN** the interactive coding surface has transcript history
- **WHEN** the user opens model, provider, profile, resume, file, or agent
  selection
- **THEN** Smith temporarily replaces the todo presentation with at most five
  matching choices directly above the fixed composer
- **AND** moving through a larger inventory scrolls that pane without covering
  or adding content to the transcript
- **AND** closing the picker restores the unchanged todo projection
- **AND** the identity footer remains visible while the composer and footer
  keep their existing screen rows

#### Scenario: Model picker opens on the current model

- **GIVEN** `zai/glm-5.3` is active and the inventory holds hundreds of
  models
- **WHEN** the user submits `/model`
- **THEN** the selection starts on `zai/glm-5.3` with its position shown
- **AND** the selected row's detail uses compact sizes, for example
  `1M context · 128k output`

#### Scenario: Runtime selections open on their current choice

- **GIVEN** the session's effort is `high`, and a provider, profile, thinking
  state, and advisor choice are each marked current
- **WHEN** the user opens `/effort`, `/provider`, `/profile`, `/think`, or
  `/advisor`
- **THEN** the selection starts on the row marked `✓ current`
- **AND** pressing Enter applies that current choice rather than the first row
- **AND** `/connect` and `/resume` still open on their first row

#### Scenario: Connection model step is the model picker

- **GIVEN** a session is idle and the user runs `/connect google`
- **WHEN** authentication is chosen and the catalog model step opens
- **THEN** the step shows `Connect Google Gemini · Choose default model` with
  `type to filter` and its position on one title row, directly above the
  composer
- **AND** each row shows the model name and `google · N context`, with the
  selected row's limits and `google/<model-id>` beneath it, as `/model`
  shows a configured model
- **AND** the identity footer sits above `↑↓ choose · enter confirm · esc
  back`
- **AND** at most five models are visible and typing filters them

#### Scenario: Default-model choice before a session sits at the bottom

- **GIVEN** no session exists and the user runs `smith setup` and chooses
  Change default
- **WHEN** the default-model choice opens
- **THEN** its title, at most five model rows, and the selected row's facts
  are drawn at the bottom of the screen, followed by one blank row and the
  controls row as the last row
- **AND** the selection starts on the current default model, marked
  `✓ current`
- **AND** choosing a model and pressing Escape on the review returns to the
  chooser on that chosen model
- **AND** nothing is written until the review is confirmed

#### Scenario: Standalone chooser before a session

- **GIVEN** no session exists yet
- **WHEN** setup, `smith --resume`, or ChatGPT login shows any choice other
  than the default model
- **THEN** its title is on the first row at the left gutter and its rows
  follow directly beneath
- **AND** no frame, centered box, or blank filler rows are drawn

#### Scenario: Numbered fixed choices

- **GIVEN** a chooser offers four credential storage methods
- **WHEN** the user presses `3`
- **THEN** the third method is chosen as if selected and confirmed
- **AND** letters typed on that list do not filter it

#### Scenario: Labels align over the whole list

- **GIVEN** an inventory whose longest label is below the visible rows
- **WHEN** the user scrolls to it
- **THEN** descriptions stay in the same column as before scrolling
- **AND** no row repeats its label as its description

#### Scenario: Filter a long model list

- **GIVEN** the model inventory is longer than the visible picker height
- **WHEN** the user types a provider or model substring
- **THEN** the visible choices filter and remain scrollable with selection in
  view
- **AND** no provider request or model-history entry is produced

#### Scenario: Picker renders without color

- **GIVEN** color and motion are disabled
- **WHEN** a picker contains active, selectable, disabled, and filtered entries
- **THEN** each state remains distinguishable through text and cursor markers
- **AND** controls remain visible in a narrow terminal
