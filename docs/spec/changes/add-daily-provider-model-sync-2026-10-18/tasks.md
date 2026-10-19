---
created_at: 2026-10-19T00:13:28Z
updated_at: 2026-10-19T01:15:02Z
completed_at: 2026-10-19T01:15:02Z
---

## 0. Approval

- [x] 0.1 Approve the daily refresh, authenticated endpoint cache, setup,
  freshness controls, sync command, and pricing boundaries in this proposal.

## 1. Endpoint snapshots

- [x] 1.1 Define strict provider/adapter/endpoint/account-scoped snapshots,
  versioned provenance, bounded cache paths, and optional per-counter rates.
- [x] 1.2 Share one bounded listing reader between setup and refresh; normalize
  existing top-level aliases, nested `limits`, explicit capabilities/modalities,
  and reviewed reasoning controls without interpreting arbitrary source fields.
- [x] 1.3 Normalize known USD price schemas with explicit units; retain valid
  counters, explicit zero, and unknown missing counters independently.
- [x] 1.4 Publish owner-private caches atomically with process locks, source
  identity validation, conditional requests, and last-good fallback.

## 2. Refresh lifecycle

- [x] 2.1 Add user-only `model_sync.enabled` and `model_sync.interval_hours`
  configuration with defaults `true` and `24`, validated range `1..=168` hours.
- [x] 2.2 Schedule non-blocking startup and recurring refreshes for all eligible
  configured sources, reusing available credentials without unlocking services
  for inactive providers, and bound concurrency and failure retry backoff.
- [x] 2.3 Catch up once after sleep/startup; cancel process-owned timers on exit;
  share cross-process freshness and avoid duplicate network requests.
- [x] 2.4 Add `smith models sync [--provider NAME] [--force]` and secret-free
  per-source outcomes; explicit commands may resolve credentials normally.

## 3. Inventory and setup

- [x] 3.1 Compose endpoint snapshots alongside existing catalog sources with
  explicit-field precedence; include new IDs without generating TOML entries.
- [x] 3.2 Adopt snapshots at safe idle rebuilds; retain active runtimes and
  already open pickers until their existing immutable selection completes.
- [x] 3.3 List discovered models before asking for a model ID, resolve limits
  before numeric entry, preserve existing values ahead of derived defaults,
  and review only real changes using the current collision transaction.
- [x] 3.4 Expose source, age, stale/failure status, and optional prices in model
  details and sync output; never claim account entitlement.
- [x] 3.5 Preserve per-binding/version price references through root, child,
  advisor, module usage, switching, and resume.

## 4. Verification and documentation

- [x] 4.1 Add deterministic fake-clock/HTTP tests for daily cadence, restart,
  sleep, force, concurrent processes, failures, offline cache, removed IDs,
  credential changes, redirects, and private file publication.
- [x] 4.2 Test Xiaoge-shaped metadata, configured inline/env/reference auth,
  no prompt when metadata exists, real override collisions, no-op re-adds,
  explicit precedence, and picker/preflight snapshot consistency.
- [x] 4.3 Test units, invalid/partial/zero prices, same-ID price changes,
  endpoint/account isolation, unpriced aliases, and historical accounting.
- [x] 4.4 Document daily running/startup behavior, opt-out, manual sync,
  offline fallback, unknown pricing, and custom-provider protocol coverage.
- [x] 4.5 Run formatting, focused tests, Clippy with warnings denied, and
  relevant workspace checks after implementation.
