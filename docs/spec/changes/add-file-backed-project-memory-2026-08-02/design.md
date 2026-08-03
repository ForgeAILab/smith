## Context

Agent Runtime already provides MemorySource and MemoryContributor as a
bounded, revisioned, sensitivity-aware context lane. Smith currently wraps
that seam with SmithMemorySource, but it contains a host-supplied in-memory
vector, is deliberately read-only, and is not installed by the standard host.
Session snapshots preserve one conversation and root AGENTS.md supplies
required project instructions; neither is a cross-session memory product.

This change fills the host-owned half of the existing contract. It introduces
durable project memory without changing canonical history, project
instructions, workspace authority, or Agent Runtime's generic API.

## Goals / Non-Goals

### Goals

- Give each canonical Smith project a private, human-readable memory that
  survives unrelated sessions.
- Enable storage, recall, and deterministic maintenance by default.
- Keep default capture visible as ordinary memory tool calls, while providing
  opt-in automatic post-turn capture with separately attributed spend.
- Reuse the existing memory context lane and its record, token, revision,
  sensitivity, and cache bounds.
- Make remember, forget, search, external edits, process crashes, and
  concurrent Smith processes deterministic and auditable.
- Prevent repository content or recalled memory from granting authority,
  changing approval policy, or silently becoming canonical history.

### Non-Goals

- Global user memory shared by every project.
- Team memory, organization sync, or any remote memory service.
- Vector embeddings, a database, or model-assisted recall selection.
- Dream-style semantic merging, pruning, or contradiction resolution.
- A second session summary, transcript, checkpoint, or compaction system.
- Saving code structure, file layout, git history, or other facts that Smith
  can derive from the current checkout.
- A new Agent Runtime storage or memory API unless implementation proves the
  existing MemorySource boundary insufficient and a separate coordinated
  proposal is approved.

## Decisions

### One host-owned directory per existing project identity

The standard terminal and headless hosts derive the existing path-safe
ProjectId from the canonical project root and resolve:

    ~/.smith/memory/<project-id>/
      MEMORY.md
      <memory-id>.md

Direct embedders remain explicit: they may inject their own MemorySource and
do not acquire ambient file persistence merely by calling the runtime factory.

The resolved memory policy has these initial product defaults:

    enabled = true
    auto_capture = false

The memory root and auto-capture authority may come only from built-in,
user-owned, or explicit session/CLI policy. Repository configuration cannot
relocate the user-state root, enable automatic capture, lower record
sensitivity, or widen memory tool authority. Disabling memory suppresses
automatic recall, remember, and capture, while explicit search and forget
remain available so a user can inspect or remove existing state.

Memory is independent of session persistence. Disabling session snapshots
does not silently redirect memory into the checkout, and disabling memory
does not delete existing files.

### Topic files have a small versioned schema

MEMORY.md is a generated index. It is never the source of record bodies and is
not edited by a model or by ordinary workspace tools. Each active record lives
in one top-level <memory-id>.md file whose name is derived from a validated
ASCII slug rather than an arbitrary path.

The initial frontmatter schema is:

    ---
    schema: 1
    id: preferred-test-fixtures
    type: feedback
    description: Prefer deterministic fixtures over timing-based tests.
    keywords:
      - tests
      - fixtures
    sensitivity: sensitive
    created_at: 2026-08-02T15:43:22Z
    updated_at: 2026-08-02T15:43:22Z
    ---

    Use deterministic fixtures when a test can avoid wall-clock timing.

The closed initial type taxonomy is user, feedback, project, and reference.
The description is a bounded recall hook, not a second body. Keywords use
Smith's normalized term rules. An empty keyword list means always eligible,
which is useful for durable collaboration preferences.

Version 1 persists every record as Sensitive. The required sensitivity field
keeps classification explicit and leaves room for later host-approved
classification, but a model, project file, or hand edit cannot lower it in
this release. Suspected credentials or registered secrets are rejected rather
than stored as Secret or silently redacted.

Existing generic bounds remain authoritative: ids are at most 96 characters,
one contributed body is at most 4,096 characters, one source returns at most
16 records and 16,384 aggregate characters, and Smith's default retrieval
policy remains no larger than eight records and 8,192 aggregate characters.
The file store accepts at most 200 valid topic files by default. Reaching the
quota fails a new remember operation; it never silently evicts an older
memory.

### MEMORY.md is a bounded generated orientation index

After every committed mutation and successful open-time repair, Smith
regenerates MEMORY.md from validated topic metadata. Entries are ordered by
structural priority, updated time, and id. The file contains only bounded
one-line links and descriptions.

The context-facing projection of the index fits one generic memory record. If
all topics do not fit, it includes the highest-priority entries plus an
omission count; memory.search still covers the complete valid store. A damaged
or manually edited index is safely replaceable because topic files remain the
source of truth.

### Publication is private, atomic, and cross-process safe

The project directory is created with user-only access and topic/index files
with user-only read/write access on supported platforms. Every path component
is validated, symlinks are rejected at the storage boundary, and resolved
files must remain below the selected project-memory directory.

Mutations acquire a project-scoped cross-process writer lease. A topic update
is written to a same-directory temporary file, durably published, and followed
by atomic index regeneration. Forget durably removes the exact selected topic
and then regenerates the index. Lock timeout or compare failure returns a
conflict instead of overwriting another process.

A crash between topic publication and index publication is repairable because
deterministic maintenance rebuilds the index from topics. Stale temporary
files are removed only when they match Smith's exact temporary-file contract.
Unknown, malformed, oversized, non-UTF-8, non-regular, or symlinked topic files
are left untouched, excluded from recall, and reported without exposing their
content.

### The file-backed source snapshots memory at context boundaries

When memory is enabled, the standard host installs a file-backed
SmithMemorySource even when the project store is empty. At each new turn
boundary it obtains one validated snapshot, computes content-derived record
revisions, and contributes the bounded index plus ranked topic records through
MemoryContributor.

External file edits become visible at the next turn boundary after validation;
Smith never watches a file and mutates an in-flight provider request. A
resumed session uses the latest valid project memory for new turns, while old
turn manifests continue identifying the exact memory revisions they used.

Automatic recall uses only deterministic local data. The initial ranking order
is:

1. number and specificity of normalized keyword matches in the latest user
   input;
2. normalized token overlap with the bounded description;
3. Smith-owned type and structural priority;
4. updated timestamp;
5. lexicographic memory id as the final stable tie-breaker.

Records with no keywords are always eligible but still obey record and
aggregate bounds. Model-assisted selection, embeddings, and vector similarity
are deferred until measurements show deterministic ranking is insufficient.

Every recalled body remains an optional Host-sourced Memory fragment with its
own revision, sensitivity, token cost, and cache class. Recall never copies the
body into canonical user history, never changes tool or approval authority,
and cannot override project or product instructions. Prompt guidance tells
the model to verify remembered claims against current files and resources.

### Dedicated tools share one storage service

The root runtime registers three Smith-owned abilities:

- memory.remember validates and creates or updates one exact record id. Its
  result reports only bounded metadata and whether the operation created,
  updated, or made no change.
- memory.forget permanently removes one exact record id. It cannot accept a
  path or free-form search expression and remains a destructive action routed
  through normal activation and approval policy.
- memory.search deterministically searches the complete current-project
  catalog and returns a bounded, sensitivity-marked set of ids,
  descriptions, revisions, and snippets.

All three call the same store and policy used by automatic capture. General
read, edit, shell, and workspace search tools gain no allowlist exception for
~/.smith. Memory tool results and errors never reveal registered secrets.
Automatic recall remains transient context; an explicit memory.search result
is an ordinary, visibly requested sensitive tool result and follows existing
session persistence policy.

Child sessions inherit the parent's already-bounded recall contributor but do
not receive remember or forget in this release. Automatic capture runs only
for the persistent root. This prevents a child from creating durable user
state or racing the parent while still allowing its reasoning to benefit from
the same project context.

### Default capture is visible; automatic capture is opt-in

With auto_capture disabled, no hidden provider call runs. The active root
agent may still call memory.remember visibly when the user asks Smith to
remember something or when durable, non-derivable collaboration context
clearly satisfies the memory policy.

When the user explicitly enables auto_capture, a host-owned coordinator runs
after a root turn is committed successfully. It does not run for cancelled,
failed, interrupted, child, review, or needs-input turns. It receives only the
new completed-turn range and a bounded memory manifest, uses a separately
attributed memory.capture provider purpose, and returns structured upsert
proposals. It has no workspace tools and cannot propose deletion.

The host validates and applies at most a small bounded number of proposals
through the same mutation service as memory.remember. If the active turn
already used remember or forget, automatic capture skips that range so it
cannot recreate a deliberately forgotten record or duplicate a visible save.
Overlapping triggers coalesce to the newest committed range.

Automatic capture has explicit output, usage, and wall-clock limits. A capture
failure, timeout, quota refusal, invalid proposal, or secret rejection emits a
safe outcome but never changes the completed user turn. Interactive clients
may finish capture in the background; headless shutdown drains pending capture
for a bounded interval after the primary response is flushed.

### Initial maintenance is deterministic, not semantic

Default-on maintenance runs when the store opens and after each mutation. It
validates the schema and containment boundary, removes only stale Smith-owned
temporary files, detects duplicate ids, enforces quotas, and regenerates the
index. It never asks a model to reinterpret content and never merges, rewrites,
expires, or deletes a valid topic based on inferred staleness.

Dream-style semantic consolidation, contradiction resolution, topic merging,
and retention pruning require a later proposal with separate scheduling,
spend, preview, recovery, and deletion policy. Team sync is likewise deferred.

### Observability exposes state, never memory bodies

Canonical events cover store open or repair, remember, forget, search,
automatic capture, and maintenance outcomes. Events and ordinary diagnostics
may include operation, counts, sizes, type, sensitivity, content-derived
revision, duration, and a redaction-safe id fingerprint. They never include
memory bodies, descriptions, keywords, raw paths, capture prompts, or model
output.

Status projections show whether memory and automatic capture are enabled,
valid record count, index revision, and the latest safe maintenance or capture
outcome. TUI and headless projections use the same canonical runtime events
and do not reconstruct memory state independently.

## Risks / Trade-offs

- Default-on memory creates durable user state even though automatic provider
  spend remains opt-in. Clear status, configuration provenance, and forget
  controls make that behavior inspectable and reversible.
- Markdown is auditable and easy to edit, but weaker than a transactional
  database under concurrent mutation. A narrow schema, project lock, atomic
  publication, and generated index keep the first version recoverable.
- Deterministic ranking is cheaper and explainable but may miss semantically
  related memories. The search tool and bounded index provide explicit
  recovery while telemetry can justify a later selector.
- Always-sensitive records reduce cache reuse. This is preferable to allowing
  a model or repository to under-classify personal cross-session context.
- Automatic capture can save stale or low-value claims. It is opt-in,
  upsert-only, bounded, visible in status, and unable to delete; deterministic
  validation and user controls remain authoritative.
- Human edits can make a topic invalid. Smith excludes and reports the file
  rather than silently repairing or destroying user-authored bytes.

## Migration Plan

- No prior standard Smith memory store exists. On first use, an empty private
  project directory and generated index are created without changing existing
  sessions or the checkout.
- Keep the direct-embedder SmithMemorySource constructor compatible while the
  standard host adopts the file-backed implementation.
- Existing saved sessions resume unchanged. Only later turn context plans may
  include current project-memory revisions.
- Persist schema version 1 in every topic. Future schema changes must be able
  to validate old files before any atomic migration and require a separate
  delta when behavior changes.
- Roll out with memory enabled, deterministic maintenance enabled, and
  auto_capture disabled. Enabling auto_capture is a deliberate user or
  session-level configuration change.

## Open Questions

None for Stage 1. Global scope, semantic recall, Dream consolidation, retention
deletion, and team sync are explicitly deferred rather than left implicit.
