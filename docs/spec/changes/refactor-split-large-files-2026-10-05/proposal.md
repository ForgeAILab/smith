---
created_at: 2026-10-05T08:42:01Z
updated_at: 2026-10-05T08:42:01Z
---

## Why

Thirty Rust files in the workspace are over 1,500 lines, and sixteen are
over 2,000. The largest is `smith-tui/src/setup.rs` at 3,877 lines. The
2026-08-02 split (`refactor-large-rust-modules`) handled the five worst files
of that time, but nothing stops a file from growing back, and several have.
Large files make reviews slow and put unrelated work into the same merge
conflicts.

## What Changes

- Split every `.rs` file under `crates/` that is over 1,500 lines into
  cohesive child modules of 1,000 lines or less. The original file stays as
  the module root (`setup.rs` with children in `setup/`), so every module
  path, public item, and signature is unchanged.
- Integration test files become directory test targets
  (`tests/host_session.rs` → `tests/host_session/main.rs` plus child
  modules), so the number of test binaries does not grow.
- Moved tests keep their bodies, names, and assertions. Fixture files are not
  re-recorded; a fixture that changes means the split changed behavior.
- A workspace test fails when any `.rs` file under `crates/` exceeds 1,500
  lines, naming the file and its length.

The 30 files, by crate:

- `smith-config`: `tests/precedence.rs`, `src/resolve/provider.rs`,
  `tests/readiness_and_setup.rs`
- `smith-tools`: `src/change.rs`, `src/display.rs`
- `smith-client`: `src/status.rs`, `src/cache.rs`
- `smith-runtime` source: `host.rs`, `chatgpt.rs`, `resume_capsule.rs`,
  `cache_lifecycle.rs`, `model_catalog.rs`, `journal.rs`, `reasoning.rs`,
  `factory.rs`, `delegation.rs`
- `smith-runtime` tests: `host_session.rs`, `delegation.rs`, `composition.rs`
- `smith-tui`: `setup.rs`, `render/tests/transcript.rs`,
  `render/transcript.rs`, `transcript.rs`, `app/tests/reducer.rs`,
  `picker.rs`, `render/tests/composer.rs`, `app/state.rs`
- `smith-cli`: `setup.rs`, `tui_driver.rs`, `main_tests/fixtures.rs`

## Impact

- Affected specs: `code-organization`
- Affected code: the 30 files above and new child modules beside them; one
  new test in `smith-runtime/tests/architecture.rs`
- Compatibility: no behavior, public API, serialized format, configuration
  key, command, key binding, or rendered output changes
- Dependencies: none added

## Approval Boundary

Approved 2026-10-05 ("lets do 1 and clear all large files. you may use
codex"; limit chosen: over 1,500 lines). Approval covers moving code between
files, the visibility needed for sibling modules inside one parent
(`pub(super)` or `pub(crate)`, never wider), and the guard test. It does not
cover renaming items, changing logic, or editing tests beyond their module
declarations and imports.
