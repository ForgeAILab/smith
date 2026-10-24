## MODIFIED Requirements

### Requirement: A child's state reads the same everywhere

Every surface that names a child agent's state or durability SHALL use one
lowercase word set: `running`, `idle`, `interrupted`, `stopped`, `failed`,
`expired`, and `durable` or `ephemeral`. Qualifiers SHALL be words, not debug
structures. Machine-readable headless output MUST NOT change.

A lead SHALL continue to read running throughout its logical task episode while
parked for required descendants, even with no active provider request. Activity
text MAY say it is waiting for descendants, but MUST NOT invent provider thinking
or report its interim parking reply as final task completion.

#### Scenario: A running child

- **GIVEN** a child agent has an active turn
- **WHEN** it appears in the transcript, the delegated-work panel, `/agents`,
  or the child inspector
- **THEN** its state reads `running` on each of them

#### Scenario: An interrupted child

- **GIVEN** a child was interrupted with an exact checkpoint
- **WHEN** `/agents` lists it
- **THEN** its state reads `interrupted (resumable)`

#### Scenario: Lead waits for its worker

- **GIVEN** child-1 has parked with child-1.1 still required
- **WHEN** any surface projects the lead
- **THEN** the lead remains running with evidence-based waiting activity
- **AND** it is not shown as idle or completed until the episode settles

### Requirement: Child sessions are reached through their parent

Smith SHALL NOT list a child agent's session as a resumable session: the
resume pickers and both forms of `smith sessions list` MUST omit sessions
that belong to a parent session. A child stays reachable through its parent
(`/agent`, follow-up). Explicit `--resume <ID>` for a known child MUST retain
validated lineage and resolve through its owning parent path or report that
parent context is required; it MUST NOT promote that child to Root authority.

The same rule SHALL include Layer-2 descendants: catalog/lineage ownership, not
filename spelling, controls exclusion from root resume lists. Root inspection
MUST reach retained grandchildren through their owning coordinator without
provider execution; a missing/incompatible ownership record MUST NOT silently
promote a child into a new authorized root session.

#### Scenario: A session that spawned a child

- **GIVEN** a session that spawned one durable child agent
- **WHEN** the user runs `smith sessions list` or opens `/resume`
- **THEN** the parent session is listed once
- **AND** no `child-session-…` entry is listed

#### Scenario: Retained grandchild appears in storage

- **GIVEN** a known Layer-2 snapshot is retained beside root snapshot files
- **WHEN** the resume picker and parent child inspector enumerate sessions
- **THEN** the grandchild is reachable under its parent tree and not as an independent root
- **AND** inspection does not execute or resume that grandchild

### Requirement: Child inspection renders the retained conversation

The selected child view SHALL use the main conversation's rendering and
visibility rules for the delegated task, assistant messages, permitted
provider-exposed reasoning, tool calls/results, and final reply. Retained
history SHALL remain readable through scrolling or paging; a short activity
label or bounded lifecycle log SHALL NOT substitute for available conversation
content. Existing composer focus and root navigation behavior SHALL remain.

Authorized Layer-2 conversation inspection SHALL preserve the same rendering,
reasoning visibility, paging, source attribution, and retained/live stream joining
as Layer-1 inspection. Selecting a grandchild MUST NOT replace Root's canonical
conversation or lose its composer draft. Unavailable history segments SHALL be
reported rather than fabricated from progress labels.

#### Scenario: Inspect a child after it has already performed work

- **GIVEN** a child committed assistant text, a tool call, and its result before
  the TUI subscribed to the child's stream
- **WHEN** the user selects that child
- **THEN** its delegated task and committed conversation are hydrated from
  canonical history and shown in order
- **AND** subsequent live events join that view without gaps or duplicate rows

#### Scenario: Read older work beyond the client memory window

- **GIVEN** a child's retained conversation exceeds the in-memory row limit
- **WHEN** the user scrolls or pages back
- **THEN** earlier retained content remains accessible from canonical storage
- **AND** large tool results have the same access behavior as in the main view

#### Scenario: Inspect a recovered child

- **GIVEN** the parent restarted and a retained child has no live handle
- **WHEN** the user opens that child's view
- **THEN** its retained transcript is displayed without executing the child
- **AND** unavailable historical segments are labeled explicitly

#### Scenario: Return to the main conversation

- **GIVEN** the user has read a child's conversation
- **WHEN** the user dismisses child inspection
- **THEN** the root transcript, scroll position, and composer draft are restored

#### Scenario: Inspect a grandchild while Root is composing

- **GIVEN** Root has an unsent draft and a grandchild has retained tool history
- **WHEN** the user selects child-1.1
- **THEN** the inspector renders that grandchild history under existing visibility rules
- **AND** dismissing it restores Root transcript position and the exact draft

## ADDED Requirements

### Requirement: Delegated-work surfaces project a bounded hierarchy

The delegated-work panel, agent reports, navigation, and /agent inspection SHALL
project authorized child IDs in numeric parent-before-descendant order and indent
Layer-2 rows under their Layer-1 lead. Each row MUST use its own profile, activity,
workspace, clocks, and lifecycle evidence. A Layer-2 ChildSpawned event MUST NOT
consume Root's pending spawn correlation entry. Existing human lifecycle words
and versioned machine-readable contracts SHALL remain consistent.

#### Scenario: Grandchild event interleaves a root spawn

- **GIVEN** Root has a pending child-2 spawn and child-1 emits ChildSpawned for child-1.1
- **WHEN** the TUI reduces the grandchild event
- **THEN** Root's pending spawn entry remains available for child-2
- **AND** child-1.1 is attributed to its actual parent/profile and indented under child-1

#### Scenario: Double-digit IDs are listed

- **GIVEN** retained child-2, child-2.1, and child-10 identities
- **WHEN** a tree surface lists them
- **THEN** numeric segments preserve parent-before-descendant ordering
- **AND** lexical ordering is not used to misgroup the tree

### Requirement: Direct child questionnaires share one attributed root surface

Authorized DirectHost Layer-1 questions SHALL use a bounded serialized root
presentation queue keyed by exact originating session/turn/call/request identity.
Concurrent questions MUST NOT overwrite one another or be answered by selected
row identity. The modal SHALL identify its source child, preserve root composer
state, and close/cancel the exact request on shutdown. A task answer MUST NOT be
projected or treated as tool approval.

#### Scenario: Two leads request clarification

- **GIVEN** two authorized Layer-1 agents submit distinct questionnaires
- **WHEN** their requests reach the interactive surface together
- **THEN** they are presented in a bounded serialized order with source identity
- **AND** each answer resumes only its originating request

#### Scenario: Answer arrives after cancellation

- **GIVEN** a child question was cancelled during shutdown
- **WHEN** a delayed modal response arrives
- **THEN** it cannot resume that child or authorize another operation
- **AND** other queued questions and root draft remain correctly attributed

### Requirement: Advisory triage surfaces preserve user intent and fallback

Interactive and headless surfaces SHALL consume the same host triage admission
result and MAY show bounded routing, clarification, or fallback diagnostics. They
MUST NOT fabricate root assistant prose, duplicate user submissions, discard
attachments, leak bearer credentials, or block a turn solely because the sidecar
is absent. Detailed child implementation transcripts SHALL remain inspectable
without being copied wholesale into the Root conversation.

#### Scenario: Triage is unavailable in either surface

- **GIVEN** the same enabled endpoint is unavailable for interactive and headless requests
- **WHEN** a user submits a task
- **THEN** both surfaces admit ordinary root handling exactly once
- **AND** any fallback diagnostic is bounded and secret-safe

### Requirement: Headless completion follows the full delegated episode

Headless completion SHALL remain pending while required Layer-1/Layer-2 episodes,
terminal persistence, continuation admission, or final Root delivery are pending.
It MUST NOT exit when a lead only parks, and MUST NOT remain alive solely for a
retained idle record, stale wait timer, forwarded lifecycle event, or cache policy.
Typed input-required, cancellation, and shutdown outcomes SHALL retain their
existing bounded terminal handling.

#### Scenario: Lead final synthesis follows worker completion

- **GIVEN** a lead has parked after spawning a required worker
- **WHEN** the worker completes, the lead integrates, and Root receives final delivery
- **THEN** headless stays alive through those boundaries and then completes normally
- **AND** no premature parking answer is reported as final output
