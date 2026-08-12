---
created_at: 2026-08-04T16:15:58Z
updated_at: 2026-08-08T22:39:11Z
completed_at: 2026-08-08T22:39:11Z
---

## 0. Approval and upstream contract gate

- [x] 0.1 Approve pool-per-provider scope, exhaustion-first rotation, and the
  two open questions in `design.md`. Resolved: the proactive threshold ships
  active; headless runs never rotate (member chosen once at session start);
  and rotation is user-confirmed through a modal rather than silent, because
  switching abandons the prompt cache and resubmits the turn.
- [x] 0.2 Create the coordinated Agent Runtime
  `add-provider-rate-limit-snapshots-2026-08-04` proposal covering per-adapter
  header parsing, the normalized snapshot observation, and the typed
  limit-exhaustion error with reset time.
- [x] 0.3 Approve and implement the upstream change before exposing any Smith
  policy that depends on it.

## 1. Credential pool configuration (`smith-config`)

- [x] 1.1 Extend the provider declaration with an ordered `credentials` pool;
  keep `credential` as a pool of one; reject duplicates and unparseable
  entries with sourced errors.
- [x] 1.2 Thread pools through resolve, provenance, inventory, and setup
  readiness without weakening redaction.
- [x] 1.3 Unit-test layering, provenance, duplicate rejection, and legacy
  single-credential equivalence.

## 2. Rotation policy and persistence (`smith-runtime`)

- [x] 2.1 Extend factory preflight to validate every pool member reference and
  surface the active member in `FactoryPreflight`.
- [x] 2.2 Consume the typed limit-exhaustion error: cooldown bookkeeping from
  reported reset times (bounded default when absent), next-eligible selection,
  at-most-once replay per remaining member, never after an accepted stream.
- [x] 2.3 Add the rotation-offer policy modeled on `smith-host::approval`: a
  `RotationPolicy` seam, an interactive prompt carrying the outgoing member,
  its reset time, the eligible members with meters, and the prompt-cache cost;
  a fail-closed headless policy that never rotates; and start-of-session member
  selection for headless runs.
- [x] 2.4 Apply the proactive threshold: offer rotation at a turn boundary when
  the active member's latest snapshot is at or above the configured percentage,
  asking at most once per turn.
- [x] 2.5 Persist the sticky active member in user-scope state and restore it
  on session start. Stored by credential *reference* rather than pool position,
  so editing `credentials` keeps the same account rather than the same slot.
  Rotation outcomes are recorded as redaction-safe transcript notices and, for
  unattended runs, in machine output; server-reported usage reaches the journal
  through the upstream `RateLimitObservation` runtime event. A dedicated
  Smith-side journal record for the rotation *decision* is not part of this
  slice — no consumer reads one, and the decision is already visible in both
  surfaces.
- [x] 2.6 Deterministic tests with the fake provider: confirmed rotation,
  declined rotation, all-exhausted failure with earliest reset, cooldown
  expiry, accepted-stream no-replay, headless never-rotate, threshold offer,
  restart stickiness.

## 3. Usage surfaces (`smith-tui`, `smith-cli`)

- [x] 3.1 Retain the latest snapshot per member and render meters with
  unknown-stays-unknown semantics next to, not mixed into, token counters.
- [x] 3.2 Add the account picker (pool order, meters, cooldowns, active
  marker) and the manual switch command.
- [x] 3.3 Render the rotation offer as a modal stating the prompt-cache cost,
  and record both confirmation and refusal in the transcript.
- [x] 3.4 Project active member, snapshots, and rotation outcomes through
  versioned machine output for `smith -p`, which never prompts or rotates.
- [x] 3.5 Snapshot/interaction tests for picker, rotation modal, transcript
  notices, and headless projection.

## 4. Validation

- [x] 4.1 `cargo test` across affected crates plus an end-to-end fake-provider
  scenario driving exhaustion → rotation → recovery.
- [x] 4.2 Re-validate the change with the spec toolkit and update deltas if
  implementation diverged.
