# client-surfaces Specification

## Purpose
What each Smith surface shows — the interactive TUI, headless output, informational results, setup, and composer — and how it is laid out.
## Requirements
### Requirement: Basic interactive TUI

Running `smith` without non-interactive flags SHALL open a Ratatui/Crossterm
coding surface containing a scrollable transcript, streaming response,
composer, command access, tool calls/results, approval prompts, provider/model
selection, and session create/resume. The surface MUST be driven by the same
resolved Smith runtime factory and shared events as every other host.

#### Scenario: Run a basic coding turn

- **GIVEN** valid provider configuration
- **WHEN** the user enters a prompt and approves a requested tool
- **THEN** the TUI streams model text and tool state without blocking input
- **AND** persists the canonical turn for resume

### Requirement: Operational status in the TUI

The TUI SHALL display current provider/model, token and provenance status,
cache state, active monitors, direct children, running background shell tasks,
and queued notifications. An estimated or unknown value MUST be visually
distinguishable from a provider-reported value.

#### Scenario: Provider switch leaves estimated context

- **GIVEN** the user switches to a provider that has not reported usage
- **WHEN** the status line updates
- **THEN** it labels context tokens estimated
- **AND** does not reuse the prior provider's verified cache indicator

#### Scenario: Running background task is visible

- **GIVEN** a background shell task is running
- **WHEN** the user views operational status
- **THEN** the task is listed as active work with its task ID
- **AND** it disappears from active work after its terminal notification

### Requirement: Non-interactive prompt mode

Smith SHALL accept `smith -p <prompt>` and `smith -p -` for stdin. Callers MUST
be able to select project, session/resume, provider, model, approval policy, and
background-exit policy through stable arguments or configuration. Headless mode
MUST use the same runtime factory as the TUI.

#### Scenario: Prompt comes from stdin

- **GIVEN** a caller pipes a prompt to `smith -p -`
- **WHEN** the agent finishes successfully
- **THEN** Smith writes the requested output format to stdout
- **AND** uses stderr for progress and diagnostics

### Requirement: Versioned machine output

Non-interactive mode SHALL support `text`, `json`, and `stream-json`. JSON MUST
be one versioned final result envelope; stream JSON MUST be newline-delimited
versioned runtime events followed by a terminal result. Machine-readable stdout
MUST NOT contain progress prose or terminal color escapes.

#### Scenario: External CLI consumes stream JSON

- **GIVEN** `--output-format stream-json`
- **WHEN** the agent streams text, calls a tool, reports usage, and completes
- **THEN** stdout contains one parseable event per line in causal order
- **AND** the final line communicates terminal status and session ID

### Requirement: Fail-closed headless approval

When no TTY is available, an action requiring user approval MUST produce a
structured approval-required outcome and stable non-success exit status unless
the caller supplied an explicit policy authorizing it.

#### Scenario: Headless mutation lacks policy

- **GIVEN** an external caller runs `smith -p` without a TTY or mutation policy
- **WHEN** the model requests a patch
- **THEN** Smith does not modify the file
- **AND** returns an approval-required result rather than waiting indefinitely

### Requirement: Explicit active-work exit policy

The TUI MUST request confirmation before exiting with active monitors,
children, or background shell tasks. Non-interactive mode SHALL support
`error`, `wait`, and `stop` background-exit policies and MUST default to
`error`.

#### Scenario: Headless turn finishes with a persistent monitor

- **GIVEN** the final answer is ready while a persistent monitor remains
- **WHEN** the caller did not choose an exit policy
- **THEN** Smith emits an active-work error describing the monitor
- **AND** does not silently orphan it

#### Scenario: Headless turn finishes with a running background task

- **GIVEN** the final answer is ready while a background shell task remains
- **WHEN** the caller chose the `wait` background-exit policy
- **THEN** Smith waits for the task's terminal state before exiting
- **AND** reports its terminal state in machine output

### Requirement: Host-appropriate UI contributions

Declarative extension status/widgets SHALL render in the TUI. In
non-interactive mode, presentation-only contributions MUST be omitted
predictably without failing the agent run, while their underlying data events
MAY remain in machine output.

#### Scenario: Status extension runs headlessly

- **GIVEN** a trusted extension registers a TUI status item
- **WHEN** `smith -p` runs
- **THEN** Smith does not attempt terminal rendering
- **AND** the extension's non-visual lifecycle hooks continue according to
  policy

### Requirement: macOS and Linux terminal support

Smith SHALL support macOS and Linux terminals with keyboard-only operation,
visible focus, resize handling, and reduced-motion behavior. Platform-specific
process cleanup MUST be covered by automated tests.

#### Scenario: Terminal resizes during streaming

- **GIVEN** the TUI is displaying a streaming response
- **WHEN** the terminal size changes
- **THEN** content reflows without losing transcript or focus state

### Requirement: Slash-command interception

Composer input whose first non-whitespace character is `/` SHALL be
intercepted and dispatched as a local command. Intercepted input MUST NOT be
sent to the provider, and an unknown command MUST produce a local error that
points at command discovery, with no provider request or spend.

#### Scenario: Known command dispatches locally

- **GIVEN** the composer contains a registered command such as `/model`
- **WHEN** the user submits it
- **THEN** Smith runs the mapped host action
- **AND** no provider request is issued

#### Scenario: Unknown command fails locally

- **GIVEN** the composer contains an unregistered command
- **WHEN** the user submits it
- **THEN** Smith renders a local error referencing `/help`
- **AND** no provider request is issued

### Requirement: Command discovery and host-action mapping

Smith SHALL provide `/help` listing every registered command with a one-line
description. Built-in commands MUST map to existing host actions (for
example, the model picker and session controls) rather than duplicating their
logic, so a command and its keybinding behave identically.

#### Scenario: Help lists registered commands

- **GIVEN** the user submits `/help`
- **WHEN** Smith renders the response locally
- **THEN** every registered command appears with a one-line description

#### Scenario: Command matches its keybinding

- **GIVEN** a host action is reachable by both a keybinding and a command
- **WHEN** the user invokes the command
- **THEN** the same host action runs with the same behavior as the keybinding

### Requirement: Literal slash passthrough

Smith SHALL provide a documented escape that sends a message beginning with a
slash to the model as an ordinary prompt.

#### Scenario: Escaped slash message reaches the model

- **GIVEN** the user applies the documented escape to input starting with `/`
- **WHEN** they submit it
- **THEN** the message is sent to the provider verbatim as a prompt
- **AND** no local command is dispatched

### Requirement: Context visibility in local status

Smith SHALL render the latest enforced context plan inside `/status`, including
percent left, counted input tokens, input budget, model window, reserved
tokens, count provenance, and bounded totals by segment kind. The display MUST
distinguish the latest request plan from cumulative provider-reported session
input and MUST name the absence of a plan before the first turn. Context
inspection MUST remain local and MUST NOT issue a provider request.

#### Scenario: Status shows the latest enforced plan

- **GIVEN** the runtime emitted a `ContextPlanned` event
- **WHEN** the user submits `/status`
- **THEN** Smith shows the latest plan's used tokens, budget, percent left,
  model window, reserves, confidence, and segment totals
- **AND** cumulative provider input is labelled as session usage rather than
  active context
- **AND** no provider request is issued

#### Scenario: Status before the first context plan

- **GIVEN** no turn has produced a `ContextPlanned` event
- **WHEN** the user submits `/status`
- **THEN** Smith states that context has not been planned yet
- **AND** it shows declared capacity and reserves without inventing usage
- **AND** no provider request is issued

### Requirement: Focused context visualization

Smith SHALL provide `/context` as a local inline visualization of the latest
enforced context plan. It SHALL show model and input-budget capacity, percent
left, bounded totals by segment category, free input space, reserved
output/reasoning capacity, count provenance, and compaction state. The
visualization MUST remain legible without color, MUST NOT retain or reveal raw
context content, and MUST NOT issue a provider request.

#### Scenario: Context command visualizes the latest enforced plan

- **GIVEN** the runtime emitted a `ContextPlanned` event
- **WHEN** the user submits `/context`
- **THEN** Smith appends an inline usage map and category legend for that plan
- **AND** the legend distinguishes used segments, free input space, and reserve
- **AND** exact or estimated provenance and compaction state are stated in words
- **AND** no provider request is issued

#### Scenario: Context command before the first plan

- **GIVEN** no turn has produced a `ContextPlanned` event
- **WHEN** the user submits `/context`
- **THEN** Smith states that usage is unavailable until the first turn
- **AND** it visualizes declared input capacity and reserves without inventing
  segment usage
- **AND** no provider request is issued

### Requirement: Single-focus conversational interaction

The interactive TUI SHALL keep the composer as its only persistent focus
target. Transcript navigation SHALL work through global shortcuts, background
activity SHALL render inline, and absent or hidden regions MUST NOT participate
in focus order. A temporary read-only view MAY borrow the transcript region,
but MUST be dismissible with one key and MUST NOT take focus.

#### Scenario: Tab does not leave the composer

- **GIVEN** no modal or command menu is open
- **WHEN** the user presses `Tab` or `Shift+Tab`
- **THEN** Smith does not move focus to the transcript, inbox, or another
  persistent region
- **AND** the composer remains ready for input

#### Scenario: Transcript scroll is global

- **GIVEN** the composer is active and the transcript has older content
- **WHEN** the user presses a transcript scroll shortcut
- **THEN** the transcript scrolls without entering a separate transcript mode
- **AND** sending a prompt restores follow-newest behavior

#### Scenario: Background activity remains visible

- **GIVEN** a child or monitor emits progress while the user is composing
- **WHEN** Smith renders the event
- **THEN** the event reaches its surface without stealing focus — a monitor as
  a concise attributed transcript notice, a child as panel activity and an
  inspectable log entry
- **AND** detailed child state remains available through `/agent` and panel
  selection

#### Scenario: A read-only child view borrows the transcript region

- **GIVEN** a delegated child is selected from the panel
- **WHEN** Smith renders its read-only view over the transcript region
- **THEN** the composer keeps focus and its draft
- **AND** one dismissal key restores the root timeline unchanged
- **AND** the view participates in no focus order

### Requirement: Unified command discovery

Smith SHALL expose one typed command registry shared by slash completion,
`/help`, and `Ctrl+P`. Typing `/` at the start of a composer draft MUST open a
filterable menu; `Tab` MUST complete without executing, and `Enter` MUST execute
the selected command locally when permitted.

#### Scenario: Slash opens filtered completion

- **GIVEN** the composer is empty
- **WHEN** the user types `/rev`
- **THEN** the command menu filters to matching registered commands with
  descriptions and argument hints
- **AND** no provider request is issued

#### Scenario: Tab completes without execution

- **GIVEN** a command-menu result is selected
- **WHEN** the user presses `Tab`
- **THEN** Smith completes the command text in the composer
- **AND** does not execute a host action or send a provider request

#### Scenario: Ctrl-P shares the registry

- **GIVEN** the user opens command discovery with `Ctrl+P`
- **WHEN** the menu appears
- **THEN** it uses the same commands, descriptions, parser, and host actions as
  slash completion

#### Scenario: Busy command fails locally

- **GIVEN** a host command requires an idle runtime
- **WHEN** the user invokes it during an active model turn
- **THEN** Smith keeps the draft and reports the idle requirement locally
- **AND** does not queue, execute, or send the command to the provider

### Requirement: Focused built-in command set

Smith SHALL initially register only commands backed by implemented product
capabilities: help, status, session/runtime selection, child inspection,
change inspection/review/recovery, and exit. `/help` MUST group primary and
advanced commands without advertising unavailable capabilities.

#### Scenario: Help exposes the complete implemented set

- **WHEN** the user invokes `/help`
- **THEN** Smith lists `/help`, `/status`, `/new`, `/resume`, `/model`,
  `/provider`, `/profile`, `/agent`, `/diff`, `/review`, `/undo`, `/revert`,
  and `/quit`
- **AND** every listed command has a one-line description

#### Scenario: Status is local and honest

- **WHEN** the user invokes `/status`
- **THEN** Smith reports resolved model/profile, permission mode, context
  provenance, session, child, Git, and change-attribution state
- **AND** unknown or unavailable values are labelled rather than guessed
- **AND** no provider request is issued

### Requirement: Inline informational command results

Smith SHALL render read-only local command results as attributed blocks in the
normal transcript instead of opening a blocking viewer. These blocks MUST keep
composer input available, participate in normal transcript scrolling and
follow behavior, remain bounded, and MUST NOT be sent to the provider or added
to canonical model conversation history.

#### Scenario: Status appears in the conversation

- **GIVEN** the interactive composer is available
- **WHEN** the user invokes `/status`
- **THEN** a titled status block is appended to the transcript
- **AND** the composer remains immediately available without a close step
- **AND** no provider request is issued

#### Scenario: Consecutive local results remain visible

- **GIVEN** one informational command result is already in the transcript
- **WHEN** the user invokes another informational command
- **THEN** Smith appends the new titled result after the earlier result
- **AND** does not replace, cover, or dismiss the earlier result

#### Scenario: Diff states render inline

- **WHEN** `/diff` produces a patch, empty result, non-Git outcome, binary
  notice, oversized notice, or conflict
- **THEN** Smith renders the bounded result inline with its state stated in
  text
- **AND** normal transcript scrolling remains available

#### Scenario: Interactive safety surfaces remain modal

- **WHEN** Smith needs command selection, tool approval, provider-spend
  confirmation, undo confirmation, or revert confirmation
- **THEN** Smith may open the corresponding modal with explicit controls
- **AND** informational command results themselves never require dismissal

#### Scenario: Local results do not become model context

- **GIVEN** an informational result is visible in the transcript
- **WHEN** the user sends the next provider prompt or resumes the session
- **THEN** Smith does not represent that local result as a user or assistant
  conversation message
- **AND** protected local status or patch detail is not exposed to the
  provider merely because it was displayed

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

#### Scenario: Limits resolve automatically after the model is entered

- **GIVEN** the user has entered the model ID in either flow
- **WHEN** the endpoint's model listing or a same-name trusted catalog entry
  supplies a context window
- **THEN** numeric limit entry is skipped and the resolved values and source
  are shown in review
- **AND** the resolution probe sends no inference request and stays within its
  bounded time and size

#### Scenario: Resolution fails or finds nothing

- **GIVEN** the endpoint is unreachable, lists nothing for the model, and no
  catalog name matches
- **WHEN** the bounded resolution attempt ends
- **THEN** the flow asks only for the total context window and derives the
  input and output ceilings without showing more numeric fields
- **AND** the failed attempt is not an error the user must dismiss

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

Runtime pickers SHALL render as a compact pane directly above the fixed
composer with at most five matching rows visible. They MUST preserve the
transcript region instead of drawing a centered modal over it. Setup and
pre-host selection MAY retain a larger standalone presentation when no coding
transcript/composer exists.

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

### Requirement: Explicit no-prompt credential setup

Smith SHALL offer plaintext user-config storage as an explicit no-prompt
authentication choice and SHALL support changing only an existing provider's
credential storage. Every review and result surface MUST state the at-rest
risk while redacting the value.

#### Scenario: User reviews config authentication

- **GIVEN** authentication offers Keychain, environment, and local-config
  choices
- **WHEN** the user selects “Store in config (no prompts)”
- **THEN** setup explains same-user process exposure and backup risk
- **AND** the API-key field remains masked
- **AND** review shows `api_key = [redacted]`

#### Scenario: User migrates an existing provider

- **GIVEN** provider `zai` already has a valid endpoint, model, limits, and a
  `keychain:` credential reference
- **WHEN** the user runs `smith setup credential --provider zai`, selects
  config storage, enters a key, and confirms
- **THEN** setup changes only that provider's credential source
- **AND** full preflight uses the unchanged provider/model without opening the
  Keychain
- **AND** the next ordinary Smith startup opens no credential-service prompt

#### Scenario: Credential migration fails preflight

- **GIVEN** setup has atomically published a candidate config containing the
  inline key
- **WHEN** runtime preflight fails
- **THEN** setup restores the exact prior config bytes
- **AND** errors, review state, temporary files, stdout, and stderr contain no
  key value

#### Scenario: Existing non-prompting source remains selected

- **GIVEN** a provider already uses `env:` or `api_key`
- **WHEN** the user reviews or cancels credential migration
- **THEN** Smith does not consult the Keychain
- **AND** cancellation writes nothing

### Requirement: Catalog-backed model picker

Smith SHALL list catalog-backed models for recognized configured providers in
the existing searchable `/model` picker. Entries MUST remain
provider-qualified, deterministic, bounded for large catalogs, and coherent
with direct selection and `/provider` cascading behavior.

#### Scenario: OpenRouter picker is not limited to local TOML

- **GIVEN** OpenRouter is configured with one explicit local model
- **AND** the prepared catalog snapshot contains additional valid OpenRouter
  models
- **WHEN** the user opens `/model`
- **THEN** the picker includes the explicit model and additional catalog-backed
  models under the OpenRouter provider
- **AND** filtering can match model ID, display name, provider, or capability
  detail

#### Scenario: Z.AI Coding Plan lists its supported catalog

- **GIVEN** Smith's `zai/glm-4.7` quick start is active
- **AND** the prepared Z.AI Coding Plan catalog contains other valid models
- **WHEN** the user opens `/model`
- **THEN** those models appear as distinct `zai/<model-id>` choices
- **AND** `zai/glm-4.7` remains marked current

#### Scenario: Provider picker uses catalog model count

- **GIVEN** a configured provider has several selectable catalog models
- **WHEN** the user opens `/provider` and chooses it
- **THEN** the provider detail shows the selectable catalog-augmented count
- **AND** Smith opens `/model` filtered to that provider rather than applying
  an arbitrary model

#### Scenario: Incompatible catalog model is explained locally

- **GIVEN** a catalog entry is deprecated or lacks text output, tool calling,
  complete valid limits, or a usable input budget under effective reserves
- **WHEN** Smith prepares or filters model choices
- **THEN** deprecated entries are omitted and other incompatible entries are
  non-selectable with a bounded reason
- **AND** confirming a disabled entry sends no provider request

#### Scenario: Directly choose a catalog model

- **GIVEN** `openrouter/vendor/model` is a unique selectable catalog-backed
  choice
- **WHEN** the user submits `/model openrouter/vendor/model`
- **THEN** Smith applies provider `openrouter` and model `vendor/model`
  atomically
- **AND** preserves nested slashes inside the provider model ID

#### Scenario: Large catalog remains usable

- **GIVEN** a configured provider contributes hundreds of catalog models
- **WHEN** `/model` is opened in a narrow or wide terminal
- **THEN** rendering remains bounded to the viewport and filtering remains
  keyboard-first
- **AND** deterministic ordering, selection, Enter, and Escape behavior remain
  unchanged

#### Scenario: Picker opens while offline

- **GIVEN** networking is unavailable
- **AND** Smith has a valid last-good or embedded catalog snapshot
- **WHEN** the user opens, searches, cancels, or confirms `/model`
- **THEN** picker behavior uses only the prepared snapshot
- **AND** displays no network or credential prompt

#### Scenario: Advertised model is unavailable to the account

- **GIVEN** a catalog-backed model passes local metadata preflight
- **BUT** the provider later rejects it for account, plan, or region reasons
- **WHEN** the first provider request fails
- **THEN** Smith reports the provider error without removing or rewriting user
  configuration
- **AND** does not misrepresent catalog advertisement as verified entitlement

### Requirement: Stable baseline categories in context visualization

`/context` SHALL always name system instructions and tool schemas in a stable
order without revealing their content. Before the first enforced plan their
counts MUST be unknown rather than zero; after a plan their display totals MUST
be derived from canonical segment totals and MUST remain visible when zero.

#### Scenario: Context before the first plan names stable request classes

- **GIVEN** no turn has emitted a `ContextPlanned` event
- **WHEN** the user submits `/context`
- **THEN** Smith lists system instructions and tool schemas as not counted yet
- **AND** their counts render as unknown rather than zero
- **AND** no provider request is issued

#### Scenario: Planned context aggregates instruction segments for display

- **GIVEN** a context plan contains system, developer, and ability instruction
  segment totals
- **WHEN** the user submits `/context`
- **THEN** the focused view shows their sum as `system instructions`
- **AND** canonical status and telemetry retain each original segment kind

#### Scenario: Baseline category has an honest zero

- **GIVEN** a context plan has no tool-schema segment
- **WHEN** the user submits `/context`
- **THEN** the tool-schema legend row remains visible with a zero count
- **AND** the usage grid allocates no nonzero cells to that category

### Requirement: Local reasoning controls

Smith SHALL expose idle-only `/think` and `/effort` controls using the shared
command and picker grammar. Choices MUST be limited to the resolved
provider/model capability snapshot, MUST apply to the next whole turn, and
MUST NOT issue a provider request merely to inspect or change a setting.

#### Scenario: Toggleable model changes thinking for the next turn

- **GIVEN** the idle provider/model supports optional thinking
- **WHEN** the user selects `/think off`
- **THEN** Smith records a session override and confirms it locally
- **AND** the next complete turn uses the disabled setting
- **AND** no request is issued by the command itself

#### Scenario: Effort selector contains only supported levels

- **GIVEN** the resolved provider/model advertises `low`, `medium`, and `high`
- **WHEN** the user opens `/effort`
- **THEN** the picker contains only those efforts plus the provider default
- **AND** selecting one uses the same validation as a direct command argument

#### Scenario: Fixed reasoning exposes no false control

- **GIVEN** the model reasons but its controls are fixed or unknown
- **WHEN** the user opens `/think` or `/effort`
- **THEN** Smith explains locally which control is unavailable and why
- **AND** it does not infer support, send a probe, or mutate the session

#### Scenario: Mandatory reasoning cannot be disabled

- **GIVEN** the capability snapshot marks reasoning mandatory
- **WHEN** the user opens `/think` or submits `/think off`
- **THEN** the UI omits or disables the off choice with a written reason
- **AND** the direct command fails locally before provider I/O

### Requirement: Reasoning status and lifecycle visibility

Smith SHALL show the effective thinking state, effort when applicable, and
configuration/provider/session provenance in local status and context output.
Session overrides MUST survive compatible resume, MUST be revalidated on a
provider/model change, and MUST never alter an already-running turn.

#### Scenario: Status distinguishes default from override

- **GIVEN** a session effort overrides the provider/model default
- **WHEN** the user submits `/status` or `/context`
- **THEN** Smith shows the effective effort and labels it a session override
- **AND** raw reasoning content is not shown

#### Scenario: Model switch invalidates an override

- **GIVEN** the session has an effort unsupported by a newly selected model
- **WHEN** Smith switches and rebuilds the provider/model runtime
- **THEN** it clears the incompatible override with an explicit local notice
- **AND** it does not map the value to a guessed nearest effort

#### Scenario: Busy turn cannot change reasoning mid-loop

- **GIVEN** a turn is running or waiting on a tool continuation
- **WHEN** the user attempts to change thinking or effort
- **THEN** Smith refuses the command locally as busy
- **AND** every request in the active turn retains its original setting

### Requirement: Explicit allow-all shorthand

Smith SHALL accept valueless `--yolo` as an explicit invocation-level alias
for `--approval allow-all`. The alias MUST pass through the same typed approval
selection and runtime policy as the long form, MUST NOT create a distinct
approval mode, and MUST NOT widen the selected profile's tool or permission
set.

#### Scenario: Trusted run uses the shorthand

- **GIVEN** a selected build profile exposes a prepared mutating tool
- **WHEN** the user explicitly starts Smith with `--yolo`
- **THEN** Smith resolves the invocation approval mode as `allow-all`
- **AND** applies the same central authorization and execution path as
  `--approval allow-all`

#### Scenario: Plan remains read-only

- **GIVEN** the selected plan profile removes edit and shell capabilities
- **WHEN** the user explicitly starts Smith with `--yolo`
- **THEN** the plan profile still cannot request or execute edit or shell
- **AND** approval policy does not restore any removed capability

#### Scenario: Approval spellings conflict

- **WHEN** one invocation supplies both `--yolo` and `--approval`, repeats
  `--yolo`, or assigns a value to `--yolo`
- **THEN** Smith rejects the invocation before runtime construction
- **AND** does not silently choose an approval policy by argument order

### Requirement: Headless execution follows an explicitly active goal

An ordinary headless prompt SHALL retain its existing one-turn lifecycle unless
that turn explicitly creates or activates a goal. Once a goal is active, the
headless host SHALL remain subscribed across attributed conditional internal
turns until the goal reaches a stopped state or existing process/global limits
terminate execution.

#### Scenario: Ordinary prompt completes without a goal

- **GIVEN** a headless prompt neither restores nor explicitly creates a goal
- **WHEN** its explicit turn completes
- **THEN** `smith -p` exits under existing one-turn semantics
- **AND** emits no goal record or continuation turn

#### Scenario: Explicit headless goal completes

- **GIVEN** the prompt explicitly creates a persistent active goal
- **WHEN** several internal continuations eventually mark it complete
- **THEN** the headless host observes every attributed turn and exits after the
  complete state commits
- **AND** reports the final answer and final goal usage evidence

#### Scenario: Headless goal stops without completion

- **GIVEN** an active headless goal becomes paused, blocked, usage-limited, or
  budget-limited
- **WHEN** that state commits
- **THEN** automatic continuation stops and the process exits predictably
- **AND** output distinguishes the stopped reason from successful completion

#### Scenario: Headless goal needs user interaction

- **GIVEN** no bidirectional interaction broker is configured
- **WHEN** goal work reaches a material questionnaire requirement
- **THEN** the goal becomes blocked and headless execution returns the existing
  structured `interaction_required` outcome
- **AND** includes the final goal snapshot without fabricating an answer

### Requirement: Machine output projects goal lifecycle explicitly

Goal-aware text, JSON, and JSON Lines output SHALL preserve existing non-goal
field meanings while adding bounded typed goal projections. Machine output MUST
identify final goal status, stable goal identity, usage provenance, optional
budget, actual overshoot, active elapsed time, stopped reason, and number of
continuation turns without reconstructing state from prose.

#### Scenario: JSON goal result is complete

- **GIVEN** a goal-aware headless run completes successfully
- **WHEN** Smith writes its final JSON record
- **THEN** it includes one optional structured final-goal object and
  continuation count
- **AND** existing assistant text, usage, turn, and terminal fields retain their
  documented meaning

#### Scenario: JSON Lines streams goal progress

- **GIVEN** a headless goal runs across several turns
- **WHEN** JSON Lines output is selected
- **THEN** each typed goal update and attributed turn lifecycle is emitted in
  canonical order
- **AND** consumers need not parse assistant or diagnostic text to follow state

#### Scenario: Budget overshoots by one request

- **GIVEN** the provider reports usage only after a response that crosses the
  budget
- **WHEN** machine output reports the budget-limited terminal state
- **THEN** it includes actual reported usage and the requested budget
- **AND** does not claim the budget was a pre-spend hard cap

### Requirement: Interactive and headless goal semantics are equivalent

Smith SHALL commit equivalent goal transitions, usage accounting, internal-turn
identities, tool effects, and persistence in interactive and headless hosts
given identical resolved policy, persisted goal state, provider events, and
user-independent inputs. Presentation and availability of live user controls
may differ without changing canonical goal behavior.

#### Scenario: Same deterministic goal fixture runs on both surfaces

- **GIVEN** identical persistent sessions and scripted provider/tool outcomes
- **WHEN** TUI and headless hosts execute the fixture
- **THEN** their canonical goal states, usage totals, turn sequence, and tool
  results are equivalent
- **AND** only their local rendering/output projections differ

#### Scenario: Both surfaces shut down

- **GIVEN** an active goal exists when the current Smith process shuts down
- **WHEN** either surface completes bounded shutdown
- **THEN** both persist equivalent latest goal state and stop all work
- **AND** neither surface starts detached continuation after exit

### Requirement: Pending user input is visibly distinguished

The interactive TUI SHALL render bounded, text-labelled previews for pending
steers, rejected-steer follow-ups, and explicit future turns in the existing
anchored composer region. It MUST distinguish process-local pending state from
canonical transcript history and MUST remain understandable without color.

#### Scenario: Steer waits for a safe boundary

- **GIVEN** an accepted steer has not yet committed
- **WHEN** the TUI renders the busy surface
- **THEN** it labels the input as pending for the active turn
- **AND** shows the interrupt-for-steer hint without adding a canonical user row

#### Scenario: Several future turns are queued

- **GIVEN** queued previews exceed the per-section line budget
- **WHEN** the TUI renders at a supported terminal size
- **THEN** it shows the bounded leading previews and an overflow count
- **AND** does not displace the composer or create an unbounded pane

#### Scenario: Todo and pending input coexist

- **GIVEN** public todo state and pending user input both exist
- **WHEN** no modal or picker owns the anchored area
- **THEN** the renderer allocates bounded rows to both within the existing
  anchored budget
- **AND** cursor placement remains attached to the composer

### Requirement: Busy key guidance matches behavior

The TUI and `/help` SHALL describe the conditional `Enter`, `Tab`, `Alt+Up`,
and `Esc` behavior while work is serving. Idle profile cycling and overlay
selection hints MUST remain accurate in their respective states.

#### Scenario: Ordinary prompt is ready during work

- **GIVEN** eligible work is serving and an ordinary draft is non-empty
- **WHEN** Smith renders composer guidance
- **THEN** the guidance identifies `Enter` as steer and `Tab` as queue
- **AND** identifies the configured queued-input edit action when a future turn
  exists

### Requirement: Smith-owned pointer text selection

Smith terminal surfaces SHALL enable button and drag mouse reporting and SHALL
own pointer text selection, because terminal mouse reporting is global and
all-or-nothing on the button: wheel scrolling cannot be received without also
taking the drag that native selection requires. Smith SHALL therefore provide
selection and clipboard copy itself, and all required interactions SHALL remain
available from the keyboard.

Selection SHALL address rendered cells rather than transcript text, and the
copied text SHALL be read from the rendered frame, so that a drag copies
exactly the glyphs beneath it.

#### Scenario: User copies visible transcript text

- **GIVEN** Smith is showing stable transcript content in an interactive
  terminal
- **WHEN** the user drags across visible text and releases the left button
- **THEN** Smith highlights the dragged cells and puts their text on the
  platform clipboard
- **AND** a successful copy reports nothing, leaving the highlight as its
  receipt
- **AND** a failed clipboard write is reported rather than passing silently

#### Scenario: User selects outside the transcript

- **GIVEN** Smith is showing footer, composer, or picker text
- **WHEN** the user drags across it
- **THEN** the selection spans those cells the same way it spans transcript
  cells

#### Scenario: A drag that selects nothing leaves the clipboard alone

- **GIVEN** the user drags across blank cells, or clicks without moving
- **WHEN** the button is released
- **THEN** Smith does not write to the clipboard
- **AND** a click that never moved dismisses any existing highlight

#### Scenario: Moving content discards a stale highlight

- **GIVEN** a highlight is painted over rendered cells
- **WHEN** the transcript scrolls, a runtime event appends output, or the
  terminal is resized past the selection
- **THEN** Smith clears the highlight rather than marking the text that moved
  into those cells

#### Scenario: Keyboard operation remains complete

- **GIVEN** Smith enables mouse reporting
- **WHEN** the user edits the composer, scrolls the transcript, navigates a
  picker, or answers a modal
- **THEN** the documented keyboard controls provide the complete interaction
- **AND** no interaction requires the pointer
- **AND** bracketed paste continues to work independently of mouse reporting

#### Scenario: A hovering pointer costs nothing

- **GIVEN** Smith has enabled mouse reporting
- **WHEN** the user moves the pointer across the terminal with no button held
- **THEN** Smith does not request all-motion reporting and receives no event

### Requirement: In-session provider connection

Smith SHALL provide `/connect [PROVIDER]` as an idle-only local command that
selects a provider and one of its supported authentication methods. The
picker SHALL include a generic OpenAI-compatible entry that runs the same
reviewed add-provider ceremony as `smith setup add-provider`, so endpoints
without a built-in flow are connectable in-session. The connection ceremony
MUST NOT send an inference request, and durable changes MUST use the reviewed
user-scope credential transaction.

#### Scenario: Connect OpenRouter from the provider picker

- **GIVEN** the user submits `/connect` while the session is idle
- **WHEN** they select OpenRouter, choose protected API-key storage, enter a
  key, and confirm the secret-free review
- **THEN** Smith stores the key through the reviewed credential transaction
- **AND** records the standard OpenRouter provider endpoint without requiring
  the user to type it
- **AND** sends no inference request during connection

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

### Requirement: Interactive OAuth ceremony

The connection surface SHALL support browser-URL and device-code login states
with explicit progress, cancellation, timeout, retry, and completion. It MUST
display only public authorization instructions and MUST NOT retain or render
authorization codes, access tokens, refresh tokens, PKCE verifiers, or callback
payloads.

#### Scenario: Complete browser login

- **GIVEN** the selected trusted auth method returns a public authorization URL
- **WHEN** the user completes Smith's loopback PKCE flow and token exchange
- **THEN** Smith marks the connection ready with its non-secret method/backend
  identity
- **AND** no token value enters the transcript, render state, or diagnostic

#### Scenario: Complete device-code login

- **GIVEN** browser callback login is unsuitable and device login is available
- **WHEN** Smith displays the verification URL and one-time user code
- **THEN** the user can complete login in another browser
- **AND** Smith stops polling at success, cancellation, expiry, or its bounded
  deadline

#### Scenario: Cancel OAuth login

- **GIVEN** an OAuth ceremony is waiting for completion
- **WHEN** the user presses Escape or Ctrl-C
- **THEN** Smith cancels the trusted login backend and closes temporary local
  listeners or tasks
- **AND** restores the prior connection and terminal state without writing a
  credential

### Requirement: Connection removal and visibility

Smith SHALL provide `/disconnect [PROVIDER]` and local connection status.
Disconnecting MUST clear Smith-owned credential material while preserving
unrelated provider/model setup.

#### Scenario: Disconnect ChatGPT

- **GIVEN** Smith owns a ChatGPT token bundle in its owner-only auth file
- **WHEN** the user confirms `/disconnect chatgpt`
- **THEN** Smith atomically removes the auth-file entry and provider credential
  source
- **AND** does not read, mutate, or depend on a Codex or OpenCode auth cache
- **AND** does not query or remove a legacy Smith Keychain entry

#### Scenario: Disconnect an inactive API-key provider

- **GIVEN** a configured inactive provider uses Smith-owned protected storage
- **WHEN** the user confirms `/disconnect` for that provider
- **THEN** Smith removes the reviewed credential entry and provider credential
  source
- **AND** preserves its endpoint, models, limits, and profiles

#### Scenario: Disconnect the only active provider

- **GIVEN** the current session has no other usable provider
- **WHEN** the user requests disconnection
- **THEN** Smith requires a replacement connection or session exit before
  committing
- **AND** never leaves the session presented as runnable without authentication

### Requirement: Consistent prompt-cache visibility

Smith SHALL project the same canonical cache state and derived missed-token
facts through the interactive footer, `/status`, exit/session summary, final
JSON, and streaming JSON. The footer's `CH` value SHALL represent the latest
completed root turn's provider-reported cache-read share of total prompt input,
including billed failed attempts. Explicit zero MUST render as `0%` and absent
evidence MUST render as unknown.

#### Scenario: Completed turn reports a zero cache read

- **GIVEN** a root turn has reported prompt-input usage
- **AND** its provider explicitly reports zero cache-read tokens
- **WHEN** the turn completes
- **THEN** the footer renders `CH 0%`
- **AND** `/status` and machine output retain the matching canonical state

#### Scenario: Cache-read evidence is absent

- **GIVEN** a root turn reports input usage but no cache-read observation
- **WHEN** the turn completes
- **THEN** the footer renders cache hit rate as unknown
- **AND** no surface turns the omission into zero or a miss

#### Scenario: TUI and final JSON consume the same events

- **GIVEN** a deterministic turn includes a partial cache miss and one failed
  retry
- **WHEN** it is reduced by the TUI and headless hosts
- **THEN** both report equivalent state, expected, observed, missed, and
  cache-read percentage values
- **AND** stream JSON retains the attempt-level canonical events

### Requirement: Cache notices remain local presentation

An interactive cache-miss notice SHALL be a bounded local transcript block and
MUST NOT enter canonical conversation history or provider context. Human
headless mode SHALL write an enabled significant miss notice to stderr while
keeping answer stdout unchanged.

#### Scenario: User sends another prompt after a miss notice

- **GIVEN** a cache-miss notice is visible in the transcript
- **WHEN** the user sends the next prompt
- **THEN** the notice is absent from the provider request
- **AND** the canonical user and assistant history is unchanged by the notice

#### Scenario: Headless text output reports a miss

- **GIVEN** notices are enabled for a headless text run
- **AND** its completed turn crosses a significance threshold
- **WHEN** Smith exits successfully
- **THEN** stdout contains only the requested answer
- **AND** stderr may contain the factual cache-miss diagnostic

### Requirement: Cache lifecycle state is explainable across surfaces

Smith SHALL project the same bounded cache lifecycle facts through interactive
status, session/exit summaries, final JSON, and streaming JSON events. Human
and machine surfaces SHALL keep these values
separate when available:

- structurally preserved prefix tokens;
- provider-reported cache-read and cache-write tokens;
- provider cache status and guarantee timestamp;
- exact cache identity/revision without private prompt content;
- requested and effective maintenance mode;
- maintenance call budget and calls used;
- scheduled, suppressed, completed, or suspended disposition and reason; and
- separately attributed synthetic usage and cost.

An unknown value MUST remain unknown rather than zero, warm, expired, or
guaranteed. Existing cache-miss visibility and `CH` projections SHALL continue
to use the canonical attempt evidence from
`add-prompt-cache-miss-visibility-2026-08-08`.

#### Scenario: Structurally reusable prefix has no provider evidence

- **GIVEN** context planning preserves a stable prefix
- **AND** the provider reports no cache observation
- **WHEN** status and final JSON are rendered
- **THEN** both expose the structural count separately
- **AND** provider cache status remains unknown
- **AND** neither surface calls it a verified hit

#### Scenario: Scheduled maintenance is suppressed

- **GIVEN** a keepalive was scheduled and later suppressed by real activity
- **WHEN** interactive and machine status update
- **THEN** both expose the bounded suppression reason and unchanged call usage
- **AND** neither reports a provider attempt or cost

#### Scenario: Active cache-miss projection remains composed

- **GIVEN** a provider explicitly reports zero after an expected reusable plan
- **WHEN** the new lease projection and existing cache-miss projection reduce
  the same canonical attempt
- **THEN** every surface reports one consistent miss and suspension state
- **AND** no second miss count or re-billed-token value is derived

### Requirement: Parked parent and automatic continuation are visible

Interactive and headless lifecycle output SHALL distinguish an idle ordinary
session, `parked-awaiting-child`, an admitted
`delegation.child-completion` turn, and an adaptive cache-maintenance attempt.
The projection MUST NOT imply that a provider stream remains open while parked
or that cache maintenance is child execution.

#### Scenario: Parent waits without provider work

- **GIVEN** a parent is parked with one running child
- **WHEN** status or machine output is inspected
- **THEN** it identifies the parked state and pending child
- **AND** reports no active parent provider turn unless one actually exists

#### Scenario: Child completion wakes the parent

- **GIVEN** a child-completion internal turn is admitted
- **WHEN** lifecycle and usage output are inspected
- **THEN** the turn is attributed to `delegation.child-completion`
- **AND** any cache observation or provider usage belongs to that real
  continuation rather than the prior parked interval

#### Scenario: User input wins the race

- **GIVEN** user input wins admission over a ready child outcome
- **WHEN** clients render the boundary
- **THEN** they show one active user turn
- **AND** do not render a phantom concurrent child-completion turn

### Requirement: Resume-capsule diagnostics reveal no sensitive content

Smith SHALL keep resume-capsule diagnostics bounded and redaction-safe. Status
and machine output MAY expose schema revision, freshness, summary
purpose/model/revision, source coverage, and last successful persistence
boundary. They MUST NOT expose raw canonical history, private prompt bodies,
credentials, protected interaction content, provider cache contents, or
unbounded summary text.

#### Scenario: Handoff summary is persisted

- **GIVEN** a same-model handoff checkpoint updates the capsule
- **WHEN** final JSON reports the capsule projection
- **THEN** it exposes purpose, model/revision, timestamp, coverage, and outcome
- **AND** omits the summary body and stable-prefix content

#### Scenario: Exact state conflicts with summary

- **GIVEN** recovery detects a summary inconsistency
- **WHEN** Smith presents the diagnostic
- **THEN** it reports bounded field/category and authoritative-source metadata
- **AND** does not copy conflicting private text into logs or status

### Requirement: Synthetic cache traffic never appears as conversation

Keepalive and handoff-checkpoint request/response content SHALL be absent from
canonical transcripts, replayed conversation, copied answers, and model history.
Clients MAY render bounded local lifecycle diagnostics and separately
attributed usage, but those blocks MUST remain noncanonical.

#### Scenario: User continues after a handoff checkpoint

- **GIVEN** a handoff checkpoint completed during a parked interval
- **WHEN** the user sends the next real prompt
- **THEN** provider context contains no synthetic checkpoint instruction or
  response as a canonical turn
- **AND** the resume capsule may contribute only through its reviewed bounded
  continuation projection

#### Scenario: Journal replay reconstructs maintenance

- **GIVEN** canonical redaction-safe maintenance lifecycle events were journaled
- **WHEN** the TUI replays them
- **THEN** it can reconstruct status and usage diagnostics
- **AND** it cannot fabricate ping, pong, or summary text into conversation

### Requirement: Account usage visibility and manual switching

When the active provider has a credential pool, the TUI SHALL show which pool
member is active, offer an accessible picker listing every member with its
usage meter, cooldown state, and provenance-safe display name, and accept an
explicit manual switch to any member not currently in cooldown. A rotation
offered after limit exhaustion SHALL be presented as a modal the user answers,
stating the cache cost of switching, and its outcome SHALL be recorded in the
transcript. Headless runs SHALL select their member once at session start and
keep it for the whole run, projecting the active member and the typed
exhaustion outcome through the versioned machine output without ever prompting
or rotating.

#### Scenario: Inspect pool usage

- **GIVEN** the active provider declares a two-member pool
- **WHEN** the user opens the account picker
- **THEN** both members appear in pool order with usage percentage or unknown,
  cooldown state, and which one is active
- **AND** no credential value or secret fragment is displayed

#### Scenario: Manually switch the active account

- **GIVEN** the picker is open and the second member is eligible
- **WHEN** the user selects it
- **THEN** subsequent attempts use the second member
- **AND** the sticky selection persists for future sessions
- **AND** the transcript records the manual switch

#### Scenario: Rotation is offered as a modal and announced

- **GIVEN** an attempt hits limit exhaustion mid-task with an eligible member
  available
- **WHEN** the runtime offers rotation
- **THEN** a modal names the outgoing and incoming members, the outgoing
  member's reset time, and warns that switching resends the turn without the
  provider-side prompt cache
- **AND** confirming it replays the attempt and writes a rotation notice to the
  transcript
- **AND** declining it writes the exhaustion outcome to the transcript instead

#### Scenario: A headless run never rotates

- **GIVEN** `smith -p` starts with a two-member pool and exhausts its member
  mid-run
- **WHEN** the attempt fails with the typed limit-exhaustion error
- **THEN** the run fails with that error and the earliest reset time
- **AND** no prompt is rendered and no member switch occurs
- **AND** machine output names the member the run started on

### Requirement: Command-line reasoning effort selection

Smith SHALL accept `--effort <NAME>` anywhere the shared selection parser
accepts selection flags, including `smith`, `smith -p`, `smith config explain`,
and `smith sessions list`. The flag MUST support both spaced and inline forms,
and the client MUST reject a missing value or more than one supplied value.

#### Scenario: Selection surfaces accept both flag forms

- **GIVEN** the invocation uses one provider-advertised effort name
- **WHEN** the user supplies `--effort high` or `--effort=high` to `smith`,
  `smith -p`, `smith config explain`, or `smith sessions list`
- **THEN** the shared selection parser accepts the invocation flag
- **AND** the selected effort remains available to the corresponding client
  surface

#### Scenario: Missing effort value is rejected

- **GIVEN** the invocation contains `--effort` without a value
- **WHEN** Smith parses the command line
- **THEN** Smith rejects the invocation with a non-success usage outcome
- **AND** it does not start the requested client surface

#### Scenario: Repeated effort value is rejected

- **GIVEN** the invocation supplies `--effort` twice, in either supported form
- **WHEN** Smith parses the command line
- **THEN** Smith rejects the invocation with a non-success usage outcome
- **AND** it does not silently choose one value by argument order

### Requirement: Discoverable invocation effort option

Smith SHALL list `--effort <NAME>` in the `RUN OPTIONS` section of `--help` as
selecting a provider-advertised reasoning effort.

#### Scenario: Run help describes effort selection

- **GIVEN** the user requests Smith command-line help
- **WHEN** Smith renders `--help`
- **THEN** `RUN OPTIONS` includes `--effort <NAME>`
- **AND** its description identifies the value as a provider-advertised
  reasoning effort

### Requirement: Local failure for an unadvertised invocation effort

Smith SHALL fail locally, with a non-success exit status, when the requested
invocation effort is not advertised by the resolved provider/model binding.
The diagnostic MUST name the requested value and list the supported
alternatives, without performing credential lookup or issuing a provider
request.

#### Scenario: Unsupported effort names the available alternatives

- **GIVEN** the user supplies an effort absent from the selected binding's
  advertised ladder
- **WHEN** Smith processes the invocation
- **THEN** Smith exits with a non-success status and names the requested effort
- **AND** the diagnostic lists the supported alternatives before any credential
  lookup or provider request

### Requirement: Explicit effort survives interactive startup boundaries

Interactive startup recovery MUST preserve the meaning of an explicitly typed
`--effort`. If the current binding cannot honor that flag, the recovery path
MUST fail with the reasoning diagnostic instead of clearing the typed value and
starting with a notice. When Smith composes a child-profile runtime, it MUST
omit the invocation flag so an uncontrollable child binding does not abort the
parent startup.

#### Scenario: Recovery refuses an unhonorable explicit effort

- **GIVEN** an interactive invocation explicitly supplies `--effort`
- **AND** startup recovery reaches a binding that cannot honor the requested
  reasoning selection
- **WHEN** the recovery path evaluates the binding
- **THEN** startup fails with the reasoning diagnostic
- **AND** Smith does not silently clear the typed flag or start with a notice

#### Scenario: Child profile does not inherit invocation effort

- **GIVEN** the parent invocation explicitly supplies `--effort`
- **AND** Smith composes a runtime for a child profile whose binding cannot
  control reasoning
- **WHEN** the child runtime is started
- **THEN** the child does not receive the parent invocation flag
- **AND** the uncontrollable child binding does not abort parent startup

### Requirement: Invocation effort provenance is user-facing

Smith MUST identify an invocation-supplied effort in `smith config explain
reasoning.effort` output with the source "command-line flag `--effort`". It
MUST NOT render the mechanical `--reasoning-effort` spelling for that source.

#### Scenario: Config explanation uses the typed flag spelling

- **GIVEN** the user supplies an invocation effort with `--effort`
- **WHEN** the user runs `smith config explain reasoning.effort`
- **THEN** the effective entry identifies its source as "command-line flag
  `--effort`"
- **AND** the output does not identify the source as `--reasoning-effort`

### Requirement: Invocation effort remains distinct from reasoning state

The `--effort` flag SHALL select an advertised effort only. It MUST NOT turn
reasoning on or off, and an explicit in-session `/effort` selection MUST remain
the higher-precedence control for the active session's subsequent turn.

#### Scenario: Invocation effort does not toggle thinking

- **GIVEN** the user starts Smith with an advertised `--effort` value
- **WHEN** Smith applies the invocation selection
- **THEN** it selects the requested effort without changing reasoning enabled
  state
- **AND** it does not interpret the flag as a thinking on/off switch

#### Scenario: In-session effort outranks invocation effort

- **GIVEN** the invocation supplies one advertised effort
- **AND** the user selects a different effort through `/effort` in the session
- **WHEN** Smith prepares the next complete turn
- **THEN** the in-session effort is effective for that turn
- **AND** the invocation flag does not override the explicit `/effort`

### Requirement: Context window command

The TUI SHALL accept `/context <NAME|default>` to set or clear the session's
context window override. It SHALL apply the change at the next idle boundary
and persist it on resume. `/context` with no argument SHALL keep its current
report and add the available windows, marking the active one.

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

### Requirement: Current installed-agent model choices

Smith SHALL list GPT-6 Astra, GPT-6.1 Sol, GPT-6 Sol, and GPT-6 Luna as
selectable models for an installed Codex CLI. Smith SHALL use Claude Code's stable rolling model
aliases rather than pinning dated Claude CLI model identifiers.

#### Scenario: Codex exposes current GPT-6 choices

- **GIVEN** `codex` is available on `PATH`
- **WHEN** the user opens Smith's model picker
- **THEN** `cli/codex/gpt-6-astra`, `cli/codex/gpt-6.1-sol`, `cli/codex/gpt-6-sol`, and
  `cli/codex/gpt-6-luna` are selectable

#### Scenario: Claude tracks the latest version through an alias

- **GIVEN** an updated `claude` executable is available on `PATH`
- **WHEN** the user selects `cli/claude-code/opus`
- **THEN** Smith passes `opus` to Claude Code
- **AND** Claude Code resolves that alias to its current Opus release

### Requirement: Mandatory Anthropic effort selection is capability-driven

Smith SHALL make its existing `/think`, `/effort`, `--effort`, `/status`, and
`/context` surfaces use the resolved capability snapshot for an exact binding
configured with mandatory `anthropic-effort` controls, without provider
probing or special-casing the model name. They MUST NOT expose raw reasoning
content.

#### Scenario: Fable 5.1 effort picker opens locally

- **GIVEN** the idle binding advertises `low`, `medium`, `high`, `xhigh`, and
  `max` through trusted Anthropic metadata
- **WHEN** the user opens `/effort`
- **THEN** the picker lists those levels in advertised order plus provider
  default
- **AND** opening or choosing the setting sends no provider request

#### Scenario: Mandatory adaptive thinking cannot be disabled

- **GIVEN** the binding marks Anthropic adaptive thinking mandatory
- **WHEN** the user opens `/think` or submits `/think off`
- **THEN** the off choice is unavailable with a written reason
- **AND** the direct command fails locally without provider I/O

#### Scenario: Invocation effort keeps ordinary provenance

- **GIVEN** the user starts the exact binding with `--effort xhigh`
- **WHEN** Smith explains or displays the effective reasoning state
- **THEN** the selection is `xhigh` and retains command-line provenance
- **AND** status never renders the model's raw reasoning content

### Requirement: Interactive provider retry progress is explicit

The interactive TUI SHALL distinguish an initial provider attempt, a scheduled
retry backoff, an in-flight retry, and final retry exhaustion using words and
attempt counts rather than color alone. Attempt totals and delays MUST come
from the runtime's retry decision metadata; when exact metadata is absent,
Smith MUST use bounded generic wording instead of fabricating progress.

#### Scenario: TUI waits through retry backoff

- **GIVEN** attempt 1 of 3 fails and the runtime schedules attempt 2 after a
  positive delay
- **WHEN** the TUI receives the finished-attempt event
- **THEN** the live row reads `Retrying 2/3` and shows the remaining backoff
- **AND** an informational notice leads with retry 2 of 3 before the bounded
  redaction-safe provider cause

#### Scenario: Retried provider request is still waiting

- **GIVEN** the runtime has started attempt 2 of 3 after a failed first attempt
- **AND** the provider has not produced output yet
- **WHEN** the TUI redraws the live row
- **THEN** it continues to read `Retrying 2/3`
- **AND** shows the existing sending-phase elapsed time so the wait is not
  confused with backoff or generic work

#### Scenario: Retry succeeds

- **GIVEN** a later provider attempt succeeds
- **WHEN** its output commits and the turn completes
- **THEN** the TUI clears retry progress
- **AND** does not render the transient attempt failure as a terminal error

#### Scenario: Retry budget is exhausted

- **GIVEN** attempt 3 of 3 fails with a redaction-safe provider error
- **AND** the runtime schedules no further attempt
- **WHEN** the turn reaches its terminal outcome
- **THEN** the TUI renders one attributed `failed after 3/3 attempts` error
- **AND** no row or notice claims another retry is pending

#### Scenario: Exact retry metadata is unavailable

- **GIVEN** the TUI replays an older event that classifies an error retryable
  but carries no attempt total or scheduled delay
- **WHEN** it renders that event
- **THEN** it does not invent `x/x`, a delay, or an admitted retry
- **AND** preserves a bounded generic provider diagnostic

### Requirement: Every offered setup entry is actionable

Guided setup SHALL offer only entries that start a flow. Selecting any listed
entry and confirming it MUST advance to that entry's next step or report why
it cannot proceed; a confirmation MUST NOT be silently ignored.

#### Scenario: A listed provider path is confirmed

- **GIVEN** guided setup lists a provider or action entry
- **WHEN** the user selects it and presses Enter
- **THEN** setup shows that entry's next step
- **AND** the selection screen does not remain unchanged

#### Scenario: An entry has no first-run flow

- **GIVEN** a provider descriptor has no flow available in guided setup
- **WHEN** Smith builds the setup choices
- **THEN** the entry is omitted, or shown as unavailable with the command
  that does support it
- **AND** an automated check fails when an offered entry has no handler

### Requirement: Setup text names the values it writes

Labels and review text in guided setup SHALL be derived from the provider,
model, limits, and catalog revision that setup will write. Smith MUST NOT show
a model name or catalog revision that differs from the published value.

#### Scenario: Quick-start review

- **GIVEN** the user chooses a quick-start provider path
- **WHEN** setup shows its menu label and its review
- **THEN** both name the model that will be written
- **AND** the catalog revision shown equals the revision recorded in
  configuration

### Requirement: Usage help at every command level

The command line SHALL print usage and exit successfully for `smith help`,
and for `-h` or `--help` after any subcommand. A parse error SHALL name the
problem and print exactly one recovery hint.

#### Scenario: Help after a subcommand

- **WHEN** the user runs `smith setup --help`, `smith config --help`, or
  `smith sessions --help`
- **THEN** Smith prints usage covering that subcommand to stdout
- **AND** exits with status 0 without opening setup or a session

#### Scenario: Unknown argument

- **WHEN** the user runs `smith` with an unrecognised option or subcommand
- **THEN** stderr names the argument and gives one hint to run help
- **AND** the exit status is the documented usage-error status

### Requirement: Interactive launch without a terminal is refused clearly

When configuration is ready and no prompt is supplied, Smith SHALL verify
that it has an interactive terminal before entering the alternate screen. If
it does not, Smith MUST exit with a message that names the cause and the
headless alternative, and MUST NOT surface a raw operating-system error.

#### Scenario: Standard input is a pipe

- **GIVEN** configuration is ready
- **WHEN** the user runs `echo hi | smith`
- **THEN** stderr explains that the interactive surface needs a terminal and
  that `smith -p` accepts a prompt on standard input
- **AND** no session is created and no provider request is sent

### Requirement: Configured background exit policy is honoured

A headless run SHALL resolve its background-exit policy from the
`--background-exit` flag, then the resolved `background.exit_policy`
configuration value, then the default `error`.

#### Scenario: Policy set only in configuration

- **GIVEN** configuration sets `background.exit_policy = "wait"`
- **AND** the caller passes no `--background-exit` flag
- **WHEN** a headless turn finishes with a running background task
- **THEN** Smith waits for the task's terminal state before exiting

#### Scenario: Flag overrides configuration

- **GIVEN** configuration sets `background.exit_policy = "wait"`
- **WHEN** the caller passes `--background-exit stop`
- **THEN** Smith applies `stop`

### Requirement: Session listing is readable on a terminal

`smith sessions list` SHALL print a header row, aligned columns, and the
last-updated time in local time when standard output is a terminal. When
standard output is not a terminal it MUST keep the existing tab-separated
row format.

#### Scenario: Listing on a terminal

- **GIVEN** the project has saved sessions
- **WHEN** the user runs `smith sessions list` in a terminal
- **THEN** each column has a heading
- **AND** the update time is a local date and time, not a millisecond count

#### Scenario: Listing through a pipe

- **WHEN** the output of `smith sessions list` is piped to another program
- **THEN** each session is one tab-separated row with the fields and order
  documented for the Claude Code plugin

### Requirement: One transcript row per tool call

The interactive TUI SHALL render each tool call as one row that names the
tool by its reviewed label and reviewed argument summary, with the result
nested beneath it. Smith MUST NOT render a second row that restates the same
call, and MUST NOT print argument names in place of protected values.

#### Scenario: A shell call completes

- **GIVEN** the model runs a shell command that prints twenty lines
- **WHEN** the call completes successfully
- **THEN** the transcript shows one call row with a success marker
- **AND** at most four result lines beneath it followed by a count of the
  remaining lines and the expand key

#### Scenario: A user shell shortcut

- **GIVEN** the user submits `!ls`
- **WHEN** the command completes
- **THEN** the transcript shows the command once, marked as the user's, with
  its result nested beneath
- **AND** no change notice appears when no file changed

#### Scenario: A user shell shortcut survives resume

- **GIVEN** the user ran `!ls` and it finished, then quit Smith
- **WHEN** the user resumes the session
- **THEN** the transcript shows the same `! ls` row with its nested result,
  after the message that preceded it
- **AND** the record came from a per-session file beside the snapshot that
  holds the redacted command and its bounded result, not from the event
  journal or model-visible history

#### Scenario: A saved shortcut whose place is gone

- **GIVEN** a saved shortcut names a history position the restored history
  does not reach
- **WHEN** the session resumes
- **THEN** that shortcut is not shown, rather than shown in another place

#### Scenario: Protected arguments

- **GIVEN** a tool call whose arguments are not reviewed for display
- **WHEN** its row renders
- **THEN** the row shows the tool label and reviewed summary only
- **AND** it does not list argument names or a "details unavailable" note

### Requirement: Folded detail has one expand key

The TUI SHALL fold long tool output, approval detail, and long diffs to a
bounded preview, and SHALL expand and fold them with one documented key that
works whether or not a prompt is open.

#### Scenario: Expanding a result

- **GIVEN** a tool row shows a bounded preview and a remaining-line count
- **WHEN** the user presses the expand key
- **THEN** the bounded full detail is shown in place
- **AND** pressing it again folds the detail

### Requirement: Progress is one row outside the transcript

While a turn is active the TUI SHALL show exactly one progress row directly
above the composer, carrying elapsed time, token flow when the provider has
reported it, and the interrupt key. The row MUST NOT be a transcript entry.

#### Scenario: A turn is running

- **GIVEN** a turn has been running for twelve seconds
- **WHEN** the screen is drawn
- **THEN** one progress row shows the elapsed time and the interrupt key
- **AND** appending transcript content does not move or duplicate it

#### Scenario: Token flow is estimated

- **GIVEN** the only token count available is an estimate
- **WHEN** the progress row renders it
- **THEN** the number carries the estimate marker

### Requirement: Streamed and committed text render identically

The TUI SHALL render assistant text through one Markdown renderer while it
streams and after it commits. Committing a block MUST NOT change the layout
of text already on screen. Links SHALL keep a visible target.

#### Scenario: A streamed answer commits

- **GIVEN** an answer containing bold text, a list, and a code fence is
  streaming
- **WHEN** the attempt commits
- **THEN** the rendered lines are the same as immediately before the commit

#### Scenario: A link

- **GIVEN** assistant text contains a Markdown link whose label is not its
  URL
- **WHEN** it renders
- **THEN** both the label and the URL are visible or selectable

### Requirement: Approvals lead with the action

An approval SHALL present, in order: what will run or change, where and under
what deadline, the applicable warning, the question, and the choices. The
identity hash, full permission list, and raw arguments MUST remain available
through the expand key and MUST NOT precede the action. The transcript SHALL
remain scrollable while the prompt is open.

#### Scenario: A shell approval

- **GIVEN** a prepared shell call needs approval
- **WHEN** the prompt renders
- **THEN** the command is the first content line
- **AND** each choice is on its own line with its key
- **AND** the identity hash and raw arguments appear only after expanding

#### Scenario: Reviewing the work behind a prompt

- **GIVEN** an approval is open
- **WHEN** the user presses a transcript scroll key
- **THEN** the transcript scrolls and the prompt stays open and unanswered

### Requirement: Lists show the name first and metadata last

Command menus, the command palette, and resource pickers SHALL render each
row as a name column, then one short description, then state. Argument
grammar, provenance, and limits SHALL appear only for the selected row.
Rows MUST be cut at a word boundary with an ellipsis.

#### Scenario: Command menu

- **GIVEN** the command menu is open
- **WHEN** it renders five rows
- **THEN** descriptions start in the same column
- **AND** only the selected row shows its argument grammar

#### Scenario: Model picker at 44 columns

- **GIVEN** the model picker is open in a 44-column terminal
- **WHEN** it renders
- **THEN** every row shows the model name and its current or unavailable
  state
- **AND** provenance appears only on the selected row's detail line

### Requirement: Informational results are aligned and word-wrapped

`/help`, `/status`, `/context`, and `/diagnostics` SHALL render labels and
values in aligned columns with word wrapping and a hanging indent. A result
MUST NOT break a word across lines, and a result longer than the pane SHALL
open at its beginning.

#### Scenario: Help in a narrow terminal

- **GIVEN** a 44-column terminal
- **WHEN** the user invokes `/help`
- **THEN** no command name or word is split across lines
- **AND** the first visible line is the start of the result

#### Scenario: Diagnostics with unknown values

- **GIVEN** cache state has not been reported
- **WHEN** the user invokes `/diagnostics`
- **THEN** each fact is on its own line under a group heading
- **AND** an unreported value reads as unknown in words

#### Scenario: Diagnostics values are shown as stored

- **GIVEN** a diagnostics value contains Markdown characters such as
  backticks
- **WHEN** `/diagnostics` renders
- **THEN** the value is shown exactly as stored, without Markdown styling

### Requirement: A child's state reads the same everywhere

Every surface that names a child agent's state or durability SHALL use one
lowercase word set: `running`, `idle`, `interrupted`, `stopped`, `failed`,
`expired`, and `durable` or `ephemeral`. Qualifiers SHALL be words, not debug
structures. Machine-readable headless output MUST NOT change.

#### Scenario: A running child

- **GIVEN** a child agent has an active turn
- **WHEN** it appears in the transcript, the delegated-work panel, `/agents`,
  or the child inspector
- **THEN** its state reads `running` on each of them

#### Scenario: An interrupted child

- **GIVEN** a child was interrupted with an exact checkpoint
- **WHEN** `/agents` lists it
- **THEN** its state reads `interrupted (resumable)`

### Requirement: The composer supports line editing

The composer SHALL move the cursor between the lines of a multi-line draft
with Up and Down, reaching history only from the first or last line, and
SHALL support start-of-line, end-of-line, delete-word, delete-to-start, and
delete-to-end keys. A leading `!` SHALL switch the prompt marker and hint to
shell mode.

#### Scenario: Moving within a draft

- **GIVEN** a three-line draft with the cursor on the last line
- **WHEN** the user presses Up
- **THEN** the cursor moves to the second line
- **AND** history is not recalled

#### Scenario: Shell mode

- **GIVEN** an empty composer
- **WHEN** the user types `!`
- **THEN** the prompt marker and the hint row indicate shell mode

### Requirement: Changing the session's configuration keeps the screen

The interactive TUI SHALL keep its screen and application when a model,
profile, effort, thinking, or context-window change, an MCP recomposition,
or a provider connection rebuilds the host for the same session: the
transcript as shown, folded and expanded state, scroll position, the
composer, and composer history. It SHALL re-derive status, resources,
children, and usage from the new host, return the live turn to idle, and
append one notice that names what changed. It MUST NOT rebuild the
transcript from history for the same session, and MUST NOT leave the
alternate screen except while another flow draws its own screen.

#### Scenario: Switching model

- **GIVEN** a session with a finished turn, a `/status` result on screen,
  and an earlier prompt in composer history
- **WHEN** the user picks another model with `/model`
- **THEN** the earlier turn and the `/status` result are still in the
  transcript, followed by one notice naming the old and new model
- **AND** Up in the empty composer recalls the earlier prompt
- **AND** the footer names the new model

#### Scenario: Cycling profiles with Tab

- **GIVEN** an idle session with an empty draft and several main profiles
- **WHEN** the user presses Tab
- **THEN** the transcript and scroll position are unchanged apart from one
  profile notice

#### Scenario: Switching to another session

- **GIVEN** a session with composer history
- **WHEN** the user resumes a different session with `/resume`
- **THEN** the transcript shows that session's history
- **AND** composer history is kept

#### Scenario: Connecting a provider

- **GIVEN** an idle session with a transcript
- **WHEN** the user connects a provider through `/connect` and finishes or
  cancels its flow
- **THEN** Smith returns to the same transcript and composer

#### Scenario: A rebuild while a turn runs

- **GIVEN** a turn is running
- **WHEN** a session reconfiguration is requested
- **THEN** Smith refuses it with a notice that it requires an idle turn
- **AND** the running turn continues

### Requirement: The transcript renders only what changed

The interactive TUI SHALL render a transcript block again only when that
block, the width, the fold state, or the theme changed, and SHALL draw only
the rows in view. The rendered output MUST be identical to rendering every
block from scratch.

#### Scenario: A long conversation while a turn streams

- **GIVEN** a transcript of thousands of blocks and a streaming answer
- **WHEN** frames are drawn while the answer grows
- **THEN** only the streaming block is rendered again on each frame
- **AND** the screen is identical to a full render

#### Scenario: Expanding tool output

- **GIVEN** a cached transcript
- **WHEN** the user presses Ctrl+O
- **THEN** every block is rendered with the new fold state
- **AND** the screen is identical to a full render

### Requirement: Scrolling has no row ceiling

The transcript SHALL scroll to any row of a transcript of any length.
Transcript scroll offsets MUST NOT be limited to 65,535 rows.

#### Scenario: A very long transcript

- **GIVEN** a transcript taller than 65,535 rows
- **WHEN** the user scrolls to its start
- **THEN** the first row is shown

### Requirement: Drawing does not change application state

The TUI SHALL compute layout and scroll bounds in a step before drawing, and
drawing SHALL read the application without changing it.

#### Scenario: Two draws of the same state

- **GIVEN** an application state after layout was applied
- **WHEN** it is drawn twice
- **THEN** both frames are identical and the state is unchanged

### Requirement: Model output budget visibility

Smith's model-selection surfaces SHALL distinguish a model's advertised output
ceiling from the effective request output budget. The picker MUST identify an
automatically derived budget without presenting it as catalog metadata or a
user-authored override.

#### Scenario: Automatic budget makes a catalog model selectable

- **GIVEN** a catalog model advertises equal 500,000-token context and output
  ceilings
- **AND** Smith derives a 32,768-token automatic request budget
- **WHEN** the user filters `/model` to that entry
- **THEN** the row is selectable and shows both the 500,000-token ceiling and
  32,768-token automatic request budget
- **AND** it does not tell the user to add a local model-limit override

#### Scenario: Configured budget is distinguishable

- **GIVEN** an explicit profile or session value supplies the effective request
  output budget
- **WHEN** Smith renders model details or a reserve diagnostic
- **THEN** it labels the value as configured rather than automatic
- **AND** keeps catalog provenance attached only to the advertised ceiling

### Requirement: Starting-work orientation
Smith SHALL show a compact local guide in an empty transcript and SHALL make
help readable from the beginning of the newly requested result. The guide
SHALL use existing commands and SHALL not become canonical history.

#### Scenario: New configured session
- **WHEN** a configured session has no transcript content and is idle
- **THEN** Smith shows how to submit a task, choose a model, connect a provider,
  and discover commands using the existing terminal design tokens
- **AND** the guide disappears when transcript content or active work exists

#### Scenario: Open help after existing conversation
- **WHEN** the user invokes `/help` or the empty-composer `?` shortcut
- **THEN** the viewport begins at the new help result when it exceeds the
  available height, retaining the complete registry and keyboard reference
- **AND** scrolling and subsequent ordinary conversation remain functional

### Requirement: Predictable command-menu activation
Smith SHALL resolve the highlighted completion when the input names no exact
command and SHALL retain exact command arguments and host safeguards.

#### Scenario: Choose from the unfiltered menu
- **WHEN** the user types `/`, highlights `/status`, and presses Enter
- **THEN** Smith runs the local status command without requiring its name
- **AND** the menu shows at most five command rows, scrolling its selected
  window without covering the conversation

#### Scenario: Search by intent
- **WHEN** the user searches `switch` and no command name starts with that term
- **THEN** matching registered command descriptions are offered
- **AND** Tab completes without execution and Enter activates the selection

#### Scenario: Explicit arguments remain authoritative
- **WHEN** input is `/model local/example-model` or a recognized command with
  invalid arguments
- **THEN** the original command parser processes those exact arguments
- **AND** invalid arguments are not silently discarded to activate another row

### Requirement: Actionable picker state
Smith SHALL distinguish empty inventory from unmatched search and SHALL show
selection availability before optional descriptive metadata.

#### Scenario: Filter has no matches
- **GIVEN** a resource inventory contains entries
- **WHEN** its filter matches none
- **THEN** Smith says there are no matches and offers a filter-clearing action
- **AND** Ctrl+U clears the query without selecting or applying a resource

#### Scenario: State survives long metadata
- **WHEN** a current or unavailable resource has long capability metadata
- **THEN** its state label precedes that metadata at normal and narrow widths
- **AND** unavailable resources remain non-selectable

#### Scenario: Narrow picker controls
- **WHEN** a resource picker is open at 44 columns
- **THEN** Enter/choose and Escape/cancel remain visible using shortened hints
- **AND** optional filter guidance yields before those essential actions

### Requirement: Setup correction retains non-secret values
Smith SHALL restore previously entered provider names, endpoints, and model IDs
when the user navigates backward to those fields. Changing a reviewed value
SHALL still invalidate any pending collision approval and require review again.

#### Scenario: Correct an endpoint after choosing authentication
- **WHEN** the user uses Shift+Tab to return to the endpoint or provider field
- **THEN** the existing non-secret value is available for editing
- **AND** no configuration is written until the updated review is confirmed

### Requirement: Skill visibility and trust command

Smith SHALL provide a built-in command that lists every skill in the session's
bounded index grouped by source layer, showing each skill's name, description,
and whether it can activate. The command MUST state the reason a skill cannot
activate, MUST show which entries a higher layer shadowed, MUST report every
skill-discovery problem, and MUST offer a way to grant trust to a workspace
skill awaiting confirmation.

#### Scenario: Inspect the catalog

- **GIVEN** a session with built-in skills, a user skill, and a workspace skill
- **WHEN** the user runs the skills command
- **THEN** each skill is listed under its source layer with its description
- **AND** each entry states whether it can activate

#### Scenario: A workspace skill is withheld

- **GIVEN** a project skill nobody has approved
- **WHEN** the user runs the skills command
- **THEN** the entry states that it needs approval
- **AND** names the command that would grant it

#### Scenario: A skill file could not be used

- **GIVEN** a skill directory whose `SKILL.md` is malformed
- **WHEN** the user runs the skills command
- **THEN** the problem is listed with the skill's name and the reason
- **AND** the remaining skills are still listed

#### Scenario: A higher layer shadows a name

- **GIVEN** a user skill and a built-in skill with the same name
- **WHEN** the user runs the skills command
- **THEN** both entries are shown
- **AND** the display identifies which one activates

#### Scenario: Grant trust from the command

- **GIVEN** a workspace skill awaiting confirmation
- **WHEN** the user grants trust through the command
- **THEN** Smith displays the skill's project-relative path and content
  identity before recording the decision
- **AND** the skill becomes activatable in the same session without the user
  restarting Smith

#### Scenario: Newly trusted skill joins at a safe boundary

- **GIVEN** the user granted trust to a workspace skill
- **WHEN** a turn is in progress
- **THEN** the catalog is not exchanged until the session is idle
- **AND** the session keeps its identity and transcript when it is

### Requirement: Installed agents render once in selection surfaces

Model and profile selection surfaces SHALL present installed coding agents
through the curated `cli/<kind>/<model>` namespace exactly once per agent
model, including the local installation check, even when the selection
inventory also enumerates a provider-qualified pair referencing the same
agent. A profile selecting an installed agent MUST appear selectable without
requiring any `[models]` declaration.

#### Scenario: Model picker shows one row per installed-agent model

- **GIVEN** a profile references `google/cli/claude-code/sonnet`
- **AND** the inventory enumerates the provider-qualified pair
- **WHEN** the user opens the model picker
- **THEN** `cli/claude-code/sonnet` appears exactly once from the curated
  namespace with its built-in limit labeling
- **AND** no duplicate provider-qualified row for the same agent model is
  shown

#### Scenario: Profile picker no longer marks installed-agent profiles unavailable

- **GIVEN** profiles `cc` and `cx` select `cli/claude-code/sonnet` and
  `cli/codex/gpt-6-astra` with no `[models]` declarations
- **WHEN** the user opens the profile picker
- **THEN** both profiles are selectable with their resolved provider/model
  pair
- **AND** neither is disabled with a profile-does-not-resolve reason

### Requirement: Versioned Smith client protocol

TUI, headless, and other presentation clients SHALL observe sessions through
Smith-owned, versioned, redaction-preserving event projections with stable
Smith IDs, and MUST NOT depend on the concrete Agent Runtime event enum.
In-process hosts drive sessions through the Agent Runtime session handle;
canonical persistence and execution MUST remain on Agent Runtime.

#### Scenario: Agent Runtime adds an event variant

- **GIVEN** a compatible Agent Runtime revision adds a canonical event
- **WHEN** Smith updates its adapter
- **THEN** Smith explicitly maps, bounds, or intentionally omits that event
- **AND** unchanged clients continue to consume their supported Smith protocol
  version

#### Scenario: TUI and GPUI observe one session

- **GIVEN** two Smith clients subscribe to the same composed session
- **WHEN** a turn streams text, prepares a tool, requests approval, and finishes
- **THEN** both receive causally ordered Smith events with stable Smith IDs
- **AND** neither client receives a direct runtime handle or mutable runtime
  internals

#### Scenario: Session is resumed from canonical state

- **GIVEN** Smith resumes Agent Runtime canonical events and snapshots
- **WHEN** a client subscribes after reconstruction
- **THEN** Smith rebuilds the same bounded client projection
- **AND** no Smith client event is treated as an independent canonical journal

### Requirement: Notices have a kind and a place

Every notice SHALL carry a typed kind that fixes its label and whether it is a
transcript row or keypress feedback. Feedback — a reply to the user's own
keypress that changes nothing — SHALL show in the hint row until the next
key and MUST NOT become a transcript entry. Everything else SHALL stay a
transcript row with its current label.

#### Scenario: A refused command

- **GIVEN** a turn is running
- **WHEN** the user submits `/model`
- **THEN** the hint row says the command needs an idle turn and the draft is
  kept
- **AND** no transcript row is added
- **AND** the next keypress clears the message

#### Scenario: A provider retry

- **GIVEN** a provider request is being retried
- **WHEN** Smith reports the retry
- **THEN** the report is a transcript row as before

### Requirement: Child details read as words

The agent inspector and `/agents` SHALL name a child's workspace in the same
words the spawn row uses and SHALL say in words whether an exact resume is
available. They MUST NOT print debug-formatted values.

#### Scenario: Inspecting a read-only child

- **GIVEN** a child running in a read-only view of the project
- **WHEN** the user opens its inspector
- **THEN** the workspace reads as read only, not `ReadOnlyView`
- **AND** resumability reads as words, not `resumable false`

### Requirement: Setup previews are readable to the end

First-run setup SHALL let the user scroll a review or configuration-collision
preview taller than its frame to its last line, with the footer keys always
visible.

#### Scenario: A long merge preview at 44x16

- **GIVEN** a configuration-collision preview longer than the frame
- **WHEN** the user presses Down or PageDown
- **THEN** the preview scrolls to its last line
- **AND** the footer keys remain on screen

### Requirement: Streaming tables do not reflow

The TUI SHALL show a Markdown table that is still arriving at the end of a
streaming answer as its header and a dim note that the table is being
received, and SHALL draw it in full once it is complete. Rows already on
screen MUST NOT move while the table streams.

#### Scenario: A table arrives row by row

- **GIVEN** an answer whose last element is a table still receiving rows
- **WHEN** a later row is wider than the earlier ones
- **THEN** nothing already drawn shifts
- **AND** the complete table is drawn aligned once the table ends or the
  answer commits

### Requirement: Line editing works in a slash draft

The TUI SHALL apply the composer's line-editing keys (Ctrl+A, Ctrl+E,
Ctrl+U, Ctrl+K, Ctrl+W, Alt+B, Alt+F, Left, Right, Home, End, Delete) to a
draft that starts with `/` exactly as to any other draft, while the command
menu stays open and refreshes its matches.

#### Scenario: Clearing a refused command

- **GIVEN** the draft `/agentx` was refused as an unknown command and is
  still in the composer
- **WHEN** the user presses Ctrl+U
- **THEN** the draft is empty
- **AND** typing `/agent` produces the draft `/agent`

### Requirement: Child details state each fact once

The agent inspector and `/agent` SHALL name each fact about a child once,
label every count, use Smith's compact token form, render the child's result
as Markdown, and offer `/agent resume` only when an exact checkpoint exists.
The `/agent` output MUST be headed by the command the user typed.

#### Scenario: Inspecting a finished child without a checkpoint

- **GIVEN** a resumed session whose child finished and has no exact
  checkpoint
- **WHEN** the user runs `/agent child-1`
- **THEN** the session id, durability, turn count, and token count each
  appear once, with the turn count labelled
- **AND** the result's Markdown is rendered, not shown as `**` and
  backticks
- **AND** no `/agent resume` command is offered

### Requirement: Capability activation stays in detail

The TUI SHALL show capability activation notices only when work detail is
expanded and in `/diagnostics`, never in the default transcript.

#### Scenario: A tool is activated through the registry

- **WHEN** a turn activates tools through `registry.search`
- **THEN** the default transcript shows no `activation epoch` line
- **AND** Ctrl+O and `/diagnostics` still show the activated capabilities

### Requirement: Child-agent approvals read like the spawn row

The approval for starting a child agent SHALL name the operation, the task,
and the child's tools and workspace in the words the spawn row uses, and
SHALL list a turn, token, or time limit only when one is set. It MUST NOT
print null values, the unlimited sentinel, internal field names, or internal
target or permission identifiers.

#### Scenario: Approving a write-capable child

- **GIVEN** the model asks to start a child with all tools in the shared
  workspace and no limits
- **WHEN** the approval opens
- **THEN** it reads as starting a child agent with the task, `tools all`,
  and `workspace shared`
- **AND** no `null`, `4294967295`, `deadline_ms`, `delegation.spawn`, or
  `child-agent:session-` text appears
