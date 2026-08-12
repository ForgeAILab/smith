---
created_at: 2026-08-05T00:00:00Z
updated_at: 2026-08-08T22:39:12Z
completed_at: 2026-08-08T22:39:12Z
---

## 1. Defaults and loop translation

- [x] 1.1 Change the built-in default for `limits.turn_time_limit_ms` to `0`.
- [x] 1.2 Translate a value of `0` to `None` on the shared loop config so a turn
  has no wall-clock deadline; keep a positive value as `Some(value)`.
- [x] 1.3 Update the loop limit field docstring to record that `0` removes the
  ceiling.

## 2. Tests and fixtures

- [x] 2.1 Update the runtime test fixture's resolved limits to the new default.

## 3. Documentation

- [x] 3.1 Update the configuration reference example and defaults table.
