# runtime-integration Specification

## Purpose
TBD - created by archiving change add-smith-agent-harness. Update Purpose after archive.
## Requirements
### Requirement: Shared runtime is the canonical mechanism

Smith SHALL use the versioned `agent-runtime` facade as the sole owner of model
and context planning, provider normalization, the provider/tool loop, tool
execution, cancellation, runtime events, usage accounting, and registry
mechanism. Smith MUST NOT maintain a parallel implementation or public contract
for those behaviors.

#### Scenario: Smith executes a configured turn

- **GIVEN** Smith has resolved product configuration and host policy
- **WHEN** it starts a session and sends user input
- **THEN** the turn executes through `agent-runtime::Runtime`
- **AND** the TUI and persistence layers consume shared messages and events

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

#### Scenario: Read-only repository question
- **GIVEN** read and mutation abilities are installed
- **WHEN** deterministic retrieval classifies the request as read-only
- **THEN** the active epoch contains the dependency-complete read subset
- **AND** edit and shell are not advertised merely because they are installed

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

Smith SHALL dispatch a pending real-user follow-up before allowing an idle-only
goal continuation attempt at the same terminal boundary. An accepted steer to
a goal-owned serving turn MUST remain real user input and MUST NOT change goal
identity, authority, or accounting policy implicitly.

#### Scenario: Goal and queued user input reach an idle boundary

- **GIVEN** an active goal is eligible for automatic continuation
- **AND** Smith has one queued real-user turn
- **WHEN** the serving turn reaches its terminal boundary
- **THEN** Smith submits the real-user turn first
- **AND** the goal controller observes busy or waits for the later boundary

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
