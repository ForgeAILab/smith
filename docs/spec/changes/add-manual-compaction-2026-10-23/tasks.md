---
created_at: 2026-10-23T00:00:00-04:00
updated_at: 2026-10-24T10:33:43Z
completed_at: 2026-10-23T09:46:55Z
---

# Tasks

- [x] Register bare `/compact` with idle-only execution.
- [x] Expose manual compaction through the hosted session using the protected runtime boundary.
- [x] Render bounded completion, no-op, busy, shutdown, and failure notices.
- [x] Verify parser/help/completion and host persistence/failure behavior with focused tests.
- [x] Document manual compaction and run formatting plus relevant crate checks.

## Follow-up (2026-10-24): name refusal causes

- [x] Split the lumped busy outcome into already attempted, not idle, runtime busy, request pending, and event gap, each with its own notice.
- [x] Let an explicit request bypass a durable gate the runtime has not consumed in this process; restore the gate when the request is not admitted.
- [x] Report history the committed summary already covers as no eligible history, without replacing the capsule summary with a failed projection.
- [x] Resolve compaction settings preflight from the session's environment instead of re-reading the process environment.

## Verification

- `cargo test -p smith-client -p smith-tui --lib --quiet`
- `cargo test -p smith-runtime --test host_session --quiet`
- `cargo test -p smith-runtime --lib --quiet`
- `cargo test -p smith-cli --bin smith --quiet`
- `cargo clippy -p smith-cli -p smith-client -p smith-tui -p smith-runtime --all-targets -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`
