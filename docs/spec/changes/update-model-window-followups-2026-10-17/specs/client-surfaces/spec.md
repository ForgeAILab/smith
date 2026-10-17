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
effort, advisor, and context-window pickers SHALL open on the current choice,
so Enter keeps it; lists of actions or destinations (connect, disconnect,
resume, account, file and agent references) keep opening on their first row.

Setup's default-model chooser (setup's Change default, and the catalog model
step of `/connect`) SHALL be the `/model` picker in a different place: the
same title row with filter and position, at most five rows in the `/model`
row text (model name, `provider · N context`, `✓ current`), the selected
row's facts beneath it, opening on the current model until the flow
remembers a choice of its own. Inside a session it SHALL sit directly above
the composer with the identity footer above its controls row, as `/model`
does. Before a session exists it SHALL sit at the bottom of the screen with
one blank row and then its single controls row last. Escape still returns to
the previous setup step. A provider model row selects and writes exactly what
it did before; installed-agent rows follow the requirement `Setup lists
installed agents as /model does`.

Every other screen shown before a session exists (setup's other steps, the
ChatGPT and xAI login screens, the connection review, and the
`smith --resume` picker) SHALL sit at the bottom of the screen, sized to its
content: its title, its content, one blank row, and its controls, with the
last controls row on the last row of the screen. Content taller than the
screen SHALL use the whole screen and scroll as before.

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

#### Scenario: Standalone screen before a session sits at the bottom

- **GIVEN** no session exists yet
- **WHEN** setup, `smith --resume`, or ChatGPT login shows any step other
  than the default model
- **THEN** its last controls row is the last row of the screen, with one
  blank row above it
- **AND** its title and content sit directly above that blank row, with every
  row above the title left blank
- **AND** no frame, centered box, or blank filler rows are drawn between its
  rows

#### Scenario: Long setup review keeps the whole screen

- **GIVEN** a 44×16 terminal and a setup review longer than the screen
- **WHEN** the review is shown
- **THEN** its title is on the first row and its controls on the last row
- **AND** the review scrolls between them with its position shown

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

### Requirement: Context window command

The TUI SHALL accept `/context <NAME|default>` to set or clear the session's
context window override. It SHALL apply the change at the next idle boundary
and persist it on resume. `/context` with no argument SHALL keep its current
report and add the available windows, marking the active one. When the
session is idle and the active model has more than one named window,
`/context` with no argument SHALL also open a `Choose context window` picker
directly above the composer, in the compact pane `/model` uses, opening on
the current window. Enter SHALL switch to the chosen window as `/context
NAME` does, and Escape SHALL close the picker and leave the report. Headless
and machine `/context` SHALL keep returning only the report.

#### Scenario: Switch window while idle

- **GIVEN** the active model declares windows `272k` and `872k`
- **WHEN** the user runs `/context 872k` while the session is idle
- **THEN** the runtime is rebuilt with the `872k` limits
- **AND** the status line shows `872k`

#### Scenario: Model has one window

- **GIVEN** the active model declares no windows
- **WHEN** the user runs `/context 872k`
- **THEN** the TUI reports that the model has no selectable windows
- **AND** the session is unchanged

#### Scenario: Resume keeps the selection

- **GIVEN** a session ran with the `/context 872k` override
- **WHEN** that session is resumed
- **THEN** the `872k` window is still active

#### Scenario: Bare context offers the window picker

- **GIVEN** the session is idle and the active model declares windows
  `128k`, `1m`, and `32k`, with `128k` active
- **WHEN** the user submits `/context`
- **THEN** the usage report is appended to the transcript as before
- **AND** `Choose context window` opens directly above the composer with
  `128k` selected and marked `✓ current`
- **AND** moving to `1m` and pressing Enter switches the window as
  `/context 1m` does

#### Scenario: Bare context without a choice to make

- **GIVEN** the active model declares one window or none, or a turn is
  running
- **WHEN** the user submits `/context`
- **THEN** only the usage report is appended
- **AND** no picker opens

## ADDED Requirements

### Requirement: Models configured with named windows are listed

The selection inventory SHALL list a model configured only with named
context windows wherever configured models are listed (`/model`, `/provider`
counts, and setup's default-model choice), with the limits the runtime would
apply: the session's selected window for the active model, otherwise the
model's `default_context_window`. A window without `max_input_tokens` MUST
preview its input limit as its context minus the model's output ceiling, as
the runtime derives it.

#### Scenario: Active windows-only model is listed and current

- **GIVEN** `remote/window-model` declares windows `128k`, `1m`, and `32k`
  with only `context_tokens`, `default_context_window = "128k"`, and
  `max_output_tokens = 8192`
- **AND** it is the active model
- **WHEN** the user opens `/model` and types `window`
- **THEN** `window-model  remote · 128k context` is listed and marked
  `✓ current`
- **AND** its detail shows the `119.8k` input limit the runtime derives from
  the `128k` window

#### Scenario: Inactive windows-only model uses its default window

- **GIVEN** `remote/wide-model` declares only named windows with
  `default_context_window = "272k"`
- **AND** another model is active
- **WHEN** the user opens `/model` or setup's Change default
- **THEN** `wide-model  remote · 272k context` is listed and selectable

### Requirement: Setup lists installed agents as /model does

Setup's default-model choice SHALL list installed coding agents with the
`/model` agent rows: one row per agent model in the curated
`cli/<kind>/<model>` namespace, with the same name, description, detail, and
installation check, and no provider-qualified row for the same agent model.
Choosing an agent row MUST write the `cli/<kind>/<model>` model with the
current default's provider, as `/model` keeps the current provider, or with
the first usable configured provider when the current default's provider is
not usable. Provider model rows keep setup's rule that every listed pair can
become the default.

#### Scenario: Agent rows read as in /model

- **GIVEN** profile `cc` selects `google/cli/claude-code/opus` and `claude` is
  on `PATH`
- **WHEN** the user runs `smith setup` and chooses Change default
- **THEN** `opus  Claude Code CLI · 200k context` appears once
- **AND** no row reads `cli/claude-code/opus  google · 200k context`
- **AND** an agent whose program is not on `PATH` is shown unavailable with
  the reason `/model` gives

#### Scenario: Choosing an agent keeps the current provider

- **GIVEN** the current default is `remote/example-model`
- **WHEN** the user chooses `sonnet  Claude Code CLI` in Change default and
  confirms the review
- **THEN** the review names `remote/cli/claude-code/sonnet`
- **AND** the written default profile selects provider `remote` and model
  `cli/claude-code/sonnet`
