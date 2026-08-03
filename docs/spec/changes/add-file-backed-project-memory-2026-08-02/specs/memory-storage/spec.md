## ADDED Requirements

### Requirement: Default project-scoped private memory store
Smith's standard terminal and headless hosts SHALL provide a file-backed
project-memory store by default. The store SHALL be rooted at
`~/.smith/memory/<project-id>/`, where `<project-id>` is the existing
path-safe identity derived from the canonical project root, and SHALL NOT use
the checkout as a fallback location. On supported platforms, Smith SHALL
create the project directory with user-only access and memory files with
user-only read/write access.

#### Scenario: First use creates only private user state
- **WHEN** a standard host starts with the built-in memory policy for a project that has no memory directory
- **THEN** Smith creates or opens that project's private memory store and a valid generated index without modifying the checkout

#### Scenario: Canonical project identity isolates stores
- **WHEN** two sessions resolve to the same canonical `ProjectId`
- **THEN** they resolve to the same memory directory, while a different `ProjectId` resolves to a different directory

#### Scenario: Unsafe root fails closed
- **WHEN** an enabled memory root or any required path component is symlinked, escapes its selected project directory, or cannot satisfy the private-store policy
- **THEN** Smith refuses to open the store with a content-free diagnostic and does not follow the path or fall back into the workspace

#### Scenario: Direct embedder remains explicit
- **WHEN** an application embeds Smith through the runtime factory without selecting the standard host memory policy
- **THEN** Smith does not create or read an ambient file-backed store and continues to use only the embedder-supplied `MemorySource`, if any

### Requirement: Versioned Markdown topics and generated index
The store SHALL represent each active memory as one top-level Markdown topic
file named from a validated ASCII memory id. A schema-version-1 topic SHALL
contain bounded frontmatter for `id`, `type`, `description`, `keywords`,
`sensitivity`, `created_at`, and `updated_at`, followed by the bounded body.
The initial type taxonomy SHALL be `user`, `feedback`, `project`, and
`reference`, and every valid version-1 topic SHALL have `sensitive`
sensitivity. `MEMORY.md` SHALL be a bounded, deterministic index generated
from valid topic metadata; topic files, rather than the index, SHALL be the
record source of truth.

#### Scenario: Remember creates a typed topic and refreshes the index
- **WHEN** Smith commits a valid new memory record
- **THEN** it publishes one schema-version-1 topic file and regenerates `MEMORY.md` with a bounded link and description for that topic

#### Scenario: Valid manual topic edit is recognized
- **WHEN** a user manually edits a topic into another valid schema-version-1 value outside an active turn
- **THEN** Smith validates the edited topic at the next store snapshot and regenerates the index from the edited metadata

#### Scenario: Manual index edit is repairable
- **WHEN** `MEMORY.md` is missing, damaged, or manually changed while its topic files remain valid
- **THEN** deterministic maintenance replaces it with the canonical bounded index without changing the topic bodies

#### Scenario: Invalid topic is preserved but excluded
- **WHEN** Smith encounters a malformed, oversized, non-UTF-8, non-regular, symlinked, unsupported-version, or sensitivity-downgraded topic file
- **THEN** it leaves that file untouched, excludes it from recall and the generated index, and reports a content-free validation outcome

#### Scenario: Large catalog produces a bounded orientation index
- **WHEN** all valid topic entries cannot fit in the generic size of one memory record
- **THEN** `MEMORY.md` contains the highest structurally ranked bounded entries plus an omission count, while explicit memory search can still inspect the complete valid catalog

### Requirement: Atomic and conflict-safe mutation
All store mutations SHALL acquire a project-scoped cross-process writer
lease, validate containment and the expected record state, publish topic
updates through same-directory temporary files and atomic replacement, and
regenerate the index atomically. Lock timeout or compare failure SHALL return
a conflict instead of overwriting another process. Forget SHALL durably
remove only the exact validated memory id selected by the caller.

#### Scenario: Concurrent writers do not lose an update
- **WHEN** two Smith processes attempt incompatible updates to the same project store
- **THEN** only a writer holding the lease with a valid expected state commits, and the other receives a conflict without overwriting the committed topic

#### Scenario: Crash between topic and index publication is repaired
- **WHEN** a process stops after atomically publishing a topic mutation but before publishing the corresponding index
- **THEN** the next successful store maintenance reconstructs `MEMORY.md` from the committed topic files

#### Scenario: Forget removes only an exact id
- **WHEN** a confirmed forget operation names one valid memory id
- **THEN** Smith durably removes only that topic, regenerates the index, and does not interpret the id as a path, glob, prefix, or search expression

#### Scenario: Stale temporary cleanup is narrowly scoped
- **WHEN** open-time maintenance finds interrupted-write artifacts and unrelated unknown files
- **THEN** Smith removes only stale files matching its exact temporary-file contract and leaves all unknown files untouched

### Requirement: Bounded storage rejects unsafe content without eviction
The store SHALL enforce the existing generic record bounds, Smith's configured
store bounds, and a default maximum of 200 valid topic files. A create that
would exceed quota SHALL fail without evicting an existing record. Smith SHALL
reject bodies or metadata detected as credentials or registered secrets
rather than persist them, silently redact them, or classify them below
`sensitive`.

#### Scenario: Store quota is reached
- **WHEN** a new remember operation would exceed the valid-topic quota
- **THEN** the operation fails with a bounded safe result and no existing topic or index entry is evicted

#### Scenario: Record exceeds a generic bound
- **WHEN** a proposed id, description, keyword set, body, or aggregate store contribution exceeds its configured bound
- **THEN** Smith rejects the mutation before publication and leaves the prior store revision unchanged

#### Scenario: Proposed memory contains a secret
- **WHEN** a remember or automatic-capture proposal contains a detected credential or registered secret in its body or metadata
- **THEN** Smith rejects the entire proposal without writing a topic and without echoing the sensitive value in results, errors, or events
