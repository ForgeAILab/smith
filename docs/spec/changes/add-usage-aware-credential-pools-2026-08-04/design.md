## Context

Smith resolves exactly one credential reference per provider
(`ResolvedProvider.credential`), and Agent Runtime already owns renewable
credential leases with bounded authentication-rejection replay. Nothing today
observes provider rate-limit state: adapters discard the `x-ratelimit-*` /
`anthropic-ratelimit-*` / `x-codex-*` header families, and a usage-limit
rejection surfaces as an undifferentiated provider error. Codex
(`codex-rs/codex-api/src/rate_limits.rs`) shows the normalization shape:
per-window `used_percent`, window duration, and `resets_at`, keyed by limit id,
parsed centrally from response headers and broadcast as events.

## Goals / Non-Goals

- Goals:
  - Declare several accounts for one provider as an ordered credential pool.
  - Observe server-reported usage per pool member with honest provenance
    (absent stays absent; never invented client-side).
  - Rotate automatically on limit exhaustion, respect reported reset times,
    and keep the choice sticky and visible.
  - Keep manual control: inspect members, switch explicitly, override policy.
- Non-Goals:
  - Cross-provider or cross-model failover (a pool is same adapter, same
    endpoint, same model semantics — only the credential varies).
  - Client-side token budgeting or predictive throttling.
  - Writing or refreshing browser-login session files Smith does not own.
  - Evading provider rate limits: pools exist for legitimately separate
    accounts/budgets; the design keeps every rotation visible and auditable.

## Decisions

- Decision: pools are per-provider, ordered, and reuse `CredentialRef`
  verbatim. Alternatives considered: account objects with their own identity
  and metadata (heavier config schema for no mechanism gain); cross-provider
  pools (changes model behavior mid-session, rejected).
- Decision: the usage signal is mechanism in Agent Runtime (header parsing in
  each direct adapter, one normalized snapshot type, a typed limit-exhaustion
  error), consumed as policy in Smith. Alternative: Smith-side response
  sniffing (would re-open the transport boundary the runtime owns).
- Decision: rotation triggers on typed limit exhaustion, and additionally on a
  configurable proactive threshold (`rotate at >= N% used`) that ships active.
  Alternatives: round-robin (defeats prompt-cache locality entirely);
  threshold declared but inert (defers the capability for no mechanism gain
  once the snapshot exists).
- Decision: **rotation is offered, never silent.** Switching members abandons
  the provider-side prompt cache, so the replayed attempt resubmits the whole
  context uncached — a real cost the user should agree to rather than absorb by
  surprise. On exhaustion the runtime raises a rotation prompt naming the
  outgoing member, its reset time, and the eligible members with their meters;
  confirming replays the attempt on the chosen member, declining fails the turn
  with the typed exhaustion error. This also keeps every rotation explicit for
  the terms-of-service reason below. Alternatives: silent automatic rotation
  (cheapest to build, but spends the user's cache and their second account
  without asking); prompt only above a cost threshold (no honest threshold
  exists — the cache loss is the same size every time).
- Decision: the rotation prompt is a Smith host policy modeled on the approval
  gate (`smith-host::approval`), not an `InteractionBroker` questionnaire.
  Interaction is tool-call-scoped and carries task content; a credential
  rotation is a runtime-level host decision with the same fail-closed
  requirements as approval. Reusing the approval shape gives the headless rule
  below for free.
- Decision: headless runs never rotate. `smith -p` selects its member once at
  session start — the persisted sticky member when it is eligible, otherwise
  the first eligible member — and keeps it for the whole run. On exhaustion the
  run fails with the typed error and the earliest reset time. A script's
  credential must not change under it mid-run, and there is no surface to
  answer a rotation prompt. Alternative: headless auto-rotation (makes an
  unattended run silently spend a second account's budget).
- Decision: rotation replay reuses the existing bounded-replay discipline:
  never after a stream is accepted, at most one replay per remaining eligible
  member per user attempt, cooldown from server `resets_at` (bounded default
  when absent).
- Decision: the sticky active member persists in user-scope state (not the
  session journal), like other runtime selector choices; sessions resume onto
  the persisted member.

## Risks / Trade-offs

- Provider terms of service: pooling accounts to dodge limits can violate ToS.
  Smith ships the feature for legitimately separate accounts; rotation is
  explicit in the transcript and never silent, and documentation says so.
- Prompt-cache locality: a rotation abandons server-side prompt cache; the
  first turn after rotation pays uncached input. Exhaustion-first triggering
  and stickiness bound this cost.
- Clock skew / absent `resets_at`: cooldowns fall back to a bounded default
  and are always re-testable by manual switch.
- Header drift: provider header families change; parsing is best-effort and
  absence must degrade to "unknown", never to zero or to a fabricated meter.

## Migration Plan

Additive only. Existing single-`credential` declarations parse unchanged as a
pool of one; no configuration rewrite, no journal format change. The
coordinated Agent Runtime change lands and is pinned before Smith exposes the
policy.

## Open Questions

Both questions raised during proposal are resolved in Decisions above: the
proactive threshold ships active, and headless runs never rotate (member chosen
once at session start, kept for the run). Rotation being user-confirmed rather
than automatic was decided alongside them.
