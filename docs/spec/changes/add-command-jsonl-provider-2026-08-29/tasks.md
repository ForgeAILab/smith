---
created_at: 2026-08-30T03:17:16Z
updated_at: 2026-09-05T04:28:00Z
completed_at:
---

## 0. Approval and dependency gate

- [x] 0.1 Approve this Smith proposal and its revision-1 protocol boundary.
- [x] 0.2 Confirm the approved Agent Runtime
  `add-command-provider-framework-2026-08-29` implementation and Smith
  consumer-conformance gate pass in the sibling checkout.
- [x] 0.3 During coordinated development, add the provider leaf package to
  Smith's documented ignored sibling patch and enable `command-provider`
  through a temporary exact-revision `agent-runtime-provider` dependency in
  `smith-runtime`. Do not patch the sibling facade while it contains unrelated
  unfinished harness changes; replace this temporary seam with the facade
  feature at the immutable-revision gate.
- [x] 0.4 Re-read the active Gemini provider change before editing shared
  provider config/factory seams and preserve its dedicated-model and native
  adapter work.

## 1. Command-provider configuration

- [x] 1.1 Add strict `CommandProviderSection` and resolved types for absolute
  executable, fixed args, workspace-or-absolute cwd, and explicit environment
  values under `[providers.<name>.command]`.
- [x] 1.2 Flatten every command field into the existing provenance chain and
  expose it through configuration explanation without rendering environment
  literals or resolved credential values.
- [x] 1.3 Validate the `command-jsonl` option matrix: required command table and
  executable; no HTTP endpoint, provider credential/pool/rotation, headers, or
  response normalization; no command table on another adapter kind.
- [x] 1.4 Reject relative/unresolvable executable and cwd values, NUL or
  oversized args/environment, duplicate/conflicting environment declarations,
  and an empty or malformed process declaration before credential or process
  I/O.
- [x] 1.5 Enforce source authority: every winning process-bearing field must be
  user-controlled; project layers may select an existing user provider/model
  but cannot define or override `kind = "command-jsonl"` or `command.*`.
- [x] 1.6 Resolve command environment credential references through the
  existing bounded credential resolver and register every resulting secret
  with persistence redaction before provider construction.
- [x] 1.7 Add parsing, round-trip, precedence, provenance, unknown-field,
  option-matrix, project-authority, secret-redaction, and project-selection
  tests.

## 2. Revision-1 protocol adapter

- [x] 2.1 Add Smith-owned, `deny_unknown_fields`, versioned probe, request, and
  stdout-frame protocol types without serializing `ProviderRequest` or
  `ProviderStreamEvent` wholesale.
- [x] 2.2 Implement the fixed revision-1 provider-local model record: text-only
  streaming, tools and usage enabled, reasoning/structured-output/cache/
  continuation disabled, custom local-command authentication.
- [x] 2.3 Implement `CommandAdapter::probe` and `parse_probe` for
  `--smith-provider-probe <model>`, including exact protocol/revision/model
  checks and bounded redaction-safe implementation metadata.
- [x] 2.4 Implement attempt preparation for `--smith-provider-attempt`, strict
  capability/content validation, typed purpose mapping, and one newline-
  terminated stdin request whose sensitive content never enters argv or Debug.
- [x] 2.5 Implement the attempt-local JSONL decoder for text, tool fragments,
  disjoint usage, finish, and classified error frames; reject unknown or
  unsupported frames, missing usage, malformed fragments, and protocol drift.
- [x] 2.6 Add protocol golden fixtures and unit tests for every accepted frame,
  every rejected revision-1 feature, malformed/oversized data, terminal
  ordering, redacted diagnostics, and lossless tool request/result mapping.

## 3. Factory and runtime composition

- [x] 3.1 Add `command-jsonl` to `AVAILABLE_ADAPTER_KINDS` and the private
  adapter mapping without changing the open provider-kind configuration
  contract.
- [x] 3.2 Build `CommandProcessConfig` only from fully resolved and authorized
  values, clear ambient environment through the framework, use direct argv,
  and map framework configuration errors to bounded Smith diagnostics.
- [x] 3.3 Run the explicit command preflight before terminal entry or runtime
  construction; map absence, timeout, output bounds, unsuccessful exit,
  malformed output, and incompatibility without exposing stdout/stderr or
  secrets.
- [x] 3.4 Construct `CommandProvider` as the ordinary `Arc<dyn Provider>`, add
  its provider-local model record before catalog resolution, and leave the
  existing runtime/tool/MCP/approval/retry/event/persistence composition path
  unchanged.
- [x] 3.5 Skip inapplicable HTTP transport, renewable credential, credential-
  pool, reasoning-dialect, and provider-cache wrappers while preserving shared
  response policy, deadlines, summaries, and safe provider switching.
- [x] 3.6 Keep injected-provider tests and factory preflight deterministic: an
  unavailable configured adapter still fails, while command fixtures use an
  explicit test executable rather than ambient PATH or user state.

## 4. Integration and surface parity tests

- [x] 4.1 Add a deterministic executable fixture implementing probe and
  attempt modes for success, tool continuation, retryable error, malformed
  output, timeout, cancellation, stderr flood, and descendant-process cleanup.
- [x] 4.2 Prove one text turn reaches the same canonical transcript, usage,
  journal, and terminal result through TUI and `smith -p` projections.
- [x] 4.3 Prove a connected MCP tool schema reaches the command request, its
  emitted tool call runs only through Smith approval/authority, and the next
  fresh process receives the canonical tool result.
- [x] 4.4 Prove retryable failure creates a second visible runtime attempt and
  second process, with no hidden bridge retry or duplicate committed output.
- [x] 4.5 Prove cancellation, deadline, dropped stream, malformed stdout,
  missing/duplicate terminal, non-success exit, and output limits leave no
  detached process tree and produce one classified outcome.
- [x] 4.6 Prove the child receives no undeclared ambient environment, argv and
  diagnostics contain no prompt/secret, stderr is never rendered or persisted,
  and a project cannot alter an executable invocation selected by user config.
- [x] 4.7 Prove native-provider fixtures and projects without command providers
  remain behaviorally unchanged.

## 5. Documentation and release gates

- [x] 5.1 Document the `command-jsonl` configuration, protocol revision 1,
  executable trust/data-egress warning, environment references, bridge-author
  contract, and native-provider parity.
- [x] 5.2 Document that Codex app-server and similar coding CLIs are autonomous
  agent backends, not compatible command providers, and link their future work
  to a separate proposal.
- [x] 5.3 Run formatting, all-target Clippy with warnings denied, focused
  config/protocol/factory tests, workspace and all-features tests, TUI/headless
  golden fixtures, and the Agent Runtime Smith consumer-conformance gate.
- [ ] 5.4 Pin the immutable Agent Runtime revision containing the command
  framework, remove the temporary direct provider dependency in favor of the
  facade's `command-provider` feature, then run the supported macOS and Linux
  process tests and verify the exact Git dependency builds without sibling
  checkouts or an uncommitted Cargo patch.

## 6. Release review blockers

- [x] 6.1 Preserve the legacy snapshot fixture, review runtime-main LCM API
  incompatibility, and test legacy load/re-save without rewriting audit history.
- [x] 6.2 Replace obsolete musl rejection with offline installation checks for
  x86_64 and aarch64 portable Linux artifacts; run npm platform tests.
- [x] 6.3 Verify the published exact runtime revision and Smith's entire
  workspace in a clean checkout with no sibling Cargo patches.

Verification (2026-09-05): Smith resolves every runtime package from published
commit `93f476bc6e6cbb3e73deef3dde065d132e425d34` on
`fix/smith-command-provider-compat`. The direct leaf dependency is removed;
`smith-runtime` enables the facade's `command-provider` feature. The ignored
sibling patch is disabled and backed up in Git's local metadata. A separate
checkout without Cargo patches passes 1,500 workspace tests (6 ignored),
all-target/all-feature Clippy with warnings denied, formatting, dependency
policy, and a Rust 1.88 all-target `smith-runtime` check. Smith packages declare
no optional features, so the all-feature workspace suite also exercises the
normal workspace feature set. Offline installer tests pass for both Linux
architectures, and all 4 npm tests pass.

The pinned runtime passes 1,177 all-feature workspace tests (1 ignored),
including Smith consumer conformance, all-target/all-feature warning-denied
Clippy, formatting, dependency policy, and the Rust 1.86 facade/provider check.
Task 5.4 remains open only for real Linux process execution; Linux and live
provider integration were not run on this macOS host. See `verification.md`.

Verification (2026-09-05, release prep): the pinned revision is now immutable
by reference as well as by digest. `93f476bc6e6cbb3e73deef3dde065d132e425d34`
carries the annotated upstream tag `smith-baseline-v0.1.0`, so `--locked`
keeps resolving after `fix/smith-command-provider-compat` is retired; before
the tag the commit was reachable from that branch alone. Upstream `main`
already carries the same command-provider work on top of lossless context
memory, so nothing remains to merge; moving the pin forward is the separately
scoped `SemanticSummary*` -> `Lcm*` migration, measured here at 83 references
across 11 Smith files with no crate-public legacy import path.

A detached worktree at the release commit, with no sibling checkout and no
Cargo patch, builds `cargo build -p smith-cli --release --locked` and reports
`smith 0.1.0`. The workspace passes 1,502 all-feature tests (6 ignored),
warning-denied Clippy, formatting, and dependency policy; installer and npm
tests pass. Real Linux process execution is still the one open clause, and
CI's `Linux · Rust 1.88` leg covers it on push.
