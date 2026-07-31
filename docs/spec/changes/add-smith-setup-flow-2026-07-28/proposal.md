---
created_at: 2026-07-28T19:10:01Z
updated_at: 2026-07-28T21:22:22Z
---

## Why

A fresh Smith installation cannot reach the TUI: complete configuration
resolution runs first and fails on the missing provider or model. The user must
currently discover and hand-author the provider, credential reference, model,
and enforceable limits that Smith needs before the product can guide them.
After configuration, Smith still asks users to type opaque model, provider,
profile, and session identifiers even though those choices already exist
locally and can be presented safely.

## What Changes

- Distinguish a genuinely unconfigured installation from malformed or partial
  configuration without weakening Smith's existing fail-closed resolver.
- Automatically open a guided setup surface for an unconfigured interactive
  `smith` launch, and expose the same flow explicitly as `smith setup`.
- Guide the user through provider, authentication, model, and final review.
  Setup choices come from Smith-owned descriptors filtered by the adapters
  actually available in the pinned runtime; custom OpenAI-compatible endpoints
  remain supported.
- Ship a GLM quick-start descriptor matching Smith's existing live-provider
  contract: Z.AI Coding Plan, `glm-4.7`, its documented endpoint and enforceable
  limits, a conservative per-request output cap, and a typed response
  compatibility policy that treats a reasoning-only completion as visible
  assistant content without disabling GLM thinking.
- Make setup reusable rather than one-shot: `smith setup add-provider` adds a
  provider and first model, while `smith setup add-model --provider <name>`
  adds another model/limits record to any configured provider. Interactive
  setup offers the same actions and can select the resulting default profile.
- Make runtime selection discoverable: `/model`, `/provider`, `/profile`, and
  `/resume` without an argument open searchable local pickers, while their
  explicit argument forms remain direct shortcuts.
- List models as valid provider/model pairs across the effective
  configuration, and switch both values atomically so a model is never
  accidentally attached to the wrong provider.
- Show project-scoped resume choices newest-first with bounded local metadata
  rather than requiring the user to know an opaque session ID. An interactive
  `smith --resume` without an ID opens the same picker before host creation.
- Store only non-secret selections in user configuration. Store an entered API
  key in the operating-system credential service, or record an environment
  variable reference when the user chooses externally managed credentials.
- Require trusted catalog metadata or explicit limits for the selected model.
  Setup MUST NOT guess a context window for an unknown model.
- Commit user configuration safely, re-run full resolution and runtime
  preflight, then start the ordinary TUI. Setup itself creates no session and
  sends no provider request.
- Keep `smith -p` and other non-interactive invocations non-mutating and
  fail-closed, with a stable diagnostic that points to `smith setup`.

## Impact

- Affected specs: `configuration`, `client-surfaces`.
- Affected code: `crates/smith-config` (readiness inspection, credential
  enrollment, typed provider response policy, selectable inventory, and safe
  user-config edits),
  `crates/smith-runtime` (setup descriptors, trusted model metadata, and
  response-stream normalization plus session-list metadata), `crates/smith-tui`
  (pure setup and selection state/rendering), and `crates/smith-cli`
  (entry-point orchestration).
- Active-change coordination: depends on
  `add-smith-agent-harness-2026-07-23`, whose resolved configuration, runtime
  factory, credential references, session persistence, and terminal lifecycle
  remain authoritative, and on `add-smith-slash-commands-2026-07-26`, whose
  registry remains the single command source. It coordinates with
  `update-smith-interaction-model-2026-07-27` by reusing its modal-selection
  interaction rather than adding a second command system. This change adds
  narrow pre-runtime setup/selection states and does not relax
  invalid-configuration or headless behavior.
- No Agent Runtime mechanism change is required.
