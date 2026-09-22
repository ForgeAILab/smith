# Proposal: Auto-resolve custom model limits in setup

## Why

`smith setup add-provider` and `/connect openai-compatible` ask every custom
model for three hand-typed numbers (context, maximum input, maximum output)
even when a machine-readable answer exists. The endpoint itself frequently
advertises limits on `GET /v1/models` (OpenRouter `context_length`, Groq
`context_window`, LiteLLM `max_input_tokens`/`max_output_tokens`), and upstream
model IDs served by local gateways usually match a trusted Models.dev catalog
entry by name. Meanwhile catalog-backed models already get limits and a
percentage-derived request budget for free, which is the experience custom
endpoints should approach.

Today's spec forbids this: setup "MUST NOT guess limits" and inventory guidance
says "without … querying the provider". That guard exists so a model never
*runs* on invented numbers. The distinction this change draws is between
guessing (silent fabrication at runtime) and pre-resolution (labeled values
from a provider advertisement or trusted catalog, shown in review and
editable before anything is written).

## What Changes

- After the model ID is entered in the add-provider/add-model flows, setup
  performs a bounded best-effort resolution, in priority order:
  1. `GET {base_url}/models` (≤5 s, ≤1 MiB, no inference request), matching the
     entry whose `id` equals the typed model and reading the common limit
     fields plus OpenRouter's nested `top_provider.context_length`.
  2. A same-name trusted-catalog match: exact model ID, then case-folded, then
     last path segment (for gateway IDs like
     `Qwen/Qwen2.5-Coder-32B-Instruct`), then a final segment with `.`, `_`,
     and `-` separators folded together (for aliases such as
     `claude-fable-5-1` → `anthropic/claude-fable-5.1`), preferring entries
     with complete limits and never dropping a version component.
- From a resolved or manually entered context, the two remaining ceilings are
  derived automatically:
  maximum input = the context window; maximum output = the same percentage
  rule as the automatic request budget (`min(32 768, context/4)`).
- Resolved values skip numeric entry entirely; review shows one provenance
  line naming the source (endpoint listing, catalog match, and/or derived
  defaults). Back opens a single context-window field with its value
  pre-filled and editable.
- When nothing resolves, setup asks only for the total context window and then
  continues with the two derived ceilings. Separate maximum-input and
  maximum-output fields are removed from the interactive flow.
- Runtime resolution, inventory, and preflight are untouched: a model still
  cannot run without enforceable limits, and nothing is written without the
  reviewed setup commit.

Out of scope: model-ID suggestions from the listing, runtime catalog refreshes
during setup, changes to the quick-start/provider pickers, non-interactive
resolution, and any runtime-side defaulting.

## Impact

- Affected specs: `configuration` (one modified requirement),
  `client-surfaces` (one modified requirement)
- Affected code: `smith-config/src/catalog.rs` (same-name lookup),
  new `smith-runtime/src/probe.rs` (`/models` probe + parser),
  refreshed `smith-runtime/data/models-dev-seed.json` (current reviewed
  Models.dev snapshot),
  `smith-tui/src/setup.rs` (resolve effect, single context fallback, defaults, review
  provenance), `smith-cli/src/setup.rs` (effect handling, credential/endpoint
  plumbing, derivation), PTY fixtures move to a refused localhost endpoint so
  they stay offline
- Security/cost: one unauthenticated-or-bearer GET per model entry, bounded
  time and size, no inference request, no secret persisted by the probe;
  pre-resolved values are user-reviewed before commit

## Approval Boundary

Approval authorizes the two resolution sources, the single context-window
fallback, separator-normalized same-name aliases, the refreshed embedded
snapshot, the derived ceilings, the skip/review-provenance behavior, and the
test changes above. It does
not authorize runtime or inventory limit defaults, catalog refreshes during
setup, model suggestions, or any change to what preflight enforces.
