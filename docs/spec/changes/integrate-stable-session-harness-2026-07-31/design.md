## Context

Smith already has the right ownership split:

```text
smith-runtime  product composition and policy
smith-host     workspace, approval, persistence, journal, change attribution
smith-tui      pure state/reducer/rendering
smith-cli      terminal and headless I/O loops
```

The current wiring still reflects the older runtime:

- `smith-runtime::factory` installs one short `INSTRUCTIONS` string and a fixed
  vector of tools.
- `smith-cli::run_tui` maps `Action::Interrupt` to
  `SessionHandle::cancel(UserRequested)`, which permanently cancels the root
  token.
- `smith-tui::App::apply` appends every `TextDelta` and `ReasoningDelta`
  immediately without attempt identity or rollback.
- `smith-host::InteractiveApproval` is the only live model-to-user wait channel.
- `FileSessionStore` is sound and atomic, but Agent Runtime calls it during
  shutdown rather than after every completed turn or pending boundary.

The target composition is:

```text
SmithRuntimeFactory
  ├── Smith prompt/context sections
  ├── Smith ability sources and activation policy
  ├── Smith tool implementations
  ├── InteractionBroker (TUI or unavailable/headless)
  ├── ApprovalBroker (prepared actions only)
  ├── SmithCheckpointStore
  ├── ArtifactStore + summary model policy
  └── shared Agent Runtime harness

smith-cli host loop
  ├── runtime events
  ├── approval requests
  ├── questionnaire requests
  ├── keyboard/input
  └── pure App actions

smith-tui App
  ├── committed transcript
  ├── speculative attempt buffers
  ├── prompt queue + one temporary overlay
  └── local plan/artifact/status projections
```

## Goals / Non-Goals

### Goals

- Preserve one runtime composition path across all Smith surfaces.
- Make interrupt reusable, retry streaming correct, and replay equivalent.
- Resume complete manifests and exact pending work without repeating side
  effects.
- Make approvals display the exact prepared resource and arguments.
- Let the agent ask a small structured questionnaire when a material user
  choice is truly required.
- Exercise Agent Runtime's live ability and harness path with Smith's current
  built-ins before adding extension sources.
- Improve authored coding behavior through versioned prompt sections and
  scenario evaluations.
- Add recoverable context offloading without placing private artifacts in the
  project tree.

### Non-Goals

- Put provider/tool execution logic in `smith-tui` or `smith-cli`.
- Reuse approval as a general question UI or let an answer grant authority.
- Keep failed retry text in the canonical transcript.
- Restart children, monitors, or tool processes automatically after a crash.
- Enable workspace skills merely because a file claims to be trusted.
- Add MCP, arbitrary subprocess extensions, nested agents, or a daemon in this
  change.
- Add a permanent plan/artifact/inbox pane contrary to the current interaction
  model.

## Decisions

### Smith migrates only through the one factory

`smith-runtime::factory` remains the sole mapping from resolved product policy
to Agent Runtime. It creates Smith-owned prompt sections, ability sources,
interaction readiness, stores, artifacts, and standard harness components.
TUI, `smith -p`, children, tests, and future embeddings select host adapters
but cannot assemble a different turn pipeline.

The coordinated Agent Runtime dependency is first exercised through a sibling
development override. Release source changes only after the runtime has a real
tag or exact Git revision and both compatibility suites pass.

### Interrupt targets the active turn

`Action::Interrupt` calls `interrupt_current_turn(UserRequested)`. The App
enters `Activity::Interrupting` until the matching `TurnCompleted` event.
Shutdown and reconfiguration use terminal session cancellation only after the
host stops accepting new work.

If no turn is active, interrupt is a local no-op with no cancellation token
mutation. A later composer submission must work after an interruption.

### The reducer owns speculative presentation state

`App` tracks attempt buffers keyed by `(RequestId, AttemptId)`. Text and
reasoning deltas render with a speculative marker while their attempt is live.
On commit they merge into the canonical transcript. On discard they are
removed and may leave a concise retry notice; raw failed text does not survive
in normal transcript replay.

Journal replay feeds the same reducer events and must produce the same
committed transcript as the live path. Usage from discarded attempts remains
visible in status/diagnostics.

### Prepared approval and questionnaires are separate prompt types

Smith replaces its approval request payload with the runtime's immutable
`PreparedToolCall`. The modal emphasizes:

```text
tool + exact canonical target
material arguments / reviewed patch
typed permissions and broad authority warnings
deadline
allow once / allow scoped session / deny
```

Parallel prepared calls are presented as one deterministic batch or a stable
queue. One prompt never silently supersedes and denies another.

For `edit`, the prepared filesystem resource is the logical target file.
Creating that file is permitted only when its parent directory already exists;
the invocation never creates unprepared ancestors. Atomic replacement may use
one randomized sibling temporary as a trusted implementation detail of that
logical write. The temporary is never model-selectable, never retained as a
session grant, and must leave no durable sibling on either success or any
recoverable failure.

The questionnaire path has independent types and an independent responder. It
supports one to three questions in a short wizard/overlay:

- stable prompt and option labels;
- arrow/number navigation for choices;
- composer-backed free-form input where allowed;
- explicit Submit, Decline, and Cancel behavior;
- no implicit selection that Enter can accidentally approve;
- restored pending state after restart.

Questionnaire answers enter the current turn as the standard ability result.
They never call approval APIs, alter permission scopes, or remember an
authority grant.

Smith activates direct user interaction for root sessions only by default. A
child that needs a decision returns an attributed structured `needs_input`
result through the existing parent safe-boundary path; the parent decides
whether to ask the user. A future profile may authorize direct child prompts,
but it must be explicit so concurrent children cannot unexpectedly compete for
the terminal.

### Headless interaction is explicit

The normal `smith -p` composition reports no interactive questionnaire
readiness, so the ability is omitted. If a forced/replayed request reaches the
broker, Smith returns a versioned `interaction_required` terminal result and a
stable non-success exit rather than waiting on stdin.

A future bidirectional `stream-json` response protocol requires a separate
proposal. Plain prompt stdin is not treated as an asynchronous answer channel.

Headless prepared approval keeps its existing fail-closed rule: absent an
explicit allowing policy, return `approval_required` and do not wait.

### Checkpoints are protected separately from the journal

Smith implements Agent Runtime's `CheckpointStore` under the existing
project-scoped user state. The store uses authenticated encryption with a
user-scoped key and atomic/transactional replacement. There is no silent
plaintext fallback for exact mid-turn state.

The reviewed implementation fixes the following dependency and platform
contract:

- `chacha20poly1305` 0.11.0 from RustCrypto supplies
  XChaCha20-Poly1305. It is MIT OR Apache-2.0, declares Rust 1.85 (below
  Smith's Rust 1.88 floor), and its upstream security notes report an
  NCC Group audit with no significant findings. The 192-bit/24-byte XChaCha
  nonce supports a fresh operating-system-random nonce per replacement without
  a durable nonce counter.
- The already-selected `keyring` v3 backend stores the random 32-byte binary
  key through `set_secret`/`get_secret`. Smith keeps
  `default-features = false` and enables only `apple-native` for macOS and
  `sync-secret-service` for Linux. Other targets report protected durability
  unavailable; they never fall back to a plaintext key file or mock backend.
- Generated, keyring-returned, serialized, and decrypted plaintext buffers are
  zeroized after use. The retained key has redacted `Debug` output and is
  zeroized on drop.
- A user-global owner-only advisory lease serializes the
  absent/read/create/read key-enrollment sequence across Smith processes.
  A per-session owner-only advisory lease serializes
  load/validate-successor/encrypt/rename, so two store instances cannot both
  publish sibling revisions. Advisory locks are released by the operating
  system after crashes and cannot become stale marker files.
- The encrypted envelope and lock files live under directories forced to
  `0700`; sibling temporaries and final files are forced to `0600`. Writes use
  `create_new`, file fsync, rename, and directory fsync. Symlinked leaf
  directories and files are refused.

The dependency gate is `cargo deny check` against the repository's explicit
license/source policy and macOS/Linux target graph. Its result is recorded in
the implementation task list whenever this decision is changed. Primary
review sources are the
[RustCrypto crate metadata and security notes](https://docs.rs/chacha20poly1305/0.11.0/chacha20poly1305/)
and the
[keyring v3 binary-secret/platform documentation](https://docs.rs/crate/keyring/3.6.3/source/README.md).
The 2026-07-31 review passed advisories, bans, licenses, and sources; the
existing duplicate-version policy emitted warnings only.

If the protection key is unavailable, Smith may continue with redacted
completed-turn snapshots if product policy permits, but must report that
mid-turn crash recovery is unavailable. It cannot claim durable approval or
question recovery.

The JSONL journal remains redacted observability. Each checkpoint records the
latest durable event sequence; each terminal snapshot records the checkpoint
revision. Startup reconciles:

1. compatible checkpoint and journal watermark;
2. complete journal tail for presentation only;
3. previously active ephemeral work as interrupted;
4. no automatic replay of a provider call or side effect without an
   idempotent recorded state.

### Smith registers abilities, not a static product tool list

The initial registry includes accurate descriptors for:

| Ability | Default posture |
| --- | --- |
| `read`, `list`, `search` | read-only, low risk |
| `edit` | exact prepared file write, approval by policy |
| `shell` | broad workspace/process/network upper bound |
| `agent` | root-only delegation with existing limits |
| `ask_user` | interactive-host readiness, no authority |
| todo state | pure checkpointed state mutation |
| `artifact.read` | bounded session-private read |

Initial retrieval activates the smallest dependency-complete subset. A
read-only repository question must not advertise `edit` or `shell` merely
because they are installed. An editing request may activate them according to
trust and approval policy.

The TUI surfaces activation/lifecycle information in `/status` and concise
notices, not a permanent registry pane.

### Prompt policy is versioned and sectioned

Smith replaces the one-line prompt with independently versioned fragments:

```text
identity
workflow
workspace and trust boundary
inspection-before-edit guidance
tool-use guidance
verification and evidence requirements
approval behavior
questionnaire guidance
delegation guidance
response style
activated skills
memory
current project context
```

The default workflow is:

```text
understand → inspect → plan when multi-step → modify → verify → report evidence
```

Smith explicitly forbids claiming a command or test succeeded unless a
committed tool result shows it ran successfully. Dynamic sections remain
separate so Agent Runtime can budget, cache, activate, and compact them.

Questionnaire guidance says to ask only when a material choice cannot be
safely inferred and to continue autonomously for routine reversible details.

### Skills and memory keep Smith-owned trust policy

Source precedence is deterministic:

```text
built-in < user profile < trusted workspace < session override
```

Metadata may be indexed before activation, but a workspace skill body is
loaded as privileged instructions only when project trust, provenance,
revision, and activation policy permit it. Session overrides cannot grant
tool authority.

Memory contributors are bounded, versioned, sensitivity-aware, and separate
from canonical conversation history. Smith chooses what to store and retrieve;
the runtime supplies the contributor/checkpoint mechanism.

### Todos, artifacts, and summaries fit the transcript-first UI

Todo updates render as a compact inline plan block and are inspectable through
`/status` or a future local `/plan` command. They do not create a permanent
pane and the prompt uses them for genuinely multi-step work, not every turn.

Large outputs are stored in a Smith session-private artifact root through the
generic `ArtifactStore`. The transcript shows a bounded preview and reference;
a temporary scrollable artifact view may inspect it without copying it into
the project.

Smith selects the semantic-summary model/purpose, spend limit, and retention
policy. Original turn groups are stored before summarization. Failed or
unvalidated summaries leave the deterministic structural plan unchanged.

### Scenario evaluations gate harness changes

In addition to trait/unit conformance, Smith maintains full-stack fixtures for:

- read-only question activates only read capabilities;
- editing request prepares and approves an exact path;
- interrupt then later turn;
- failed partial stream then successful retry;
- questionnaire answer resumes the same turn;
- approval/question pending across restart;
- large output offload and bounded reread;
- multi-step todo lifecycle;
- skill activation from trusted versus untrusted workspace;
- semantic summary with recoverable originals;
- delegation result at a safe boundary;
- live reducer and journal replay equivalence.

## Risks / Trade-offs

- Coordinated breaking changes temporarily require a sibling runtime override.
  The release gate prevents publishing that state.
- Speculative rendering is more stateful than append-only deltas, but it keeps
  latency and corrects retry transcripts.
- Encrypted checkpoints introduce key-management failure modes. Explicitly
  disabling mid-turn durability is safer than plaintext fallback.
- Capability retrieval can make tool availability less obvious. `/status`,
  activation events, and deterministic fixtures keep it inspectable.
- Richer prompt sections consume context. Separate fragments and evaluation
  prevent a monolithic prompt from crowding out the task.
- Agent questions can become an avoidance mechanism. Bounded schemas and prompt
  policy restrict them to material decisions.

## Migration Plan

1. Update the sibling runtime override and migrate compile-time API/event
   changes in a dedicated branch; keep the pinned release source unchanged.
2. Change interrupt and speculative reducer behavior, then pass live/replay
   transcript tests before migrating persistence.
3. Migrate Smith tools to preparation and descriptors; use current built-ins as
   the first activation evaluation.
4. Add the protected checkpoint store and completed-turn persistence, then
   pending approval/question recovery.
5. Add questionnaire UI/broker and headless unavailable results.
6. Replace prompt composition with versioned sections and enable live ability
   activation.
7. Add todos, trusted skills, memory, artifacts, offloading, and semantic
   summaries in that order.
8. Reconcile the older harness change's unfinished overlapping tasks and
   archive completed superseded changes.
9. Pin the released runtime revision and run macOS/Linux, MSRV, security,
   replay, and full-stack evaluation gates before publication.

## Open Questions

- A future bidirectional machine protocol for questionnaire responses is out of
  scope and requires separate approval.
