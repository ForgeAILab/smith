---
created_at: 2026-10-25T19:42:24Z
updated_at: 2026-10-25T19:42:24Z
---

# Web research that reaches and reads its sources

Status: Stage 1 proposal. Nothing here changes behavior until approved.

## Why

Live runs on Smith 0.8.0 (2026-10-25) show that research answers are accurate
when they arrive, but the plumbing around them is slow, expensive, and loses
sources:

- An orchestrator asked to "do deep research" on four projects spawned four
  `research` children. All four had `web_fetch` rejected ("unknown or not
  authorized"): a read-only child view drops every tool with network or
  data-egress effects, and Smith's `read_only_network_modules` applies only to
  the root posture. The children answered from provider-hosted search snippets
  alone; one stalled and the root redid its work.
- The same run made 73 `artifact.read` calls and used about 884K input tokens
  in 7 minutes. Child reports of 20–35 KB were read back 2 KiB per call, the
  page bound derived from the 8 KiB inline threshold, and every call resent
  the whole conversation.
- 34–48% of each child report's bytes were inline Gemini citation links,
  `[domain](https://vertexaisearch.cloud.google.com/grounding-api-redirect/…)`,
  167–240 bytes each, 29–79 per report. The link hides the cited page, and a
  page cited in several text blocks is linked again in each.
- With pages kept inline (`tool_output_inline_bytes = 65536`), the same
  five-release question went from 131 s, 12 `artifact.read` calls and ~100K
  input tokens to 50 s, none, and ~42K.

Two defects found in the same runs are fixed separately on
`fix/web-research-reads` and are prerequisites here: `artifact.read` returned
a JSON array of byte values whenever a page edge split a UTF-8 character, and
`web_fetch` stored each page twice.

## What Changes

- **Read-only children keep reviewed network-read tools.** Agent Runtime's
  `DelegationConfig` gains a host-reviewed tool allowlist admitted to
  read-only child views; Smith fills it from the module tools that
  `[agent] read_only_network_modules` already admits to read-only postures
  (`web_fetch`, backend `web_search`, `view_image` by default). Write tools stay
  excluded and the approval policy still applies. **Behavior change:** this
  applies to every read-only child, not only `research`; setting the list to
  `[]` restores today's strict view.
- **Larger artifact pages.** The `artifact.read` page bound derives from the
  per-call output ceiling (`limits.tool_output_limit_bytes`, default 64 KiB)
  instead of the inline threshold: one quarter of it, so 16 KiB by default
  instead of 2 KiB. Offloading thresholds and previews are unchanged.
- **Compact hosted citations.** The Gemini adapter resolves grounding redirect
  links to their destination URL with one bounded, non-following request each,
  and every hosted adapter links a source once per assistant message.
  A failed resolution keeps the redirect.
- **Research guidance.** When web tools or hosted search are on the run's
  surface, a short conditional instruction section asks for primary sources,
  real page URLs, and bounded reports; root delegation guidance asks research
  children for concise findings with one source list.

## Impact

- Affected specs: `child-agents`, `context-working-set`, `provider-runtime`,
  `harness-policy`.
- Affected code: Agent Runtime `delegation/{types,lifecycle}.rs`,
  `agent-runtime-provider/src/{gemini,hosted}.rs`, `harness/artifacts.rs`
  (upstream copy of the page-edge fix); Smith `smith-runtime/src/delegation.rs`,
  `tool_output.rs`, prompt sections, and the runtime pin.
- Requires one Agent Runtime release and a Smith pin move, bundling all runtime
  work here.
- Overlaps `add-jev-agent-mode-and-two-layer-delegation-2026-10-23` only in
  `child-agents`; neither change edits the other's requirements.
