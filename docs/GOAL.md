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

The first public-release gates are complete. Smith pins one immutable upstream
runtime revision, the hosted macOS/Linux matrix and dependency policy pass
against it, and the four-platform release workflow has produced a successful
release candidate. The remaining release action is publishing the stable
`v0.0.1` tag from a green `main` revision.

## Latest compatibility evidence

On 2026-08-09, hosted CI passed from clean `main` commit `0aff697` against
Agent Runtime revision `0a07231649d81ccb40f2395a9924f8bd6027baf9`:

- the complete Smith workspace and Agent Runtime consumer-conformance gates
  passed on macOS and Linux with Rust 1.88;
- the Linux stable-toolchain leg passed, including advisory, license, ban, and
  source policy checks;
- the npm bootstrapper syntax and package dry-run gates passed; and
- formatting and all-target warning-denied Clippy passed on the pinned
  toolchain.

The `v0.0.1-rc.3` release also proved the release workflow can build and publish
x86_64 and ARM64 archives for macOS and glibc Linux, generate checksums, create
a GitHub prerelease, and publish the matching npm prerelease.

## Deliberately deferred

Static musl release binaries, deeper-than-one agent nesting, a concrete monitor
executor, and a bidirectional headless interaction protocol stay out of this
release goal. They should build on the released runtime
capability/delegation/security contracts rather than create temporary
Smith-local mechanisms.
