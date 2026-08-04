# Validation evidence

Validated on 2026-08-01 (America/Toronto).

## Automated

- `cargo test -p smith-cli`: 59 unit tests, 14 black-box CLI contracts, and 11
  PTY setup tests passed; the quota-spending provider test remained explicitly
  ignored.
- `cargo clippy -p smith-cli --all-targets -- -D warnings`: passed.
- Parser coverage proves `--yolo` resolves to `ApprovalMode::AllowAll` and
  rejects values, repetition, and conflicts with `--approval`.
- Black-box coverage gives a plan profile an edit-and-shell prompt under
  `--yolo` and proves no workspace mutation capability activates.

## Installed-binary live provider validation

- `code --yolo` selected `zai/glm-5.2`, activated `edit`, created the exact
  proof file, and completed successfully.
- `plan --yolo` selected `zai/glm-4.7`, exposed neither `edit` nor `shell`,
  returned `PLAN_STILL_READ_ONLY`, and created no proof file. Its only
  activated state-writing capability was session-local `write_todos`, which
  cannot mutate the project workspace.
- The installed `smith --help` identifies `--yolo` as an alias for
  `--approval allow-all`.
