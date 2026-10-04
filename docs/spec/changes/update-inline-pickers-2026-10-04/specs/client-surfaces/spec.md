## MODIFIED Requirements

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
- **AND** its first line welcomes the user and asks them to choose how to
  connect a model
- **AND** it states that nothing is sent to a provider until setup completes,
  without naming internal objects such as agent sessions or provider requests

#### Scenario: User completes setup

- **GIVEN** the user has reviewed valid provider, credential-reference, model,
  and limit choices
- **WHEN** they confirm and full preflight succeeds
- **THEN** setup closes and Smith starts the ordinary TUI with the resolved
  provider/model
- **AND** the first agent session uses the same runtime factory as every other
  interactive run

#### Scenario: User cancels setup

- **GIVEN** setup is open on its first step
- **WHEN** the user presses Escape
- **THEN** Smith restores the terminal, prints `Setup cancelled · nothing was
  written`, and exits successfully without starting a session
- **AND** no setup config or credential change is committed

#### Scenario: User chooses GLM quick start

- **GIVEN** Smith is unconfigured
- **WHEN** the user chooses Quick start with GLM
- **THEN** setup preselects the reviewed Z.AI endpoint, GLM model, and trusted
  limits
- **AND** review states in plain words that a GLM answer sent only as
  reasoning is shown as the reply, and that thinking stays on
- **AND** asks only for credential enrollment and final confirmation before
  preflight

#### Scenario: ChatGPT login chosen from setup

- **GIVEN** the user chose Connect ChatGPT in setup
- **WHEN** the login method list is shown and the user presses Escape
- **THEN** setup returns to the step it came from
- **AND** Smith does not exit

### Requirement: Honest and accessible setup presentation

Setup SHALL remain usable with keyboard-only input, narrow terminals, no
color, and reduced motion. Secret fields MUST be masked without copying their
contents into the transcript, and the review step MUST label every model limit
as explicit or catalog-backed. Setup text MUST name the values it will use
rather than placeholders, MUST NOT tell the user to run a command that cannot
be typed on that screen, and MUST NOT use internal terms (`PKCE`, `auth.json`,
`provider request`, `pending action`).

#### Scenario: Review without color

- **GIVEN** color is disabled
- **WHEN** setup renders the review step
- **THEN** provider, endpoint, credential reference, model, limits, provenance,
  destination path, and what confirming will do remain distinguishable in
  text, each on its own labelled row
- **AND** no secret value is shown

#### Scenario: Review uses compact sizes and a short path

- **GIVEN** setup reviews a model with a 1,000,000-token context and a
  131,072-token output ceiling from the trusted catalog
- **WHEN** the review renders
- **THEN** the limits read `1M context · 128k output · trusted catalog`
  followed by its revision
- **AND** a destination under the home directory is written with `~`
- **AND** the last row says that confirming writes that file and then checks
  the connection

#### Scenario: Credential choices name the real entry

- **GIVEN** the user is choosing how to store the key for provider `zai`
- **WHEN** the credential methods are listed
- **THEN** the existing-entry choice names `keychain:smith/zai`
- **AND** no description contains an unfilled placeholder

#### Scenario: Terminal is narrow

- **GIVEN** the terminal is narrower than the preferred setup width
- **WHEN** any setup step renders
- **THEN** content wraps or scrolls without hiding the current field,
  validation error, Back, Cancel, or Continue action

### Requirement: Meaningful project-session picker

The resume picker SHALL list saved sessions for the current canonical project
newest-first. Each row SHALL lead with the one-line recent-user preview, then
the update time as a relative age, the turn count with a correctly inflected
noun, and provider/model. The session ID SHALL appear in the selected row's
detail, not in every row. It MUST NOT expose reasoning, assistant content,
tool arguments/results, or secret material.

#### Scenario: Several sessions can be resumed

- **GIVEN** the current project has several compatible saved sessions
- **WHEN** `/resume` or `smith --resume` opens the picker
- **THEN** entries are ordered newest-first, each led by its latest prompt,
  for example `What does lib.rs define?   2 min ago · 1 turn · zai/glm-5.3`
- **AND** every session can be reached by scrolling, with a position count
  when the list is longer than the screen
- **AND** confirming one resumes exactly its full session ID

#### Scenario: Older session lacks summary metadata

- **GIVEN** a compatible snapshot predates the listing metadata
- **WHEN** it appears in the resume picker
- **THEN** it remains selectable by ID and update time
- **AND** unavailable preview fields are labelled unknown rather than guessed

#### Scenario: Current session appears in the list

- **GIVEN** the active session is persisted, holds a user message, and is
  present in the inventory
- **WHEN** the resume picker opens
- **THEN** that entry is marked current
- **AND** confirming it is a no-op

#### Scenario: Project has no saved sessions

- **GIVEN** the current project has no compatible saved session with a user
  message
- **WHEN** `/resume` opens the picker
- **THEN** Smith states that there is nothing to resume
- **AND** points to `/new` without displaying sessions from another project

### Requirement: Interactive no-ID process resume

An interactive `smith --resume` invocation without a session ID SHALL open the
project-session picker before creating a host or new session. Explicit
`--resume <SESSION_ID>` behavior SHALL remain unchanged. Headless,
machine-output, piped, or non-TTY use without an ID MUST fail locally and point
to `smith sessions list`; Smith MUST NOT silently choose the newest session.
Cancelling the picker exits without starting a session, and the picker's text
MUST say so.

#### Scenario: Interactive startup resume omits the ID

- **GIVEN** configuration is ready and the current project has saved sessions
- **AND** the terminal is interactive
- **WHEN** the user runs `smith --resume` without a value
- **THEN** Smith opens the same project-session picker used by `/resume`,
  drawn from the top-left of the screen without a frame
- **AND** creates no host or session until the user chooses one

#### Scenario: Nothing to resume

- **GIVEN** the current project has no saved session with a user message
- **WHEN** the user runs `smith --resume` interactively
- **THEN** the picker says `No sessions to resume in this project · esc exits`
- **AND** Escape exits successfully without creating a session

#### Scenario: Headless startup resume omits the ID

- **GIVEN** the caller supplies a prompt or lacks an interactive terminal
- **WHEN** `--resume` has no session ID
- **THEN** Smith exits with a stable local usage error pointing to
  `smith sessions list`
- **AND** sends no provider request and chooses no session

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
there is no earlier step.

Runtime pickers SHALL render directly above the fixed composer with at most
five matching rows visible. They MUST preserve the transcript region instead
of drawing a centered modal over it. Choosers shown before a session exists
SHALL draw from the top-left of the screen, sized to their content.

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

#### Scenario: Standalone chooser before a session

- **GIVEN** no session exists yet
- **WHEN** setup, `smith --resume`, or ChatGPT login shows a choice
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

### Requirement: In-session provider connection

Smith SHALL provide `/connect [PROVIDER]` as an idle-only local command that
selects a provider and one of its supported authentication methods. The
picker SHALL include a generic OpenAI-compatible entry that runs the same
reviewed add-provider ceremony as `smith setup add-provider`, so endpoints
without a built-in flow are connectable in-session. The connection ceremony
MUST NOT send an inference request, and durable changes MUST use the reviewed
user-scope credential transaction. Every connection step SHALL render inside
the session above the composer, with the transcript and identity footer still
drawn; the outcome SHALL be reported as a notice in the session.

#### Scenario: Connect OpenRouter from the provider picker

- **GIVEN** the user submits `/connect` while the session is idle
- **WHEN** they select OpenRouter, choose protected API-key storage, enter a
  key, and confirm the secret-free review
- **THEN** Smith stores the key through the reviewed credential transaction
- **AND** records the standard OpenRouter provider endpoint without requiring
  the user to type it
- **AND** sends no inference request during connection

#### Scenario: Connection steps keep the session on screen

- **GIVEN** a session with transcript history
- **WHEN** the user runs `/connect openrouter` or chooses ChatGPT in
  `/connect`
- **THEN** the credential, review, account, and login progress steps appear
  above the composer under the title of the connection, not `Smith setup`
- **AND** the transcript remains visible throughout
- **AND** Escape on the first connection step returns to the composer with
  the draft and session unchanged

#### Scenario: Connection outcome is visible

- **WHEN** a connection or disconnection completes or fails
- **THEN** its result appears as a notice in the session after the rebuild
- **AND** nothing is printed to the normal screen behind the session

#### Scenario: Connect a custom OpenAI-compatible endpoint

- **GIVEN** the user submits `/connect` while the session is idle
- **WHEN** they select the OpenAI-compatible entry and complete the add-provider
  ceremony (distinct provider name, endpoint, authentication, and first usable
  model with enforceable limits) through the secret-free review
- **THEN** Smith adds the provider through the same reviewed user-config
  transaction as `smith setup add-provider`
- **AND** sends no inference request during connection
- **AND** the new provider/model becomes selectable after the session rebuild

#### Scenario: Reconnect an existing provider

- **GIVEN** a configured provider already has endpoint, models, limits,
  profiles, and a selected default
- **WHEN** the user connects that provider with a replacement credential
- **THEN** Smith changes only its authentication source
- **AND** preserves all unrelated provider and selection fields

#### Scenario: Connect while work is active

- **GIVEN** a turn, approval, child, or runtime replacement is active
- **WHEN** the user invokes `/connect`
- **THEN** Smith refuses or defers the action through the ordinary idle-boundary
  policy
- **AND** does not start login or credential persistence

## ADDED Requirements

### Requirement: Escape steps back one step

In setup, `/connect`, and ChatGPT or xAI login, Escape SHALL return to the
previous step with its values kept, and SHALL cancel the flow only on its
first step. Ctrl+C SHALL cancel the whole flow from any step. Shift+Tab SHALL
keep moving back as it does today. Cancelling MUST commit no configuration
or credential change.

#### Scenario: Escape on the key field

- **GIVEN** setup is on the API key field after the user chose Quick start
  with GLM and protected storage
- **WHEN** the user presses Escape
- **THEN** setup shows the credential method list with protected storage
  selected
- **AND** nothing has been written

#### Scenario: Ctrl+C deep in setup

- **GIVEN** setup is on its review step
- **WHEN** the user presses Ctrl+C
- **THEN** setup ends as a cancellation from the first step would
- **AND** nothing has been written

### Requirement: Sessions without a user message are not offered

Smith SHALL NOT offer a session that holds no user message for resumption:
the resume pickers and the terminal table of `smith sessions list` MUST omit
it, and the exit report MUST NOT print a `resume with …` line for it. A
session whose user message failed before any provider usage SHALL still be
offered. The tab-separated `smith sessions list` output for pipes MUST keep
every row.

#### Scenario: Start and quit without typing

- **GIVEN** the user starts Smith and quits without submitting a prompt
- **WHEN** Smith exits and the user later opens `/resume` or runs
  `smith sessions list` in a terminal
- **THEN** the exit report has no `resume with …` line
- **AND** that session is not listed

#### Scenario: A prompt that failed is still offered

- **GIVEN** the user submitted a prompt and the provider request failed
  before reporting usage
- **WHEN** the user opens `/resume`
- **THEN** the session is listed with its prompt as the row's name
