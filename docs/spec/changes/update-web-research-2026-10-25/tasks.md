---
created_at: 2026-10-25T19:42:24Z
updated_at: 2026-10-25T19:42:24Z
completed_at:
---

## 0. Prerequisites (landed separately on `fix/web-research-reads`)

- [x] 0.1 `artifact.read` trims partial UTF-8 characters at page edges in
  Smith's reader, so text pages never come back as byte arrays.
- [x] 0.2 `web_fetch` carries the page body once, in content only.

## 1. Agent Runtime (one release)

- [ ] 1.1 Add `DelegationConfig::read_only_tool_names`; admit listed tools in
  both read-only child filters; tests for scope `read_only`, a read-only
  workspace with scope `all`, and an unlisted network tool staying excluded.
- [ ] 1.2 Resolve Gemini grounding redirect citations to their `Location`
  target (no follow, no body, bounded per request and in total, failure keeps
  the redirect); tests with a fixture redirect server.
- [ ] 1.3 Link each hosted source once per assistant message across text
  blocks; update renderer tests.
- [ ] 1.4 Cut `ArtifactReadTool` pages at a character boundary (upstream copy
  of 0.1) so other hosts get it.
- [ ] 1.5 Release, merge to runtime main.

## 2. Smith

- [ ] 2.1 Pin the runtime release.
- [ ] 2.2 Fill `read_only_tool_names` from the tools that
  `agent.read_only_network_modules` admits; child-view tests for the default
  list and for `[]`.
- [ ] 2.3 Derive the `artifact.read` page bound from
  `limits.tool_output_limit_bytes / 4`, capped at the runtime maximum; update
  the reader tests and `/context` reporting.
- [ ] 2.4 Add the conditional web-research instruction section and the
  research sentence in root delegation guidance; cache-prefix and base-harness
  ceiling tests.
- [ ] 2.5 Update `docs/configuration.md` (web tools, child agents, artifact
  pages).

## 3. Verification

- [ ] 3.1 Full gate (`--no-fail-fast`, 1.88 Clippy, CI's extra steps).
- [ ] 3.2 Rerun the three live tests (research, code, orchestrator) and record
  calls, time, tokens, and citation bytes against the 2026-10-25 baseline.
