---
created_at: 2026-07-29T02:05:43Z
updated_at: 2026-07-29T03:13:32Z
---

## Why
Smith resolves Keychain credentials during every startup, which can repeatedly
open an operating-system password/access prompt. For the initial product,
storing an explicitly entered key in Smith's already owner-only user config is
the smallest self-contained no-prompt path.

## What Changes
- Add an explicit `api_key = "<value>"` provider setting accepted only from
  owner-only `~/.smith/config.toml`.
- Keep Keychain and environment references available, while setup offers
  “Store in config (no prompts)” with a plaintext-at-rest warning.
- Add a focused setup command for migrating an existing provider credential
  without re-entering its endpoint, model, or limits.
- Treat inline values as secret-bearing from deserialization through
  provenance, explanation, setup previews, diagnostics, runtime construction,
  events, journals, snapshots, and machine output.
- Require restrictive config permissions, atomic setup writes, redacted
  collision review, and exact rollback.

## Impact
- Affected specs: `configuration`, `client-surfaces`, `provider-runtime`
- Affected code: `crates/smith-config`, `crates/smith-cli`, setup documentation
- This change supersedes the reference-only credential rules in the active
  `add-smith-agent-harness` and `add-smith-setup-flow` changes for user config
  only. Project configuration remains unable to supply credential material.
