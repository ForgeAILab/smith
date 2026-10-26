---
created_at: 2026-10-25T19:42:24Z
updated_at: 2026-10-26T07:02:30Z
---

# Web research that reaches and reads its sources

Status: approved 2026-10-25; revised 2026-10-26 with the owner's answers to
the rerun findings (fetched pages inline, softer fetch guidance with a source
budget, cleaner page text). Shipped on main through c29dce6f and archived
2026-10-26; the code and orchestrator reruns are deferred (task 3.2b).

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

Reruns on the implementation (2026-10-26) fixed sources and citations but
moved the cost to page reads: four research children made 210 `web_fetch`
calls and 703 `artifact.read` calls, about 17M input tokens at default
settings. 110 fetched pages were over the 8 KiB inline threshold (median
20 KB, 90th percentile 98 KB); 81 were raw source files, 26 GitHub API JSON,
and 3 HTML. Each was offloaded to a 1 KiB excerpt and read back in 2 KiB
pages of escaped JSON. The guidance that made research fetch primary pages
("read the page itself rather than a search snippet") set no limit. The HTML
pages carried avoidable noise: an empty permalink link on every heading and
image URLs.

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
- **Fetched pages come back as text.** A `web_fetch` result stays inline up
  to the same per-call output ceiling (64 KiB by default). A longer page shows
  its beginning up to that ceiling, and the complete result is stored as an
  artifact whose reference names the `artifact.read` offset where the unshown
  text starts. Other tools keep the 8 KiB inline threshold.
- **Cleaner page text.** HTML conversion also drops page chrome (a page-level
  `header`, buttons, frames, form controls, media, landmark roles for
  navigation, banners, footers and dialogs, and hidden elements), replaces
  images with their alt text, removes links with no text, and keeps the text
  of links that point back into the same page or nowhere.
- **Research guidance.** When web tools or hosted search are on the run's
  surface, a short conditional instruction section asks for primary sources,
  real page URLs, and concise reports. It asks the model to start from search
  results, fetch a page when a snippet leaves a claim unsettled, and keep to a
  source budget (about ten pages when the task sets none). Root delegation
  guidance asks for research children's findings as text with one source
  list, naming artifacts for bulky data, and gives each research child its
  questions and a source budget.

## Impact

- Affected specs: `child-agents`, `context-working-set`, `provider-runtime`,
  `harness-policy`, `tool-execution`.
- Affected code: Agent Runtime `delegation/{types,lifecycle,persistence}.rs`,
  `agent-runtime-provider/src/{gemini,hosted}.rs`, `harness/artifacts.rs`
  (upstream copy of the page-edge fix); Smith `smith-runtime/src/delegation.rs`,
  `tool_output.rs`, prompt sections, `modules/web/src/html.rs`, and the runtime
  pin.
- Requires one Agent Runtime release and a Smith pin move, bundling all runtime
  work here. The delivery-budget, `agent result`, fetched-page, page-text and
  guidance changes are Smith-only.
- Overlaps `add-jev-agent-mode-and-two-layer-delegation-2026-10-23` only in
  `child-agents`; neither change edits the other's requirements.
