---
created_at: 2026-08-04T16:15:58Z
updated_at: 2026-08-04T16:22:10Z
---

## Why

A provider account that exhausts its usage window today stops Smith cold: the
turn fails with an opaque provider error and the user must wait out the reset
or hand-edit configuration to point at another credential. Users who
legitimately hold several accounts for one provider (personal and work plans,
several API keys with separate budgets) have no way to declare them together,
see how consumed each one is, or have Smith move to the next one when the
active one is spent. Codex demonstrates the mechanism half (server-reported
rate-limit snapshots parsed from response headers) but still fails hard at
exhaustion; Smith can close the loop with policy.

## What Changes

- Allow a provider declaration to carry an ordered **credential pool** of
  existing credential references instead of exactly one. A single `credential`
  remains valid and behaves as a pool of one. Pool entries reuse the reviewed
  `CredentialRef` forms, layering, provenance, and redaction rules unchanged.
- Add a coordinated Agent Runtime change
  (`agent-runtime:add-provider-rate-limit-snapshots-2026-08-04`) so direct
  provider adapters parse provider rate-limit/usage headers into a normalized,
  redaction-safe **rate-limit snapshot** observation, and classify usage-limit
  rejections as a distinct typed **limit-exhaustion** error carrying the
  server-reported reset time when present.
- Add Smith runtime **usage-aware rotation** policy: on limit exhaustion of the
  active pool member, rotate to the next member not in cooldown and replay the
  attempt within existing replay bounds; place exhausted members in cooldown
  until their reported reset time; persist the sticky active member; never
  rotate after a response stream has been accepted.
- Surface per-member usage in the TUI and machine output: status shows the
  active pool member, a picker lists members with usage meters and cooldowns,
  rotation is announced in the transcript, and a command switches members
  manually.

## Impact

- Affected specs: `configuration`, `provider-runtime`, `usage-accounting`,
  `client-surfaces`
- Affected code: `smith-config` (pool declaration, resolve, provenance),
  `smith-runtime` (factory preflight, rotation state, cooldowns, persistence),
  `smith-tui` (status, picker, commands), `smith-cli` (machine output),
  coordinated `../agent-runtime` change (adapter header parsing, typed
  exhaustion error, versioned observation)
- Explicitly out of scope: cross-provider failover (pools never span
  adapters/endpoints), client-side usage estimation (snapshots are
  server-reported only), and any OAuth ceremony changes (session-json
  credentials stay read-only, owned by their originating tools)
