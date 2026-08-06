---
created_at: 2026-08-06T00:00:00Z
updated_at: 2026-08-06T00:00:00Z
---

## 1. Defaults and loop translation

- [x] 1.1 Change the built-in default for `limits.max_tool_steps` to `0`.
- [x] 1.2 Translate a value of `0` to `None` on the shared loop config so the
  tool loop has no step ceiling; keep a positive value as `Some(value)`.
- [x] 1.3 Update the limits section docstring to record that `0` removes the
  ceiling.

## 2. Tests and fixtures

- [x] 2.1 Update the runtime test fixture's resolved limits to the new default.
- [x] 2.2 Update the composition test's mapped-defaults assertion to expect no
  tool-step ceiling.

## 3. Documentation

- [x] 3.1 Update the configuration reference example and defaults table.
