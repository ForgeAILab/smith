## Context

Smith has three related values with different meanings:

- the model profile's `max_output_tokens` is the provider-advertised hard
  ceiling;
- the resolved top-level/profile `max_output_tokens` is the amount an ordinary
  provider request may generate;
- `context.output_reserve` is the amount the context planner holds back from
  admitted input.

The latter two are optional. Today both inventory and runtime context policy
fall back directly to the model ceiling when they are absent. That is safe only
when catalogs publish a much smaller output ceiling than context. A valid model
whose output ceiling equals context is consequently visible but disabled with
"leaves no input budget."

Trusted quick-start records already demonstrate the intended distinction by
carrying a `request_output_tokens` value below the model ceiling. Dynamic
catalog models need the same distinction without a per-model Smith release.

## Goals / Non-Goals

### Goals

- Make valid catalog-backed text/tool models selectable without user-authored
  limit guesses when their output ceiling equals or nearly equals context.
- Keep the automatic request size deterministic, conservative, explainable,
  and bounded by the frozen model profile.
- Guarantee that the picker and constructed runtime use the same calculation.
- Preserve explicit configuration precedence and fail-closed validation.

### Non-Goals

- Correct or reinterpret catalog ceilings.
- Probe a provider or credential to discover account-specific limits.
- Optimize the automatic budget independently for each vendor or model family.
- Change reasoning reserve semantics or explicit-config validation.

## Decisions

### Derive one Smith-owned automatic request budget

When the resolved configuration has no explicit `max_output_tokens`, Smith
derives:

```text
automatic_request_output = min(
    model_max_output,
    32_768,
    max(1, context_tokens / 4),
    context_tokens - reasoning_reserve - 1,
)
```

The derivation fails rather than producing zero when the reasoning reserve
already leaves no input and output space. Integer arithmetic is checked or
saturating at the boundary, so malformed or extreme metadata cannot wrap.

The 32,768-token product cap is large enough for long coding-agent responses
and matches the scale of an existing trusted quick-start request budget. The
quarter-window bound keeps small-context models useful for input. The final
bound preserves at least one token of input after the reasoning reserve; the
existing context planner remains responsible for all stronger admission and
compaction policy.

The automatic budget is derived policy, not catalog metadata. Smith retains
the advertised model ceiling and its Models.dev provenance unchanged.

### Keep request and reserve precedence distinct

The effective ordinary request limit is the explicit resolved
`max_output_tokens`, otherwise the automatic value. The effective context
output reserve is explicit `context.output_reserve`, otherwise that effective
request limit.

Explicit values remain authoritative. If an explicit request or reserve plus
the reasoning reserve consumes the window, the model remains unavailable and
the diagnostic names those values; Smith does not clamp user configuration.

### Share the calculation across inventory and runtime

`smith-config` owns a small typed output-budget policy helper because both its
catalog-augmented inventory and `smith-runtime` need it. Inputs are only the
resolved explicit values, immutable model limits, and reasoning reserve. The
result distinguishes automatic from explicit request and reserve sources.

Inventory uses the result for selectability and exposes bounded detail to the
CLI resource mapper. Runtime uses the same result when building `LoopConfig`
and `ContextPolicy`; the effective values participate in the immutable harness
identity and child-runtime construction. No second formula is maintained in
the TUI or factory.

### Make the distinction visible

The model picker continues to show the advertised context/input/output
ceilings and adds the effective request budget with an `automatic` or
configured source label. A model such as Grok 4.6 therefore remains visibly a
500,000-token-ceiling model while showing a 32,768-token automatic request
budget. Disabled diagnostics continue to explain explicit reserve conflicts.

## Risks / Trade-offs

- Omitted request limits will now send a concrete maximum to providers. This is
  a deliberate behavior change and may produce shorter responses than a
  provider-specific implicit default, but it is deterministic and can be
  overridden up to the resolved ceiling.
- A universal 32,768-token cap is product policy rather than vendor guidance.
  Combining it with the model ceiling and quarter-window bound avoids vendor
  guessing while keeping the default useful across context sizes.
- The active Google Gemini proposal also touches catalog inventory and picker
  paths. Stage 2 must preserve its provider-specific metadata and tests; the
  automatic policy is provider-neutral and does not change endpoint bindings.

## Migration Plan

- No configuration or persisted-session migration is required.
- Existing explicit request limits and reserves retain their current meaning.
- Remove comments and tests that encode output-ceiling equality as a reason to
  prefer an older xAI default, but do not change the default xAI model in this
  change.
- Rollback restores the absent-request behavior and ceiling fallback; no stored
  data must be rewritten.

## Open Questions

- None. The default cap and quarter-window rule are versioned Smith product
  policy and can be revised through a later change with focused evidence.
