# Smith release goal

## North star

Ship a trustworthy terminal coding agent whose interactive TUI and `smith -p`
are two presentations of the same resolved, persisted Agent Runtime session.
The first release is useful when it can inspect and change a real project with
a real OpenAI-compatible provider, while making authority, provenance, limits,
and failures visible instead of guessing.

## Definition of done for the first vertical slice

- Smith consumes one exact released Agent Runtime source and composes it only
  through `smith-runtime`.
- Configuration, provider/model limits, credential references, the canonical
  project workspace, approval policy, and persistence all resolve before the
  terminal enters raw or alternate-screen mode.
- The OpenAI-compatible adapter streams text, fragmented tool calls, usage, and
  failures through the shared runtime vocabulary.
- `read`, `list`, `search`, `edit`, and `shell` are bounded and workspace
  confined. Root sessions compose task questions, todos, and depth-one
  delegation through the ability registry. Authority-bearing calls prepare an
  exact resource/permission set, are approved interactively or fail closed
  headlessly, and keep protected argument values out of ordinary events and
  machine output.
- Sessions create, list, save, and resume with canonical history, identity,
  usage, manifests, ordered redacted event journals, separately protected
  in-flight checkpoints, and session-authorized artifacts.
- `smith` and `smith -p` select the same project/profile/provider/model and
  produce equivalent runtime behavior. Headless text, JSON, and JSON Lines
  contracts have stable exit semantics.
- Smith's workspace tests, warning-denied Clippy gate, and Agent Runtime's
  Smith consumer-conformance suite pass against the exact release source on
  macOS and Linux.

## Runtime co-development boundary

During local co-development, the workspace uses the adjacent
`../agent-runtime` checkout with an exact `=0.1.0` package requirement. Smith
does not edit that checkout, opt into unfinished event payloads, or duplicate
runtime mechanisms. Any runtime source change must pass Smith's adapter,
composition, persistence, CLI, and consumer-conformance gates again.

A distributable Smith release must replace the sibling path with an immutable
semantic version or Git revision. The local path is not a release pin.

## Current vertical-slice status

Implemented:

- one configuration-to-runtime factory and one shared session host;
- credential-at-construction and pre-terminal failure boundaries;
- production streaming HTTP/OpenAI-compatible composition;
- session-scoped capability activation, versioned prompt sections, todos,
  trusted skills, bounded memory, artifact offloading, semantic summaries, and
  safe-boundary child delivery;
- invocation-specific prepared authority plus separate interactive approval
  and questionnaire channels;
- completed-turn snapshots, ordered redacted journals, protected checkpoint
  recovery, and explicit create/list/resume;
- TUI history restoration, provider/model/profile switching at an idle
  save/rebuild/resume boundary, and provenance-aware usage/context status;
- attempt-scoped speculative TUI reduction with live/journal replay
  equivalence;
- `smith -p` argument/stdin prompts, selection flags, schema-v2
  text/JSON/JSONL projections, and structured approval/interaction-required
  status.

The remaining release gates are an immutable upstream runtime reference,
execution of the configured macOS/Linux release workflow against that
reference, platform visual/dependency evidence, and the still-open release
checklist items in
`docs/spec/changes/integrate-stable-session-harness-2026-07-31/`.

## Latest compatibility evidence

On 2026-07-25, the complete local gate passed on macOS and isolated
Linux/aarch64 against clean Agent Runtime revision
`4e052f2eeb488367c5932a8aa511d5c3880dbdbe`:

- 380 Smith workspace tests passed on macOS and 381 passed on Linux, where the
  additional case covers non-UTF-8 project paths;
- all 57 Agent Runtime testkit and consumer-conformance tests passed on both
  hosts;
- all-target warning-denied Clippy passed on Rust 1.88 on both hosts and on the
  current macOS toolchain;
- the four-target dependency policy passed its advisory, license, ban, and
  source checks;
- formatting and strict change-spec validation passed; and
- a manifest-level architecture test proves that the full `agent-runtime`
  facade is a production dependency only of `smith-runtime`.

That revision is compatibility evidence, not a release pin: the runtime
checkout still has no declared remote or published immutable source. The
fail-closed CI workflow is configured and linted, but cannot provide
hosted evidence until that source exists and the required repository and
revision variables are set.

The packaging audit fails at that same boundary rather than silently bundling
the sibling checkout: Cargo cannot prepare `smith-config` because
`agent-runtime-core` is not published in the configured registry. A real
runtime registry release or immutable Git source is therefore required before
Smith can package or publish.

## Deliberately deferred

A concrete monitor executor, nested agents, extension/MCP hosting,
prompt-cache keepalive, Forge integration, and a bidirectional headless
interaction protocol stay out of this release goal. They should build on the
released runtime capability/delegation/security contracts rather than create
temporary Smith-local mechanisms.
