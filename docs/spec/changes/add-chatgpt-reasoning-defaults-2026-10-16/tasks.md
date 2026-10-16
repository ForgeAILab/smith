---
created_at: 2026-10-16T23:00:00Z
updated_at: 2026-10-16T23:00:00Z
completed_at:
---

## 1. Implementation
- [x] 1.1 Add a built-in reasoning branch for the exact ChatGPT endpoint,
      limited to trusted ChatGPT model records. It resolves the
      OpenAI-effort dialect, mandatory-on, ladder `low` through `ultra`,
      default effort `medium`, and a capability source naming the trusted
      ChatGPT binding.
- [x] 1.2 Explicit `[models]` reasoning settings keep their precedence over
      the built-in values.

## 2. Tests
- [x] 2.1 With no `[models]` block, `chatgpt/gpt-5.6-sol` and
      `chatgpt/gpt-6-astra` resolve the ladder, mandatory-on, default
      `medium`, and the openai-effort dialect. A request with `xhigh` emits
      that effort.
- [x] 2.2 An explicit block (for example a different `default_effort`)
      overrides the built-in value.
- [x] 2.3 An unknown model id on the ChatGPT endpoint still resolves no
      inferred control.

## 3. Verification
- [ ] 3.1 Full gate.
- [ ] 3.2 Live: with a scratch config that has no ChatGPT `[models]` blocks,
      `/status` for a Sol or Astra profile shows the effort ladder.
