# client-interaction Specification

## Purpose
TBD - created by archiving change integrate-stable-session-harness. Update Purpose after archive.
## Requirements
### Requirement: Attempt-aware transcript reduction

Smith clients SHALL buffer provider text and reasoning by request and attempt
until Agent Runtime commits or discards that attempt. Live reduction and
journal replay MUST produce an equivalent committed transcript.

#### Scenario: First attempt streams then retries
- **GIVEN** a failed attempt emitted partial visible text
- **WHEN** the runtime discards it and a later attempt commits
- **THEN** the final transcript contains only the committed attempt
- **AND** status may still report usage and a bounded retry diagnostic for the
  failed attempt

### Requirement: Interrupt affects only the active turn

The interactive interrupt action SHALL cancel the current turn and enter a
visible interrupting state without terminally cancelling the session. Session
cancellation remains reserved for confirmed shutdown or revocation.

#### Scenario: Prompt follows an interruption
- **GIVEN** the user interrupted a streaming turn
- **WHEN** its cancelled terminal event arrives and the user submits again
- **THEN** the later turn executes normally on the same session
- **AND** the composer does not inherit a cancelled root token

### Requirement: Prepared approval is exact and queued

The TUI SHALL display the immutable prepared action, exact canonical target,
material arguments, permissions, broad-authority warning, and deadline before
approval. Multiple pending actions MUST use deterministic batching or queuing
and MUST NOT silently supersede each other.

#### Scenario: Parallel calls require approval
- **GIVEN** several prepared calls await decisions
- **WHEN** Smith presents them
- **THEN** every call receives one explicit decision or terminal cancellation
- **AND** no prompt is dropped merely because another prompt arrived

### Requirement: Questionnaire has a distinct interaction surface

The interactive TUI SHALL present agent-originated questionnaires as a
temporary accessible overlay supporting bounded choices, optional free-form
answers, explicit submit/decline, cancellation, deadline, and restored pending
state. Its responder MUST be separate from security approval.

#### Scenario: User selects a design option
- **GIVEN** the active turn asks a structured design question
- **WHEN** the user selects and submits one option
- **THEN** Smith returns the typed answer to the same turn
- **AND** the answer grants no tool authority

### Requirement: Non-interactive interaction fails predictably

An ordinary headless Smith run MUST omit questionnaire readiness or return a
versioned `interaction_required` non-success outcome when no bidirectional
interaction protocol is configured. It MUST NOT wait indefinitely or treat
prompt stdin as an asynchronous answer.

#### Scenario: Headless model requests clarification
- **GIVEN** no interactive broker is configured
- **WHEN** a forced or resumed questionnaire reaches the host
- **THEN** Smith terminates with a structured interaction-required result
- **AND** no answer is fabricated

### Requirement: Child questions route through the parent by default

Smith MUST keep direct questionnaire readiness root-only unless an explicit
agent profile grants attributed child interaction. A child without readiness
SHALL return a structured needs-input result through the parent safe-boundary
path.

#### Scenario: Child encounters ambiguity
- **GIVEN** a child needs a material user choice
- **AND** its profile has no direct interaction readiness
- **WHEN** it returns needs-input
- **THEN** the parent receives the attributed request
- **AND** no competing child overlay opens automatically

### Requirement: Agent-first idle composer

The interactive TUI SHALL identify the active root agent mode, provider/model,
project/branch, and context confidence at the idle point of action without
adding a permanent header, shortcut strip, or focusable region. The composer
remains the sole persistent focus. `?` from an empty composer and `/help` SHALL
render the same bounded local command/composer guide without a provider request
or canonical history entry.

#### Scenario: Empty idle composer at normal width
- **GIVEN** no overlay or turn is active and the composer is empty
- **WHEN** Smith renders at a normal terminal width
- **THEN** it shows the active agent mode beside provider/model and project
- **AND** it does not render a persistent shortcut strip

#### Scenario: Empty idle composer at narrow width
- **GIVEN** the terminal is 44 columns wide
- **WHEN** the identity cannot all fit
- **THEN** Smith removes low-priority path detail first
- **AND** retains agent/activity, approval state, model, and honest context
  provenance without relying on color

#### Scenario: Local help is available on demand
- **GIVEN** no overlay is open and the composer is empty
- **WHEN** the user presses `?` or submits `/help`
- **THEN** Smith renders the same bounded command and composer guide locally
- **AND** creates no provider request or canonical model-history entry

#### Scenario: Interrupted draft is recoverable before exit
- **GIVEN** the composer contains an unsent draft
- **WHEN** the user presses `Ctrl+C` once
- **THEN** Smith clears the composer and stores the draft in bounded local
  recall history
- **AND** `Up` restores the draft without a provider request

#### Scenario: Consecutive control-C presses exit
- **GIVEN** the user has pressed `Ctrl+C` once and no intervening key
- **WHEN** the user presses `Ctrl+C` again within one second
- **THEN** Smith exits from either idle or active work

#### Scenario: Tab cycles an idle root mode
- **GIVEN** the runtime is idle, the composer is empty, and no overlay is open
- **WHEN** the user presses `Tab`
- **THEN** Smith selects the next authorized root agent mode at a safe boundary
- **AND** focus remains in the composer without sending a provider request

#### Scenario: Tab does not replace overlay behavior
- **GIVEN** command completion, reference completion, approval, or a
  questionnaire is open
- **WHEN** the user presses `Tab`
- **THEN** the overlay's documented navigation/completion behavior runs
- **AND** the active root mode does not change

### Requirement: Unified typed reference completion

Smith SHALL provide one `@` completion surface for bounded canonical workspace
files and registered child-agent presets. Resolution MUST occur locally before
provider spend and MUST retain type, provenance, authority, and size metadata.
A bare `@token` that matches a known file or child agent resolves as that
reference. A bare `@token` that matches neither SHALL pass through as literal
text, so ordinary prose containing scoped package names, social handles, or
other leading-at signs sends without an attachment error. Explicit typed
prefixes (`@file:`, `@agent:`) that fail to resolve, and ambiguous names that
collide between files and agents, MUST still report a local bounded error.

#### Scenario: Attach a workspace file

- **GIVEN** the user selects `@src/lib.rs` from file completion
- **WHEN** they submit the prompt
- **THEN** Smith prepares and authorizes an exact workspace read
- **AND** contributes bounded content or an artifact reference with file
  provenance to the planned request

#### Scenario: Unresolvable bare at sign is literal text

- **GIVEN** a draft contains a bare `@token` that matches no workspace file or
  child agent
- **WHEN** the user submits it
- **THEN** Smith sends the prompt with the `@token` as literal text
- **AND** performs no provider request, attachment, or unauthorized read beyond
  the ordinary prompt

#### Scenario: Explicit typed reference escapes the workspace

- **GIVEN** a draft contains an explicit `@file:` or `@agent:` reference that
  does not resolve, or an ambiguous name that is both a file and an agent
- **WHEN** the user submits it
- **THEN** Smith keeps the draft and reports a local bounded error
- **AND** performs no provider request or unauthorized read

#### Scenario: Literal at sign

- **GIVEN** the draft contains the documented `@@` escape
- **WHEN** the user submits it
- **THEN** Smith sends one literal leading `@` at that position
- **AND** does not open or resolve a reference

### Requirement: Prepared local shell shortcut

Input beginning with one non-whitespace `!` SHALL execute as a local shell
action only through Smith's canonical prepared tool executor. It MUST use the
same schema validation, exact workspace, broad permission bound, authorization,
approval, deadline, cancellation, scheduling, output bounding, artifacts, and
checkpoint semantics as a model-requested shell call.

#### Scenario: User submits a shell shortcut
- **GIVEN** the composer contains `!cargo test`
- **WHEN** the user submits it
- **THEN** Smith displays the prepared action and applies resolved approval
  policy before execution
- **AND** renders the committed result locally without a provider request

#### Scenario: Shell approval is denied
- **GIVEN** the prepared shortcut requires approval
- **WHEN** the user denies it
- **THEN** no process starts and the transcript records a bounded denial
- **AND** the denial grants no future authority

#### Scenario: Literal exclamation prompt
- **GIVEN** the draft begins with `!!`
- **WHEN** the user submits it
- **THEN** Smith sends a normal user prompt beginning with one `!`
- **AND** starts no local shell action

### Requirement: Replay-equivalent anchored todo pane

Smith SHALL derive one replaceable todo projection from versioned runtime
events. Public items SHALL render in a bounded, non-focusable pane immediately
above the composer and MUST NOT enter transcript history. Sensitive plan item
text MUST NOT render. `/details` SHALL toggle bounded tool lifecycle detail
without revealing protected arguments. Live reduction and journal replay MUST
produce the same todo projection. A compact picker SHALL temporarily replace
the todo presentation in the anchored pane without changing that projection.

#### Scenario: Multi-step coding turn advances
- **GIVEN** a turn has a public multi-step plan
- **WHEN** plan lifecycle events arrive
- **THEN** the authored todo items update in place immediately above the
  composer
- **AND** the transcript retains only the quiet working timer and ordinary
  attributed events

#### Scenario: Turn reaches a terminal result
- **GIVEN** an anchored todo pane is visible
- **WHEN** the turn succeeds, fails, is interrupted, or reaches a limit
- **THEN** the reconciled terminal todo remains visible until the next turn
  starts
- **AND** Smith commits no aggregate `work` row to the transcript

#### Scenario: Compact interaction replaces the todo presentation
- **GIVEN** an anchored public todo projection is visible
- **WHEN** the user opens command or resource completion
- **THEN** the compact picker replaces the todo presentation directly above
  the fixed composer
- **AND** closing the picker restores the unchanged todo projection
- **AND** the temporary picker controls disappear with it

#### Scenario: Details remain redaction-safe
- **GIVEN** a prepared tool contains a command, edit body, or sensitive answer
- **WHEN** the user invokes `/details`
- **THEN** Smith shows only the reviewed typed projection and lifecycle evidence
- **AND** never reconstructs raw values from redacted events

### Requirement: Addressable child continuation UX

Smith SHALL expose durable existing children separately from new child presets
through typed completion, `/agent`, and `/timeline`. The surface MUST show the
stable child ID, bounded role/task label, durability, model, workspace/tool
posture, state, task usage, and resumability. Selecting or inspecting a child
MUST preserve the root composer draft and MUST NOT itself spend provider tokens.

#### Scenario: Target an idle existing child

- **GIVEN** a durable idle child is relevant to the user's next task
- **WHEN** the user selects its `@child-id` reference and confirms the
  follow-up
- **THEN** Smith sends the task to that exact child as a new turn
- **AND** clearly distinguishes it from selecting a preset that would spawn a
  new child

#### Scenario: Inspect an interrupted child

- **GIVEN** a durable child is interrupted and has a compatible checkpoint
- **WHEN** the user opens it through `/agent`
- **THEN** Smith shows an explicit resume action and the renewed provider/tool
  spend boundary
- **AND** does not resume until that action is confirmed

### Requirement: Live, replay, and headless child state are equivalent

Smith SHALL keep the interactive reducer, persisted journal replay, and
versioned headless projection equivalent for durable child identity, lifecycle, resumability,
cumulative limits, and latest bounded outcome. Protected prompts, tool
arguments, answers, and checkpoints MUST NOT be reconstructed into these
presentation surfaces.

#### Scenario: Restart and continue a child in stream JSON mode

- **GIVEN** a headless consumer observed a child complete before process exit
- **WHEN** a resumed run follows up that child
- **THEN** additive machine events retain the same child and session identity
  with an explicit follow-up transition
- **AND** replay produces the same committed state without protected content

### Requirement: Quiet successful turn terminal

The interactive TUI SHALL treat a successful `TurnCompleted` event as both a
state transition and one concise attributed transcript notice. It MUST close
active streaming/work state, return to idle, and append `turn · completed`
with canonical elapsed duration when valid start/completion timestamps are
available, while retaining canonical lifecycle, usage, journal, and timeline
evidence. Non-success terminals requiring explanation or action MUST remain
visible.

#### Scenario: A visible answer completes successfully

- **GIVEN** a turn has committed visible assistant text
- **AND** its canonical start and completion timestamps form a valid interval
- **WHEN** its successful terminal event arrives
- **THEN** the TUI closes the turn and returns to idle
- **AND** it appends one attributed completion notice with that elapsed time

#### Scenario: Tool or reasoning work completes without answer text

- **GIVEN** a successful turn reports no visible assistant output
- **WHEN** its terminal event arrives
- **THEN** the TUI appends the same successful completion notice
- **AND** it does not label the turn `reasoning only` or infer why text was
  absent

#### Scenario: Successful duration is below one second

- **GIVEN** canonical timestamps show a successful turn lasted less than one
  second
- **WHEN** the completion notice renders
- **THEN** it uses bounded millisecond precision or `<1ms`
- **AND** it does not round the duration to `0s`

#### Scenario: Successful duration is unavailable

- **GIVEN** the start timestamp is absent or later than the completion
  timestamp
- **WHEN** the successful terminal is reduced
- **THEN** the notice says `turn · completed` without a duration
- **AND** it does not substitute local reducer or replay processing time

#### Scenario: A turn does not complete successfully

- **GIVEN** a turn is cancelled, reaches a limit, needs input, or fails
- **WHEN** its terminal event arrives
- **THEN** the TUI renders a concise attributed non-success notice
- **AND** includes locally measured elapsed time when available

#### Scenario: Journaled success is replayed

- **GIVEN** a successful turn's canonical start and completion envelopes were
  journaled
- **WHEN** the transcript is reconstructed by replay
- **THEN** replay reaches the same idle and terminal work state as live
  reduction
- **AND** it derives duration only from the canonical timestamp interval

### Requirement: Unified profile selection surfaces

Smith SHALL present one local profile inventory with clear main, child, or both
placement labels. Startup and `/profile` SHALL select a main-enabled profile;
`@name <task>` SHALL select a child-enabled profile; retained child IDs MUST
remain visibly distinct from profile-based spawn choices.

#### Scenario: Switch the main agent profile
- **GIVEN** Smith is idle and a main-enabled profile is locally valid
- **WHEN** the user selects it with `/profile`
- **THEN** Smith previews or applies one atomic safe-boundary rebuild with the
  profile's behavior and runtime summary
- **AND** status identifies the active profile, posture, provider/model, and
  provenance without exposing raw instructions or secrets

#### Scenario: Select a child-enabled profile
- **GIVEN** the `@` picker contains a child-enabled profile and a retained child
  with a different stable identity
- **WHEN** the user filters or selects an entry
- **THEN** the picker labels the profile as a new child preset and the retained
  identity as a follow-up target
- **AND** it never interprets one as the other

#### Scenario: Profile is unavailable for the requested placement
- **GIVEN** a profile is valid only for `main`
- **WHEN** the user attempts to invoke it as a child
- **THEN** Smith fails locally with its placement and available alternatives
- **AND** preserves the draft before provider spend or child allocation

### Requirement: Main profile cycle order

Smith SHALL replace root-mode cycling with a validated order of main-enabled
profiles. Cycling MUST occur only while idle with an empty composer and no
overlay, and each change MUST use the same safe-boundary profile application
as `/profile`.

#### Scenario: Cycle configured profiles
- **GIVEN** `profile_order` contains distinct valid main-enabled profiles
- **WHEN** the user cycles from an empty idle composer
- **THEN** Smith selects the next profile in declared order
- **AND** atomically updates prompt, posture, provider/model, limits, and status

#### Scenario: Ordered profile is child-only
- **GIVEN** `profile_order` names a profile not enabled for main use
- **WHEN** configuration is resolved
- **THEN** Smith rejects the order with the profile declaration source
- **AND** does not create a partially selectable cycle

### Requirement: Composer input history and reverse search

The interactive TUI SHALL maintain one bounded, process-local composer history
for accepted input and non-blank drafts cleared by the first `Ctrl+C`. It MUST
support lossless arrow-key navigation and local reverse search without
creating a provider request, canonical conversation entry, or durable session
record until the user explicitly accepts and submits recalled input.

#### Scenario: Accepted input enters bounded history
- **GIVEN** the composer contains a non-blank prompt, command, shell shortcut,
  or child-agent request that passes local validation
- **WHEN** Smith accepts the input for its provider, local action, or
  confirmation flow
- **THEN** Smith records the exact pre-normalization composer text in bounded
  process-local history
- **AND** suppresses an adjacent exact duplicate

#### Scenario: Rejected input remains unrecorded
- **GIVEN** composer input fails local parsing, reference resolution,
  availability, or busy-state validation
- **WHEN** the user attempts to submit it
- **THEN** Smith leaves the draft in the composer
- **AND** does not add a history entry

#### Scenario: First control-C stashes the current draft
- **GIVEN** the composer contains a non-blank unsent draft
- **WHEN** the user presses `Ctrl+C` once
- **THEN** Smith records the exact draft in the shared composer history and
  clears the composer
- **AND** retains the existing one-second second-press exit window

#### Scenario: Arrow keys browse without losing the current draft
- **GIVEN** no overlay owns input and composer history is non-empty
- **WHEN** the user presses `Up` from an empty or non-empty composer
- **THEN** Smith preserves the current text as a scratch draft and recalls the
  newest history entry
- **AND** repeated `Up` and `Down` traverse older and newer entries
- **AND** `Down` after the newest entry restores the scratch draft exactly

#### Scenario: Editing recalled input leaves navigation
- **GIVEN** the composer displays a recalled history entry
- **WHEN** the user edits that text
- **THEN** Smith keeps the edited text in the composer and exits active history
  navigation
- **AND** does not record the edit until it is accepted or stashed

#### Scenario: Control-R searches the same history
- **GIVEN** no other overlay owns input
- **WHEN** the user presses `Ctrl+R` and types a query
- **THEN** Smith shows the newest case-insensitive substring match from the
  shared composer history
- **AND** repeated `Ctrl+R` cycles through older matches with bounded wrapping
- **AND** the search performs no provider or persistence I/O

#### Scenario: Reverse search is accepted or cancelled losslessly
- **GIVEN** reverse search is open with an original composer draft
- **WHEN** the user presses `Enter` on a match
- **THEN** Smith places the exact match in the composer without submitting it
- **AND** when the user presses `Esc` instead, Smith restores the original
  draft exactly

#### Scenario: Existing overlays retain keyboard ownership
- **GIVEN** a picker, approval, questionnaire, or confirmation overlay is open
- **WHEN** the user presses `Up`, `Down`, or `Ctrl+R`
- **THEN** Smith preserves that overlay's documented keyboard behavior
- **AND** does not start composer history navigation or search

### Requirement: Local goal controls use one typed host path

The interactive TUI SHALL provide local goal summary, create, edit, budget,
pause, resume, and clear commands through the existing command registry and
one typed goal-control service. Intercepted controls MUST issue no provider
request and MUST use the same validation regardless of direct arguments or
future picker presentation.

#### Scenario: User creates a goal locally

- **GIVEN** the eligible session is idle with no unfinished goal
- **WHEN** the user submits `/goal <objective>`
- **THEN** Smith validates and commits the goal locally without a provider
  request for the command itself
- **AND** the controller may then start an attributed internal goal turn

#### Scenario: User requests a summary

- **GIVEN** any current goal status
- **WHEN** the user submits bare `/goal`
- **THEN** Smith renders bounded objective, status, elapsed time, token usage
  provenance, budget, and stopped reason locally
- **AND** creates no canonical user message or provider request

#### Scenario: User changes a goal budget

- **GIVEN** an idle goal has a positive budget or is budget-limited
- **WHEN** the user submits `/goal budget <positive-tokens|none>`
- **THEN** Smith validates and commits the new optional budget locally
- **AND** a stopped goal remains stopped until the user separately resumes it

#### Scenario: User mutates a busy goal unsafely

- **GIVEN** a turn is serving
- **WHEN** the user attempts create, edit, budget, resume, or clear
- **THEN** Smith refuses the mutation locally as busy and preserves the draft
  or command arguments
- **AND** the serving goal state remains unchanged

#### Scenario: Objective requires deferred attachment handling

- **GIVEN** a goal objective exceeds the direct bound or depends on image/paste
  attachment materialization
- **WHEN** the user attempts creation or edit
- **THEN** Smith reports the unsupported bounded-objective requirement locally
- **AND** does not create attachment files or silently truncate the objective

### Requirement: Goal-aware interruption pauses automatic work

The interactive goal pause action SHALL be available while a goal turn is
serving. It MUST serialize the pause request with interruption and final
accounting so the goal reaches one paused state; ordinary non-goal interruption
retains its existing turn-local behavior.

#### Scenario: Pause active goal turn

- **GIVEN** an active goal owns the serving turn
- **WHEN** the user invokes `/goal pause` or the documented goal-aware interrupt
- **THEN** Smith enters visible interrupting state, cancels that turn, and
  commits `paused` after final accounting
- **AND** no automatic continuation is admitted between interruption and pause

#### Scenario: Interrupt ordinary turn

- **GIVEN** no active goal owns the serving turn
- **WHEN** the user invokes the ordinary interrupt action
- **THEN** Smith cancels only that turn under the existing contract
- **AND** creates or changes no goal state

### Requirement: Compact replay-equivalent goal visibility

The TUI SHALL derive one compact non-focusable goal projection from restored
state and typed runtime events. It SHALL distinguish goal status from the
per-turn todo pane, show token and elapsed provenance honestly, remain legible
without color, and produce equivalent live and journal-replay state.

#### Scenario: Active goal renders with a todo plan

- **GIVEN** an active persistent goal and a public todo plan for the current
  turn
- **WHEN** Smith renders the composer area
- **THEN** compact goal status remains distinguishable from todo item progress
- **AND** neither projection is duplicated into ordinary transcript history

#### Scenario: Goal reaches a stopped state

- **GIVEN** the current goal becomes paused, blocked, usage-limited,
  budget-limited, or complete
- **WHEN** the typed event commits
- **THEN** the compact projection updates status and bounded reason in place
- **AND** status persists across later idle rendering and compatible resume

#### Scenario: Token usage is unknown

- **GIVEN** an unbudgeted goal lacks provider-reported usage evidence
- **WHEN** Smith renders goal status or summary
- **THEN** it labels token usage unknown while retaining derived elapsed time
- **AND** does not display zero as if it were reported

#### Scenario: Journaled goal state is replayed

- **GIVEN** the journal contains durability-aligned goal updates
- **WHEN** the TUI rebuilds presentation from replay
- **THEN** it reaches the same goal projection as live reduction
- **AND** replay triggers no host control or automatic turn

### Requirement: Goal commands are discoverable and bounded

`/help` SHALL list the supported goal command grammar and state that goal work
may span multiple provider turns. Errors for missing goals, invalid status,
busy mutation, invalid objective, and invalid budget MUST be local, bounded,
and actionable.

#### Scenario: User opens local help

- **GIVEN** goal capability is available for the current session
- **WHEN** the user submits `/help`
- **THEN** help lists summary, create, edit, budget, pause, resume, and clear
  forms
- **AND** names persistent multi-turn execution and local control behavior

#### Scenario: Goal capability is unavailable

- **GIVEN** the session is ephemeral, a child, or a review surface
- **WHEN** the user attempts `/goal`
- **THEN** Smith explains locally why persistent goals are unavailable
- **AND** neither provider work nor local goal state is created

### Requirement: Busy ordinary input has explicit steer and queue intent

Smith SHALL give ordinary user input explicit steer and queue intent while
eligible provider-backed work is serving and no overlay owns input. `Enter` on
a valid ordinary prompt targets the active turn, while `Tab` on a non-empty
ordinary prompt explicitly queues a future turn. Local commands, shell
shortcuts, child-agent actions, approvals, and questionnaires MUST retain their
distinct validation and input ownership.

#### Scenario: Enter steers serving work

- **GIVEN** an eligible provider-backed turn is serving
- **AND** the composer contains a valid ordinary user prompt
- **WHEN** the user presses `Enter`
- **THEN** Smith targets the serving runtime turn with that input
- **AND** does not submit a separate whole turn merely to stage it

#### Scenario: Tab queues a later turn

- **GIVEN** a turn is serving and the composer contains a valid ordinary user
  prompt
- **WHEN** the user presses `Tab` outside an overlay
- **THEN** Smith stores that prompt as a bounded process-local future turn
- **AND** does not send it to Agent Runtime until an eligible terminal boundary

#### Scenario: Existing input owner keeps precedence

- **GIVEN** a palette, picker, approval, questionnaire, or confirmation owns
  input
- **WHEN** the user presses `Enter` or `Tab`
- **THEN** that surface retains its documented behavior
- **AND** Smith neither steers nor queues the composer draft accidentally

### Requirement: Pending input remains ordered and editable where safe

Smith SHALL keep accepted-but-uncommitted steers, rejected-steer follow-ups,
and explicit future turns in separate bounded FIFO state. The user MAY restore
the newest explicit future turn for editing, but MUST NOT edit an input already
accepted by the active runtime turn.

#### Scenario: User edits the newest queued turn

- **GIVEN** two explicit future turns are queued and no modal owns input
- **WHEN** the user invokes the queued-input edit shortcut
- **THEN** Smith removes the newest queued entry and restores it exactly to the
  composer
- **AND** preserves the older entry in FIFO order

#### Scenario: Rejected steer precedes an ordinary queue

- **GIVEN** a steer is rejected because the serving work is not steerable
- **AND** an ordinary future turn is already queued
- **WHEN** the serving work completes successfully
- **THEN** Smith dispatches the rejected steer follow-up first
- **AND** retains the ordinary queued turn for a later boundary

### Requirement: Terminal handling is lossless and exactly once

Smith SHALL remove pending steer text from process-local state only after the
runtime reports its committed or discarded disposition. A successful terminal
boundary SHALL start at most one queued follow-up, while interruption and other
non-success outcomes MUST preserve uncommitted input without duplication.

#### Scenario: Steer commits within the active turn

- **GIVEN** Smith displays an accepted pending steer
- **WHEN** Agent Runtime reports that steer committed at a safe boundary
- **THEN** Smith appends its user transcript row at that boundary exactly once
- **AND** removes only the matching pending preview

#### Scenario: Interrupt sends pending steers immediately

- **GIVEN** one or more steers are accepted but uncommitted
- **WHEN** the user invokes the documented interrupt-for-steer action
- **THEN** Smith interrupts the serving turn
- **AND** after cancellation merges and submits the still-uncommitted steers as
  one ordinary turn in FIFO order
- **AND** never resubmits a steer already reported committed

#### Scenario: Turn fails with pending input

- **GIVEN** pending or queued user input exists
- **WHEN** the serving turn fails, reaches a limit, or returns needs-input
- **THEN** Smith restores the uncommitted material for explicit user review
- **AND** performs no automatic provider spend for that material

### Requirement: Registered composer material is edited atomically

Smith SHALL treat each complete composer placeholder backed by registered
large-paste or clipboard-image material as one logical editing unit. A
registered placeholder MUST expose cursor positions only at its start and end,
MUST be removed as a whole by adjacent backward or forward deletion, and MUST
retain its complete compact label while editable. When input is committed,
Smith MUST replace registered pasted-text labels with their exact stored text
in the user transcript while retaining registered image labels.

#### Scenario: Move across a pasted-text placeholder

- **GIVEN** the composer contains text followed by a registered
  `[Pasted text #N +L lines]` placeholder followed by more text
- **WHEN** the user moves horizontally across the placeholder
- **THEN** one `Left` or `Right` press moves between its end and start boundary
- **AND** the cursor never stops inside the label

#### Scenario: Delete a pasted-text placeholder backward

- **GIVEN** the cursor is immediately after a registered pasted-text
  placeholder
- **WHEN** the user presses `Backspace`
- **THEN** Smith removes the complete placeholder from the composer
- **AND** no fragment of its label or raw content remains in that draft

#### Scenario: Delete an image placeholder forward

- **GIVEN** the cursor is immediately before a registered clipboard-image
  placeholder
- **WHEN** the user presses `Delete`
- **THEN** Smith removes the complete placeholder from the composer
- **AND** that image is not included when the edited draft is prepared

#### Scenario: Adjacent placeholders remain distinct units

- **GIVEN** two registered placeholders are adjacent in the composer
- **WHEN** the user moves or deletes at their shared boundary
- **THEN** Smith targets exactly the placeholder indicated by the movement or
  deletion direction
- **AND** leaves the other placeholder and its material unchanged

#### Scenario: Ordinary text remains character-addressable

- **GIVEN** the composer contains Unicode text, an image path, or text shaped
  like an unregistered paste or image placeholder
- **WHEN** the user moves through or deletes that text
- **THEN** Smith applies ordinary Unicode-safe character editing
- **AND** does not expand paste content or attach an image for that text

#### Scenario: Commitment expands text but retains image labels

- **GIVEN** a draft contains registered text-paste and clipboard-image
  placeholders that were not deleted
- **WHEN** Smith commits the input and renders its user transcript entry
- **THEN** each pasted-text label is replaced by its exact stored content in
  that transcript entry and in provider text
- **AND** each image label remains visible in that transcript entry
- **AND** each real clipboard image is submitted as image content in
  placeholder order

#### Scenario: Uncommitted projections stay compact

- **GIVEN** registered pasted-text or image material is still editable, queued,
  or recalled from composer history
- **WHEN** Smith renders that uncommitted input
- **THEN** Smith keeps its registered compact labels instead of expanding raw
  pasted text
- **AND** the labels retain atomic movement and deletion behavior while their
  material remains registered
