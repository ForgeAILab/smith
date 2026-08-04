## Context

The latest `ContextPlanned` event contains authoritative category totals but is
only emitted after Smith assembles a real provider request. The current
`/context` renderer filters absent/zero categories and renders only free input
plus reserve before the first turn. This is numerically honest but visually
ambiguous: users cannot tell whether system instructions and tool schemas are
part of the request shape or were forgotten.

Agent Runtime already models `ReasoningConfig { effort, max_tokens }`,
`ReasoningSupport::{Unsupported, Fixed, Controllable}`, pre-network capability
validation, and a bounded model-request interceptor. Smith currently supplies
no reasoning config. Its OpenAI-compatible adapter serializes a non-empty
reasoning config as `reasoning_effort`, which is not equivalent to every
supported endpoint's wire contract.

Current provider documentation confirms three distinct control shapes:

- OpenAI Chat Completions exposes model-dependent `reasoning_effort` values;
  supported values can include `none`, `minimal`, `low`, `medium`, `high`,
  `xhigh`, and `max`.
- OpenRouter exposes a unified `reasoning` object and per-model metadata for
  supported efforts, default state, token-budget support, and mandatory
  reasoning.
- Z.AI GLM exposes `thinking.type = enabled|disabled` and supports turn-level
  switching, while the documented API does not provide a general effort
  ladder.

Therefore “the model reasons” and “the user can control reasoning” cannot be
represented by one boolean.

## Goals / Non-Goals

### Goals

- Make stable model-facing context classes visible without fabricating counts.
- Let users change thinking state and effort when the resolved API/model
  contract supports those controls.
- Keep controls source-explainable, locally discoverable, persisted, and
  identical across the TUI and headless composition path.
- Keep one reasoning setting stable for a whole turn and every continuation.
- Reject unsupported settings before provider I/O.

### Non-Goals

- Showing or retaining raw system prompts, tool schemas, or reasoning text in
  `/context` or `/status`.
- Assuming all reasoning models accept effort, `none`, or an explicit toggle.
- Adding sampling, verbosity, or arbitrary provider-extension editors.
- Changing reasoning while a turn is running.
- Making a network request merely to populate a selector.

## Decisions

### Context uses stable baseline rows and honest unknowns

The display layer defines two baseline categories in stable order:

1. `system instructions`, the sum of `system_instruction`,
   `developer_instruction`, and `ability_instruction` plan totals;
2. `tool schemas`, the `tool_schema` plan total.

After a context plan, both legend rows render even when the total is zero. The
fixed grid allocates no cells to a zero category, but the legend remains
visible. Other dynamic categories retain their existing sorted rendering.
Canonical status and telemetry retain the unaggregated runtime totals.

Before the first plan, the grid still shows only known capacity and reserve.
The legend additionally shows:

```text
◇ system instructions: ? (not counted yet)
◆ tool schemas: ? (not counted yet)
```

Unknown is not zero. Smith does not pre-plan a synthetic request because tool
activation depends on the submitted turn. The text continues to state that
usage is unavailable until the first plan.

### Reasoning presence and controls are separate typed facts

Smith resolves one redaction-safe `ReasoningControlProfile` beside the model
profile. It records:

- reasoning support: unsupported, fixed, or controllable;
- switch behavior: unavailable, optional, or mandatory-on;
- ordered supported effort names;
- optional exact token-budget support and bounds;
- provider wire dialect;
- provider/model default state and effort when known;
- provenance for every field.

`reasoning = true` from Models.dev continues to map only to fixed reasoning.
Controllability requires a richer trusted catalog record, exact
provider/model metadata, or explicit owner-controlled configuration. Project
configuration cannot claim a provider dialect or capability until the project
is trusted, following the existing execution/configuration boundary.

Profile configuration may request defaults under a typed reasoning section:

```toml
[profiles.work.reasoning]
enabled = true
effort = "high"
```

Resolution refuses an unsupported value and names the capability source. An
omitted section preserves the provider/model default; it does not synthesize
`low` or silently turn reasoning on.

### UI controls share one validation path

The command registry adds:

```text
/think [on|off|default]
/effort [level|default]
```

With no argument, each command opens the existing bounded picker grammar. The
thinking picker disables or omits `off` for mandatory reasoning and explains
fixed/unsupported models. The effort picker contains only the resolved effort
levels. A Z.AI binding may therefore expose `/think` while `/effort` reports
`effort is not adjustable for this provider/model` locally.

Commands require an idle root session and issue no provider request. A direct
argument and a picker selection call the same validator. `/status`, `/context`,
and command results name the effective value and whether it came from the
provider default, layered profile, or session override. The compact footer may
show a non-default override beside the model only when width permits; it does
not add a new permanent region.

### One immutable reasoning selection governs a whole turn

The host owns a session reasoning controller. A command commits a new override
only while idle. At turn acceptance, the controller snapshots the effective
selection; a bounded model interceptor applies that same selection to every
provider attempt and tool continuation in the turn. The active turn cannot
observe a half-updated state.

The override is additive persisted session state. Resume revalidates it against
the newly resolved capability snapshot. A provider/model change retains it only
when semantically supported; otherwise Smith clears it with an explicit local
notice instead of mapping to a nearest value. Newly created child sessions
inherit the parent's resolved selection, while already-running children keep
their immutable selection.

### Provider dialect adaptation is exact and bounded

Smith adapts the typed selection at the provider boundary:

- OpenAI effort dialect: emit `reasoning_effort` only for an advertised effort;
  `off` is allowed only when `none` is advertised.
- OpenRouter dialect: emit the unified `reasoning` object using the advertised
  effort, explicit enabled state, or token budget; mandatory metadata removes
  `off` before UI and request validation.
- Z.AI dialect: emit `thinking.type` as `enabled` or `disabled`; no
  `reasoning_effort` field is emitted unless a future trusted capability record
  explicitly adds it.

Exact endpoint binding follows the existing catalog trust boundary. An unknown
OpenAI-compatible endpoint receives no inferred control. Vendor-extension keys
are constructed from typed values, bounded, and cannot override messages,
tools, model identity, or another normalized request field.

The resolved Agent Runtime capability is `Controllable` only when the selected
request can be represented by the exact dialect. Unsupported or fixed settings
fail locally before network I/O; Smith does not enable permissive downgrade.

## Risks / Trade-offs

- Catalogs that expose only a reasoning boolean cannot populate controls. This
  intentionally leaves some capable models fixed until richer metadata exists.
- Provider-supported values can change. Frozen metadata keeps one run
  reproducible; a later host rebuild may expose a refreshed set.
- Effort names are provider-defined. Smith validates bounded names and ordered
  advertised choices rather than hard-coding one universal enum.
- Reasoning effort can affect output-token consumption, latency, and cost. The
  UI states that it applies to the next turn and `/status` exposes the value;
  Smith does not estimate cost.
- A session override may become invalid after switching models. Clearing it
  explicitly is less convenient than guessing but preserves semantic honesty.

## Migration Plan

- Additive configuration fields default to provider/model behavior.
- Older catalog snapshots deserialize with fixed/unknown controls and remain
  usable; a schema bump is required only if richer normalized fields are
  persisted into the catalog cache.
- Older saved sessions have no override and preserve current behavior.
- Update `DESIGN.md` and configuration reference before implementation, then
  add deterministic wire, reducer, replay, and picker fixtures. Live provider
  tests remain explicit and spend-capped.

## Open Questions

- None. Unknown capability metadata deliberately produces no control; adding a
  new provider dialect later requires trusted metadata and conformance tests.
