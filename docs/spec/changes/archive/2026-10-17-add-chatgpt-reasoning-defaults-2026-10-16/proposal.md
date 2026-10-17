---
created_at: 2026-10-16T23:00:00Z
updated_at: 2026-10-17T20:03:08Z
---

## Why

`/connect chatgpt` writes a reasoning block only for `gpt-5.6-terra`. Every
other trusted ChatGPT model (Sol, Luna, Astra, the GPT-6 models) resolves no
reasoning controls on the ChatGPT endpoint, so a new user who connects
ChatGPT and picks one of them gets no effort control. The owner hand-wrote
`[models."chatgpt/<model>".reasoning]` blocks to work around it. A September
draft (`cleanup/2026-10-01/agent-a0712…`, deleted in the 2026-10-16 branch
cleanup) wrote a block for every model at connect time. It never landed, and
the owner chose to port the fix.

## What Changes

- The exact ChatGPT endpoint (`https://chatgpt.com/backend-api/codex`)
  becomes trusted reasoning evidence for Smith's trusted ChatGPT model
  records, as the OpenAI and xAI endpoints already are. Those models
  resolve:
  - the OpenAI-effort dialect;
  - reasoning on by default, with no `off`;
  - efforts `low`, `medium`, `high`, `xhigh`, `max`, `ultra`;
  - default effort `medium`.
  This is the same block `/connect chatgpt` writes for Terra.
- An explicit `[models."chatgpt/<model>".reasoning]` block still wins, layer
  by layer, as for every other built-in default.
- A model id on the ChatGPT endpoint that is not a trusted record keeps
  resolving no inferred control.
- `/connect chatgpt` keeps writing the Terra block. It becomes redundant but
  stays harmless and explicit.

## Impact

- Affected specs: `provider-runtime`
- Affected code: `crates/smith-runtime/src/reasoning.rs` (built-in
  controls), possibly a helper in `crates/smith-config/src/setup.rs` to ask
  whether a provider/model is a trusted ChatGPT record.
