---
created_at: 2026-08-02T01:10:27Z
updated_at: 2026-08-02T01:15:43Z
completed_at: 2026-08-02T01:15:43Z
---

## 1. CLI Contract

- [x] 1.1 Parse valueless `--yolo` as explicit `ApprovalMode::AllowAll`.
- [x] 1.2 Reject values, duplicates, and conflicts with `--approval`.
- [x] 1.3 Show the alias and its exact meaning in command help.

## 2. Safety and Documentation

- [x] 2.1 Prove `--yolo` cannot restore mutation capabilities removed by a
  read-only plan profile.
- [x] 2.2 Document the trusted-boundary warning and profile-narrowing
  invariant in README, configuration, and security references.

## 3. Validation

- [x] 3.1 Run focused parser and black-box CLI contracts.
- [x] 3.2 Build and install Smith, then live-test allow-all code behavior and
  fixed read-only plan behavior through `--yolo`.
