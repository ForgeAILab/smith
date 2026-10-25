# runtime-integration Specification

## Purpose
How Smith composes and drives Agent Runtime as its canonical mechanism, and how runtime events reach Smith's clients.
## Requirements
### Requirement: Shared runtime is the canonical mechanism

Smith SHALL use the versioned `agent-runtime` facade as the sole owner of model
and context planning, provider normalization, the provider/tool loop, tool
execution, cancellation, canonical runtime events, usage accounting, and
registry mechanism. Smith MUST NOT maintain a parallel implementation of those
behaviors. Smith SHALL expose a versioned product-level session and event
projection to presentation clients, but that projection MUST adapt the shared
mechanism and MUST NOT become a second execution loop, journal, or source of
canonical state.

#### Scenario: Smith executes a configured turn

- **GIVEN** Smith has resolved product configuration and host policy
- **WHEN** it starts a session and sends user input
- **THEN** the turn executes through `agent-runtime::Runtime`
- **AND** canonical persistence consumes shared runtime events
- **AND** presentation clients consume the versioned Smith projection

#### Scenario: Shared mechanism needs a new capability

- **GIVEN** Smith needs a provider or execution mechanism absent from the
  pinned runtime
- **WHEN** the ownership boundary is evaluated
- **THEN** the mechanism is proposed and implemented in Agent Runtime first
- **AND** Smith consumes it only after a compatible version is available

### Requirement: Versioned dependency with local override

A releasable Smith manifest MUST depend on Agent Runtime through an exact
semantic version or exact Git revision. A sibling path MAY be used only through
an uncommitted, git-ignored Cargo patch that can be removed without source
changes.

#### Scenario: Build Smith without sibling checkouts

- **GIVEN** only the Smith repository and its declared package sources are
  available
- **WHEN** a normal release build resolves dependencies
- **THEN** it obtains the pinned Agent Runtime source
- **AND** it does not require `../agent-runtime`

#### Scenario: Develop Smith and Agent Runtime together

- **GIVEN** both repositories are sibling checkouts
- **WHEN** a developer enables the documented local Cargo patch
- **THEN** Smith resolves the runtime from the local path
- **AND** removing the patch restores the pinned source without Rust or
  manifest dependency-line edits

### Requirement: Complete preflight runtime composition

Before terminal entry or provider network I/O, Smith SHALL resolve and validate
the selected provider, credential reference, model, model profile or catalog,
context policy, product prompt, loop limits, tools, approval policy, workspace,
stores, observers, and shutdown policy. Smith MUST fail closed when any required
input is missing or inconsistent.

#### Scenario: Model limits are missing

- **GIVEN** the selected model has no explicit profile and no catalog source
  provides enforceable limits
- **WHEN** Smith prepares the runtime
- **THEN** startup fails with a model-profile diagnostic
- **AND** no provider request is sent
- **AND** the terminal is not left in raw or alternate-screen mode

#### Scenario: Resolved composition is valid

- **GIVEN** every required provider, model, context, and host-policy value
  resolves
- **WHEN** Smith builds the runtime
- **THEN** it maps those values through `RuntimeBuilder`
- **AND** the emitted model-profile and planning events identify the resolved
  provider, model, revisions, limits, and provenance

### Requirement: One Smith composition path

Smith SHALL use one runtime factory for the interactive TUI, non-interactive
CLI, deterministic tests, direct child sessions, and future Forge adapter.
Host-specific presentation MAY differ, but runtime behavior MUST NOT be
reimplemented at an entry point.

#### Scenario: Compare TUI and headless execution

- **GIVEN** identical resolved configuration, host adapters, and fake-provider
  input
- **WHEN** a turn runs through the TUI and `smith -p`
- **THEN** both construct equivalent shared runtime policy
- **AND** their canonical shared events and usage differ only in declared
  presentation metadata

### Requirement: Coordinated runtime compatibility gate

Smith SHALL maintain integration tests for its resolved builder composition and
participate in Agent Runtime's Smith consumer conformance gate. A compatible
runtime update MUST NOT be accepted while either gate fails.

#### Scenario: Runtime adds a required model-profile field

- **GIVEN** a candidate runtime revision changes construction or event behavior
- **WHEN** the Smith integration and shared consumer suites run
- **THEN** any missing Smith mapping fails before the dependency is updated
- **AND** the migration is documented rather than hidden by permissive defaults

### Requirement: Coordinated stable runtime pipeline

Every Smith surface SHALL use the one Smith runtime factory over a compatible
Agent Runtime release implementing session-scoped planning, prepared
invocations, attempt-scoped output, structured turn control, checkpoints, and
activation epochs. Smith MUST NOT retain fallback copies of those mechanisms.

#### Scenario: TUI and headless run the same fixture
- **GIVEN** identical resolved Smith policy and fake-provider input
- **WHEN** the TUI and headless hosts execute the fixture
- **THEN** their canonical runtime semantics and committed event sequence are
  equivalent
- **AND** only presentation projections differ

### Requirement: Smith built-ins use ability activation

Smith SHALL register its built-in coding tools and standard harness components
through Agent Runtime abilities with accurate affordances, typed permission
upper bounds, risk, context cost, readiness, provenance, and revision. The
provider tool surface MUST be materialized from a frozen activation epoch.
Every epoch SHALL contain the core tools the session's posture and capability
limits authorize (`read`, `list`, `search`, `edit`, `shell`, `task_output`,
`task_stop`) from the first provider request, without depending on
retrieval. Capabilities outside the core SHALL enter an epoch only through
pre-activation or the agent's own discovery.

#### Scenario: Core tools on the first request
- **GIVEN** a build-posture session with no capability limits
- **WHEN** the first provider request is sent, whatever the prompt says
- **THEN** the tool list contains `read`, `list`, `search`, `edit`, `shell`,
  `task_output`, and `task_stop`

#### Scenario: Read-only posture
- **GIVEN** a profile whose posture is read-only
- **WHEN** the first provider request is sent
- **THEN** the tool list contains `read`, `list`, and `search`
- **AND** `edit` and `shell` are not advertised

#### Scenario: Core set does not fit
- **GIVEN** a capability budget smaller than the authorized core tools
- **WHEN** the session starts
- **THEN** startup fails and names the budget
- **AND** no core tool is silently dropped

### Requirement: One product composition path

Smith SHALL map prompt sections, ability sources, approval, interaction,
workspace, stores, artifacts, memory, tools, provider, model, clock, and
observers through `smith-runtime::factory` for terminal, headless, child, test,
and embedded surfaces.

#### Scenario: Child runtime is constructed
- **GIVEN** a root agent delegates a read-only child
- **WHEN** Smith constructs the child runtime
- **THEN** it uses the same factory and shared harness mechanism
- **AND** product policy narrows delegation and mutation abilities without
  creating a second execution loop

### Requirement: One Smith factory composes durable child recovery

Smith SHALL compose Agent Runtime's child catalog, child session/checkpoint
stores, lifecycle leases, policy revisions, and recovery operation through the
same `smith-runtime::factory` path used by root, TUI, headless, test, and
embedded sessions. It MUST NOT add a Smith-local child loop, reconstruct exact
state from journals, or bypass runtime authorization/checkpoint semantics.

#### Scenario: Rebuild a recovered read-only child

- **GIVEN** a durable read-only child is idle after parent restart
- **WHEN** a follow-up requires its runtime to be lazily reconstructed
- **THEN** the one factory composes the original compatible provider/model,
  protected stores, workspace, and narrowed read-only ability view
- **AND** the child still receives no delegation-management ability

#### Scenario: Current policy would widen authority

- **GIVEN** a recovered child record declares a narrower tool/workspace policy
  than current defaults
- **WHEN** Smith rebuilds it
- **THEN** the factory retains the recorded upper bound or fails closed
- **AND** defaults do not silently grant edit, shell, network, interaction, or
  child-management authority

### Requirement: Conditional internal goal turns

Agent Runtime SHALL provide a bounded provenance-bearing internal turn that can
be accepted only if the session remains idle at one serialized decision
boundary. Internal goal input MUST NOT append a user-role canonical history
message, queue ahead of real user work, bypass ordinary runtime policy, or
survive as unattributed background work.

#### Scenario: Idle session accepts a continuation

- **GIVEN** a persistent root session is idle with an active goal
- **WHEN** its controller conditionally requests one goal continuation
- **THEN** Agent Runtime accepts an attributed internal turn
- **AND** applies the normal provider, context, tool, approval, workspace,
  checkpoint, cancellation, retry, and global-limit contracts

#### Scenario: Real user input wins the idle race

- **GIVEN** an active goal becomes idle while the user submits a prompt
- **WHEN** user and automatic work compete for the session boundary
- **THEN** the automatic request returns busy or is skipped rather than queued
- **AND** the user's ordinary turn retains submission priority

#### Scenario: Continuation history is inspected

- **GIVEN** one or more internal goal turns completed
- **WHEN** canonical session history is persisted or resumed
- **THEN** it contains no fabricated user continuation message
- **AND** lifecycle/checkpoint evidence still identifies each internal turn and
  its goal source

### Requirement: Reusable goal harness component

The shared runtime SHALL own goal state decoding, validation, context
contribution, tool-result mutation, usage accounting inputs, turn-commit
mutation, typed event projection, and controller deduplication as one versioned
harness capability. Smith MUST consume that capability through its one runtime
factory and MUST NOT retain a parallel TUI or headless goal state machine.

#### Scenario: Goal tool mutation commits

- **GIVEN** a goal tool returns a valid state mutation
- **WHEN** the harness processes its exact tool output
- **THEN** the component state, model-facing result, and typed goal event commit
  at one durability-aligned boundary
- **AND** live clients never observe a goal event for a discarded mutation

#### Scenario: Goal context is planned

- **GIVEN** a goal exists for the serving turn
- **WHEN** Agent Runtime builds the provider request
- **THEN** it contributes bounded goal identity, objective, status, usage, and
  remaining-budget evidence as a versioned no-cache fragment
- **AND** the context planner budgets and fingerprints that fragment normally

#### Scenario: Terminal event is replayed twice internally

- **GIVEN** the controller observes duplicate or replayed terminal evidence for
  one goal generation
- **WHEN** it evaluates automatic continuation
- **THEN** at most one new internal turn is accepted
- **AND** deduplication does not suppress a later distinct active generation

### Requirement: Goal ability scope is explicit

Smith SHALL install a stable dormant goal tool ability only for persistent root
sessions so direct natural-language goal requests do not depend on heuristic
intent classification. Tool instructions MUST restrict creation to explicit
user or higher-priority intent, and goal context/state/events SHALL remain
absent until a goal exists. Child, review, and explicitly ephemeral sessions
MUST NOT advertise or inherit root goal control.

#### Scenario: Ordinary persistent root turn

- **GIVEN** no goal exists and the user expresses no goal intent
- **WHEN** Smith freezes the turn's ability epoch
- **THEN** the fixed eligible-session goal tool schemas remain stable
- **AND** no goal context fragment, state, event, or automatic turn appears

#### Scenario: Existing goal resumes

- **GIVEN** a persistent root session restores a current goal
- **WHEN** the next ability epoch is derived
- **THEN** required goal tools and context are available without a new creation
  phrase
- **AND** the exact component revision appears in composition evidence

#### Scenario: Child session is constructed

- **GIVEN** a root session with an active goal delegates a child
- **WHEN** Smith derives the child's scoped ability view and snapshot
- **THEN** the child receives no root goal state or goal tools
- **AND** child work remains attributable through the existing delegation path

### Requirement: Non-goal runtime behavior remains unchanged

Adding goal support SHALL preserve ordinary user-turn submission, canonical
history, provider/tool execution, interruption, persistence, replay, and
headless terminal semantics when no goal exists or goal capability is inactive.

#### Scenario: Existing no-goal fixture runs

- **GIVEN** a pre-change deterministic TUI/headless fixture with no goal intent
- **WHEN** it runs through the goal-capable build
- **THEN** its canonical messages, committed tool results, lifecycle outcomes,
  and usage semantics are equivalent
- **AND** no goal event or automatic turn appears

### Requirement: Smith delegates steering semantics to Agent Runtime

Smith SHALL use Agent Runtime's typed active-turn steering admission and
disposition contracts. It MUST NOT emulate steering by submitting another
whole turn, mutating an in-flight provider request, parsing provider output, or
treating the generic monitor/child injection inbox as an indistinguishable user
steer.

#### Scenario: Runtime accepts a steer

- **GIVEN** Smith tracks the eligible serving turn identity
- **WHEN** the runtime accepts a matching ordinary user input as a steer
- **THEN** Smith retains the returned stable steer identity until disposition
- **AND** the runtime owns safe-boundary delivery and same-turn continuation

#### Scenario: Serving turn changes during submission

- **GIVEN** Smith's tracked turn identity becomes stale before steering
- **WHEN** Agent Runtime returns a typed mismatch or no-active-turn result
- **THEN** Smith retries at most once against a reported eligible active turn or
  falls back to ordinary idle submission
- **AND** the input is neither lost nor submitted twice

### Requirement: User input wins automatic continuation admission

Smith SHALL dispatch pending real-user input before allowing an idle-only goal
or child-completion continuation attempt at the same serialized boundary. An
accepted steer to an automatically owned serving turn MUST remain real user
input and MUST NOT change goal, delegation, authority, cache, or accounting
identity implicitly.

Goal and child-completion controllers SHALL use Agent Runtime's same
conditional idle-only admission mechanism. They MUST defer locally before
calling Runtime or handle `ChildCompletionAdmission::Busy` when user input has
won, and MUST NOT queue a competing provider turn behind it. Protected child
outcomes remain must-deliver until included at a safe boundary or consumed by
a later admitted continuation.

#### Scenario: Goal and queued user input reach an idle boundary

- **GIVEN** an active goal is eligible for automatic continuation
- **AND** Smith has one queued real-user turn
- **WHEN** the serving turn reaches its terminal boundary
- **THEN** Smith submits the real-user turn first
- **AND** the goal controller observes busy or waits for a later boundary

#### Scenario: Child completion and user input race

- **GIVEN** a parked parent receives a terminal child outcome
- **AND** real user input becomes ready at the same boundary
- **WHEN** Agent Runtime serializes admission
- **THEN** the real-user turn wins
- **AND** the child outcome is included safely or remains protected
- **AND** no child-completion provider turn starts concurrently

#### Scenario: Goal and child completion are both eligible

- **GIVEN** an idle parent has an active goal and ready terminal child outcomes
- **WHEN** automatic continuation admission is arbitrated
- **THEN** Agent Runtime admits at most one attributed continuation
- **AND** the other source remains eligible for a later boundary without loss
  or duplicate provider work

### Requirement: Pending input is process-local until runtime commitment

Smith SHALL label steering and future-turn queue state as process-local and
MUST include it in live-work exit policy. Session journals and replay MUST
represent only runtime-committed input; they MUST NOT fabricate commitment for
pending state lost to an unclean process exit.

#### Scenario: Process exits before steer commitment

- **GIVEN** a steer was accepted in process but no committed disposition was
  recorded
- **WHEN** a later process resumes the canonical session
- **THEN** replay does not claim that the steer entered model history
- **AND** Smith does not invent or automatically resend unavailable text

### Requirement: Experimental Smith-native ChatGPT authentication

Smith SHALL offer ChatGPT subscription login as an explicitly experimental,
unsupported direct integration. Smith SHALL perform the trusted browser PKCE
or device-code ceremony itself, exchange and refresh tokens at the fixed
issuer, and persist only its own versioned bundle in the fixed owner-only
plaintext `~/.smith/auth.json` store.

#### Scenario: Connect with ChatGPT in a browser

- **GIVEN** the user selects experimental ChatGPT browser login from `/connect`
- **WHEN** Smith opens the reviewed authorization URL and receives a valid
  state-bound loopback callback
- **THEN** Smith exchanges the code directly, validates the returned account
  identity, and commits the owner-only auth-file/config transaction
- **AND** no Codex process or another client's auth cache is required
- **AND** no Keychain or Secret Service operation occurs

#### Scenario: Device-code login is disabled by policy

- **GIVEN** the account or workspace does not permit device-code login
- **WHEN** the issuer rejects that method
- **THEN** Smith reports a fixed classified policy failure
- **AND** retains browser login or API-key choices without selecting one
  automatically

#### Scenario: Callback state is forged

- **GIVEN** a browser callback has a missing or mismatched state or an
  unexpected target
- **WHEN** Smith's loopback listener receives it
- **THEN** Smith rejects the callback, writes no credential, and closes the
  bounded ceremony
- **AND** callback parameters are absent from diagnostics and render state

### Requirement: Direct ChatGPT Responses execution

Smith SHALL call the fixed experimental ChatGPT Codex Responses backend
directly through its normal Agent Runtime provider path. Status and help MUST
identify Smith as execution owner and label the public support boundary.

#### Scenario: Start ChatGPT-backed work

- **GIVEN** Smith-native login and direct-provider preflight succeed
- **WHEN** the user selects a trusted ChatGPT model
- **THEN** Smith sends canonical work through the dedicated Responses adapter
- **AND** the ordinary Smith runtime owns tools, approvals, persistence,
  cancellation, recovery, events, and usage
- **AND** no external agent loop is started

#### Scenario: Required Smith policy is unavailable

- **GIVEN** a request cannot be represented without violating a Smith tool,
  approval, checkpoint, or recovery guarantee
- **WHEN** the adapter preflights or decodes it
- **THEN** the request fails before unsafe work is accepted
- **AND** does not silently substitute Codex behavior

### Requirement: No external client dependency or credential reuse

Smith MUST NOT launch Codex for login or inference, extract Codex/OpenCode
managed tokens, or read another client's token cache. The trusted integration
MAY pin public native-client parameters and the currently observed direct
backend only behind the approved experimental disclosure.

#### Scenario: Codex is not installed

- **GIVEN** no Codex executable or auth cache is present
- **WHEN** the user connects and runs the experimental ChatGPT provider
- **THEN** login and inference remain available through Smith's own OAuth,
  auth file, refresh source, and HTTP adapter
- **AND** no behavior changes based on Codex installation state

#### Scenario: Direct contract becomes incompatible

- **GIVEN** the undocumented OAuth or backend behavior changes incompatibly
- **WHEN** Smith detects a fixed protocol, authentication, or stream failure
- **THEN** it reports the experimental integration as unavailable with a
  redaction-safe actionable message
- **AND** points to OpenAI Platform API-key access as the supported fallback

### Requirement: One appended budget notice before the compaction boundary

When the remaining input budget crosses a configured threshold, Smith SHALL
append exactly one bounded notice to the conversation for the current
compaction window, informing the model that the context boundary is near. The
notice MUST be appended after existing content so it never rewrites history,
and MUST NOT be repeated until a new compaction window begins.

#### Scenario: The notice is delivered once per window
- **GIVEN** a session whose remaining input budget has crossed the notice
  threshold
- **WHEN** two further turns are planned without compaction occurring
- **THEN** exactly one notice appears in the conversation

#### Scenario: The notice does not rewrite history
- **GIVEN** a session that receives the budget notice
- **WHEN** the provider request is assembled
- **THEN** every message preceding the notice is byte-identical to the previous
  request's corresponding message

#### Scenario: A new window re-arms the notice
- **GIVEN** a session that received the notice and then compacted
- **WHEN** the remaining budget crosses the threshold again
- **THEN** one further notice is delivered

### Requirement: Session usage is reported and recorded

On exit, Smith SHALL report the session's token totals per counter kind with
their provenance, never presenting a derived or estimated count as
provider-reported. Smith SHALL also append one bounded usage record per session
to a durable log containing the session identity, model, turn count, per-counter
totals with confidence, the number of compaction windows, and the number of
budget notices and semantic summaries produced.

#### Scenario: Exit reports totals with provenance
- **GIVEN** a session whose input counts were provider-reported and whose
  reasoning counts were estimated
- **WHEN** the user exits the TUI
- **THEN** the summary marks the estimated counts as estimated
- **AND** it does not mark them as reported

#### Scenario: The usage record carries no conversation content
- **GIVEN** a completed session
- **WHEN** its usage record is appended
- **THEN** the record contains counts, identities, and trigger tallies
- **AND** it contains no prompt text, tool arguments, or file contents

#### Scenario: Compaction behavior is analyzable across sessions
- **GIVEN** several completed sessions
- **WHEN** their usage records are read
- **THEN** each records how many compaction windows and semantic summaries
  occurred
- **AND** a threshold change can be evaluated against them

### Requirement: Semantic summarization triggers on context pressure

Semantic summarization SHALL be triggered by measured input-budget pressure
rather than by a count of completed turns. The trigger MUST compare usage
accumulated after the stable cached prefix against a configured fraction of the
resolved input budget. A minimum completed-turn count MUST remain as an
eligibility floor so a short session is never summarized, but reaching that
count alone MUST NOT trigger summarization.

#### Scenario: A long but small session is not summarized
- **GIVEN** a session with ten completed turns whose post-prefix usage is well
  under the configured fraction of the input budget
- **WHEN** the next turn is planned
- **THEN** no semantic summary is produced
- **AND** no summary model call is made

#### Scenario: A short session with a large tool result is summarized
- **GIVEN** a session past the minimum turn floor whose post-prefix usage
  crosses the configured fraction after one large tool result
- **WHEN** the next turn is planned
- **THEN** a semantic summary is produced

#### Scenario: The turn floor prevents summarizing a young session
- **GIVEN** a session below the minimum completed-turn floor
- **WHEN** post-prefix usage crosses the configured fraction
- **THEN** no semantic summary is produced

#### Scenario: A large stable prefix does not pull the trigger forward
- **GIVEN** two sessions with identical conversation bodies
- **AND** one activates substantially more skills and project instructions
  than the other
- **WHEN** both are planned
- **THEN** neither triggers summarization earlier than the other on account of
  its prefix size

### Requirement: Child completion uses conditional internal turns

Smith SHALL call Agent Runtime's `try_admit_child_completion_if_idle` with a
`ChildCompletionAdmissionRequest` and expected `ChildOutcomeCursor`. Runtime
may accept the bounded provenance-bearing child-completion turn only if the
parent remains idle at one serialized decision boundary and returns
`ChildCompletionAdmission::{Accepted, Busy, Stale, Shutdown, Conflict}`. An
accepted turn SHALL be attributed as `delegation.child-completion`, consume
every ready protected terminal outcome in deterministic order, and use the
parent runtime composition applicable at admission. When that composition
runs turns on an external agent backend, the accepted turn SHALL run on that
backend as "Internal turns on a harness session run on the harness"
requires.

The internal input MUST NOT append a fabricated user-role canonical message,
queue ahead of real user work, bypass the provider, context, tool, approval,
workspace, checkpoint, cancellation, retry, and global-limit contracts an
ordinary turn on that session observes, or survive as unattributed
background work. Admission identity and outcome consumption SHALL be crash-
and replay-safe so one committed continuation is not repeated.

Agent Runtime SHALL own canonical admission and cache operation/evidence event
payloads used by this turn. Smith may consume and project those redaction-safe
events into product surfaces, but MUST NOT define local RuntimeEvent variants
or a second normalized provider event vocabulary.

#### Scenario: Idle parked session accepts child completion

- **GIVEN** a root session on a model provider is idle in
  `parked-awaiting-child`
- **AND** one or more protected terminal outcomes are ready
- **WHEN** its controller conditionally requests continuation
- **THEN** Agent Runtime accepts one attributed internal turn
- **AND** normal context planning, provider execution, usage, and cache
  observation apply

#### Scenario: Idle parked harness session accepts child completion

- **GIVEN** a root session on a harness is idle in `parked-awaiting-child`
- **AND** one or more protected terminal outcomes are ready
- **WHEN** its controller conditionally requests continuation
- **THEN** Agent Runtime accepts one attributed internal turn
- **AND** the turn runs on the harness with no provider request

#### Scenario: Parent became busy before admission

- **GIVEN** terminal outcomes were injected while the parent appeared idle
- **WHEN** another turn wins before conditional admission commits
- **THEN** the child-completion attempt returns
  `ChildCompletionAdmission::Busy`
- **AND** the outcomes remain protected for the next safe boundary

#### Scenario: Continuation history is inspected

- **GIVEN** one or more child-completion internal turns committed
- **WHEN** canonical history, lifecycle events, or checkpoints are inspected
- **THEN** no fabricated user continuation message is present
- **AND** attributed evidence identifies each internal turn and consumed
  outcome set

#### Scenario: Replay sees a committed child continuation

- **GIVEN** a child-completion turn and its outcome-consumption watermark
  committed before a crash
- **WHEN** Smith resumes and replays the journal
- **THEN** presentation can reconstruct the attributed lifecycle
- **AND** replay cannot schedule the committed provider turn again

### Requirement: Parked state performs no provider work by itself

Smith SHALL perform no provider work solely because a session is parked.
Entering, remaining in, listing, inspecting, persisting, resuming, or leaving
`parked-awaiting-child` SHALL perform no provider request by itself. Provider
work may begin only for admitted real-user, goal, or child-completion turns or
for separately authorized cache maintenance under the prompt-cache policy.
Parking MUST NOT be represented by a synthetic canonical assistant message.

#### Scenario: Parent parks while child continues

- **GIVEN** a parent turn completes and a child remains running
- **WHEN** Smith records the parked lifecycle state
- **THEN** provider invocation count is unchanged
- **AND** the child coordinator remains able to deliver terminal evidence

#### Scenario: Resumed session renders parked evidence

- **GIVEN** a persistent session records that its prior process parked while a
  child was active
- **WHEN** a later process reconciles and renders the session
- **THEN** it performs no provider request merely to show that evidence
- **AND** existing child durability policy determines interrupted or terminal
  state

#### Scenario: Cache maintenance is separately authorized

- **GIVEN** a parked parent is eligible for one adaptive maintenance request
- **WHEN** Smith dispatches it under the cache policy
- **THEN** the attempt carries a synthetic cache purpose rather than a parked
  continuation purpose
- **AND** parking alone is not reported as the provider-work cause

### Requirement: Immutable resolved harness composition

Smith SHALL resolve declarative harness input into one immutable,
provenance-bearing `ResolvedHarness` before runtime construction. The record
MUST include identity, provider/model, modules, trust, contributions, requested
and granted capabilities, approval, persistence, context, and delegation
policy; the public composition root MUST accept the resolved record rather than
re-resolving raw files, environment values, or mutable host options.

#### Scenario: Two hosts resolve identical input

- **GIVEN** TUI and headless hosts receive equivalent declarations and host
  services
- **WHEN** each resolves its harness
- **THEN** their `ResolvedHarness` policy records compare equivalent
- **AND** both pass through the same public factory

#### Scenario: Module contribution exceeds its grant

- **GIVEN** a module declares a tool that requires network authority
- **AND** the host grants the module no network capability
- **WHEN** Smith resolves the harness
- **THEN** resolution refuses or disables that contribution before runtime
  construction
- **AND** the declaration itself grants no authority

### Requirement: Every runtime event projects to a known client event

Smith SHALL mirror every event variant of the pinned Agent Runtime in its
client event projection, and MUST fail to build, not degrade to an unknown
event at runtime, when the pinned runtime adds a variant Smith does not
mirror. A context or output budget failure SHALL reach the user.

#### Scenario: A budget cannot be satisfied

- **GIVEN** a turn whose context or output budget cannot be met
- **WHEN** the runtime reports the budget failure
- **THEN** the TUI shows which budget failed with the requested and allowed
  token counts
- **AND** headless stream output carries it as a typed event

#### Scenario: The runtime adds an event

- **GIVEN** a runtime revision with an event variant Smith does not mirror
- **WHEN** Smith is built against it
- **THEN** the build fails in the projection test that names the variant

### Requirement: One composition path for command and native providers

Smith SHALL construct `command-jsonl` through the same
`smith-runtime::factory` boundary and shared `Provider` contract used by native
HTTP providers. Provider transport differences MUST NOT create a second
context, tool/MCP, approval, retry, event, persistence, child, TUI, or headless
composition path.

#### Scenario: Compare command-provider surfaces

- **GIVEN** identical resolved policy and a deterministic command-provider
  fixture
- **WHEN** the TUI and `smith -p` execute the same turn
- **THEN** their canonical messages, attempts, tool results, usage, and
  terminal lifecycle are equivalent
- **AND** only declared presentation metadata differs

#### Scenario: Switch between native and command providers

- **GIVEN** an idle persisted session used a native provider on its previous
  turn
- **WHEN** the user selects a compatible command provider at the existing safe
  turn boundary
- **THEN** Smith rebuilds the same session through the one factory with the new
  `Arc<dyn Provider>`
- **AND** canonical history and Smith policy remain local
- **AND** prior provider cache state does not transfer

### Requirement: Compatible command-provider dependency gate

Smith SHALL enable command-provider composition only from an exact immutable
Agent Runtime revision containing the approved bounded process framework.
Smith's factory integration and Agent Runtime's Smith consumer-conformance
suite MUST both pass before the adapter becomes available.

#### Scenario: Pinned runtime lacks the feature

- **GIVEN** Smith's exact Agent Runtime revision does not expose the approved
  `command-provider` facade feature and contracts
- **WHEN** a build or configuration requests `command-jsonl`
- **THEN** the adapter remains unavailable rather than falling back to a Smith-
  local process implementation
- **AND** existing native providers remain buildable and unchanged

#### Scenario: Compatible runtime revision is adopted

- **GIVEN** the candidate runtime includes the approved command framework
- **WHEN** Smith enables the feature and runs both compatibility gates
- **THEN** direct argv, bounds, terminal validation, cancellation, and process-
  tree semantics match the reviewed upstream contract
- **AND** the exact revision is recorded in Smith's manifest and lockfile

### Requirement: Host services have explicit runtime ownership

Smith's composition path SHALL receive host services explicitly and retain them
for the lifetime they serve. Mutable process-global installation MUST NOT be
used for background tasks or other host adapters, and direct embedders MUST be
able to construct multiple isolated Smith hosts in one process.

#### Scenario: Factory composes background-capable tools

- **GIVEN** the resolved host supplies a background task service
- **WHEN** the one Smith factory assembles built-in tools
- **THEN** every background-capable tool receives that exact service
- **AND** host exit policy and shutdown use the same owned service instance

#### Scenario: Concurrent hosts use different fakes

- **GIVEN** two deterministic tests compose hosts with different fake services
- **WHEN** their turns run concurrently
- **THEN** each fake observes only its own calls
- **AND** construction order does not select a process-wide winner

### Requirement: External turns dispatch host tools only through Agent Runtime

Agent Runtime SHALL give an external agent backend a turn-scoped handle that
lists the tool specifications the host offers for the turn and executes one
call through the runtime's own tool path. Smith MUST use that handle for
every bridged call and MUST NOT invoke a tool implementation directly.

A bridged call SHALL resolve against the session's authorized tool view and
SHALL NOT require the tool to be in an activation epoch or count against the
activation budget. The call and its result SHALL commit to canonical history
in the order they occurred relative to the text the backend reported, and
the handle MUST refuse a call once its turn has reached a terminal event.

#### Scenario: A bridged call during an external turn

- **GIVEN** an external turn in flight
- **WHEN** the backend submits a call through the handle
- **THEN** the runtime emits the ordinary tool request and result events
  attributed to that turn
- **AND** no provider request is made

#### Scenario: A tool that was never activated

- **GIVEN** an authorized tool outside the session's activation epoch
- **WHEN** the backend calls it through the handle
- **THEN** the call is dispatched

#### Scenario: History after a bridged call

- **GIVEN** the backend reported text, then made a bridged call, then
  reported more text
- **WHEN** the turn completes
- **THEN** canonical history holds the first text with the call, the tool
  result, and the remaining text, in that order

#### Scenario: A call after the turn ended

- **GIVEN** an external turn that has published its terminal event
- **WHEN** a call is submitted through its handle
- **THEN** the handle refuses it and nothing is dispatched

#### Scenario: Process ends during a bridged call

- **GIVEN** a bridged call in flight when the process exits
- **WHEN** the session is resumed
- **THEN** the turn is finalized as interrupted
- **AND** the call is not executed again

### Requirement: Internal turns on a harness session run on the harness

Agent Runtime SHALL execute an admitted internal turn on the external agent
backend when the session's turns run on one, continuing the backend's own
conversation, and MUST NOT call the session's provider.
Admission, attribution, outcome consumption, and replay safety SHALL be the
ones the internal turn already has.

The internal input SHALL be sent to the backend as bounded text marked as a
delivery from Smith. It MUST NOT be appended to Smith's canonical history as
a user message.

#### Scenario: A child completes while a harness parent is idle

- **GIVEN** a root session on a harness, parked awaiting a child
- **WHEN** the child completes and the completion turn is admitted
- **THEN** the turn runs on the CLI, resuming its session
- **AND** the provider is not called
- **AND** the turn is attributed as `delegation.child-completion`

#### Scenario: Continuation history on a harness session

- **GIVEN** a committed child-completion turn on a harness session
- **WHEN** canonical history is inspected
- **THEN** it contains no fabricated user message for the delivery

#### Scenario: Replay after a committed harness continuation

- **GIVEN** a child-completion turn on a harness committed before a crash
- **WHEN** Smith resumes and replays the journal
- **THEN** the CLI is not invoked again for that outcome

### Requirement: Agent-driven capability discovery

The agent SHALL be able to enumerate the capabilities its session
authorizes, search them by name, summary, and description text, and activate
chosen ones by registry id for the next provider request, within the
capability budget. A search or listing SHALL return only authorized entries.
A search with no match SHALL say so and report how many capabilities exist
per domain. Every session SHALL be told, in one developer instruction, which
registered tools outside its pinned core it may activate. Two registered tools
SHALL NOT be treated as redundant only because they share permissions.

#### Scenario: Browse without a query
- **WHEN** the agent calls `registry.search` with an empty query
- **THEN** the result lists authorized capabilities by id and one-line summary
- **AND** nothing is activated by the listing

#### Scenario: Activate by name
- **GIVEN** the agent has seen `skill:release-notes` in a listing
- **WHEN** it calls `registry.activate` with that id
- **THEN** the capability is active on the next provider request

#### Scenario: No match is not no capability
- **GIVEN** a query that matches nothing
- **WHEN** the search returns
- **THEN** the result states that nothing matched
- **AND** reports the number of available capabilities per domain

#### Scenario: Description text matches
- **GIVEN** a capability whose description mentions "port scan" and whose
  keyword list does not
- **WHEN** the agent searches for "port scan"
- **THEN** the capability is among the candidates

#### Scenario: Unknown or unauthorized id
- **WHEN** the agent calls `registry.activate` with an id outside its
  authorized view
- **THEN** the call returns an error and nothing is activated

#### Scenario: Activatable tools are named up front
- **GIVEN** a session that registers `agent` and `web_search` but does not
  pin them, and whose limits deny `tool:web_search`
- **WHEN** the first provider request is planned
- **THEN** its instructions name `agent` as activatable
- **AND** they do not name `web_search` or any pinned core tool

#### Scenario: Web tools with equal permissions both stage
- **GIVEN** `web_fetch` and `web_search` registered with the same permissions
- **WHEN** the agent searches for "web search fetch"
- **THEN** both tools can bind, and neither is rejected as redundant

### Requirement: Independently selected compaction route and instruction

Smith SHALL bind configured compaction targets through its normal credential,
model-limit, and provider preflight. Custom instructions SHALL participate in
summary adapter identity. Manual overrides SHALL remain attempt-local.
Changing a summary route or instruction SHALL preserve canonical originals
and provenance and SHALL NOT authorize fabricated state or discard prior
exact persistence when a new summary cannot be produced.

#### Scenario: A dedicated model produces a summary

- **WHEN** the configured summary model differs from the conversation model
- **THEN** only the summary request uses that route and its usage attribution
- **AND** subsequent conversation turns keep the conversation route

#### Scenario: A summary default changes during a session

- **WHEN** an idle session adopts a new summary model or instruction
- **THEN** exact canonical history remains recoverable and incompatible optional summary state cannot prevent startup

#### Scenario: An overridden attempt fails

- **WHEN** an attempt with custom instructions fails
- **THEN** originals and the previous committed summary remain available
- **AND** the override does not change later automatic summaries

### Requirement: Wait timeout uses host internal-turn admission

Smith SHALL admit `delegation.wait-timeout` continuations through the session's
existing idle-only internal-turn admission with an `InternalTurnSource` of
kind `delegation.wait-timeout`, and SHALL NOT require a new Agent Runtime
primitive for the timer. Before admission Smith SHALL re-check the child's
coordinator status and SHALL drop the wake when the child is terminal. An
accepted turn SHALL use the parent runtime composition applicable at
admission, including a harness backend when the session runs on one, and
SHALL observe the same provider, context, tool, approval, workspace,
checkpoint, cancellation, retry, and global-limit contracts as an ordinary
turn.

#### Scenario: Timeout wake on an idle parent

- **GIVEN** a parked parent whose armed timer expired for a running child
- **WHEN** Smith requests idle-only internal admission
- **THEN** runtime accepts one turn attributed `delegation.wait-timeout`
- **AND** no user-role message is appended

#### Scenario: Timeout wake on a harness session

- **GIVEN** a root session running on a harness with an expired wait timer
- **WHEN** the wake is admitted
- **THEN** the turn runs on the harness as other internal turns do
