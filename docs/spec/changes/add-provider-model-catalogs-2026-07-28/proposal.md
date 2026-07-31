---
created_at: 2026-07-29T03:50:50Z
updated_at: 2026-07-29T04:42:27Z
---

## Why

Smith's `/model` picker currently enumerates only provider/model pairs named by
local model records or profiles. A configured OpenRouter account therefore
shows only the models the user manually copied into TOML, and Smith's Z.AI
Coding Plan quick start exposes only `glm-4.7`, even though both configured
providers have larger catalogs with the metadata Smith needs for safe
preflight.

## What Changes

- Add a Smith-owned, schema-validated Models.dev snapshot and last-good cache,
  following OpenCode's separation between provider activation and catalog
  population.
- Bind only known configured OpenAI-compatible endpoints to catalog providers:
  OpenRouter to `openrouter`, and Smith's Z.AI Coding Plan endpoint to
  `zai-coding-plan`. A catalog can add models to a configured provider, but it
  cannot create a provider, endpoint, credential, adapter, or header.
- Keep model enumeration pure and credential-free. The host prepares an
  immutable local catalog snapshot before building `SelectionInventory`;
  opening or filtering `/model` performs no network or credential I/O.
- Merge catalog models with explicit `[models."<provider>/<model>"]` records
  field by field. Explicit Smith configuration keeps higher precedence, and
  every resolved limit retains catalog revision/retrieval provenance.
- Normalize Models.dev's `context`, optional `input`, and `output` limits into
  Agent Runtime's three required enforcement limits without inventing a
  context window. When no separate input ceiling is published, the total
  context window is the maximum input ceiling and the existing context policy
  still holds back output/reasoning reserves.
- List catalog-backed provider/model pairs in `/model` and in provider model
  counts. Text, tool-capable, non-deprecated models with complete valid limits
  are selectable; incompatible entries remain searchable but disabled with a
  local reason.
- Pass the same frozen catalog source used by the picker into runtime
  preflight, so choosing an undeclared catalog model can rebuild the session
  safely and atomically without persisting a generated model record.
- Refresh public catalog metadata with bounded, credential-free HTTPS I/O and
  atomic cache publication. Refresh failure never blocks startup or removes a
  last-good/bundled catalog; a successful refresh becomes visible only to a
  later host rebuild.
- Add deterministic fixture, cache-corruption, offline, large-picker,
  cross-provider, precedence, and runtime-preflight coverage. No live provider
  test is required.

## Impact

- Affected specs: `configuration`, `provider-runtime`, `client-surfaces`.
- Affected code: `crates/smith-config` (catalog-augmented but I/O-free
  selection inventory), `crates/smith-runtime` (validated snapshot/cache and
  `ModelCatalogSource` construction), `crates/smith-cli` (catalog lifecycle and
  host injection), and `crates/smith-tui` (catalog metadata and disabled
  reasons in large model pickers).
- External data: the public `https://models.dev/api.json` schema and a generated
  bundled subset for OpenRouter and Z.AI Coding Plan. Provider credentials are
  never sent to Models.dev.
- Active-change coordination: this change depends on the approved and
  implemented `add-smith-setup-flow-2026-07-28` inventory/picker work. It keeps
  that change's rule that enumeration and picker interaction perform no
  network access; catalog refresh is a separate host concern. The dependency
  should be archived before this change is applied or archived.
- No breaking configuration change and no Agent Runtime mechanism change are
  required.
