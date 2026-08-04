---
created_at: 2026-08-02T02:44:11Z
updated_at: 2026-08-02T02:44:11Z
---

## Why

An agent running inside Smith is regularly asked to configure Smith itself:
edit `.smith/config.toml`, define a profile, wire a provider credential, or
explain approval and persistence behavior. At runtime the active workspace is
an arbitrary user project, so Smith's own reference documentation is not
present and the agent must guess or interrogate the binary. Smith already
resolves a deterministic skill catalog with a built-in layer
(`crates/smith-runtime/src/skills.rs`), but nothing ships in that layer.

## What Changes

- Ship built-in harness reference skills in the existing built-in skill
  layer. Initial set: configuration and agent profiles
  (`docs/configuration.md`), headless protocol (`docs/headless-protocol.md`),
  persistence and recovery (`docs/persistence-recovery.md`), and the security
  model (`docs/security.md`).
- Embed each skill body at compile time from the shipped reference document
  it mirrors, keeping the repository documentation the single source of
  truth. Authored names, descriptions, and keywords make each skill
  discoverable through descriptor-first retrieval without body I/O.
- Seed the shared `smith-runtime` factory with the built-in set so the TUI
  and `smith -p` expose one identical index; a direct embedder that supplies
  its own `SmithSkillSources` keeps full control.
- Preserve existing precedence and trust: built-ins are the lowest layer,
  carry host-policy trust, may be shadowed by user, trusted-workspace, or
  session declarations, and grant no tool, permission, credential, or
  approval authority when activated.

## Impact

- Affected specs: `harness-policy`
- Affected code: `crates/smith-runtime/src/skills.rs`,
  `crates/smith-runtime/src/factory.rs`, new
  `crates/smith-runtime/src/built_in_skills.rs`, `docs/skills.md`
