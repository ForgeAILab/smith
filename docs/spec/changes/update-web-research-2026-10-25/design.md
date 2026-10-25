## Context

Measurements come from seven live headless runs on 2026-10-25 against the
owner's configuration (`research` and `orchestrator` on Gemini 4 Argon with
hosted search, `code` on glm-5.3 with the DuckDuckGo backend). Accuracy was
spot-checked against the cited blog posts and source files and held in every
case; every problem below is cost, latency, or lost sources.

The read-only child filter lives in Agent Runtime
(`delegation/lifecycle.rs`): a `ToolViewScope::ReadOnly` spec and any
`WorkspacePolicy::ReadOnlyView` both apply `tool_is_read_only`, which admits
only tools whose permission upper bound is within `FsRead`, `ClockRead`, and
`RandomRead`. `web_fetch` and backend `web_search` declare network and
data-egress permissions, so they never reach a read-only child. Hosted search
is a provider capability rather than a tool and does reach children, which is
why the `research` children still answered.

## Goals / Non-Goals

- Goals: research children can fetch and search; offloaded results are read
  back in a few calls; citations name real pages without dominating the text;
  agents are told to research from primary sources and report concisely.
- Non-Goals: a new deep-research mode or tool, provider deep-research models,
  JavaScript rendering, PDF extraction, raising the default inline threshold,
  or changing which tools the root's read-only posture admits.

## Decisions

- **Allowlist in the runtime, filled by the host.** `DelegationConfig` gains
  `read_only_tool_names: Vec<String>`, a sibling of `delegation_tool_names`.
  Both read-only filters admit a tool that is in this list or passes
  `tool_is_read_only`. The runtime does not decide what is safe to admit; Smith
  passes exactly the tools its existing `read_only_network_modules` review
  admits to a read-only posture, so the root and children apply one rule. The
  neutral runtime default is an empty list, so other hosts are unaffected.
  - Alternative considered (works today, no release): spawn a read-only-posture
    profile with `tools: "all"` and `workspace: "shared"`. Verified live: the
    child gets `web_fetch` and no `edit` or `shell`. Rejected as the fix
    because it depends on the model choosing a shared workspace, gives up the
    runtime's read-only view, and leaves build-posture children with a
    read-only scope unable to fetch. It remains the interim workaround.
- **Page bound from the output ceiling.** Artifact reads are already exempt
  from offloading, so the inline threshold is the wrong reference. A quarter
  of `limits.tool_output_limit_bytes` keeps the old headroom for JSON escaping
  and byte arrays (up to four characters per byte) and gives 16 KiB at
  defaults, under the runtime's 64 KiB `MAX_ARTIFACT_READ_BYTES`. A 35 KB
  report becomes 3 reads instead of 18. Models may still request less.
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
  read-only root, approvals still apply to the child's calls, and the owner's
  existing switch (`read_only_network_modules = []`) turns it off for both.
- Larger pages put more bytes in each request; total bytes read are the same
  or lower because fewer requests resend the conversation.
- Redirect resolution adds latency at the end of a cited response (bounded by
  the total budget) and depends on Google's redirect service behaving as
  measured. Failure degrades to today's output.

## Migration Plan

1. Land the prerequisite fixes on `fix/web-research-reads` (done in Smith).
2. Agent Runtime branch: allowlist, redirect resolution, once-per-message
   rendering, and the upstream page-edge fix; release one version.
3. Smith: pin that release, wire the allowlist, change the page bound, add the
   guidance; gate, push a branch for CI, then merge to main.
4. Runtime work merges to runtime main in the same cycle.

## Open Questions

- Should the child allowlist default to the root's list (proposed) or require
  a separate opt-in?
- Is 16 KiB the right default page, or should the research profile also raise
  `tool_output_inline_bytes` in the owner's configuration?
- Is a request to Google's redirect host per cited source acceptable, or
  should resolution be opt-in?
