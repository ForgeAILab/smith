---
created_at: 2026-10-19T00:13:28Z
updated_at: 2026-10-19T03:34:17Z
---

## Why

Xiaoge now advertises `selected-large`, but Smith's custom-provider model list
still contains an older manually configured ID. Users should discover current
models and their published metadata before setup asks for numbers, and Smith
should retain that metadata and any published prices between daily refreshes.

## What Changes

- Discover OpenAI-compatible models through each configured, authorized
  provider's `GET <base_url>/models`. Offer discovered IDs in add-model setup,
  resolve their limits automatically, and use manual entry only for missing
  metadata. Existing explicit values take precedence over derived defaults.
- Cache validated model IDs, names, limits, supported modalities, capabilities,
  compatible reasoning controls, and published per-counter pricing with source,
  revision, account scope, and retrieval time. Unknown fields confer no runtime
  behavior; absent prices remain unknown.
- Refresh the public Models.dev cache and eligible configured endpoint caches
  every 24 hours while Smith runs; catch up on startup. Keep last-good metadata
  usable offline and adopt updates at an idle host-rebuild boundary.
- Add `smith models sync [--provider NAME] [--force]` for an immediate,
  inference-free refresh and a user-only `model_sync` control for disabling or
  changing the refresh interval.
- Preserve explicit models, defaults, and versioned pricing for already
  recorded usage. A refresh never changes the selected model automatically.

## Immediate correction

The existing detection-first setup contract already requires automatic limits.
The inline-config credential omission and missing nested `limits` aliases are
ordinary bug fixes under that contract, tracked separately from approval for
this new synchronization capability. The local `xiaoge/selected-large` entry
has been added using Xiaoge's published limits so it is selectable immediately.

## Impact

- Affected specs: `provider-runtime`, `configuration`, `client-surfaces`,
  `usage-accounting`.
- Affected code: `smith-config` catalog types, inventory and provenance;
  `smith-runtime` bounded probes, cache loader and runtime catalog composition;
  `smith-cli` setup, CLI and host lifecycle; `smith-tui` setup choices;
  `smith-client` price references; configuration and command documentation.
- Existing public Models.dev requests remain credential-free. Authenticated
  endpoint discovery is a separate cache source bound to the configured
  provider, endpoint, adapter, and credential/account scope.
- Coordination: the active Google proposal owns the optional `models.toml`
  migration. This change neither implements nor depends on that migration;
  generated metadata stays in cache files and explicit overrides keep their
  existing configuration locations.

## Approval Boundary

Approve daily process-owned synchronization, endpoint-backed model choices,
metadata and price caching, detection-first setup, and the explicit sync
command. No OS scheduler or service is installed. Refresh happens while Smith
is running and catches up the next time it starts. Other provider protocols
retain their existing reviewed catalogs until a listing adapter is added.
