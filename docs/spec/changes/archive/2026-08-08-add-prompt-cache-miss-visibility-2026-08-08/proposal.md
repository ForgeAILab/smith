---
created_at: 2026-08-08T22:32:53Z
updated_at: 2026-08-09T01:20:01Z
---

## Why

Smith can show positive cache-read tokens, but it cannot currently tell a user
that an otherwise reusable prefix was re-billed. Its TUI reducer accumulates
`CacheObservation` only when adapters report a positive read, the Smith-owned
ChatGPT adapter drops explicit zero, and headless JSON exposes only the
pre-request cache plan. A provider-reported zero, an omitted cache field, and a
first request that is merely eligible all collapse into the same absence.

Pi demonstrates useful product surfaces—per-turn cache-read percentage,
significant miss notices, and cumulative re-billed tokens—but its consecutive
prompt-usage comparison and universal five-minute idle hint are approximations.
Smith can consume Agent Runtime's exact plan/attempt correlation instead and
keep elapsed idle time as factual context rather than proof of expiry.

## What Changes

- Depend on
  `agent-runtime:add-prompt-cache-miss-evidence-2026-08-08` and consume its
  canonical, attempt-attributed `CacheStateChanged` events in live, replay, and
  headless reducers.
- Update the Smith-owned experimental ChatGPT Responses adapter to preserve
  explicit zero cache reads/writes and omission using the shared
  presence-aware provider event.
- Add a per-turn cache projection. The footer's `CH` value is the latest
  completed root turn's provider-reported cache-read share of total prompt
  input; explicit zero renders `0%` and omitted evidence renders `?`.
- Add cache state, expected/observed/missed tokens, and derived hit metrics to
  `/status`, the exit/session usage summary, final JSON, and stream JSON without
  changing canonical model conversation.
- Track cumulative derived “cache re-billed” tokens and miss count separately
  from disjoint usage counters. When compatible pricing is available, derive
  the incremental cost relative to a cache read and retain its provenance.
- Add `cache.miss_notices` as an explainable layered Boolean, defaulting to
  `false`. When enabled, append one local transcript notice for a completed
  root turn whose misses total at least 20,000 tokens or whose derived extra
  cost is at least $0.10.
- Phrase idle context factually—`Cache miss after 9m idle`—and never claim
  “expired” or “likely expired” from time alone. A model/provider identity
  switch starts a new cache identity and is not counted as a miss.

## Dependencies and Coordination

- Hard dependency:
  `agent-runtime:add-prompt-cache-miss-evidence-2026-08-08`.
- `add-usage-aware-credential-pools-2026-08-04` touches status, usage, and
  client surfaces. The implementations must preserve both account and cache
  projections rather than replacing either.
- `add-cli-reasoning-effort-flag-2026-08-08` and
  `add-mcp-servers-2026-08-07` touch configuration/client-surface specs but
  have no semantic overlap.
- The existing local Cargo and benchmark changes are outside this proposal and
  are not modified during Stage 1.

## Impact

- Affected specs: `prompt-cache`, `configuration`, `client-surfaces`,
  `usage-accounting`
- Affected code: `crates/smith-runtime/src/chatgpt.rs`,
  `crates/smith-tui/src/{status,app,transcript,usage_log}.rs`,
  `crates/smith-cli/src/{headless,tui_driver,local_command,runtime_host}.rs`,
  `crates/smith-config`, session replay tests, and user documentation
- Compatibility: Smith's config and JSON additions are additive; the
  Agent Runtime dependency update is a coordinated pre-1.0 API migration
- Network behavior: none beyond the provider request Smith already makes

## Out of Scope

- A new `/settings` or `/session` command
- Configurable notice thresholds beyond the on/off setting
- Cache keepalive, prewarm, retention, compaction, or breakpoint-policy changes
- Anthropic Cache Diagnostics, provider response-ID threading, or diagnostic
  beta headers
- Calling a miss “expired” without future provider diagnostic evidence
- Dedicated child-panel cache UX or delegated cache-waste aggregation

## Approval Boundary

Approval authorizes the Agent Runtime consumer migration, Smith-owned adapter
normalization, cache reducer and persistence projections, the `CH`/status/exit
and headless surfaces, the opt-in notice setting, the fixed significance
thresholds, and the factual wording rules in `tasks.md`. It does not authorize
provider diagnostics, synthetic requests, cache-retention policy, new slash
commands, customizable thresholds, or model-facing transcript content.
