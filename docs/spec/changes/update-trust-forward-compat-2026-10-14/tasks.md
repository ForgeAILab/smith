---
created_at: 2026-10-14T12:55:57Z
updated_at: 2026-10-14T13:26:31Z
completed_at: 2026-10-14T13:25:03Z
---

## 1. Store

- [x] 1.1 In `crates/smith-config/src/trust.rs`, read each persisted record
  into a raw JSON value. A record that is not an object with string `kind` and
  `label` fields is fatal. Otherwise, parse it as a `TrustRecord` when
  possible and keep the raw value either way. Do not add a catch-all variant
  to `ExecutableKind`, because it would be constructible and would persist
  under the wrong name.
- [x] 1.2 Make `status` and `find` see understood records only, and make
  `records()` return understood records only.
- [x] 1.3 In `record`, replace by the raw `kind` and `label` strings, so a
  record that is not understood is replaced and not duplicated.
- [x] 1.4 In `persist`, write the raw value for each record that was not
  replaced, and a fresh serialization for each new or replaced one.
- [x] 1.5 Keep the `version` check and the not-JSON error loud and unchanged.

## 2. Tests

- [x] 2.1 A file with one known kind and one unknown kind opens, and the
  known decision's status is unchanged.
- [x] 2.2 An unknown-kind `allow` record and a known-kind record with an
  unknown decision value both leave their artifacts untrusted.
- [x] 2.3 After `record()` adds an unrelated decision, the persisted JSON
  still contains the unknown record with the same fields and values,
  including any extra fields.
- [x] 2.4 Recording a decision for a kind and label whose existing record has
  an unknown decision value leaves exactly one record, the new one.
- [x] 2.5 A wrong `version`, a record that is not an object, and a record with
  no `kind` still fail with the existing errors.
- [x] 2.6 Keep `a_trust_file_written_before_slash_commands_still_loads`
  passing.

## 3. Verify

- [x] 3.1 `cargo test -p smith-config --no-fail-fast` with
  `TMPDIR=/private/tmp/smith-fixture-tmp`.
- [x] 3.2 `cargo clippy -p smith-config --all-targets -- -D warnings`.
