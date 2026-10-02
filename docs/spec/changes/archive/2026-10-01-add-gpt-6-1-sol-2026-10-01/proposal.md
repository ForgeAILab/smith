---
created_at: 2026-10-02T00:03:17Z
updated_at: 2026-10-02T00:31:27Z
---

# Proposal: Add GPT-6.1 Sol and release Smith 0.2.15

## Why

Smith 0.2.14 predates GPT-6.1 Sol. Its compiled direct ChatGPT and installed
Codex model lists omit the model even though the installed Codex catalog
advertises it. Offline installs also lack its endpoint-bound metadata.

## What Changes

- Add `gpt-6.1-sol` to the direct ChatGPT and installed Codex catalogs.
- Preserve the Codex-advertised 272k default and 872k extended context windows
  and Smith's conservative 16,384-token direct ChatGPT output ceiling.
- Refresh the embedded Models.dev catalog from its canonical public source.
- Expose reviewed OpenAI Platform 1m and 272k context choices for the model.
- Verify, publish, and locally install Smith 0.2.15.

## Impact

- Affected specs: `configuration`, `client-surfaces`.
- Affected code: trusted setup data, installed-agent model choices, endpoint
  context windows, the embedded Models.dev snapshot, and release metadata.
- The change is additive; existing profiles and model choices keep working.

## Authorization

The user explicitly requested adding `gpt-6.1-sol` and releasing a new version.
That request authorizes this scoped model update, tests, version bump, commit,
release tag, publication through the existing release workflow, and updating
the installed Smith binary. Unrelated uncommitted work stays in the original
checkout.

## Evidence

- OpenAI model specifications and effort ladder:
  https://developers.openai.com/api/docs/models/gpt-6.1-sol
- Installed Codex `models_cache.json`: exact `gpt-6.1-sol` entry advertises
  272,000 context tokens and an 872,000-token extended context window.
- Models.dev source: https://models.dev/api.json
