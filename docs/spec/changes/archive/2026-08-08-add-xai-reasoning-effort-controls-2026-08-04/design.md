## Context

Reasoning control resolution already distinguishes presence from
controllability. Exact endpoints may normalize a dialect without per-model TOML:

- OpenAI Chat Completions endpoint → `openai-effort` / `reasoning_effort`
- OpenRouter → unified `reasoning` object
- Z.AI Coding Plan → `thinking.type`
- native Gemini → catalog `thinking_level` values

xAI is already a catalog trust pair (`openai-responses` or `xai-responses` at
`https://api.x.ai/v1`). Grok reasoning models advertise effort ladders in the
frozen Models.dev snapshot. Both xAI adapter paths construct Agent Runtime's
`ResponsesProvider`, and the `openai-effort` dialect leaves typed
`ReasoningConfig` on the request for that adapter to serialize — the same path
ChatGPT/OpenAI effort selections use when no vendor rewrite is required.

## Goals / Non-Goals

### Goals

- Make `/effort`, profile `reasoning.effort`, and session overrides work for
  catalog-backed Grok reasoning models without requiring
  `[models."xai/…".reasoning]` metadata.
- Reuse the existing OpenAI-effort ladder rules and validation errors.
- Keep the endpoint binding exact: only the normalized xAI catalog URL grants
  the dialect.

### Non-Goals

- A new `ReasoningDialect` variant or xAI-only vendor extension.
- Changing default model selection (`grok-4.3` remains the setup default for
  reserve-arithmetic reasons).
- Claiming entitlement beyond the advertised catalog ladder.
- Token-budget reasoning controls.

## Decisions

### xAI endpoint mirrors OpenAI effort normalization

In `resolve_reasoning_policy`, when the resolved endpoint is the exact xAI
catalog endpoint and the model profile is not `ReasoningSupport::Unsupported`:

- dialect = `OpenaiEffort`
- efforts = catalog ladder when non-empty, else `low`/`medium`/`high`
- switch = optional only if `none` is advertised; otherwise mandatory-on
- capability source names Models.dev or the xAI Responses reasoning API

Explicit `[models."provider/model".reasoning]` metadata continues to win over
endpoint normalization.

### One shared dialect, distinct provenance text

xAI does not need a separate adaptation branch in `adapt_request`;
`OpenaiEffort` remains a no-op wrapper. Capability-source strings name xAI so
`/status` and validation errors stay explainable.

## Risks / Trade-offs

- If a future xAI model advertises efforts the Responses wire rejects, local
  validation will still allow the catalog values. That matches OpenAI behavior
  and is corrected by catalog refresh or explicit metadata, not by guessing.
- Non-reasoning Grok variants stay fixed/unsupported and gain no selector.

## Migration Plan

- No config migration. Existing xAI profiles immediately gain controllable
  effort when the model is a catalog reasoning model.
- Older sessions without overrides keep provider defaults.
