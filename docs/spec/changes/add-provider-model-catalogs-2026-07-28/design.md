## Context

`smith-config::local_inventory` currently constructs candidate pairs only from
local `[models]` keys and profiles, then drops candidates whose three required
limits cannot be resolved from explicit configuration or Smith's one embedded
`zai/glm-4.7` record. `smith-cli::start_host` gives that inventory to the TUI,
while `smith-runtime::prepare_factory_inputs` independently builds
`CatalogLayers` for the selected pair. `RuntimeRequest::catalog_sources` and
the `CachedRemote` layer already provide the runtime seam, but Smith does not
yet populate it.

OpenCode separates these responsibilities:

- its Models.dev service loads a disk cache, falls back to a bundled snapshot,
  fetches `https://models.dev/api.json` with a bounded timeout, publishes the
  cache atomically, and refreshes independently;
- its provider service activates providers from config, environment, or stored
  authentication, then fills those activated providers from Models.dev;
- explicit provider/model configuration is merged over catalog metadata, and
  deprecated or configured-blacklisted models are removed;
- its model list and picker consume the resulting in-memory provider catalog
  rather than querying a provider while the picker is open.

Research was performed against OpenCode commit
`8cbea4fbb7f2a8ccf59f44922ef7c1ff5f22e377`, principally
[`packages/core/src/models-dev.ts`](https://github.com/anomalyco/opencode/blob/8cbea4fbb7f2a8ccf59f44922ef7c1ff5f22e377/packages/core/src/models-dev.ts),
[`packages/opencode/src/provider/provider.ts`](https://github.com/anomalyco/opencode/blob/8cbea4fbb7f2a8ccf59f44922ef7c1ff5f22e377/packages/opencode/src/provider/provider.ts),
and
[`packages/opencode/src/cli/cmd/models.ts`](https://github.com/anomalyco/opencode/blob/8cbea4fbb7f2a8ccf59f44922ef7c1ff5f22e377/packages/opencode/src/cli/cmd/models.ts).
At proposal time Models.dev advertised hundreds of OpenRouter models and
multiple Z.AI Coding Plan models, so hard-coding one model per provider is not
a viable catalog strategy.

## Goals / Non-Goals

### Goals

- Show the safe, coding-capable catalog of every recognized configured
  OpenRouter or Z.AI Coding Plan provider in `/model`.
- Preserve provider-qualified identities and atomic provider/model selection.
- Use one immutable, provenance-carrying catalog snapshot for both UI
  enumeration and runtime preflight.
- Keep configuration parsing/inventory and picker interaction deterministic,
  local, and credential-free.
- Remain useful offline and fail closed on malformed or incomplete metadata.
- Let explicit Smith configuration override remote catalog fields.

### Non-Goals

- Activating any provider that is not already configured and adapter-valid.
- Sending a provider API key to Models.dev or querying provider `/models`
  endpoints from the picker.
- Treating Models.dev's advertised adapter package, API URL, request options,
  pricing, or arbitrary headers as Smith runtime configuration.
- Supporting every Models.dev provider in the first change.
- Persisting every discovered model into user TOML.
- Hot-swapping limits inside an already constructed runtime/session.
- Adding pricing, ranking, favorites, or a model-management UI.

## Decisions

### Use a host-owned validated snapshot, not picker-time discovery

The new catalog loader lives with Smith's host/runtime composition and returns
an immutable `CatalogSnapshot`. It performs the bounded filesystem/network
work. `smith-config` receives only a read-only catalog view when it builds the
selection inventory, and the TUI receives only bounded `ResourceEntry` data.

```text
bundled seed ─┐
last-good cache ──> validate + bind ──> immutable CatalogSnapshot
Models.dev ──┘                              │
                    ┌───────────────────────┴──────────────────────┐
configured providers + local records                    RuntimeRequest
                    │                                  catalog_sources
                    v                                         │
           SelectionInventory -> /model                       v
                                                   immutable model preflight
```

This preserves the existing requirement that opening, filtering, cancelling,
or confirming a picker performs no network or credential access.

Alternative considered: call each provider's `/models` endpoint when `/model`
opens. Rejected because it couples a local safety surface to provider latency,
credential access, inconsistent schemas, and provider spend/rate limits.

### Activate catalog data only through exact provider bindings

A configured provider is catalog-backed only when all of these match a
Smith-owned binding:

- its compiled adapter kind is available;
- its normalized base endpoint exactly matches the binding;
- the binding names a supported Models.dev provider.

The initial bindings are:

| Smith endpoint | Models.dev provider |
| --- | --- |
| `https://openrouter.ai/api/v1` | `openrouter` |
| `https://api.z.ai/api/coding/paas/v4` | `zai-coding-plan` |

The local provider name remains authoritative. A user may call an OpenRouter
provider `router`; its choices are still `router/<model-id>`. An endpoint match
never imports Models.dev's API URL, npm package, environment-variable names,
headers, or credentials.

Alternative considered: bind only by local provider name. Rejected because a
custom service named `openrouter` could otherwise inherit the wrong models and
limits. An explicit catalog-binding configuration key may be added later, but
is outside this change.

### Merge fields through the existing catalog precedence

The validated snapshot becomes one `ModelCatalogSource` at
`CatalogSource::CachedRemote`. Explicit model records remain
`CatalogSource::Explicit` and therefore win field by field. Smith's embedded
known-good model source remains above cached remote data. The source record
includes a schema revision, content digest, and retrieval time.

The same source instance is supplied to inventory construction and
`RuntimeRequest.catalog_sources`. Refresh creates a new snapshot/cache file; it
does not mutate the instance held by a running host.

Alternative considered: generate ephemeral `[models]` entries and run them
through configuration. Rejected because generated data would masquerade as
user configuration, pollute provenance, and create unnecessary persistence
and collision behavior.

### Normalize only enforceable limit facts

For a schema-valid Models.dev model:

- `context_tokens = limit.context`;
- `max_output_tokens = limit.output`;
- `max_input_tokens = min(limit.input.unwrap_or(limit.context), limit.context)`.

An absent separate input ceiling means the total context window is the
strongest enforceable upper bound on input. Agent Runtime's existing
`ContextPolicy` still subtracts declared output and reasoning reserves before
planning, so Smith does not infer a larger usable prompt budget. Zero values,
values outside `u32`, output above context, or an explicit input ceiling above
context invalidate the catalog entry rather than being clamped.

Catalog validity and current selectability remain separate. If the effective
Smith output and reasoning reserves would consume the entire context window,
the inventory keeps the valid advertised entry visible but disabled. Smith does
not silently lower either the provider's published output ceiling or the
user's configured reserve to make the entry pass.

Alternative considered: offer only entries with `limit.input`. Rejected because
Models.dev omits that optional field for the Z.AI Coding Plan catalog and most
OpenRouter entries, defeating the requested behavior even though it publishes
their total context and output ceilings.

### Keep incompatible catalog entries visible but non-selectable

Deprecated entries are omitted. Entries without text output, tool calling, or
complete valid limits remain searchable with a disabled reason. Selectable
entries carry display name, provider-qualified ID, context/output limits,
capability summary, catalog provenance, and current marker. `/provider` model
counts count only selectable entries.

This uses the picker's existing disabled-entry behavior and keeps unsupported
models from reaching strict runtime preflight while still explaining why a
provider's advertised model is unavailable.

Alternative considered: list every model as selectable, as a general chat
client might. Rejected because Smith always supplies coding tools and its
runtime intentionally fails unsupported capabilities before provider I/O.

### Refresh without making startup depend on the network

Smith ships a generated, schema-validated seed containing only the supported
provider catalogs. Startup loads a valid last-good cache when available and
otherwise uses the seed. A stale snapshot schedules a bounded refresh from the
exact HTTPS Models.dev API; responses have a strict byte cap, schema and
numeric validation, no credential headers, and atomic same-directory
publication. Invalid data, redirects outside the allowed origin, timeout, or
offline failure leave the current snapshot untouched.

A successful refresh is adopted on the next host rebuild or process start.
Tests inject the clock, cache directory, seed, and fetcher; they never depend on
the live service.

Alternative considered: block first startup on a fresh fetch. Rejected because
the model picker and current configured model must remain available offline.

## Risks / Trade-offs

- Models.dev can lag a provider. The bundled/last-good fallback favors
  availability and deterministic metadata over immediate freshness; the UI
  exposes source age/revision instead of claiming live provider availability.
- A provider may restrict a model by account, region, or subscription tier
  even when the catalog lists it. Full runtime preflight and the provider's
  response remain authoritative; Smith MUST NOT claim entitlement.
- The OpenRouter catalog is large. Inventory and picker tests must cover
  hundreds of entries, bounded rendering, deterministic sorting, and cheap
  filtering.
- Cached remote limits influence safety. Strict schema/numeric validation,
  exact endpoint bindings, explicit-config precedence, immutable snapshots, and
  last-good atomic publication limit that trust boundary.
- The current active setup-flow change says remote catalogs are not queried by
  inventory/pickers. This design preserves that boundary but adds a separate
  host refresh path. The dependency must be reconciled and archived in order.

## Migration Plan

- Existing configuration remains valid and gains catalog choices
  automatically only when its normalized endpoint matches a supported binding.
- Existing explicit model records and the embedded `zai/glm-4.7` record keep
  precedence and require no rewrite.
- The first release seeds OpenRouter and Z.AI Coding Plan catalogs. A generated
  seed update is reviewable as data and carries its source digest/retrieval
  timestamp.
- Removing the cache restores the bundled seed; it never removes user
  configuration or credentials.
- Rollback removes the refresh/augmentation path and returns `/model` to local
  records without a configuration migration.

## Open Questions

None. Approval of this proposal approves Models.dev as a bounded metadata
source for the two exact provider endpoints above, including the normalization
and offline-cache policy described here.
