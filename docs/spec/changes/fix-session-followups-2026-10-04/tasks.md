---
created_at: 2026-10-05T01:34:57Z
updated_at: 2026-10-05T01:34:57Z
completed_at:
---

Approved 2026-10-04 ("i think lets fix everything then push").

## 1. Implementation

- [ ] 1.1 Usage line counts user-started turns, `1 turn` / `N turns`; every
  surface that prints the session usage line follows.
- [ ] 1.2 Resume attributes restored usage per model from the usage log's
  last record for the session when its totals match; manifest rule otherwise.
- [ ] 1.3 Both forms of `smith sessions list` omit sessions without a user
  message; `LATEST PROMPT` header; plugin docs say latest prompt.
- [ ] 1.4 Remove an empty session's files when the interactive surface ends
  it.
- [ ] 1.5 GLM quick start proposes `glm-5.3` from a new trusted record;
  catalog revision bumped; older records kept.

## 2. Verification

- [ ] 2.1 Tests for each; fixtures re-recorded and reviewed; gate.
- [ ] 2.2 Live: a one-prompt session with tool calls exits with `1 turn`;
  resuming the GLM-then-Gemini session prices both models; an empty session
  leaves no files; piped listing; setup review shows `glm-5.3`.
