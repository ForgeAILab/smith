## Context

The pieces exist separately. Agent Runtime ships the Anthropic Messages
adapter, including an opt-in `thinking.block_binding` control. Smith resolves
`anthropic-messages` with
`ANTHROPIC_DEFAULT_ENDPOINT = https://api.anthropic.com/v1` and maps it to
Anthropic image sizing. The `anthropic-effort` dialect reaches the adapter as
adaptive thinking plus `output_config.effort`. What is missing is the
catalog-backed connection Google and xAI already have:

- the `anthropic-messages` descriptor is `connectable: false`, uses the
  `CustomEndpoint` flow, and fixes the official endpoint, so it asks for a model
  ID and limits for an endpoint Smith could describe itself;
- Models.dev's `anthropic` provider is not in `EXPECTED_PROVIDERS` or the seed;
- `catalog_provider_for` has no Anthropic arm, so no catalog metadata reaches
  the binding;
- `resolve_reasoning_policy` grants Anthropic controls only from explicit
  per-model metadata;
- Smith never sets the adapter's `thinking_block_binding`.

Models.dev's `anthropic` entries use the API's own model IDs (for example
`claude-opus-5-5`, `claude-sonnet-5-5`, `claude-haiku-5-5`) and publish
1M/128K limits, text/image/PDF input, tool calling, cache-aware prices, and
`reasoning_options` effort ladders. Catalog normalization already drops entries
marked `deprecated` (Opus 4.5, Sonnet 4.5). Every remaining entry with an
effort ladder is a 4.6-or-later model that accepts adaptive thinking.

Current Claude models bind each signed thinking block to the conversation
prefix that produced it: the system prompt, the tool list, and every earlier
message. For accounts created on or after 2026-08-31, the API rejects with a
400 any request that replays a block after that prefix changed, unless the
request sets `thinking.block_binding.prefix_mismatch_behavior`. Smith replays
history under a different prefix in at least one place: the semantic-summary
call sends the session's messages under its own instructions. Compaction then
replaces older history while later turns keep their blocks.

## Goals / Non-Goals

- Goals: one reviewed key-based connection; catalog-owned model metadata;
  catalog-driven effort controls at the exact official endpoint; requests that
  survive Smith's own history edits; no new runtime path.
- Non-Goals: subscription (Pro/Max) sign-in, which the `cli/claude-code/...`
  bridge already covers; cloud-platform clients; fast mode; refusal fallbacks;
  API-version migration; thinking-display changes in the adapter; block binding
  for gateways.

## Decisions

- **Provider identity.** The provider is `anthropic` with profile `claude`
  and default model `claude-opus-5-5`. The descriptor follows the xAI and
  Google pattern: the `QuickKey` flow, `CREDENTIAL_METHODS`, and catalog model
  selection. The connection sits after Google and before the generic
  OpenAI-compatible entry.
- **Endpoint is written.** Setup writes `base_url =
  "https://api.anthropic.com/v1"`, as xAI setup does, so the catalog binding is
  visible in configuration. `catalog_provider_for` also maps
  `(anthropic-messages, None)`, because an omitted URL resolves to the same
  official endpoint. Any other endpoint, including a trailing path or a
  gateway, returns no catalog.
- **Reasoning is always on, default `high`.** At the exact endpoint, a
  catalog-advertised effort ladder yields dialect `anthropic-effort`, the
  advertised efforts in order, mandatory-on reasoning, and default effort
  `high` when the ladder contains it.
  - Why a default: the adapter attaches `block_binding` only to a request that
    carries an effort, so without one an omitted `thinking` field would leave
    the request exposed to the history-editing 400.
  - Why `high`: it is the API's own default on every catalog model except Opus
    5.5 and Haiku 5.5 (which default to `medium`). It is also the floor the
    API guidance gives for coding work.
  - Why mandatory-on: omitting `thinking` means no thinking on Opus 4.6/4.7/4.8
    and Sonnet 4.6, while `disabled` is a 400 on Opus 5.5, Sonnet 5.5, and
    Fable. A user-facing off switch would mean different things per model, so
    Smith does not offer one.
  - Users change the level with `/effort` or `[profiles.claude.reasoning]`.
- **Block binding is `drop_block` on the official endpoint.** The factory sets
  `ThinkingBlockBinding::DropBlock` on `AnthropicConfig`, using the same
  exact-endpoint predicate as the catalog binding, whether the dialect came
  from the catalog or from explicit metadata. The adapter then adds the
  `thinking-binding-controls-2026-08-01` beta and the field on every request
  that has an effort. A mismatched block is dropped and the request proceeds
  instead of failing. Gateways are left alone, because they may reject the
  beta.
- **Requests without a selection get the lowest effort.** The dialect wrapper
  holds a fallback: the lowest advertised effort (`low` on every current
  ladder). When a request reaches it with no reasoning config, for example the
  semantic-summary call on the same provider, the wrapper fills that in, so
  the request carries the binding too. The lowest level keeps a short summary's
  output budget from going to thinking. Requests that already have a reasoning
  config, including main turns and advisor calls, are unchanged.
- **The default is sent, not selected.** The policy gains a
  `sends_default_effort` flag. `request_config` sends `default_effort` when it
  is set and nothing is selected. Status and pickers still treat the level as
  a default: `/effort` labels the row `default (high)`, and startup shows no
  override notice.
- **Budget-only models.** Toggle-only catalog models (Haiku 4.5) get
  presence-only reasoning and no controls. They produce no thinking blocks
  unless asked, and the adapter's `budget_tokens` path stays reachable only
  through explicit metadata.
- **Gateway entry.** The old `anthropic-messages` descriptor becomes
  "Anthropic-compatible endpoint". It collects provider name and endpoint and
  keeps the existing explicit-limits and catalog-ID-fallback behavior. The
  `anthropic-messages` setup id stays, so existing scripts and tests keep
  working.
  - The ID fallback walks catalogs in map order, so a gateway model named
    exactly like an Anthropic entry (for example `claude-fable-5-1`) now
    prefills from the `anthropic` catalog instead of OpenRouter's
    `anthropic/claude-fable-5.1` alias. Both describe the same model.

## Risks / Trade-offs

- **Cost of the default.** A default of `high` raises Opus 5.5 and Haiku 5.5
  above their API default (`medium`). Summary calls on the same provider now
  always think, at `low`. Users who want cheaper turns lower the level with
  `/effort`.
- **Dropped thinking.** `drop_block` removes the first mismatched thinking
  block and every later one for that request. After a compaction or summary
  the model can lose earlier reasoning, but not its visible transcript.
- **Empty thinking text.** The adapter sends no `display`, so current models
  return thinking blocks with empty text. The signatures still replay
  correctly; summarized display is an upstream follow-up.
- **Cache pricing.** Catalog cache-write prices assume the 5-minute TTL. This
  is unchanged from today's usage accounting.

## Migration Plan

Existing configurations keep working unchanged. A hand-written provider with
`kind = "anthropic-messages"` and either no `base_url` or exactly
`https://api.anthropic.com/v1` now gains catalog metadata, mandatory reasoning,
a `high` default effort, and block binding. Its explicit `[models]` records
still take precedence. Gateway providers are unaffected.

## Open Questions

None.
