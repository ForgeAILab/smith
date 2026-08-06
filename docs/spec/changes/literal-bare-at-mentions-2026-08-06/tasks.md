---
created_at: 2026-08-06T00:00:00Z
updated_at: 2026-08-06T00:00:00Z
completed_at: 2026-08-06T00:00:00Z
---

## 1. Parser behavior

- [x] 1.1 Change the `(false, false)` arm in `parse_references` to emit the
  `@token` span as literal text and continue, instead of returning an
  unresolved-reference error.
- [x] 1.2 Keep `@file:` and `@agent:` typed prefixes failing locally when
  unresolved.
- [x] 1.3 Keep the ambiguous `(true, true)` collision error unchanged.

## 2. Tests

- [x] 2.1 Replace `unresolved_and_outside_workspace_references_fail_locally`
  with `bare_unresolved_at_token_is_literal_text`.
- [x] 2.2 Add `npm_scoped_package_name_is_literal_text`.
- [x] 2.3 Add `explicit_typed_unresolved_references_still_fail`.
- [x] 2.4 Update the pending-input integration test so `@missing.rs` sends as
  text with no attached files.
- [x] 2.5 Update the child-lifecycle integration test so a disabled child
  profile name sends as literal text.

## 3. Spec

- [x] 3.1 Land the `client-interaction` delta modifying the "Unified typed
  reference completion" requirement.
- [x] 3.2 Reconcile the truth spec.

## 4. Verification

- [x] 4.1 `cargo test -p smith-tui` (321 passed).
- [x] 4.2 `cargo clippy -p smith-tui --lib` clean.
