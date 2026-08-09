# client-surfaces Specification

## Purpose
TBD - created by archiving change add-smith-agent-harness. Update Purpose after archive.
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
cache state, active monitors, direct children, and queued notifications. An
estimated or unknown value MUST be visually distinguishable from a
provider-reported value.

#### Scenario: Provider switch leaves estimated context

- **GIVEN** the user switches to a provider that has not reported usage
- **WHEN** the status line updates
- **THEN** it labels context tokens estimated
- **AND** does not reuse the prior provider's verified cache indicator

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

The TUI MUST request confirmation before exiting with active monitors or
children. Non-interactive mode SHALL support `error`, `wait`, and `stop`
background-exit policies and MUST default to `error`.

#### Scenario: Headless turn finishes with a persistent monitor

- **GIVEN** the final answer is ready while a persistent monitor remains
- **WHEN** the caller did not choose an exit policy
- **THEN** Smith emits an active-work error describing the monitor
- **AND** does not silently orphan it

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
in focus order.

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
- **THEN** a concise attributed notice appears inline without stealing focus
- **AND** detailed child state remains available through `/agent`

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
connection ceremony MUST NOT send an inference request, and durable changes
MUST use the reviewed user-scope credential transaction.

#### Scenario: Connect OpenRouter from the provider picker

- **GIVEN** the user submits `/connect` while the session is idle
- **WHEN** they select OpenRouter, choose protected API-key storage, enter a
  key, and confirm the secret-free review
- **THEN** Smith stores the key through the reviewed credential transaction
- **AND** records the standard OpenRouter provider endpoint without requiring
  the user to type it
- **AND** sends no inference request during connection

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
