---
created_at: 2026-07-31T08:34:33Z
updated_at: 2026-07-31T14:38:41Z
---

## Why

Smith correctly uses Agent Runtime as its execution mechanism and keeps the TUI
as a reducer, but its current composition exposes several runtime defects
directly: `Action::Interrupt` permanently cancels the session, retry deltas are
appended immediately, snapshots are saved only during orderly shutdown, and
approval prompts describe static tool effects rather than one exact prepared
action. Smith also still supplies one short prompt and a fixed tool vector
instead of exercising the ability/activation architecture.

The existing Smith harness change tracks much of the product, but it predates
the proposed session-scoped, checkpointable runtime pipeline. A coordinated
follow-up is needed to migrate Smith without duplicating runtime mechanism and
to add one missing product capability: a structured way for the agent to ask
the user for clarification or a choice. This questionnaire is not a security
approval.

## What Changes

- Consume the coordinated Agent Runtime breaking release only after its
  release-gate conformance suite passes; retain one Smith runtime factory for
  TUI, headless, children, tests, and embeddings.
- Map TUI interrupt to turn-local interruption and reserve session
  cancellation for shutdown/revocation.
- Buffer provider text/reasoning by request and attempt, render it
  speculatively, and commit or remove it on explicit runtime events so live and
  replayed transcripts are equivalent.
- Replace approval display/input with exact immutable prepared actions,
  deterministic batching, deadline/cancellation handling, and restored pending
  state.
- Add a separate Smith interaction broker and questionnaire overlay for
  agent-originated clarification or choice. Answers resume the same tool loop
  and never authorize a side effect.
- Add a protected Smith checkpoint store linked to the event journal. Persist
  completed turns immediately and resume pending approvals/questions and
  partially completed tool batches without reconstructing them from redacted
  events.
- Register Smith's built-ins through Agent Runtime abilities with typed
  permissions, affordances, risk, cost, readiness, and activation policy:
  `read`, `list`, `search`, `edit`, `shell`, and `agent`, followed by standard
  `ask_user`, todo, and artifact abilities.
- Replace the one-line prompt with versioned Smith-owned sections and dynamic
  context contributors.
- Adopt generic harness components for todos, skills, memory, artifacts,
  recoverable output offloading, semantic summaries, capability search, and
  delegation while keeping Smith's source/trust/storage/presentation policy.
- Preserve the transcript-first interaction model; new plan, artifact,
  approval, and questionnaire views are inline or temporary overlays, not
  permanent execution panes.
- Keep non-interactive execution fail-closed. A headless run without an
  explicit bidirectional interaction protocol never advertises or waits on the
  questionnaire ability.

## Impact

- Affected specs: new `runtime-integration`, `session-recovery`,
  `client-interaction`, and `harness-policy`
- Affected code: `smith-runtime`, `smith-host`, `smith-tui`, `smith-cli`,
  `smith-tools`, Smith configuration, persistence, tests, and `DESIGN.md`
- External dependency:
  `../agent-runtime/docs/spec/changes/stabilize-session-harness-pipeline-2026-07-31/`
- Public compatibility: TUI reducer state, machine event projection, headless
  terminal results, checkpoint storage, and the pinned Agent Runtime version
- Security: questionnaire answers are task input only; prepared approval stays
  the sole interactive security decision
- Persistence: exact checkpoints use host-protected storage and are not copied
  into the redacted JSONL journal

## Existing Change Coordination

- `add-smith-agent-harness-2026-07-23` remains the baseline and its completed
  work is preserved. After this proposal is approved, overlapping unfinished
  tasks for crash recovery, sidecars, inbox orchestration, ability registration,
  headless approval, and upstream security migration SHALL be marked as moved
  here rather than implemented twice.
- `update-smith-interaction-model-2026-07-27` remains authoritative for the
  composer-first, transcript-first UI. Questionnaire and prepared-approval
  states are temporary overlays with visible key hints.
- Existing setup, model catalog, tool summary, command, diff/review/undo, and
  credential changes remain intact.

## Delivery Slices

1. Runtime compatibility: pin the coordinated runtime, migrate session control,
   prepared tools, and attempt events, then pass both workspaces.
2. Correct interaction: speculative reducer, turn interruption, exact approval,
   questionnaire broker/overlay, and headless unavailable behavior.
3. Durable execution: completed-turn save, protected checkpoints, pending-state
   recovery, and journal/checkpoint watermarks.
4. Integrated harness: ability view/activation, versioned prompt sections,
   lifecycle events, and current-tool routing evaluations.
5. Standard product components: todos, trusted skills, memory, artifacts,
   semantic summaries, and recoverable offloading.

## Follow-on Roadmap

After this migration is complete, separate Smith proposals may enable MCP and
subprocess sources through the ability registry, consume a concrete isolation
backend, enrich child results with structured artifact references, introduce
evaluation-justified model profiles, or add a separate agent process for
demonstrated background or remote-client needs. The first extension source is
not part of this approval.

## Approval Boundary

Approval authorizes Stage 2 changes in this repository only after the
coordinated Agent Runtime proposal is approved. It does not authorize Agent
Runtime edits, public package release, MCP/subprocess extension rollout,
automatic project trust, a bidirectional headless protocol, or nested agents.
