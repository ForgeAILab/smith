---
created_at: 2026-10-25T19:42:24Z
updated_at: 2026-10-25T20:20:00Z
---

# Web research that reaches and reads its sources

Status: Stage 1 proposal, revised with the owner's answers of 2026-10-25.
Nothing here changes behavior until approved.

## Why

Live runs on Smith 0.8.0 (2026-10-25) show that research answers are accurate
when they arrive, but the plumbing around them is slow, expensive, and loses
sources:

- An orchestrator asked to "do deep research" on four projects spawned four
  `research` children. All four had `web_fetch` rejected ("unknown or not
  authorized"). A read-only child view keeps only tools whose permissions are
  filesystem, clock, or random reads, so it also drops `web_search`,
  `view_image`, and the child's own `artifact.read` and `artifact.list`. The
  children answered from provider-hosted search snippets alone; one stalled
  and the root redid its work.
- The same run made 73 `artifact.read` calls and used about 884K input tokens
  in 7 minutes. Child reports of 20–35 KB exceeded the 8 KiB inline threshold,
  were delivered as 1 KiB excerpts, and were read back 2 KiB per call, each
  call resending the whole conversation.
- 34–48% of each child report's bytes were inline Gemini citation links,
  `[domain](https://vertexaisearch.cloud.google.com/grounding-api-redirect/…)`,
  167–240 bytes each, 29–79 per report. The link hides the cited page, and a
  page cited in several text blocks is linked again in each.

Two defects found in the same runs are fixed separately on
`fix/web-research-reads` and are prerequisites here: `artifact.read` returned
a JSON array of byte values whenever a page edge split a UTF-8 character, and
`web_fetch` stored each page twice.

## What Changes

- **A read-only child is denied only writes.** It holds every tool a
  read-only-posture session on the same route holds: artifact reader and
  lister, web tools admitted by `[agent] read_only_network_modules`, image
  inspection, todos, and question relay. Only tools that can modify the
  workspace, write elsewhere, or spawn a process are withheld. Configuration
  (`read_only_network_modules`, an explicitly named tool scope) is the only way
  to narrow it. Agent Runtime gains a host allowlist for read-only child views,
  guarded so a listed tool that writes is still refused. **Behavior change for
  every read-only child.** Write access still needs a write posture, `tools`
  of `all`, and a writable workspace, as today.
- **Reports come back as text.** A child's reply is delivered, and returned by
  `agent result`, inline up to the per-call output ceiling
  (`limits.tool_output_limit_bytes`, default 64 KiB) instead of the 8 KiB
  offloading threshold. Only larger replies become excerpt and artifact.
  Artifact page sizes are unchanged.
- **Compact hosted citations.** The Gemini adapter resolves grounding redirect
  links to their destination URL with one bounded, non-following request each,
  and every hosted adapter links a source once per assistant message. A failed
  resolution keeps the redirect.
- **Research guidance.** When web tools or hosted search are on the run's
  surface, a short conditional instruction section asks for primary sources,
  real page URLs, and concise reports; root delegation guidance asks research
  children for findings as text with one source list, naming artifacts for
  bulky data.

## Impact

- Affected specs: `child-agents`, `context-working-set`, `provider-runtime`,
  `harness-policy`.
- Affected code: Agent Runtime `delegation/{types,lifecycle,persistence}.rs`,
  `agent-runtime-provider/src/{gemini,hosted}.rs`, `harness/artifacts.rs`
  (upstream copy of the page-edge fix); Smith `smith-runtime/src/delegation.rs`,
  `tool_output.rs`, prompt sections, and the runtime pin.
- Requires one Agent Runtime release and a Smith pin move, bundling all runtime
  work here. The delivery-budget and `agent result` changes are Smith-only.
- Overlaps `add-jev-agent-mode-and-two-layer-delegation-2026-10-23` only in
  `child-agents`; neither change edits the other's requirements.
