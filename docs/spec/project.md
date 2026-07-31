# Project Overview

## Identity

- Product and command name: Smith
- Repository folder: `tui`
- Planned library crate namespace: `smith-*`
- Planned installable package: `smith-cli`, exposing the `smith` binary
- Intended role: terminal-first Smith host over the shared `agent-runtime`, plus
  a reusable Smith policy/composition layer for future Forge integration
- Status: the first local vertical slice now runs both `smith` and `smith -p`
  through `smith-runtime`, including resolved configuration, production
  OpenAI-compatible streaming, bounded coding tools, approvals, and resumable
  persistence. Release remains blocked on an immutable Agent Runtime source
  and cross-platform release gates; the sibling runtime checkout is active
  co-development state rather than a release pin.

## Current Delivery Goal

Deliver Smith's first trustworthy end-to-end coding-agent release: compose a
pinned compatible `agent-runtime` only through `smith-runtime`; resolve the
provider, model limits, credentials, workspace, approval policy, and
persistence before entering the terminal; run real streaming
OpenAI-compatible turns with bounded read, list, search, edit, and shell tools
plus redaction-safe interactive approvals; persist and resume sessions; and
expose equivalent behavior through the TUI and `smith -p`.

The release is complete when:

1. `smith-cli` no longer constructs `RuntimeBuilder` directly and both
   interactive and headless runs use the same `smith-runtime` factory.
2. Startup fails before terminal or provider I/O when configuration,
   credentials, model limits, workspace, approval, or persistence cannot be
   resolved.
3. A configured OpenAI-compatible provider can complete a streaming,
   tool-assisted turn, while the deterministic fake remains the test seam.
4. Tool arguments remain redaction-safe in canonical events and journals,
   while approval prompts still show the exact bounded action the user is
   deciding.
5. A session can be created, cleanly saved, listed, resumed, and rendered back
   into the TUI with history and usage intact.
6. `smith -p` and the TUI produce equivalent canonical runtime behavior, and
   the Smith workspace plus Agent Runtime's Smith consumer-conformance suite
   pass formatting, Clippy, tests, and macOS/Linux CI.

Monitors, child-agent UX, extensions/MCP, cache keepalive, and Forge integration
remain roadmap work. They start only after this vertical slice is green and
the corresponding capability, delegation, and security contracts are released
by Agent Runtime.

## Tech Stack

- Language: Rust 2024 edition
- Async runtime: Tokio
- HTTP and streaming: Smith-owned production transport injected into shared
  provider adapters
- Serialization: Serde with versioned JSON-compatible protocol types
- Session persistence: append-only JSON Lines logs under user state
- TUI: Ratatui and Crossterm
- Extensions: language-neutral subprocess protocol, optional first-party
  TypeScript host, MCP, and trusted compile-time Rust registration
- Future sandbox tier: WebAssembly Component Model/WASI after the extension
  protocol is stable
- Package manager: Cargo workspace

## Conventions

- Architecture: Smith-owned configuration and product policy compose the
  host-neutral `agent-runtime` facade in process for the TUI, non-interactive
  CLI, tests, or Forge
- Public contracts: reuse shared runtime types, registries, events, usage, and
  provider/tool traits; Smith adds no parallel mechanism contract
- Configuration: deterministic layered TOML with source provenance and an
  explicit project trust boundary
- Persistence: resumable transcripts, compaction records, and usage events;
  monitors and child agents are intentionally ephemeral
- Security: least privilege, explicit approvals, redacted secrets, cancellable
  work, auditable side effects
- Code style: `cargo fmt`; Clippy warnings are errors
- Testing: unit, contract, deterministic fake-provider, replay, and end-to-end
  tests; live-provider tests are opt-in and spend-capped
- Release/deploy: pre-1.0 semver, reproducible Cargo builds, signed release
  artifacts later

## Product Principles

1. Smith is a policy-bearing host over `agent-runtime`; the TUI and `-p` mode
   share one Smith composition path.
2. “All providers” means an open provider SDK plus strong first-party adapters,
   not a permanently closed enum.
3. Session history is resumable, but background work belongs to the current
   Smith process and ends with it.
4. Cache state is provider-observed. Smith never presents a guessed TTL as a
   verified cache hit.
5. Every token and cost number records whether it was provider-reported,
   derived, estimated, or unknown.
6. Project configuration cannot execute code until the project is trusted.
7. Extensions get TypeScript ergonomics through a separate process, not an
   embedded JavaScript VM in the Rust core.
8. Forge integration must not create a dependency from Smith back into Forge.
