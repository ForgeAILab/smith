## ADDED Requirements

### Requirement: User-owned memory policy and safe defaults
Smith SHALL resolve project memory with built-in defaults `enabled = true` and
`auto_capture = false`. Only built-in, user-owned, or explicit session/CLI
policy MAY select the user-state root, enable automatic capture, or change
memory persistence authority. Repository-controlled configuration SHALL NOT
relocate the store, enable capture, lower sensitivity, widen memory tool
authority, or delete stored records. Status SHALL expose the resolved enabled
and auto-capture states without exposing memory content.

#### Scenario: Default policy starts with visible capture only
- **WHEN** no user, session, or CLI memory override is present
- **THEN** project storage, recall, and deterministic maintenance are enabled while automatic post-turn model capture is disabled

#### Scenario: Repository attempts to enable capture
- **WHEN** repository-controlled configuration requests automatic capture, a different user-state root, or lower sensitivity
- **THEN** resolved-config preflight rejects or ignores that unauthorized value according to configuration policy and reports its provenance safely

#### Scenario: User disables project memory
- **WHEN** an authorized user-owned or explicit session policy sets `enabled = false`
- **THEN** Smith suppresses ambient recall, remember, and automatic capture without deleting the project's existing memory files

#### Scenario: Session persistence is disabled independently
- **WHEN** session snapshots are disabled but project memory remains enabled
- **THEN** Smith keeps project memory in the user-state store and does not redirect it into the checkout or session transcript

### Requirement: Dedicated bounded memory abilities
The persistent root SHALL expose `memory.remember`, `memory.forget`, and
`memory.search` through Smith's ordinary ability registry, activation,
approval, execution, and event pipeline. `memory.remember` SHALL perform one
validated exact-id create or update. `memory.forget` SHALL be an exact-id
destructive operation subject to normal approval policy. `memory.search`
SHALL deterministically search the complete valid current-project catalog and
return only a bounded, sensitivity-marked result. All three SHALL use the same
store and policy service.

#### Scenario: Remember creates or updates one record visibly
- **WHEN** the root invokes `memory.remember` with a valid id, type, description, keywords, and body while memory is enabled
- **THEN** the ordinary tool lifecycle visibly reports a bounded created, updated, or unchanged result for exactly that id

#### Scenario: Forget requires an exact destructive request
- **WHEN** the root invokes `memory.forget` with one valid id
- **THEN** Smith routes the prepared exact-id deletion through normal destructive-action approval and rejects paths, globs, prefixes, and free-form queries

#### Scenario: Search covers topics omitted from recall
- **WHEN** the user explicitly searches for a term matching a valid topic that was outside automatic recall bounds
- **THEN** `memory.search` may return that topic's bounded id, type, description, revision, sensitivity, and snippet as an explicit sensitive tool result

#### Scenario: Disabled store remains manageable
- **WHEN** memory is disabled and the user invokes a memory ability
- **THEN** search and confirmed exact-id forget remain available for inspection and removal, while remember refuses safely and explains that persistence is disabled

### Requirement: Ordinary workspace tools gain no user-state exception
General read, edit, shell, glob, and workspace-search abilities SHALL NOT gain
an allowlist exception for `~/.smith` as part of this change. Access to the
memory store SHALL remain encapsulated by the dedicated memory abilities and
host-owned source.

#### Scenario: Workspace tool targets memory root
- **WHEN** an ordinary workspace-scoped ability attempts to access a path under `~/.smith/memory`
- **THEN** its existing path and authority policy applies unchanged and project-memory enablement grants no new access

#### Scenario: Memory body names a filesystem path
- **WHEN** recalled memory contains a path inside or outside the workspace
- **THEN** the path text grants no filesystem authority and any later access still requires an independently authorized ability

### Requirement: Memory is durable context, not instructions or task state
Smith SHALL describe the initial memory taxonomy and guide explicit capture
toward durable, non-derivable collaboration context. Project memory SHALL NOT
replace `AGENTS.md`, goals, plans, task queues, session history, checkpoints,
semantic summaries, current code structure, file listings, or git history.
Neither a model-authored topic nor a manually edited topic SHALL be able to
change instruction precedence, tool availability, approval policy, or runtime
state merely by being recalled.

#### Scenario: User explicitly asks Smith to remember a preference
- **WHEN** the user requests durable retention of a bounded non-secret collaboration preference
- **THEN** the root may visibly propose `memory.remember` using the appropriate typed topic rather than modifying project instructions or canonical history

#### Scenario: Candidate fact is derivable from the checkout
- **WHEN** Smith considers saving current code structure, a file listing, git history, or transient task progress
- **THEN** memory guidance directs it to use the authoritative current source or existing task/session mechanism instead of project memory

#### Scenario: Memory conflicts with current authoritative state
- **WHEN** a recalled claim conflicts with current files, resources, product policy, or project instructions
- **THEN** Smith treats the current authoritative source as controlling and does not let the memory override it

### Requirement: Content-free events and sensitivity-safe results
Canonical memory events and ordinary diagnostics SHALL expose only bounded
operation metadata such as outcome, counts, sizes, type, sensitivity,
content-derived revision, duration, and a redaction-safe id fingerprint. They
SHALL NOT include memory bodies, descriptions, keywords, raw memory paths,
capture prompts, model output, or detected secrets. Explicit search results
SHALL remain sensitivity-marked and follow existing tool-result persistence
policy.

#### Scenario: Remember succeeds
- **WHEN** Smith emits events and diagnostics for a committed remember operation
- **THEN** observers can determine the safe outcome and revision without receiving the record body, description, keywords, raw path, or plain id

#### Scenario: Validation or secret rejection fails
- **WHEN** a mutation is refused because content is invalid or secret-bearing
- **THEN** the tool result and canonical events identify a bounded refusal category without echoing the rejected content

#### Scenario: TUI and headless clients display status
- **WHEN** either client projects memory configuration or the latest maintenance or capture outcome
- **THEN** both derive the same content-free state from canonical runtime status and events rather than rereading memory files independently
