---
created_at: 2026-10-24T10:14:29Z
updated_at: 2026-10-24T10:14:29Z
completed_at:
---

## 1. Implementation

- [x] 1.1 Return the verified instruction text with the admitted pin from the
  runtime factory (`prepare_skill_invocation`), keeping
  `prepare_skill_activation` for existing callers.
- [x] 1.2 Submit a user turn from a bare skill invocation, with the
  instructions as the prompt body and optional task text appended after a
  blank line.
- [x] 1.3 Preserve the instructions verbatim in the TUI's prepared submission;
  parse composer references only in the task text.
- [x] 1.4 Re-invoking an active skill starts a turn without a duplicate
  instruction fragment.
- [x] 1.5 Update `docs/skills.md`.

## 2. Verification

- [x] 2.1 CLI skill-command tests, TUI file-command tests, and runtime
  composition skill tests cover bare, task, and re-invocation paths.
- [ ] 2.2 Full workspace gate and CI on the integration branch.
