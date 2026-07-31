## ADDED Requirements

### Requirement: Guided first-run setup

Running `smith` without a prompt in an interactive terminal SHALL automatically
open guided setup when configuration readiness is genuinely unconfigured.
`smith setup` SHALL expose the same flow explicitly. The flow MUST guide the
user through an action, provider, authentication, model, default selection,
and non-secret review, with keyboard-accessible Back and Cancel actions.

#### Scenario: Fresh interactive launch

- **GIVEN** startup readiness is unconfigured
- **AND** stdin and stderr are attached to an interactive terminal
- **WHEN** the user runs `smith`
- **THEN** Smith opens provider setup instead of printing a missing-provider
  error
- **AND** explains that no agent session or provider request exists yet

#### Scenario: User completes setup

- **GIVEN** the user has reviewed valid provider, credential-reference, model,
  and limit choices
- **WHEN** they confirm and full preflight succeeds
- **THEN** setup closes and Smith starts the ordinary TUI with the resolved
  provider/model
- **AND** the first agent session uses the same runtime factory as every other
  interactive run

#### Scenario: User cancels setup

- **GIVEN** setup is open at any step
- **WHEN** the user chooses Cancel
- **THEN** Smith restores the terminal and exits successfully without starting
  a session
- **AND** no setup config or credential change is committed

#### Scenario: User chooses GLM quick start

- **GIVEN** Smith is unconfigured
- **WHEN** the user chooses Quick start with GLM
- **THEN** setup preselects the reviewed Z.AI endpoint, GLM model, and trusted
  limits
- **AND** review states that a reasoning-only GLM completion is treated as
  visible assistant text without disabling model thinking
- **AND** asks only for credential enrollment and final confirmation before
  preflight

### Requirement: Reusable setup commands

Smith SHALL expose `smith setup add-provider` and
`smith setup add-model --provider <name>` as reusable interactive entry points.
Running `smith setup` without an action SHALL present equivalent choices for
GLM quick start, adding a provider, adding a model to an existing provider, and
changing the default profile/model.

#### Scenario: Add provider command

- **GIVEN** Smith already has a usable default configuration
- **WHEN** the user runs `smith setup add-provider`
- **THEN** the flow collects a distinct provider, authentication, and first
  usable model
- **AND** reviews the additive user-config change without starting a session

#### Scenario: Add model command

- **GIVEN** provider `acme` exists
- **WHEN** the user runs `smith setup add-model --provider acme`
- **THEN** the flow skips provider creation and collects a model plus its
  enforceable limit provenance
- **AND** lets the user choose whether to make it the default

### Requirement: Pre-runtime setup boundary

Smith SHALL keep setup behind a pre-runtime boundary. The setup surface MAY
enter the terminal before normal run configuration exists, but it MUST NOT
construct a runtime, session, approval channel, tool
registry, journal, or provider transport. Every setup exit path MUST restore
the terminal. A normal host may start only after persisted setup passes full
preflight.

#### Scenario: Setup is awaiting authentication input

- **GIVEN** the user is on the authentication step
- **WHEN** Smith renders or edits the masked field
- **THEN** no runtime session or persistence journal exists
- **AND** no provider request, tool call, or approval prompt can occur

#### Scenario: Setup operation fails

- **GIVEN** setup entered the alternate screen
- **WHEN** a credential, config, or preflight operation returns an error
- **THEN** the surface renders a bounded actionable error or exits through its
  guarded terminal lifecycle
- **AND** the shell is restored before Smith returns

### Requirement: Non-interactive setup refusal

Headless, piped, and machine-output runs MUST NOT open interactive setup or
mutate configuration. When such a run is unconfigured, Smith SHALL return a
stable non-success outcome that names the missing setup and points to
`smith setup` or the existing explicit configuration inputs.

#### Scenario: Fresh headless prompt

- **GIVEN** startup readiness is unconfigured
- **WHEN** the user runs `smith -p "hello"`
- **THEN** Smith sends no provider request and writes no config or credential
- **AND** stderr explains how to run interactive setup
- **AND** machine-readable stdout remains empty

#### Scenario: Setup command has no interactive terminal

- **GIVEN** stdin or stderr is not an interactive terminal
- **WHEN** the user invokes `smith setup`
- **THEN** Smith exits with a stable usage/configuration error
- **AND** does not prompt, read a secret, or write user state

### Requirement: Honest and accessible setup presentation

Setup SHALL remain usable with keyboard-only input, narrow terminals, no
color, and reduced motion. Secret fields MUST be masked without copying their
contents into the transcript, and the review step MUST label every model limit
as explicit or catalog-backed.

#### Scenario: Review without color

- **GIVEN** color is disabled
- **WHEN** setup renders the review step
- **THEN** provider, endpoint, credential reference, model, limits, provenance,
  destination path, and pending action remain distinguishable in text
- **AND** no secret value is shown

#### Scenario: Terminal is narrow

- **GIVEN** the terminal is narrower than the preferred setup width
- **WHEN** any setup step renders
- **THEN** content wraps or scrolls without hiding the current field,
  validation error, Back, Cancel, or Continue action

### Requirement: Discoverable runtime selector commands

Smith SHALL treat the arguments to `/model`, `/provider`, `/profile`, and
`/resume` as optional discovery shortcuts. Invoking one without an argument
MUST open the corresponding searchable local picker; invoking one with an
explicit valid identifier MUST keep the direct-selection behavior. Neither path
MUST issue a provider request merely to enumerate or validate choices.

#### Scenario: Model command has no argument

- **GIVEN** Smith is idle with multiple locally selectable models
- **WHEN** the user submits `/model`
- **THEN** a searchable model picker opens instead of a missing-name error
- **AND** no provider request is issued

#### Scenario: Resume command has no argument

- **GIVEN** the current project has saved sessions
- **WHEN** the user submits `/resume`
- **THEN** a searchable project-session picker opens instead of a missing-ID
  error
- **AND** no provider request is issued

#### Scenario: Explicit selector argument is supplied

- **GIVEN** the user knows an exact selectable profile, provider/model pair, or
  session ID
- **WHEN** they submit the corresponding command with that value
- **THEN** Smith validates and applies the direct selection without requiring
  a picker round trip

#### Scenario: Selector is invoked during a busy turn

- **GIVEN** a selector requires an idle runtime boundary
- **WHEN** the user invokes it while a model turn is active
- **THEN** Smith preserves the draft and reports the idle requirement locally
- **AND** does not open, queue, or execute the selection

### Requirement: Searchable provider, model, and profile pickers

Smith SHALL render configured runtime choices through a shared keyboard-first
picker. The model picker MUST list valid provider/model pairs across providers
and apply both values atomically. Provider selection MUST lead to a valid model
for that provider, and profile entries MUST state their resolved
provider/model.

#### Scenario: Choose a model belonging to another provider

- **GIVEN** `zai/glm-4.7` is active
- **AND** `openrouter/openai/gpt-4o-mini` is locally selectable
- **WHEN** the user chooses the OpenRouter entry from `/model`
- **THEN** Smith applies provider `openrouter` and model
  `openai/gpt-4o-mini` as one candidate selection
- **AND** does not try to run `openai/gpt-4o-mini` through provider `zai`

#### Scenario: Provider has several models

- **GIVEN** the user selects a provider with more than one selectable model
- **WHEN** the provider choice is confirmed
- **THEN** Smith opens the model picker filtered to that provider
- **AND** does not carry an incompatible model from the prior provider

#### Scenario: Provider has one model

- **GIVEN** the user selects a provider with exactly one selectable model
- **WHEN** the provider choice is confirmed
- **THEN** Smith applies that provider/model pair atomically
- **AND** full runtime preflight still validates the candidate

#### Scenario: No configured model is selectable

- **GIVEN** the local inventory contains no valid provider/model pair
- **WHEN** the model picker opens
- **THEN** it renders a non-selectable empty state
- **AND** points to `smith setup add-model` without fetching a remote catalog

#### Scenario: Picker is cancelled

- **GIVEN** a resource picker is open
- **WHEN** the user presses Escape
- **THEN** Smith restores the composer and current runtime/session selection
- **AND** applies no partial provider, model, profile, or resume value

### Requirement: Meaningful project-session picker

The resume picker SHALL list saved sessions for the current canonical project
newest-first. Each entry SHALL show a shortened session ID, update time, and
bounded locally persisted context sufficient to distinguish choices, including
turn count, provider/model, and a one-line recent-user preview when available.
It MUST NOT expose reasoning, assistant content, tool arguments/results, or
secret material.

#### Scenario: Several sessions can be resumed

- **GIVEN** the current project has several compatible saved sessions
- **WHEN** `/resume` opens the picker
- **THEN** entries are ordered newest-first with meaningful bounded labels
- **AND** confirming one resumes exactly its full session ID

#### Scenario: Older session lacks summary metadata

- **GIVEN** a compatible snapshot predates the listing metadata
- **WHEN** it appears in the resume picker
- **THEN** it remains selectable by ID and update time
- **AND** unavailable preview fields are labelled unknown rather than guessed

#### Scenario: Current session appears in the list

- **GIVEN** the active session is persisted and present in the inventory
- **WHEN** the resume picker opens
- **THEN** that entry is marked current
- **AND** confirming it is a no-op

#### Scenario: Project has no saved sessions

- **GIVEN** the current project has no compatible saved session
- **WHEN** `/resume` opens the picker
- **THEN** Smith states that there is nothing to resume
- **AND** points to `/new` without displaying sessions from another project

### Requirement: Interactive no-ID process resume

An interactive `smith --resume` invocation without a session ID SHALL open the
project-session picker before creating a host or new session. Explicit
`--resume <SESSION_ID>` behavior SHALL remain unchanged. Headless,
machine-output, piped, or non-TTY use without an ID MUST fail locally and point
to `smith sessions list`; Smith MUST NOT silently choose the newest session.

#### Scenario: Interactive startup resume omits the ID

- **GIVEN** configuration is ready and the current project has saved sessions
- **AND** the terminal is interactive
- **WHEN** the user runs `smith --resume` without a value
- **THEN** Smith opens the same project-session picker used by `/resume`
- **AND** creates no host or session until the user chooses one

#### Scenario: Headless startup resume omits the ID

- **GIVEN** the caller supplies a prompt or lacks an interactive terminal
- **WHEN** `--resume` has no session ID
- **THEN** Smith exits with a stable local usage error pointing to
  `smith sessions list`
- **AND** sends no provider request and chooses no session

### Requirement: Accessible shared resource picker

Every setup/runtime/session picker SHALL support keyboard filtering,
Up/Down selection, Enter confirmation, Escape cancellation, scrolling, active
and disabled labels, narrow terminals, no color, and reduced motion. Filtering
and selection MUST operate on bounded display metadata and MUST NOT add picker
contents to canonical model history.

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
