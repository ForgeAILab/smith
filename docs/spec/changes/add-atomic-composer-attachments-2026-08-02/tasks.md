---
created_at: 2026-08-02T18:42:57Z
updated_at: 2026-08-02T19:06:30Z
completed_at: 2026-08-02T19:06:30Z
---

## 0. Approval and baseline

- [x] 0.1 Approve the proposal, interaction design, and capability delta.
- [x] 0.2 Record focused composer, reducer, paste, image-submission, and render
  baselines without disturbing unrelated worktree changes.

## 1. Atomic composer editing

- [x] 1.1 Add registered-placeholder range discovery that uses character
  offsets, accepts paste and image labels, and ignores unregistered lookalikes.
- [x] 1.2 Make horizontal cursor movement jump between the two boundaries of a
  registered placeholder while retaining ordinary Unicode-safe movement
  elsewhere.
- [x] 1.3 Make backward and forward deletion remove an adjacent registered
  placeholder as one range while retaining single-character deletion
  elsewhere.
- [x] 1.4 Keep all other cursor placement paths on valid placeholder boundaries
  so later typing cannot split registered material accidentally.

## 2. Material and presentation integrity

- [x] 2.1 Preserve the full compact label and accent styling in the composer
  while mapping its start/end boundaries to the rendered cursor position.
- [x] 2.2 Add a committed user-transcript projection that expands pasted-text
  labels to their exact stored text while retaining image labels, without
  changing compact editable/history/pending strings or provider material.
- [x] 2.3 Prove that typed image paths and typed placeholder lookalikes remain
  ordinary text and never acquire paste expansion or image payloads.
- [x] 2.4 Update `DESIGN.md` with atomic movement/deletion semantics and the
  registered-material boundary.

## 3. Validation

- [x] 3.1 Add focused unit and integration coverage for adjacent placeholders,
  mixed Unicode text, both deletion directions, boundaries, history restore,
  queued input, raw committed transcript text, and retained clipboard-image
  labels and payloads.
- [x] 3.2 Run `cargo fmt --check`, focused `smith-tui` tests, workspace tests,
  warning-denied Clippy, and strict spec validation.

## Validation evidence

- `cargo fmt -p smith-tui -- --check`
- `cargo test -p smith-tui` — 244 unit/render tests and 4 end-to-end tests
  passed; doc tests passed.
- `cargo test --workspace` — workspace, integration, PTY, and doc tests passed;
  the credential doc example and opt-in live-provider test remained ignored by
  their existing policy.
- `cargo clippy -p smith-tui --all-targets -- -D warnings`
- Strict `add-atomic-composer-attachments` spec validation.
- Workspace-wide formatting and Clippy were also attempted. Concurrent
  user-owned provider-connect work currently leaves formatting diffs in
  `crates/smith-cli/src/resources.rs` and `crates/smith-runtime/src/factory.rs`
  and non-exhaustive `Connect`/`Disconnect` matches in
  `crates/smith-cli/src/local_command.rs` and
  `crates/smith-cli/src/runtime_host.rs`; those unrelated files were not
  changed for this implementation.
