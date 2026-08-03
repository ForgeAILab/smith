## ADDED Requirements

### Requirement: Explicit visible capture is the default
With the built-in memory policy, Smith SHALL NOT run a hidden post-turn
provider call. The persistent root MAY use `memory.remember` through the
ordinary visible tool lifecycle when the user explicitly asks for durable
retention or when a bounded non-secret fact clearly satisfies the project
memory policy.

#### Scenario: Ordinary turn with default settings completes
- **WHEN** a root turn completes while `auto_capture = false` and no memory ability was invoked
- **THEN** Smith performs no capture provider request and writes no inferred memory

#### Scenario: Explicit remember is visible
- **WHEN** the root decides to save an eligible memory under the default policy
- **THEN** it invokes `memory.remember` as an ordinary tool call whose activation, approval when required, result, and safe events are visible to the client

### Requirement: Opt-in bounded post-turn capture
When an authorized policy explicitly enables `auto_capture`, Smith SHALL use a
host-owned coordinator that considers capture only after a persistent root turn commits
successfully. It SHALL supply a separately attributed `memory.capture`
provider purpose with only the newly committed turn range and a bounded
memory manifest. The provider response SHALL be a structured, upsert-only,
bounded proposal set with no tools and no deletion capability. Every proposal
SHALL pass through the same schema, secret, quota, sensitivity, conflict, and
atomic mutation service as `memory.remember`.

#### Scenario: Eligible root turn produces valid proposals
- **WHEN** an opted-in persistent root turn commits successfully and the bounded capture response proposes eligible upserts
- **THEN** Smith validates and applies at most the configured proposal count through the shared remember path and separately accounts capture usage, output, duration, and outcome

#### Scenario: Capture proposes deletion or a sensitivity downgrade
- **WHEN** a capture response requests deletion, arbitrary file access, an unsupported operation, or sensitivity below `sensitive`
- **THEN** Smith rejects that proposal without mutating the store or granting the capture model any tool authority

#### Scenario: Turn is not eligible for capture
- **WHEN** a turn is cancelled, failed, interrupted, awaiting input, a review turn, a child turn, or otherwise not committed as persistent root work
- **THEN** the automatic capture coordinator does not issue a provider request for that range

#### Scenario: Capture receives bounded context only
- **WHEN** Smith constructs an automatic capture request
- **THEN** it includes only the eligible newly committed turn range and a bounded memory manifest, not unrestricted session history, workspace access, or general tools

### Requirement: Capture coordination is idempotent and failure-isolated
The coordinator SHALL maintain an exact committed-range cursor, coalesce
overlapping pending triggers to the newest eligible range, and skip a range
whose active turn already used `memory.remember` or `memory.forget`. Capture
timeout, provider failure, invalid output, conflict, quota refusal, or secret
rejection SHALL NOT change the status or response of the completed user turn.

#### Scenario: Turn already mutated memory visibly
- **WHEN** an opted-in root turn used remember or forget before it committed
- **THEN** automatic capture marks that committed range skipped and cannot duplicate the save or recreate a deliberately forgotten record

#### Scenario: Overlapping triggers arrive
- **WHEN** another root turn commits while capture for an earlier range is pending
- **THEN** Smith coalesces the pending work using its exact cursor so each committed range is considered at most once

#### Scenario: Capture fails after the user response completes
- **WHEN** the capture provider times out, returns invalid output, or a proposed mutation is refused
- **THEN** the completed turn remains successful and unchanged while Smith records only a safe capture outcome with separately attributed usage

#### Scenario: Client shuts down with capture pending
- **WHEN** an interactive or headless client is closing after flushing the primary response
- **THEN** interactive capture may finish in the background and headless capture receives only a bounded drain interval, after which shutdown proceeds without corrupting memory or the completed turn

### Requirement: Deterministic maintenance is enabled without semantic rewriting
When project memory is enabled, Smith SHALL run deterministic maintenance on
store open and after each mutation. Maintenance SHALL validate schema and
path containment, identify duplicate ids, enforce quotas, remove only stale
Smith-owned temporary files, and regenerate the bounded index. It SHALL NOT
invoke a model, embedding service, vector store, or remote sync, and SHALL NOT
semantically merge, rewrite, expire, or delete a valid topic.

#### Scenario: Store opens with stale index and temporary artifact
- **WHEN** Smith opens a store whose valid topics disagree with `MEMORY.md` and that contains a stale file matching Smith's temporary-file contract
- **THEN** maintenance removes only the recognized stale artifact and atomically regenerates the index from valid topic metadata

#### Scenario: Store opens with invalid and conflicting content
- **WHEN** maintenance finds an invalid topic, duplicate id, stale claim, or semantically conflicting valid memories
- **THEN** it excludes or reports structurally invalid data as specified but leaves user-authored bytes and all structurally valid topics untouched for explicit user resolution

#### Scenario: No maintenance provider spend
- **WHEN** default-on maintenance completes after open or mutation
- **THEN** Smith makes no model or remote-service call and reports only a content-free maintenance outcome

#### Scenario: Dream consolidation and team sync remain deferred
- **WHEN** valid topics appear redundant, contradictory, old, or useful to another project or teammate
- **THEN** this release performs no semantic consolidation, inferred pruning, cross-project sharing, or remote synchronization
