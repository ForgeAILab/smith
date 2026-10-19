## Context

The current host loads one immutable Models.dev snapshot and refreshes it only
when a recognized catalog-backed provider starts. Custom-provider inventory
comes from explicit configuration. Setup can probe one typed model, but does
not retain the listing or offer provider-discovered choices.

Xiaoge's current authenticated listing has one `selected-large` entry with a
1,048,576-token context/input ceiling, a 256,000-token output ceiling, nested
`limits.{context,input,output}`, and explicit tool/modality metadata. It has no
price fields. Its capabilities are evidence for that serving endpoint; another
provider's price for a similarly named model is not evidence for Xiaoge.

## Decisions

### Separate public and endpoint caches

Keep the existing Models.dev cache format and credential-free transport.
Add endpoint snapshots beneath `~/.smith/cache/provider-models-v1/`, keyed by a
bounded digest of provider name, adapter, normalized endpoint, and a non-secret
credential/account identity. Credential rotation or an account change must
invalidate that account's metadata. Reuse credential broker identity/revision
support; do not persist raw keys, authorization headers, raw HTTP documents,
or reversible credential material in snapshots.

Endpoint caches contain only normalized model metadata, source URL, content
digest, optional ETag, successful validation time, and schema/source identity.
Use owner-only directories/files, lock files, temporary writes, and atomic
replace. Absent/invalid endpoint caches contribute nothing; existing explicit
models and public fallback catalogs remain usable.

### Refresh belongs to the host

Default to `model_sync.enabled = true` and `model_sync.interval_hours = 24` in
user-owned configuration. Project configuration cannot enable authenticated
background discovery or change user-owned endpoint/credential authority. Only
authorized configured endpoints participate; support OpenAI-compatible listing
first. Existing native adapters continue to use their reviewed catalogs.

On startup, load cache immediately, then schedule stale sources without
blocking usable preflight. A host-owned timer repeats while the process runs,
including across idle periods. Sleep/wake skips missed ticks and refreshes each
due source once. On exit, cancel tasks; install no scheduler or daemon. Keep
the 24-hour policy relative to last successful validation, not wall-clock
midnight. A failed refresh does not reset freshness and uses bounded retry
backoff. Multiple processes recheck successful freshness under the file lock.

Use at most two concurrent source requests, each with a five-second total
budget and a streamed one-MiB body cap; accept at most 10,000 bounded model IDs
per source. Disable redirects and send credentials only to the configured
endpoint. Use ETag/304 where supported. Keep the public catalog's existing
eight-MiB/eight-second bounds independent. Background work reuses available
credentials and never opens an unlock prompt for an inactive provider; an
explicit sync command may resolve credentials through the normal bounded path.

### One normalizer and one immutable composition

Share endpoint parsing between setup and daily refresh. Normalize common
limit aliases plus `limits.{context,input,output}` and allowlisted metadata.
Only explicit, adapter-compatible reasoning controls can grant controls.
Unknown arbitrary extensions cannot change endpoint, auth, runtime behavior,
or entitlement. Missing required limits remain explained/disabled unless an
explicit record supplies them; no default context is invented.

For custom endpoints, precedence is explicit configuration, then that exact
endpoint's cached advertised field. Keep existing trusted public bindings and
their precedence for recognized deployments. Never attach an upstream
provider's limits or rates to a gateway based only on a shared model name.
Each field retains source/revision/retrieval provenance.

Refresh completion stages a snapshot. At the next idle safe rebuild, create a
new consistent inventory/runtime composition. Never mutate an active turn,
child, request, or open picker. A deliberate model selection uses the same
snapshot that made that picker row selectable. A cache update never chooses
a model, rewrites configuration, or discards persisted session state.

### Detection-first setup and existing values

Add-model first uses a prepared provider snapshot or performs one bounded
listing read. Present discovered models, with a manual-ID fallback. Selecting
one prefills all advertised limits and skips numeric prompts. Resolve existing
explicit values ahead of derived defaults; if no source provides a context,
ask for that one missing value. A derived output ceiling cannot replace an
existing explicit or published output ceiling.

Retain the existing exact replacement review for deliberate changes. Re-adding
an unchanged entry is a no-op, and changing only the default does not rewrite
metadata. Preserve the active Google proposal's model-file migration boundary.

### Prices remain versioned metadata

Accept documented-shaped, allowlisted price fields with explicit currency and
units: Models.dev-style USD per million tokens in `cost` and OpenRouter-style
USD per token strings in `pricing`. Normalize finite non-negative counters to
the existing integer micro-USD-per-million representation using checked decimal
arithmetic. Reject ambiguous units, invalid/overflowing counters, and unknown
currencies without disabling the model. Cache valid counters independently;
explicit zero is a price and absence is unknown. Do not infer a price for
`selected-large` from its underlying model or vendor.

Snapshot revision and effective price reference must accompany usage so a
later price change for the same model never reprices earlier attempts. Price
metadata affects presentation only and is excluded from model-facing context.

Usage log schema 6 persists frozen rates and provenance for each binding,
including advisor, vision, synthetic maintenance, and module usage. Older logs
remain readable with their model identities; historical rates that were never
recorded remain unknown rather than borrowing today's catalog rate.

## Verification

Use fake clocks, local authenticated listing fixtures, and temporary private
cache directories; no paid inference is needed. Cover daily cadence and
sleep/restart, cross-process locks, offline/failure handling, authentication
scope changes, atomic writes, parser limits, setup no-prompt/no-op paths,
snapshot consistency, price units, and old/new price revisions in one session.
