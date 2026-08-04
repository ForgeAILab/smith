---
created_at: 2026-07-31T08:34:33Z
updated_at: 2026-07-31T14:38:41Z
completed_at:
---

## 0. Coordination and Dependency Gate

- [x] 0.1 Approve this proposal and the coordinated Agent Runtime proposal.
- [x] 0.2 Map unfinished overlapping tasks in
  `add-smith-agent-harness-2026-07-23` to this change and mark them moved only
  after approval; preserve all completed work.
- [x] 0.3 Add a git-ignored sibling runtime override for implementation and
  retain the pinned release/Git source in committed manifests.
- [x] 0.4 Record Smith machine-output, event-reducer, snapshot, and approval
  compatibility fixtures before adopting the breaking runtime API.

## 1. Runtime API and Tool Migration

- [x] 1.1 Migrate the one Smith factory, direct children, tests, TUI, and
  headless surfaces to `SessionExecutionContext`, `TurnHandle`, structured
  submission errors, and distinct turn/session cancellation.
- [x] 1.2 Map `Action::Interrupt` to turn-local interruption and verify a later
  turn completes on the same session.
- [x] 1.3 Migrate all built-in Smith tools to exact preparation and invocation;
  canonicalize edit/read/search/list paths and declare shell's broad upper
  bound honestly.
- [x] 1.4 Replace approval payloads with immutable prepared actions and render
  exact target, material arguments, typed permissions, broad-authority
  warnings, deadline, and preparation fingerprint.
- [x] 1.5 Present parallel pending approvals as a deterministic batch/queue;
  never silently supersede a prompt.
- [x] 1.6 Use the registry permission type in Smith descriptors and add a test
  proving every prepared invocation remains inside its descriptor bound.

## 2. Attempt-Scoped Reducer and Interaction Correctness

- [x] 2.1 Add request/attempt-keyed speculative text and reasoning buffers to
  the pure `App` reducer.
- [x] 2.2 Commit or remove buffers only on explicit attempt terminal events;
  retain failed-attempt usage and a concise retry diagnostic.
- [x] 2.3 Make journal replay and live reduction produce equivalent committed
  transcript, tool state, status, and visible-output results.
- [x] 2.4 Add interaction prompt queue state independent from approval state
  and ensure exit/cancellation answers every responder exactly once.
- [x] 2.5 Update `DESIGN.md` for speculative output, prepared approval, the
  questionnaire wizard, restored pending state, narrow terminals, keyboard
  hints, and accessibility.

## 3. Questionnaire Capability

- [x] 3.1 Implement Smith's `InteractionBroker` adapter and register
  questionnaire readiness only for an interactive host.
- [x] 3.2 Add a temporary questionnaire overlay for one to three questions,
  choices, optional free-form answers, explicit submit/decline, cancellation,
  and deadline.
- [x] 3.3 Resume the same turn with the typed questionnaire result and prove
  the answer never calls approval or changes authority.
- [x] 3.4 Restore a checkpointed pending questionnaire with the same request
  identity and accept its answer exactly once.
- [x] 3.5 Make ordinary `smith -p` omit the ability and return a versioned
  `interaction_required` non-success result if a forced/replayed question
  reaches the host; never wait on prompt stdin.
- [x] 3.6 Add reducer/render/end-to-end tests for choice, free-form, decline,
  timeout, cancellation, restart, sensitive-answer redaction, and no-TTY
  behavior.
- [x] 3.7 Keep direct user interaction root-only by default; route a child's
  attributed `needs_input` result through its parent and test concurrent-child
  behavior.

## 4. Durable Smith Checkpoints

- [x] 4.1 Select dependency-compatible authenticated encryption and atomic or
  transactional storage after license/platform review; store the key through a
  user-scoped protected backend.
- [x] 4.2 Implement `SmithCheckpointStore` beside existing project-scoped
  session state with owner-only permissions, schema/version gates, integrity
  checks, and no silent plaintext fallback.
- [x] 4.3 Persist completed turns immediately, then add checkpoints for
  accepted input, assembled response, prepared pending actions, each committed
  tool result, and terminal completion.
- [x] 4.4 Link checkpoint and journal watermarks and keep raw pending
  arguments, sensitive answers, and artifacts out of default JSONL events.
- [x] 4.5 Resume without repeating committed provider/tool work; mark prior
  monitors and children interrupted and never restart them automatically.
- [x] 4.6 Report mid-turn durability unavailable when checkpoint protection
  cannot initialize while retaining honest redacted completed-turn behavior.
- [x] 4.7 Add crash fixtures at every boundary, corrupt/torn checkpoint cases,
  key-unavailable behavior, legacy snapshots, and manifest preservation.

## 5. Ability Activation and Prompt Composition

- [x] 5.1 Register `read`, `list`, `search`, `edit`, `shell`, and `agent` as
  abilities with accurate affordances, typed permissions, risk, cost,
  readiness, and source provenance.
- [x] 5.2 Configure session-scoped registry view, deterministic initial
  retrieval, authorization, activation epochs, and protected capability
  search through the one factory.
- [x] 5.3 Prove a read-only question activates only the read subset and an
  editing request activates the smallest authorized mutation subset.
- [x] 5.4 Replace `INSTRUCTIONS` with independently versioned identity,
  workflow, trust, inspection, tool, verification, approval, questionnaire,
  delegation, response-style, skill, memory, and project-context fragments.
- [x] 5.5 Explicitly prohibit claiming tests/commands succeeded without a
  committed successful result and evaluate the default
  understand-inspect-plan-modify-verify-report workflow.
- [x] 5.6 Surface snapshot/view/activation/context lifecycle provenance in
  `/status` and concise transcript notices without adding a permanent pane.

## 6. Standard Harness Components

- [x] 6.1 Add generic typed todo state with checkpointing and `PlanUpdated`;
  render compact inline updates and use it only for multi-step work.
- [x] 6.2 Implement descriptor-first skill sources with deterministic
  built-in, user, trusted-workspace, and session precedence; load bodies only
  after trust and activation.
- [x] 6.3 Add bounded sensitivity-aware memory contributors while keeping
  storage/retrieval policy in Smith.
- [x] 6.4 Implement a session-private artifact store and temporary bounded
  artifact view; register authorized paginated `artifact.read`.
- [x] 6.5 Replace irreversible large-output truncation with a preview/reference
  where an artifact store is available.
- [x] 6.6 Configure semantic-summary purpose/model/spend/retention policy,
  persist originals, validate provenance, and fall back to structural
  compaction on failure.
- [x] 6.7 Keep existing delegation as the standard adapter and route final
  structured child results/artifact references through safe boundaries.
  Completed and needs-input outcomes use the protected once-delivery
  coordinator path. Child artifacts remain child-owned; the coordinator
  explicitly copies observed child-turn references into parent ownership with
  validated lineage before delivering the typed parent reference.

## 7. Headless, Replay, and Product Evaluation

- [x] 7.1 Update text, JSON, and stream-JSON projections for attempt commit/
  discard, structured submission rejection, prepared approval, checkpoint
  recovery, activation, todos, artifacts, and interaction-required outcomes.
- [x] 7.2 Keep stdout machine-only and make all no-TTY approval/question paths
  terminate predictably without waiting.
- [x] 7.3 Add full-stack scenarios for read-only routing, exact edit approval,
  retry rollback, interrupt/reuse, question/resume, crash recovery, artifact
  offload/reread, todo lifecycle, trusted/untrusted skills, semantic summary,
  and child safe-boundary delivery.
- [x] 7.4 Add `live_reducer_and_journal_replay_produce_equivalent_ui_state`
  across ordinary, retry, approval, question, tool, and recovered sessions.
- [ ] 7.5 Run fmt, warning-denied Clippy, all tests, MSRV, macOS/Linux CI,
  dependency/license/security gates, and real terminal visual QA at narrow,
  normal, and wide sizes.
  - Local macOS evidence: formatting and warning-denied Clippy pass; the
    current and Rust 1.88 workspaces pass all 693 non-live tests with one
    explicitly quota-spending provider test ignored in the ordinary suite;
    that opt-in black-box test separately passes against Z.AI Coding Plan
    `glm-5.2` using an environment-backed plaintext provider credential. That
    no-keychain live matrix covers todos, list, search, exact edit, shell,
    child delegation, fail-closed approval, and the real questionnaire TUI.
    Artifact offload, capability search plus `artifact.read`, and durable
    resume were also exercised live with the protected OS checkpoint-key
    backend; the deterministic suite covers their unavailable-key fallback.
    `cargo-deny` and `cargo-audit --deny warnings` pass; the real PTY binary
    passes at 44×14, 74×24, and 120×32. Hosted Linux/macOS exact-revision CI
    and a live durable run under any future non-keychain checkpoint-key policy
    remain required.

## 8. Release and Cleanup

- [ ] 8.1 Remove migration aliases and the sibling override after both
  repositories pass against the exact released Agent Runtime revision.
- [ ] 8.2 Reconcile and archive completed/superseded Smith spec changes without
  losing their truth requirements.
- [x] 8.3 Update README, `DESIGN.md`, configuration reference, persistence/
  recovery documentation, headless protocol docs, and security threat model.
- [ ] 8.4 Publish only after the runtime release gate and every scenario in
  Section 7 pass.
