---
created_at: 2026-10-25T19:42:24Z
updated_at: 2026-10-26T07:02:30Z
completed_at: 2026-10-26T07:02:30Z
---

## 0. Prerequisites (landed separately on `fix/web-research-reads`)

- [x] 0.1 `artifact.read` trims partial UTF-8 characters at page edges in
  Smith's reader, so text pages never come back as byte arrays.
- [x] 0.2 `web_fetch` carries the page body once, in content only.

## 1. Agent Runtime (one release)

- [x] 1.1 Add `RuntimeBuilder::read_only_tool_names`; admit named tools
  without write or spawn permissions in both read-only child filters; tests for
  scope `read_only`, a read-only workspace with scope `all`, an unnamed
  network tool staying excluded, and named create and spawn tools staying
  excluded.
- [x] 1.2 Resolve Gemini grounding redirect citations to their `Location`
  target (no follow, no body, bounded per request and in total, failure keeps
  the redirect) through a host-installed `CitationResolver`; tests with a
  fixture resolver and a local redirect server.
- [x] 1.3 Link each hosted source once per assistant message across text
  blocks; update renderer tests.
- [x] 1.4 Cut `ArtifactReadTool` pages at a character boundary, deciding text
  by media type (upstream copy of 0.1) so other hosts get it.
- [x] 1.5 Release, merge to runtime main (v0.12.0, `9f3ff2f`, PRs #79 and #80).

## 2. Smith

- [x] 2.1 Pin the runtime release and drop Smith's own page-edge trim (0.1),
  which cannot see the media type, now that the runtime reader decides by it.
- [x] 2.2 Name the tools a non-write-capable child build holds through
  `read_only_tool_names`, and admit plugin tools to it by the root's read-only
  MCP rule; child-view tests for the default list in three spawn shapes and
  for `read_only_network_modules = []`.
- [x] 2.2a Install Smith's grounding redirect resolver on the Gemini adapter.
- [x] 2.3 Set `delivery_inline_bytes` from `limits.tool_output_limit_bytes`
  and pass `agent` outcomes carrying a completed child's reply through the
  offloader up to that ceiling; delivery and `agent result` tests with a
  30,000-byte reply.
- [x] 2.4 Add the conditional web-research instruction section and the
  research sentence in root delegation guidance; cache-prefix and base-harness
  ceiling tests.
- [x] 2.5 Update `docs/configuration.md` (web tools, child agents, delivery
  budget), `docs/context-working-set.md`, and `docs/security.md`.
- [x] 2.7 Keep `web_fetch` outcomes inline up to the output ceiling; above
  it, preview the page's beginning with the continuation offset; offloader
  tests for a 30 KB page, a 200 KB page read on from the named offset, and a
  30 KB shell result that still offloads.
- [x] 2.8 Soften the web-research section (fetch when a snippet leaves a claim
  unsettled, source budget of about ten pages, continuation offset) and add
  the questions-and-budget sentence to root delegation guidance; bump both
  fragment revisions; regenerate the headless fixtures.
- [x] 2.9 Drop page chrome, hidden elements, image URLs, empty links and
  same-page links in HTML conversion; conversion tests for each rule and for
  a whole-page `form`.
- [x] 2.10 Update `docs/configuration.md` and `docs/context-working-set.md`
  for inline fetched pages.
- [x] 2.6 Remove the interim spawn sentence from the owner's orchestrator
  instructions once 2.2 ships (backup
  `~/.smith/config.toml.bak-before-web-research-20261025`): done 2026-10-26.
  The main build c29dce6f is installed at `~/.local/bin/smith`, with the
  previous build kept as `smith-main-69d686ad-backup`. The interim "Spawn
  `research` children the same way…" sentence was removed from the owner's
  `orchestrator` and `orchestrator-pro` profiles (config backup
  `config.toml.bak-before-drop-research-spawn-20261026`), and Smith loads all
  three profiles. The owner's `tool_output_inline_bytes = 65536` overrides
  were left in place.

## 3. Verification

- [x] 3.1 Full gate (`--no-fail-fast`, 1.88 Clippy, CI's extra steps). The
  full locked gate ran on the pre-rebase tip 93d28a87: 3,333 passed, 0
  failed, with fmt, both Clippy toolchains, fixtures, no-default-features,
  smoke and deny all clean. After rebasing onto main, the headless fixtures
  were re-recorded (token counts only, c29dce6f) and the smith-runtime and
  web tests passed (1,064 passed, 0 failed); the full gate was not rerun on
  the merged tip. CI passed on all four jobs (run 40273121970); the Linux
  stable job's first attempt timed out on the known rare runtime conformance
  hang `queued_interrupted_turn_does_not_contaminate_history` and passed on
  rerun.
- [x] 3.2 Rerun the research live test and record calls, time, tokens, and
  citation bytes against the 2026-10-25 baseline (the code and orchestrator
  reruns are 3.2b, deferred below). Runtime pin v0.12.0, branch build,
  default inline settings: 61.6 s, 4 requests, 36.7K uncached + 39.1K cached
  tokens, 2.0K output, 5 searches, 11 `web_fetch`, 0 `artifact.read`; the
  answer had real blog.rust-lang.org URLs and one source list. Before:
  82.8 s, 70.2K + 52.2K tokens, 6 fetches, 4 `artifact.read`.

## Deferred to a follow-up change

Archived 2026-10-26 without this. The owner deferred it on 2026-10-26: the
orchestrator rerun costs about 17M tokens. No result is claimed for it.

- 3.2b Rerun the code and orchestrator live tests and record calls, time,
  tokens, and citation bytes against the 2026-10-25 baseline.
