---
created_at: 2026-10-03T00:00:00Z
updated_at: 2026-10-04T00:16:03Z
---

## Why

v0.3.2 made switching model mid-session easy, and switching between
providers breaks the session. With the owner's configuration, one turn on
the first provider and a resumed turn on the second (0.3.2, 2026-10-03):
Gemini → Z.AI, Gemini → xAI, xAI → Gemini, and Anthropic → Gemini all fail,
and Gemini → Anthropic sends Gemini's signature as Anthropic redacted
thinking. Reasoning parts carry provider-issued signatures with no record of
who issued them, so each adapter replays another provider's reasoning as its
own. The fix belongs in Agent Runtime
(`update-reasoning-history-provenance` on
`ForgeAILab/agent-runtime` branch `fix/smith-cross-provider-reasoning`).

## What Changes

- Bump the six Agent Runtime pins to the revision that records reasoning
  provenance and omits foreign reasoning from provider requests.
- Supply the new optional field wherever Smith builds a reasoning part (its
  ChatGPT Responses binding, the command-JSONL provider, image history, and
  tests).
- Verify with the live cross-provider matrix and the cache comparison, then
  drop the known issue from the v0.3.2 notes and QA record.

## Impact

- Affected specs: provider-runtime (Safe provider and model switching).
- Affected code: root `Cargo.toml` and `Cargo.lock` (runtime revision);
  `smith-runtime` `chatgpt.rs`, `command_provider.rs`, `image_history.rs`,
  `advisor.rs` tests; `smith-tui` tests that build reasoning parts.
- No change to configuration, persistence format compatibility (the field is
  optional), or same-model provider requests.
