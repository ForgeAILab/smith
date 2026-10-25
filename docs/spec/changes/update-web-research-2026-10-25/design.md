## Context

Measurements come from seven live headless runs on 2026-10-25 against the
owner's configuration (`research` and `orchestrator` on Gemini 4 Argon with
hosted search, `code` on glm-5.3 with the DuckDuckGo backend). Accuracy was
spot-checked against the cited blog posts and source files and held in every
case; every problem below is cost, latency, or lost sources.

The read-only child filter lives in Agent Runtime
(`delegation/lifecycle.rs`, predicate in `delegation/persistence.rs`): a
`ToolViewScope::ReadOnly` spec and any `WorkspacePolicy::ReadOnlyView` both
apply `tool_is_read_only`, which admits only tools whose permission upper
bound is within `FsRead`, `ClockRead`, and `RandomRead`. That drops
`web_fetch` and backend `web_search` (network, data egress), and also
`artifact.read` and `artifact.list` (host-defined `artifact.read` permission).
Hosted search is a provider capability rather than a tool and does reach
children, which is why the `research` children still answered.

## Owner decisions (2026-10-25)

1. A child should not lose tools it needs; it has every necessary tool unless
   configuration says otherwise.
2. Reports should come back as text, not through pages; only extra data or
   sources warrant artifacts. The proposed 16 KiB page change is dropped.
3. Resolving citation redirects is fine.

## Goals / Non-Goals

- Goals: research children can fetch, search, and read their own artifacts;
  child reports reach the parent as text in one step; citations name real pages
  without dominating the text; agents research from primary sources and report
  concisely.
- Non-Goals: a new deep-research mode or tool, provider deep-research models,
  JavaScript rendering, PDF extraction, changing the default inline threshold
  or artifact page size, or granting write access by default.

## Decisions

- **Read-only means no writes, decided by the host.** `DelegationConfig` gains
  `read_only_tool_names: Vec<String>`, a sibling of `delegation_tool_names`.
  Both read-only filters admit a tool that passes `tool_is_read_only`, or that
  is listed and whose permission bound has no `FsWrite`, `HostFsWrite`,
  `ExternalWrite`, or `ProcessSpawn`. The guard keeps a mistaken list from
  granting a write. Smith fills the list with the names a read-only-posture
  session holds on each child route, so the root and children apply one rule
  and the owner's existing settings narrow both. The neutral runtime default is
  an empty list, so other hosts are unaffected.
  - Alternative considered: invert the runtime predicate to deny only write
    permissions. Rejected because host-defined permissions (for example
    `artifact.read`) cannot be classified by the runtime, and Smith's
    `read_only_network_modules` setting would not be honored.
  - Interim workaround, verified live: spawn a read-only-posture profile with
    `tools: "all"` and `workspace: "shared"`; the child gets `web_fetch` and no
    `edit` or `shell`. Applied to the owner's orchestrator instructions until
    this ships.
- **Reports inline up to the output ceiling.** Smith sets the runtime's
  `delivery_inline_bytes` from `limits.tool_output_limit_bytes` (64 KiB, below
  the runtime's 1 MiB maximum) instead of the inline threshold, and its output
  offloader passes an `agent` outcome that carries a completed child's reply
  through up to the same ceiling. Both are Smith-only. In the measured run all
  three child reports (20–35 KB) would have arrived whole, removing the 45
  reads spent on them; a batch of several long replies still falls back to
  excerpts for the ones that do not fit.
- **Resolve redirects in the adapter.** Gemini's `url_citation` annotations
  and grounding chunks carry only `url` (the redirect) and `title` (usually the
  domain); the search-result step carries no URLs (checked with one direct
  Interactions request). A plain GET of the redirect answers `302` with the
  destination in `Location`, so the adapter can resolve each unique redirect
  once, without following it or reading a body, before rendering. Requests go
  only to the redirect host the provider named, with a short per-request
  timeout and a total budget; any failure keeps the redirect link.
- **One link per source per message.** The renderer already skips a URL the
  cited block contains; it now also skips a URL linked earlier in the same
  assistant message. Gemini emits a distinct redirect per grounding chunk, so
  this only helps after resolution, which is why the two ship together.
- **Guidance is conditional.** The web-research section is contributed only
  when `web_fetch`, `web_search`, or hosted search is on the surface and sits
  after the unconditional sections, so runs without web tools keep their
  cached prefix and the base-harness ceiling is unaffected.

## Risks / Trade-offs

- Admitting `web_fetch` to read-only children widens what a prompt-injected
  child can send out (URL parameters). The same exposure already exists for a
  read-only root, approvals still apply to the child's calls, and
  `read_only_network_modules = []` turns it off for both.
- Inline reports put up to 64 KiB of reply in one parent request. The parent
  read those bytes anyway; it now does so in one request instead of dozens.
- Redirect resolution adds latency at the end of a cited response (bounded by
  the total budget) and depends on Google's redirect service behaving as
  measured. Failure degrades to today's output.

## Migration Plan

1. Land the prerequisite fixes on `fix/web-research-reads` (done in Smith).
2. Agent Runtime branch: read-only allowlist with write guard, redirect
   resolution, once-per-message rendering, and the upstream page-edge fix;
   release one version.
3. Smith: pin that release, fill the allowlist, raise the delivery budget and
   pass `agent` replies through, add the guidance; gate, push a branch for CI,
   then merge to main.
4. Runtime work merges to runtime main in the same cycle.

## Open Questions

- None blocking. Whether write access should also be granted by default
  (today it needs a write posture, `tools` of `all`, and a writable workspace)
  is out of scope unless the owner asks for it.
