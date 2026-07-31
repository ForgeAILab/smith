---
created_at: 2026-07-23T20:43:51Z
updated_at: 2026-07-27T20:48:01Z
completed_at:
---

> **Revised for Agent Runtime 0.1 and approved for implementation.**
>
> The original plan assigned provider, loop, context, event, usage, registry,
> and tool-execution mechanism to Smith. Those mechanisms now live in
> `../agent-runtime`. Completed Smith host/TUI/tool work is preserved below;
> remaining tasks compose shared contracts and implement product policy only.

## 0. Approval, Provenance, and Dependency Contract

- [x] 0.1 Approve the original Smith product direction, in-process lifetime,
  extension boundary, and explicit non-goals.
- [x] 0.2 Select `MIT OR Apache-2.0` for Smith and confirm that completed Smith
  code is original rather than copied from a donor repository.
- [x] 0.3 Re-approve the revised proposal, ownership boundary, configuration
  mapping, staged provider scope, and runtime-integration delta.
- [ ] 0.4 Replace committed sibling path dependencies with an exact released
  semantic version or Git revision once Agent Runtime has a real remote and
  release reference.
- [ ] 0.5 Add a git-ignored, uncommitted Cargo `[patch]` workflow for sibling
  runtime development and prove removing it restores the pinned source without
  source edits.
- [ ] 0.6 Recheck public package names immediately before publication; retain
  `smith` as the binary name.

## 1. Existing Slice and 0.1 Compatibility

- [x] 1.1 Create the Rust 2024 workspace with `smith-host`, `smith-tools`,
  `smith-tui`, and `smith-cli`.
- [x] 1.2 Implement interactive approval and the canonical project workspace
  boundary over shared core traits.
- [x] 1.3 Implement Smith's read, list, search, edit, and shell tools over the
  shared `Tool` contract with bounded output and process-group cleanup.
- [x] 1.4 Implement the pure TUI state/reducer/renderer and the terminal host
  loop over shared runtime events.
- [x] 1.5 Prove the original fake-provider slice end to end and keep root
  `DESIGN.md` as the visual contract.
- [x] 1.6 Confirm Agent Runtime's `consumer_smith` conformance fixture passes
  against the current shared API.
- [x] 1.7 Add an explicit fake `ResolvedModelProfile` to every Smith runtime
  fixture. The production binary now receives its immutable model profile from
  resolved configuration/catalog layers instead of carrying a development
  profile of its own.
- [x] 1.8 Centralize the fake model profile/runtime fixture so tests cannot
  drift on limits, context policy, or revision identity.
- [x] 1.9 Pass `cargo test --workspace`, formatting, and Clippy with warnings
  denied before beginning real-provider work.
- [ ] 1.10 Add macOS/Linux CI plus dependency/license checks, and run the
  MSRV and architecture-boundary gates there.
- [x] 1.11 Prove the declared Rust 1.88 MSRV locally across all targets,
  workspace tests, and warning-denied Clippy; retain a test for the
  full-facade architecture boundary, and keep the locked dependency graph
  clean under the current RustSec advisory database.
- [x] 1.12 Add a fail-closed macOS/Linux/current-stable workflow, a shared
  local gate, four-target advisory/license/source policy, and a manifest-level
  architecture test; run the exact Rust 1.88 gate locally on macOS and isolated
  Linux/aarch64. Hosted workflow execution remains part of 1.10 and awaits a
  real runtime repository plus exact revision.
- [x] 1.13 Harden edit publication against repository-planted temporary-file
  symlinks by creating a randomized sibling file exclusively, writing through
  its already-open handle, and retaining an adversarial workspace-escape test.

## 2. Resolved Smith Configuration

- [x] 2.1 Define typed configuration for profiles, providers, models, context,
  loop limits, persistence, approval, background work, and secret references.
- [x] 2.2 Implement deterministic precedence: defaults, user config, project
  config, project-local config, selected profile, `SMITH_*`, CLI, then explicit
  session overrides.
- [x] 2.3 Retain per-field source provenance and implement
  `smith config explain <key>`.
- [x] 2.4 Reject unknown keys, invalid types, same-layer ambiguity, plaintext
  secrets, incompatible provider options, and unusable profile references with
  file/key/source diagnostics.
- [x] 2.5 Discover `.smith/` without treating declarative project settings as
  executable authority.
- [x] 2.6 Implement canonical-path and executable-content-hash trust for hooks,
  extensions, credential helpers, and shell-valued settings.
- [ ] 2.7 Implement user-scoped credential references with macOS Keychain and
  Linux Secret Service backends; keep encrypted-file fallback explicit and
  externally keyed.
- [x] 2.8 Add table-driven tests for every precedence pair, provenance output,
  unknown-key suggestion, trust invalidation, and secret redaction.
- [x] 2.9 Refuse repository-controlled `allow-all` and `auto_approve` values so
  opening a project cannot silently grant its model shell or write authority;
  likewise refuse repository redirection/disablement of user-scoped session
  persistence, and require those choices from user configuration or an
  explicit CLI layer.
- [x] 2.10 Apply the repository persistence boundary to both session startup
  and session listing; register resolved provider credentials with one
  non-printing redactor shared by event journals and persisted snapshots.

## 3. Shared Runtime Composition and Providers

- [x] 3.1 Add one headless Smith runtime factory—crate or module—that accepts a
  resolved run request and injected host adapters.
- [x] 3.2 Map provider/model selection, explicit profiles or
  `LayeredModelCatalog`, versioned `ContextPolicy`, retry/loop limits, prompt,
  tools, approval, workspace, stores, observers, clock, and shutdown policy
  into `RuntimeBuilder`.
- [x] 3.3 Fail before terminal entry and provider I/O when provider selection,
  credentials, model limits, context reserves, or required host policy cannot
  resolve.
- [x] 3.4 Compose model metadata with runtime precedence: explicit/session,
  provider-local, embedded, then validated cached remote; expose field
  provenance in configuration diagnostics.
- [x] 3.5 Implement a production streaming HTTP transport for the shared
  OpenAI-compatible adapter with cancellation, deadlines, bounded bodies,
  status classification, and redacted debug/error surfaces.
- [x] 3.6 Resolve a configured credential reference to shared `Secret` only at
  provider construction and prove it never appears in logs, events, request
  debug output, persisted state, or tool-visible errors.
- [x] 3.7 Make the configured shared OpenAI-compatible adapter the first
  production provider while retaining fake injection for tests/development.
- [x] 3.8 Add offline transport fixtures for text, fragmented tools, usage,
  retry, cancellation, malformed SSE, and authorization redaction.
- [x] 3.9 Implement safe-boundary provider/model switching by explicit
  save/rebuild/resume until the shared facade exposes an equivalent immutable
  reconfiguration operation.
- [ ] 3.10 Add OpenAI Responses and Anthropic only after compatible adapters
  land in a released Agent Runtime; do not implement duplicate Smith provider
  mechanism.
- [x] 3.11 Keep the full `agent-runtime` facade behind `smith-runtime` for
  production composition; expose the canonical session handle through that
  boundary so `smith-cli` does not depend on the facade directly.
- [x] 3.12 Opt the shared factory into Agent Runtime's named
  `legacy_approval_authority()` migration aid now that composed authorization
  is live for tool invocation; retain Smith's mandatory interactive/headless
  approval behavior without claiming broader upstream enforcement.
- [x] 3.13 Add an opt-in, provider-generic live smoke test that drives the
  installed Smith process through a spend-capped streaming `read`-tool turn,
  continuation request, provider-reported usage, redaction check, and clean
  shutdown. Document the Z.AI Coding Plan invocation without persisting a key.

## 4. Sessions, Events, and Usage

- [x] 4.1 Implement Smith's `SessionStore` over versioned runtime
  `SessionSnapshot`s, preserving identity counters, usage, and turn manifests.
- [x] 4.2 Implement an `EventObserver` JSONL journal under user state with
  complete-record writes, bounded records, schema identity, and redaction.
- [x] 4.3 Prove clean create/list/resume through explicit
  `StartSession::with_id` and rebuild the TUI transcript from shared history.
- [ ] 4.4 Prove crash-tail recovery by replaying the complete journal into a
  compatible snapshot or add a shared-runtime checkpoint hook before claiming
  last-turn crash durability.
- [ ] 4.5 Store large bounded sidecars by content hash only when the shared
  canonical event/snapshot retains sufficient attribution.
- [ ] 4.6 Mark formerly active Smith monitors/children interrupted on resume
  without restarting them.
- [ ] 4.7 Consume shared disjoint usage counters and attempt events unchanged;
  add Smith price references and exact/estimated/unknown cost labels without
  treating unknown as zero.
- [ ] 4.8 Add replay fixtures for legacy snapshots, run manifests, provider
  switches, incomplete JSONL tails, usage, and non-equivalent revision errors.

## 5. Client Surfaces

- [x] 5.1 Replace `smith-cli`'s hard-coded fake composition with resolved
  configuration and the shared Smith runtime factory.
- [x] 5.2 Keep terminal setup after configuration/runtime/session success so a
  startup failure cannot leave the shell in raw or alternate-screen state.
- [x] 5.3 Add provider/model/profile selection and provenance-aware model/context
  status to the TUI.
- [x] 5.4 Show edit arguments as a reviewable diff in the approval modal.
- [ ] 5.5 Stream long-running shell output through bounded Smith progress
  events without changing canonical tool-result semantics.
- [x] 5.6 Implement `smith -p` with argument/stdin prompts, project/session
  selection, provider/model selection, text output, and the same runtime
  factory as the TUI.
- [x] 5.7 Add versioned `json` and newline-delimited `stream-json`; reserve
  stdout for machine output and stderr for diagnostics/progress.
- [ ] 5.8 Fail closed for headless approval and implement explicit
  `error|wait|stop` background-exit policy.

## 6. Context Lifetime, Monitor, and Safe-Boundary Orchestration

- [x] 6.1 Map configured context reserves, capability budget, confidence guard,
  compaction watermarks, cache capability, and revision identities into shared
  context mechanism. `smith-runtime::factory` now derives absolute compaction
  thresholds from the enforced input budget, records a versioned policy, and
  attaches Agent Runtime's `SemanticCompactor` on the one shared composition
  path; proactive inactivity orchestration remains task 6.2.
- [ ] 6.2 Implement Smith's meaningful-inactivity policy and evidence-based
  cache status over shared planning/cache/usage events.
- [ ] 6.3 Add adapter-gated ephemeral keepalive with bounded response, jitter,
  transcript exclusion, spend accounting, and no second rebuild after a miss.
- [ ] 6.4 Implement command/WebSocket monitor sources, bounded spooling,
  process ownership, batching, flood protection, and terminal events.
- [ ] 6.5 Implement a bounded Smith inbox that displays notifications
  immediately but presents them to the model only through a new safe execution
  phase, never by mutating an in-flight stream.
- [ ] 6.6 Add deterministic clock and opt-in, spend-capped live cache tests.

## 7. Direct Children and Extensions

- [x] 7.1 Implement root-only direct child lifecycle operations with depth-one
  authorization, concurrency/turn/token/deadline limits, and explicit
  provider/model/workspace/approval policy. Built on the shared runtime's
  `agent-delegation` contract (`add-agent-delegation-runtime-2026-07-26`):
  `smith-runtime/src/delegation.rs` (the `agent` tool, `DelegationAuthority`
  covering `agent.delegate` through the approval surface). Two declared
  narrowings remain structured errors, not silent gaps: explicit child
  provider/model routing and isolated-worktree workspace creation.
- [x] 7.2 Reuse the Smith runtime factory for children with scoped shared
  registry/tool views; remove child-management abilities from child views
  (`SmithChildFactory`; the coordinator strips the `agent` tool and the
  `Child` surface never composes one).
- [x] 7.3 Route progress/results through the parent safe-boundary inbox and
  stop children with the parent/process (`wire_delegation` injects completed
  results must-deliver via `SessionHandle::inject`; the runtime stops
  children on parent shutdown and never restarts them on resume).
- [ ] 7.4 Specify and fixture the bounded framed-JSON extension protocol and
  optional asynchronous TypeScript host.
- [ ] 7.5 Register tools, providers, skills, MCP, and other abilities through
  shared runtime contracts/registries rather than Smith-local parallel traits.
- [ ] 7.6 Apply project hash trust, declared grants, deterministic ordering,
  timeouts, crash isolation, and absent-Node behavior.

## 8. Security, Forge Boundary, and Release

- [x] 8.1 Keep the committed Agent Runtime 0.1 approval/workspace/secret
  contracts as Smith's current product-policy boundary. Where the current
  runtime now enforces composed authorization for tool invocation, use its
  explicit compatibility authority and do not assume unfinished provider,
  activation, or sub-agent enforcement is live.
- [ ] 8.2 After that upstream security change is approved, implemented, and
  released, open a coordinated Smith migration for mandatory permissions,
  authorization-before-approval, provider transport policy, and optional WASM
  sandbox integration.
- [ ] 8.3 Add a Smith-owned embedding example and contract tests with injected
  provider, tools, approval, workspace, stores, configuration, and event
  consumers.
- [ ] 8.4 Document the one-way Forge adapter without modifying Open Forge;
  require a separate Forge proposal and opt-in rollout.
- [ ] 8.5 Run audits/fuzzing for config, SSE, JSONL, tool arguments, extension
  frames, paths, process cleanup, and secret handling.
- [ ] 8.6 Publish only after a real runtime release/revision is pinned, current
  and shared conformance gates pass, and macOS/Linux shutdown/cleanup tests are
  green.
