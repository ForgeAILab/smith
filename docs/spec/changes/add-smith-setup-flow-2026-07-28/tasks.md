---
created_at: 2026-07-28T19:10:01Z
updated_at: 2026-07-28T21:22:22Z
completed_at: 2026-07-28T21:22:22Z
---

## 1. Readiness and setup data

- [x] 1.1 Add a typed `Ready | Unconfigured | Invalid` inspection path in
  `smith-config`, keeping `Ready` equivalent to the existing resolver and
  classifying partial/malformed intent as invalid.
- [x] 1.2 Add table-driven tests for empty discovery, complete configuration,
  partial provider/model declarations, malformed files, environment/CLI
  intent, and removal of a previously usable setup.
- [x] 1.3 Define Smith-owned provider setup descriptors filtered by the
  pinned runtime's available adapters, including the GLM/Z.AI quick-start
  values and reasoning-only response policy, the generic OpenAI-compatible
  path, and versioned trusted model records.
- [x] 1.4 Implement a comment-preserving, collision-aware user-config edit
  transaction with non-secret preview, restrictive directory/file
  permissions, same-directory atomic publication, and rollback coverage.
- [x] 1.5 Add the typed
  `[providers.<name>.response].reasoning_only` option, layered
  resolution/provenance, adapter compatibility validation, and configuration
  tests for text, reasoning, omitted, invalid, and incompatible values.

## 2. Credential enrollment

- [x] 2.1 Add an injectable credential enrollment interface separate from the
  runtime's read-only resolver, with OS keychain/Secret Service store and
  cleanup/restore operations.
- [x] 2.2 Support masked API-key enrollment to
  `keychain:smith/<provider>` and an environment-reference alternative without
  copying or persisting the environment value.
- [x] 2.3 Add fake-backend tests proving secrets never appear in config
  previews, errors, debug output, render state, journals, or failed transaction
  artifacts.
- [x] 2.4 Handle unavailable/denied credential services by returning to the
  authentication step with the environment-reference option; never write a
  plaintext fallback.
- [x] 2.5 Bound read-only credential lookup so a platform unlock/access prompt
  cannot hang startup or setup preflight indefinitely, with an actionable
  environment-reference recovery diagnostic.

## 3. Setup surface and orchestration

- [x] 3.1 Add `smith setup`, `smith setup add-provider`, and
  `smith setup add-model --provider <name>` parsing, and route only genuinely
  unconfigured interactive `smith` launches into setup before `start_host`.
- [x] 3.2 Implement a pure keyboard-first setup reducer and renderer for
  action, provider, authentication, model, default selection, review, busy,
  and error states, including Back/Cancel and no-color/reduced-motion behavior.
- [x] 3.3 Add a guarded setup terminal loop that restores the terminal on
  completion, cancellation, errors, signals, and panics without constructing a
  host session.
- [x] 3.4 Connect reviewed setup effects to credential enrollment and user
  config persistence, then run complete configuration, credential, model,
  workspace, and runtime-factory preflight before starting the normal TUI.
- [x] 3.5 Keep headless, piped, machine-output, session-list, and config-explain
  paths non-interactive and non-mutating, with stable setup-required
  diagnostics and exit behavior.
- [x] 3.6 Add a redaction-safe provider-stream decorator that promotes
  non-redacted reasoning-only successful output to text only for providers
  configured for that policy, while preserving reasoning followed by ordinary
  text or tool calls.

## 4. Selection inventories and pickers

- [x] 4.1 Add a provenance-aware local inventory API for profiles, providers,
  and valid provider/model pairs, with deterministic ordering, active markers,
  ambiguity handling, and no credential or network access.
- [x] 4.2 Extend versioned session-list metadata with a bounded local user
  preview, turn count, and provider/model when known, preserving selection of
  older snapshots with labelled unknown fields.
- [x] 4.3 Add a reusable typed resource-picker reducer and renderer with
  filtering, scrolling, active/disabled states, Enter/Escape behavior, empty
  guidance, and narrow/no-color/reduced-motion coverage.
- [x] 4.4 Make `/model`, `/provider`, `/profile`, and `/resume` arguments
  optional in the command registry; route omitted arguments to the resource
  picker and keep explicit arguments as validated shortcuts.
- [x] 4.5 Apply model choices as atomic provider/model pairs, cascade provider
  choices to a filtered model picker when necessary, apply profiles coherently,
  and resume selected project sessions without provider spend.
- [x] 4.6 Support interactive `smith --resume` without an ID through the
  pre-host session picker, while keeping explicit IDs unchanged and refusing
  no-ID headless/machine-output use with a stable `smith sessions list` hint.
- [x] 4.7 Add parser, reducer, render, ambiguity, cancellation, empty-state,
  cross-provider model, backward-compatible session, and no-provider-spend
  tests for every selection path.

## 5. Verification and documentation

- [x] 5.1 Add setup reducer/render tests across narrow/wide terminals,
  masked-input tests, and cancellation/retry tests.
- [x] 5.2 Add process/pseudo-terminal tests for fresh interactive setup,
  GLM quick start, adding a provider/model, adding a second provider's model,
  successful transition to a deterministic test host, cancellation without
  writes, invalid-config refusal, headless refusal, picker selection, and
  terminal restoration.
- [x] 5.3 Add replay-provider coverage for GLM reasoning-only promotion,
  reasoning followed by ordinary text, reasoning followed by a tool call,
  redacted reasoning, and an unconfigured OpenRouter-style text stream; keep
  paid live-provider verification ignored and opt-in.
- [x] 5.4 Update `README.md`, CLI help, and `DESIGN.md` with first-run,
  `smith setup`, credential, model-limit, selector, resume, and recovery
  behavior.
- [x] 5.5 Run strict spec validation, `cargo fmt --check`,
  warning-denied Clippy, focused tests, and `cargo test --workspace`.
