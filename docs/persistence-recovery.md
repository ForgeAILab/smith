# Persistence and recovery

Smith has three deliberately separate durability surfaces:

- the canonical session snapshot for completed state;
- the redacted JSONL journal for observable history and replay;
- the protected checkpoint for exact in-flight execution.

They are not interchangeable. In particular, the journal omits raw prepared
arguments, sensitive questionnaire content, and artifact bodies, so it cannot
reconstruct pending execution by itself.

## Layout

The default root is user state, never the project workspace:

```text
~/.smith/sessions/<project-id>/
  <session-id>.snapshot.json
  <session-id>.jsonl
  <session-id>.changes.jsonl
  <session-id>.checkpoint.bin
  <session-id>.checkpoint.lock
  <session-id>.session.lock
  .artifacts/
    <hash>.metadata.json
    <hash>.content
```

`persistence.sessions_dir` may change the user-owned sessions root. Project
configuration cannot redirect it. Session/project IDs are validated as single
path components. Directories and files are owner-only, writes use
same-directory atomic replacement, and symlinked storage leaves are refused.

## Completed snapshots

The snapshot is Agent Runtime's canonical session state: conversation history,
usage ledger, monotonic identity counters, ordered manifests, and compatible
versioned extension state. Smith saves after each completed turn and during
orderly shutdown. The outer Smith snapshot schema is checked before the payload
is parsed.

For a root session, redaction-safe extension state also contains a bounded
durable-child catalog: stable child and child-session IDs, parent ownership,
immutable composition fingerprint, lifecycle state, cumulative limits/usage,
record revision, and latest safe checkpoint watermark. It contains no task
body, prepared argument, questionnaire answer, or raw child result. Those stay
in each child's own snapshot/protected checkpoint under the same project-owned
session namespace.

A profile-based child's immutable composition fingerprint includes the
child placement, effective profile name and revision, posture, resolved
provider/model, model/context policy, prompt-fragment revisions, and read-only
ceiling. Follow-up therefore reuses the exact profile identity. Resume fails
closed if the profile instructions, posture, placement, provider/model, or
source-derived revision changed; it never mixes revisions or silently spawns
a replacement. Raw profile instructions are not copied into the child catalog.

The optional `smith.reasoning.override` extension is also redaction-safe. It
contains only an explicit boolean thinking state and/or a bounded advertised
effort name. Older sessions omit it and retain provider/model defaults. Resume
re-applies it as a session override and the runtime revalidates it against the
new frozen capability snapshot before credential lookup or provider I/O;
`/think default` and `/effort default` remove their respective saved fields.

If a child returns a questionnaire to its parent, the exact request is a
sensitive child-checkpoint component. Smith reloads and re-queues that same
request while wiring the restored coordinator—still without constructing the
child provider. Public prompts may then enter the ordinary typed parent
delivery; sensitive prompts remain metadata-only there.

Ordinary JSON snapshots retain only extension namespaces explicitly classified
redaction-safe. Registered credential literals are removed from the persisted
clone; live canonical state is not rewritten. Sensitive todo, questionnaire,
memory, and summary state remains in the protected checkpoint.

A model/provider rebuild resumes the same canonical history and identity.
Persisted cache plans are an optimization: a valid plan for another resolved
model profile is discarded as a cache miss, while malformed or unsupported
component state still fails closed.

## Event journal

Every complete runtime event is serialized as one versioned JSON object per
line. The synchronous observer only enqueues into a bounded channel; one writer
owns disk order. If the writer falls behind, the journal records an explicit
`dropped` marker. An oversized event becomes an `oversized` marker rather than
a truncated JSON value.

The journal is default observability, not protected execution state. Raw tool
arguments are disabled; sensitive values and registered secrets are redacted
before serialization. A truncated final line after a crash is reported and
removed before appending resumes. Sequence gaps remain visible to replay and
the TUI.

Checkpoint publication first flushes every journal record before its watermark.
A nonterminal resume trims presentation-only journal records at or after that
watermark so the recovered turn can publish one canonical continuation rather
than duplicate terminal output. A terminal snapshot remains authoritative for
completed conversation/accounting state.

## Protected checkpoints

The latest exact `TurnCheckpoint` is encrypted with XChaCha20-Poly1305 under a
256-bit per-user key. The source is explicitly one of macOS Keychain/Linux
Secret Service, `SMITH_CHECKPOINT_KEY`, an owner-only inline
`persistence.checkpoint_key`, or an `env:`
`persistence.checkpoint_key_credential`. Inline/environment sources bypass OS
credential services entirely and decoded material is zeroized. The project,
session, turn, envelope schema, algorithm, and fresh 192-bit nonce are
authenticated. A moved, corrupted, wrong-key, truncated, or incompatible
record produces the same non-secret diagnostic.

Checkpoints can contain exact provider requests, prepared tool arguments,
questionnaires and answers, committed results, artifact references, and
versioned component state. Smith checkpoints after:

1. input acceptance;
2. complete model-response assembly;
3. prepared tool calls becoming ready;
4. each committed tool result or interaction boundary;
5. turn completion and publication.

Direct prepared composer actions use the same machine. Exact file reads and
leading-`!` shell calls checkpoint acceptance, preparation, execution intent,
raw outcome, processed/artifact result, and terminal publication. Recovery
may execute a durably prepared action once, processes a durable raw outcome
without replay, and refuses to replay an indeterminate executing action.

Only one process may own a session lifecycle, and checkpoint saves use a
cross-process writer lease plus monotonic revision checks. Two sibling writers
cannot both replace the latest state.

Durable child checkpoints and the parent-owned child catalog cross separate
atomic boundaries. After resuming the parent, Smith calls the coordinator's
provider-free recovery pass before accepting delegation commands. A compatible
child checkpoint newer than the catalog refreshes its watermark and exact
resumability; missing, regressed, terminal-without-catalog-transition, or
indeterminate provider checkpoints are reported non-resumable and never start
a replacement. The same pass restores protected returned questionnaires.

If the protected key service is unavailable, a host that can safely continue
may report mid-turn durability as unavailable; it must not claim pending
approval/question recovery. An existing protected record that cannot be opened
fails closed.

`smith setup checkpoint-key` performs an owner-only atomic config transaction.
Selecting the existing source is a no-op. If any `.checkpoint.bin` exists
under the bounded session inventory, a source change refuses before touching
config; Smith does not guess whether old encrypted state may be discarded.
This is the supported fail-closed rotation behavior until an all-checkpoint
atomic re-encryption transaction is available.

## Pending interactions

An interactive restart restores an exact pending prepared approval or
questionnaire once, with its original request/turn/call identity and deadline.
The host can answer it and resume the same turn. Prompt/answer content stays in
the checkpoint, not the journal.

A headless restart does not fabricate a decision or consume prompt stdin. A
pending approval returns metadata-only `approval_required` with exit status 4;
a pending questionnaire returns metadata-only `interaction_required` with exit
status 5. Both leave the exact pending checkpoint intact for an interactive
resume.

## Artifacts and semantic summaries

Oversized tool output is persisted before model-facing truncation. The model
and headless result receive a bounded preview and typed `ArtifactRef`; reads are
paginated, size-bounded, integrity-checked, and authorized against the owning
session. Artifact IDs and hashes are references, not bearer capabilities.

Child artifacts stay child-owned. Safe-boundary delivery explicitly copies an
observed reference into parent ownership with immutable lineage; it does not
widen the source reference.

Semantic summarization first stores the exact covered turn groups as a
protected session artifact. A dedicated tool-free summary request has its own
purpose, token cap, timeout, revision, retention, and disjoint usage source.
Failed or invalid summaries leave the structural history plan unchanged.

## Durable children and legacy ephemeral work

When both child snapshot and protected checkpoint stores are available, a
child is durable. Parent startup validates its catalog and renders retained
children without constructing providers, invoking tools, or consuming tokens.
An idle child accepts an explicit new-turn `follow_up` under its original
history and cumulative limits. An orphaned running record becomes
`interrupted`; only explicit `resume` may continue its exact safe checkpoint,
without incrementing the child task count again.

Resume refuses a missing, corrupt, terminal, regressed, policy-incompatible,
or unsafe checkpoint. A checkpoint immediately before provider I/O is unsafe:
the remote outcome is indeterminate, so replay could duplicate work. Smith
marks it non-resumable instead. Unknown or incompatible child IDs never cause
a replacement spawn.

Journal-only historical children and intentionally non-persistent children
remain `legacy_ephemeral`. On process restart unresolved legacy identities are
deterministically marked `process_exit`, displayed as interrupted/not
restarted, and written once so a second resume does not report them again.
Process-owned monitors remain ephemeral; no monitor executor is inferred from
reconciliation markers.

`/timeline` flushes and reads the redacted canonical journal, then projects
stable root turn/child IDs, terminal todo counts, shell validation-gate
outcomes, and metadata-only undo/revert/redo transactions. It never consults
protected raw invocation state or sends a provider request. Child inspection
is temporary; process-exit children remain labelled interrupted and are never
restarted by navigation.

## Operational recovery

- Use `smith sessions list` to obtain exact project-scoped identities.
- Resume interactively with `smith --resume <session-id>` when a protected
  approval/question is pending.
- Inspect retained children with `/agent`; use `@child-id …` for a new
  follow-up turn and `/agent resume <child-id>` only for the exact interrupted
  task.
- A schema, integrity, key, lifecycle-lease, or retained-gap error is a real
  recovery boundary; do not delete it merely to make startup proceed.
- Back up the whole project partition together. Copying only a JSONL journal
  cannot recover exact pending work.
- Disabling persistence creates no resumable identity and makes
  `--resume <id>` an error.
