---
created_at: 2026-10-23T21:44:00Z
updated_at: 2026-10-23T21:44:00Z
completed_at:
---

# Implementation and validation tasks

Stage 1 only: all implementation boxes remain unchecked. Approval of this proposal
is required before code changes. Complete in dependency order; preserve unrelated
working-tree changes. `evidence.md` records the earlier live model experiment,
not completion of any task below.

## 1. Agent Runtime: bounded two-layer mechanism

- [ ] 1.1 Rebase the upstream work on the inspected runtime baseline; inventory all
  parent/depth assumptions, public struct literals, schema readers, ability
  derivation, and existing testkit consumers before changing APIs.
- [ ] 1.2 Add `SessionInner` lineage (`delegation_depth`, `root_session`, `child_id`),
  accessors on `SessionHandle`, protected persistence/versioning, and explicit
  depth-0/1/2 construction/recovery validation. Preserve the immediate parent ID.
- [ ] 1.3 Add `DelegationLimits::max_depth`, default 1 for neutral consumers, a
  supported maximum of 2, and depth-based checks in coordinator construction,
  spawn, follow-up, resume/rebind, and conditional continuation admission.
- [ ] 1.4 Change `mint_child_id` to `child-N` / `child-N.M`, persist counter
  reservation, validate ownership independently of spelling, and test uniqueness
  across leads, failure, pruning, and restart.
- [ ] 1.5 Share one process-wide `Arc<DelegationCapacity>` across Root and Layer-1
  coordinators, preserving local caps/queues and release-once behavior. Test
  multi-root saturation, queued-spawn bounds, parked leads, and descendant budget
  accounting without deadlock or double-counted usage.
- [ ] 1.6 Add backward-compatible object-safe `child_builder_at_depth` and
  `on_child_session_started` hooks, with an explicit build/slot result; call the
  post-start hook before any initial, recovered, follow-up, or resumed task turn.
  Roll back partial startup and release capacity if wiring fails.
- [ ] 1.7 Strip `delegation_tool_names` at the maximum depth and for disabled
  routes; preserve host-approved `allow_child_interaction` instead of blindly
  calling `return_child_interactions_to_parent`. Keep leaf injection checks.
- [ ] 1.8 Add retained episode/turn sequencing and subscribe-before-check APIs for
  internal continuations. Do not rely only on a latest-turn watch or public
  broadcast; test fast completed turns and lagged observers.
- [ ] 1.9 Implement non-consuming pending-work snapshots and the multi-turn
  `spawn_returned_input_collector` episode loop. Keep Layer-1 Running across
  parking; transfer only its final quiescent answer and final turn identity.
- [ ] 1.10 Add protected failure/stopped terminal outcomes and identities,
  deterministic ordering, redacted model projections, recovery, and exactly-once
  acknowledgement. Wake a parked lead on ChildFailed/ChildStopped without a
  successful worker result; never report interim parking prose as success.
- [ ] 1.11 Make cancellation, NeedsInput, limits, timeout, shutdown, and explicit
  follow-up generation changes terminate/supersede episodes safely and settle
  descendants in bounded order without admitting duplicate provider turns.
- [ ] 1.12 Forward attributed Layer-2 Child* lifecycle events in `spawn_monitor`;
  preserve source identity/replay behavior and separate event observation from
  immediate-parent protected outcome ownership and accounting.
- [ ] 1.13 Add bounded authorized coordinator traversal in `list`, `status`,
  `child_events`, `with_child_history`, `child_history`, and
  `subscribe_child_history`; preserve direct-child internal capacity/mutation
  semantics and read dormant descendants without starting execution.
- [ ] 1.14 Version nested catalog/lineage/episode/failure recovery and retain
  existing namespace partitioning. Add shared testkit consumer-conformance
  coverage; publish an upstream revision/API compatibility record for Smith.

## 2. Smith configuration and provenance

- [ ] 2.1 Add typed `[jev]` model/resolved settings with disabled-by-default
  behavior, loopback endpoint, bounded timeout, finite thresholds, clarifier
  profile selection, and optional bearer credential reference.
- [ ] 2.2 Add inherited `direct_interaction: Option<bool>` (default false), retain
  sourced profile `delegation`, and carry both through deterministic profile
  resolution, inventory/revision fingerprints, explain output, and reconfiguration.
- [ ] 2.3 Validate endpoint scheme/path, 1..=10,000 ms timeout, probability ranges,
  profile eligibility, unknown fields, credential references, and authority-
  narrowing precedence. Reject repository attempts to enable remote disclosure,
  select credentials, or grant child interaction/tool authority.
- [ ] 2.4 Add config/provenance/inheritance tests, including existing profiles
  without new fields, disabled Layer-1 delegation, inherited true on depth-2
  leaves, invalid references, NaN/infinity, boundary values, and secret-safe errors.
- [ ] 2.5 Document the new effective schema and role descriptions; parse/check
  the proposal's TOML against the implemented resolver before publishing examples.

## 3. Smith runtime and pre-turn triage

- [ ] 3.1 Update all `agent-runtime*` dependency revisions and the lockfile together
  after upstream publication; do not substitute an uncommitted sibling path.
- [ ] 3.2 Extend `SmithChildRoute` and root/default/child route builders with
  effective delegation, direct interaction, and wait policy while preserving
  profile fallbacks, harness execution, modules, and capability narrowing.
- [ ] 3.3 Implement `SmithChildFactory::child_builder_at_depth` to prepare Layer-1
  AgentTool/OnceLock/timers, delegation prompt, and DelegationAuthority, or a
  strictly leaf runtime at depth 2 / delegation=false.
- [ ] 3.4 Implement `on_child_session_started` to call
  `wire_delegation_with_wait_policy`, recover the nested catalog/outcomes, and
  retain the child lifecycle in a root-owned registry with Weak back-references.
  Compose `max_depth = 2` and the same shared-capacity Arc at both levels.
- [ ] 3.5 Carry the root `InteractionBroker` into the factory and enable DirectHost
  only for approved depth-1 profiles. Preserve ReturnToParent escalation,
  headless input-required behavior, original interaction IDs, and approval
  separation; update questionnaire/delegation guidance and eligibility.
- [ ] 3.6 Extend nested session-list filtering, child transcript delegation,
  shutdown, resume capsules, and dormant recovery without provider execution
  during inspection or independent child-root resume entries.
- [ ] 3.7 Implement `JevTriageClient` `/v1/systemone` transport with bounded total
  timeout/body sizes, cancellation, no redirects, credential-safe headers, and
  user-owned remote-disclosure policy. Do not start/download a sidecar implicitly.
- [ ] 3.8 Implement inverse-actionability complement, five-level score-index math,
  normalized choice/ordinal confidence validation, exact eligible profile roster,
  model identity validation, and typed advisory decision projection.
- [ ] 3.9 Add a shared HostSession real-user submission boundary and, if required,
  an upstream provider-free authorized host-control admission API. Preserve one
  canonical user input and the same delegation approval/workspace checks; never
  fake assistant tool calls or dispatch directly around authorization.
- [ ] 3.10 Implement bounded clarify/lead/fallback transitions, late-response
  deduplication, safe clarifier-to-new-Layer-1-lead handoff, and disabled/no-I/O
  behavior. Cancellation never falls through to ordinary execution.
- [ ] 3.11 Exclude internal/timer/goal/child turns, questionnaire replies as
  standalone inputs, slash/shell commands, and unsupported multimodal requests
  from recursive triage; preserve attachments through fallback.
- [ ] 3.12 Test timeout, HTTP/auth errors, invalid/fake/wrong-model response,
  malformed/non-finite probabilities, inconsistent legends/scores/confidence,
  oversized input/output, missing profiles, low confidence, redirect, cancellation,
  and approval-denial paths with deterministic local HTTP fixtures.

## 4. Smith client, TUI, and CLI

- [ ] 4.1 Project authorized descendant statuses and explicit parent/depth identity
  through `smith-client` agent reports and `/agent` inspection without inventing
  new lifecycle words or copying full implementation transcripts into Root.
- [ ] 4.2 Add numeric hierarchical tree ordering and indentation for `child-1.1`
  in the delegated-work panel, preserving navigation, paging, clocks, profile
  identity, activity provenance, and composer/root transcript restoration.
- [ ] 4.3 Guard root pending-spawn correlation (`pending_spawns` in current source;
  `pending_agent_spawns` in the blueprint) so a Layer-2 spawn cannot pop a pending
  root spawn. Prefer source parent/call identity to guessing from arrival order.
- [ ] 4.4 Extend subscriptions and transcript hydration to live/dormant
  grandchildren with canonical ordering, redaction, stable pages, no duplicates,
  and explicit unavailable segments; deny sibling/cross-root inspection.
- [ ] 4.5 Serialize DirectHost questionnaires at the shared root surface, display
  source child identity, and route responses/cancellation to exact origin IDs.
  Test two simultaneous requests and answers arriving after child shutdown.
- [ ] 4.6 Migrate interactive, headless, and supported embedded user submissions
  to shared triage admission, with consistent bounded routing/fallback notices
  and no synthetic root model prose, duplicate user message, or hidden network I/O.
- [ ] 4.7 Keep headless alive across the full two-layer episode and final Root
  delivery, including failure wake and admission races; stop when required work
  settles rather than waiting for retained records, cache policy, or stale timers.

## 5. Deterministic integration, conformance, and rollout validation

- [ ] 5.1 Add shared runtime-testkit fixtures for Root -> Lead -> Worker success,
  both return-input escalation paths, DirectHost, failure/stopped wake, and
  protected recovery. Run them against upstream and Smith consumer composition.
- [ ] 5.2 Use barriers/fake clocks to cover each collector/admission interleaving:
  completion before parking, between subscribe/check, during persistence, rapid
  multiple continuation turns, user-priority races, lagged observation, shutdown,
  and superseded episode generations. Assert exact final-result delivery counts.
- [ ] 5.3 Test authority invariants: max-depth injection, read-only parents,
  delegation-disabled profiles, tool/workspace intersections, hostile profile
  metadata, endpoint disclosure, task answers vs approvals, unrelated child
  histories, and corrupted/incompatible recovery state.
- [ ] 5.4 Add TUI reducer/render snapshots for two layers, root spawn correlation,
  numeric ID ordering, nested transcript inspection, single-focus modals, and
  unchanged root drafts; add headless golden output and completion-wait tests.
- [ ] 5.5 Add recovery/artifact/usage tests for nested catalogs, parent-owned result
  references, exact failure/interaction state, process-wide capacity leases,
  descendant token limits, event forwarding deduplication, and root list hygiene.
- [ ] 5.6 Run targeted upstream tests and consumer conformance; run
  `cargo test -p smith-config`, `cargo test -p smith-runtime`,
  `cargo test -p smith-client`, `cargo test -p smith-tui`, and
  `cargo test -p smith-cli`, then workspace formatting/Clippy as appropriate.
  Record actual command outcomes; do not mark unrun checks successful.
- [ ] 5.7 Benchmark the newly combined inverse-actionability/four-head contract on
  held-out prompts, broader positive classes, context-present/context-absent
  variants, role-boundary and adversarial cases. Report per-class recall,
  specificity, distributions, Brier/ECE, confidence coverage, and p50/p95 latency;
  do not select and claim calibration on the same evaluation set.
- [ ] 5.8 Validate any official hosted Jev adapter separately with explicit
  disclosure opt-in; do not claim local MLX evidence covers hosted, GGUF, or
  stock Ollama inference. Keep `[jev].enabled = false` until rollout acceptance.
- [ ] 5.9 Re-run strict spec validation and resolve dependency-delta overlaps with
  `update-agent-wait-to-monitor-2026-10-23` before merge. After approved shipping,
  archive/apply this change to truth specs, update the child-agents Purpose
  from one-level to bounded two-layer terminology, and update metadata; Stage 1 does not
  mark implementation complete or archive anything.
