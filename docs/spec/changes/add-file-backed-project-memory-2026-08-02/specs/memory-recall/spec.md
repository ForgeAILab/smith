## ADDED Requirements

### Requirement: File-backed memory source in standard hosts
Smith's standard terminal and headless hosts SHALL install a file-backed
`SmithMemorySource` whenever project memory is enabled, even when the selected project
store is empty. The source SHALL snapshot validated files at each new turn
boundary and feed the existing bounded, revisioned, sensitivity-aware
`MemoryContributor`; it SHALL NOT introduce a second context lane or require a
new generic Agent Runtime storage API.

#### Scenario: Empty default store fabricates no context
- **WHEN** a standard host opens a valid project store that contains no topics
- **THEN** the memory source contributes no fabricated memory body to the turn context

#### Scenario: Memory crosses session boundaries within one project
- **WHEN** one session commits a valid memory and a later unrelated session starts with the same canonical `ProjectId`
- **THEN** the later session can recall that record through the standard memory context lane

#### Scenario: Another project cannot recall the record
- **WHEN** a session starts with a different canonical `ProjectId`
- **THEN** its memory source does not enumerate or contribute records from the first project's directory

#### Scenario: Disabled memory suppresses ambient recall
- **WHEN** the resolved policy has project memory disabled
- **THEN** Smith contributes no file-backed project memory while leaving the durable files unchanged

### Requirement: Deterministic bounded recall ranking
For automatic recall, Smith SHALL rank valid topics using only local,
deterministic data in this order: normalized keyword-match count and
specificity against the latest user input, normalized description-token
overlap, Smith-owned type and structural priority, descending `updated_at`,
and lexicographic memory id. A topic with no keywords SHALL remain eligible.
The selected index and topics SHALL obey both the generic source bounds and
Smith's default maximum of eight records and 8,192 aggregate characters.

#### Scenario: Keyword match outranks a weaker description match
- **WHEN** one topic has a more specific normalized keyword match and another has only description overlap with the latest user input
- **THEN** Smith ranks the keyword-matching topic first before applying later tie-breakers

#### Scenario: Stable tie reaches the id tie-breaker
- **WHEN** eligible topics tie on keyword score, description overlap, structural priority, and update time
- **THEN** Smith orders them lexicographically by memory id and produces the same selection for the same snapshot and input

#### Scenario: Empty-keyword preference remains eligible
- **WHEN** a valid durable preference has no keywords and no other search term matches it
- **THEN** Smith keeps the preference eligible but still subjects it to structural ordering and all record and aggregate bounds

#### Scenario: Catalog exceeds recall bounds
- **WHEN** more valid records are eligible than the configured record or aggregate-character limits permit
- **THEN** Smith contributes only the highest-ranked bounded prefix and reports bounded omission metadata rather than truncating a topic body ambiguously

#### Scenario: Recall does not spend provider capacity
- **WHEN** Smith ranks and selects project memory for a turn
- **THEN** it performs no model, embedding, vector-database, or remote-service call

### Requirement: Memory remains optional provenance-bearing context
Every recalled index or topic SHALL enter the context plan as an optional
Host-sourced Memory fragment with its own content-derived revision,
sensitivity, token cost, and cache classification. Recall SHALL NOT copy a
memory body into canonical user history, turn memory, project instructions,
tool authority, or approval state. Smith's prompt guidance SHALL require
remembered claims to be checked against current authoritative sources when
they may have changed.

#### Scenario: Context manifest identifies the recalled revision
- **WHEN** a topic is selected for a turn
- **THEN** the turn's context manifest identifies its memory source, content-derived revision, sensitivity, and accounted size without treating it as user-authored history

#### Scenario: Remembered instruction requests authority
- **WHEN** a recalled topic says to invoke a tool, bypass approval, or override project or product instructions
- **THEN** Smith treats the text only as untrusted optional memory and grants no additional tool or approval authority

#### Scenario: Resumed session sees current memory only on a new turn
- **WHEN** a saved session is resumed after a valid topic changed
- **THEN** Smith uses the latest validated topic for the next turn while preserving the original memory revisions recorded in prior turn manifests

#### Scenario: External edit does not mutate an in-flight request
- **WHEN** a topic file changes after Smith has snapshotted memory for an active provider request
- **THEN** the active request retains its original snapshot and the valid edit can become visible only at the next turn boundary

### Requirement: Child sessions receive recall without durable mutation authority
An in-process child session SHALL receive no durable memory mutation authority.
It MAY inherit the parent's already bounded recall contributor, but SHALL NOT
receive `memory.remember` or `memory.forget` and
SHALL NOT run automatic capture in this release.

#### Scenario: Child reasoning uses bounded project context
- **WHEN** a root launches a child after selecting a bounded memory snapshot
- **THEN** the child may receive that bounded recall context with the same provenance and sensitivity constraints

#### Scenario: Child attempts durable memory mutation
- **WHEN** a child requests a remember or forget ability or completes work that would otherwise trigger automatic capture
- **THEN** no durable memory mutation ability or capture trigger is available to the child
