---
created_at: 2026-10-05T01:34:57Z
updated_at: 2026-10-05T02:58:06Z
completed_at: 2026-10-05T02:58:06Z
---

Approved 2026-10-04 ("i think lets fix everything then push").

## 1. Implementation

- [x] 1.1 Usage line counts user-started turns, `1 turn` / `N turns`; every
  surface that prints the session usage line follows.
- [x] 1.2 Resume attributes restored usage per model from the usage log's
  last record for the session when its totals match; manifest rule otherwise.
- [x] 1.3 Both forms of `smith sessions list` omit sessions without a user
  message; `LATEST PROMPT` header; plugin docs say latest prompt.
- [x] 1.4 Remove an empty session's files when the interactive surface ends
  it.
- [x] 1.5 GLM quick start proposes `glm-5.3` from a new trusted record;
  catalog revision bumped; older records kept.
- [x] 1.6 User-visible counts read `1 agent` / `2 agents`, `1 compaction`,
  and so on, through one helper; tool-result text the model reads is left
  alone. Found in the live check (`1 agent(s)` in the exit report).
- [x] 1.7 Setup's and `/connect`'s catalog model list uses compact sizes
  (`131k context`), not raw integers. Found in the live check.
- [x] 1.8 Child agents' sessions are not listed by the resume pickers or
  either form of `smith sessions list`. Found in the live check
  (`child-session-…` listed after a `sol` child ran).

## 2. Verification

- [x] 2.1 Tests for each; fixtures re-recorded and reviewed; gate.
- [x] 2.2 Live: a one-prompt session with tool calls exits with `1 turn`;
  resuming the GLM-then-Gemini session prices both models; an empty session
  leaves no files; piped listing; setup review shows `glm-5.3`.
