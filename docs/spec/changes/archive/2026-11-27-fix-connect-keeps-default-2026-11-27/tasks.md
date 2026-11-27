---
created_at: 2026-11-27T20:40:00Z
updated_at: 2026-11-27T20:40:00Z
completed_at:
---

## 0. Approval

- [x] 0.1 Owner decisions (2026-11-27): Keep writes provider only; skip the
  `/connect` summary review and keep the collision review; in-session
  confirmations are choices.

## 1. Default choice

- [x] 1.1 Add `make_default` to `QuickXai`, `QuickGoogle`, and
  `QuickAnthropic`; pass it from the reducer.
- [x] 1.2 `setup_plan` calls `select_default` only when `make_default`; the
  completion summary reports what was written.
- [x] 1.3 First run skips the default choice and always makes the default.
- [x] 1.4 The setup transaction appends new profiles to an existing user
  `profile_order` and drops the key when the user file has none.

## 2. In-session connection

- [x] 2.1 Catalog key connections in a session submit right after the default
  choice; collisions still open the review.
- [x] 2.2 The in-session review ends with numbered save/back choices that stay
  visible while the review scrolls.
- [x] 2.3 Empty API-key fields show a paste placeholder.

## 3. Verification

- [x] 3.1 Unit tests for plans, order merging, reducer steps, and rendering.
- [x] 3.2 fmt plus stable and 1.88 workspace clippy pass; smith-tui and
  smith-cli tests pass, including `--no-default-features`.
- [x] 3.3 Live `/connect anthropic` against an isolated home: Keep writes
  provider only; Make default shows the collision choice and appends to
  the order.
