---
created_at: 2026-09-05T11:15:00Z
updated_at: 2026-10-05T19:58:49Z
completed_at: 2026-10-05T00:00:00Z
---

## Shipped

This change shipped in v0.2.0 without a tasks file; this records what landed.
The selection mechanism changed during implementation: an installed agent is
chosen by model id (`cli/claude-code/sonnet`), not a profile `harness` field,
and `[harness.<kind>]` holds only per-machine overrides. The configuration
delta was updated to the shipped behavior before archiving.

- [x] 1. Drive Claude Code and Codex as external agents with CLI session
  resume, usage, and zero-exit error detection (`83265bb`).
- [x] 2. Resolve owner-only `[harness.<kind>]` overrides; CLI-owned tools off
  by default (`01050bc`).
- [x] 3. Surface harness activity in the client event projection and render
  CLI-run tools distinctly (`e36a7b1`, `c3fcba3`, `20c60d5`).
- [x] 4. Select an installed agent by model id and offer installed-agent models
  in the picker (`8b8f2e9`, `d0ddce7`, `a1d101e`).
- [x] 5. Pin the runtime that makes a harness turn durable (`fa5c418`).
- [x] 6. Document installed coding agents in `docs/configuration.md`.
