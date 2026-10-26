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

Owner decisions (2026-10-26), after the reruns:

4. Fetched pages come back inline up to the output ceiling, and the fetch
   guidance is softened with a source budget; both ship together.
5. Pages should be converted so the agent reads the main content. Conversion
   was already Markdown of `<main>`/`<article>`; the rerun pages were mostly
   source files and JSON, so this change removes the measured HTML noise
   rather than adding a new extractor.

## Goals / Non-Goals

- Goals: research children can fetch, search, and read their own artifacts;
  child reports reach the parent as text in one step; citations name real pages
  without dominating the text; agents research from primary sources and report
  concisely.
- Non-Goals: a new deep-research mode or tool, provider deep-research models,
  JavaScript rendering, PDF extraction, changing the default inline threshold
  for other tools or the artifact page size, a hard fetch limit, readability
  scoring or model-summarized fetches, trimming JSON responses, or granting
  write access by default.

## Decisions

- **Read-only means no writes, decided by the host.** `RuntimeBuilder` gains
  `read_only_tool_names(names)`, set by the child factory on the builder it
  returns. Both read-only filters admit a tool that passes `tool_is_read_only`,
  or that is named and whose permission bound has no `FsWrite`, `FsCreate`,
  `FsDelete`, `HostFsWrite`, `ExternalWrite`, or `ProcessSpawn`. The guard
  keeps a mistaken list from granting a write. Smith's `child_builder` already
  applies the read-only posture (module posture, network-module admission,
  read-only built-ins, the root's MCP rule) to a child that may not write, so
  it names exactly the tools it built. The root and children apply one rule
  and the owner's existing settings narrow both. The default is an empty list,
  so other hosts are unaffected.
  - Implementation note: the approved draft put a static list on
    `DelegationConfig`. Module tool names exist only after each spawn mounts
    its modules, so a static list would have meant hardcoded names or a second
    mount. The builder carries the same list per spawn; behavior is unchanged.
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
  - The adapter's transport is pinned to the Interactions origin, so lookups
    go through a host-installed `CitationResolver`
    (`GeminiInteractionsProvider::with_citation_resolver`): at most 64 unique
    redirects, 8 in flight, 3 s in total, stopped by cancellation or the
    request deadline. Runtime ships `GroundingRedirectResolver` on its reqwest
    transport; Smith implements the trait on its own client (reqwest 0.13) so
    the binary keeps one HTTP stack.
- **One link per source per message.** The renderer already skips a URL the
  cited block contains; it now also skips a URL linked earlier in the same
  assistant message. Gemini emits a distinct redirect per grounding chunk, so
  this only helps after resolution, which is why the two ship together.
- **Fetched pages inline up to the output ceiling.** The offloader passes a
  `web_fetch` outcome through when its serialization fits
  `limits.tool_output_limit_bytes`, the same rule as a child's reply. In the
  rerun, 64 KiB covers about 80% of the offloaded pages; the default-settings
  cost came from 2 KiB read-backs, each resending the conversation, so a page
  read once inline costs one request instead of ten.
  - A longer page is still stored whole by the runtime offloader, and its
    preview becomes the page's beginning up to the ceiling (less 1 KiB for the
    reference and note) instead of a 1 KiB excerpt. The note names the byte
    offset in the stored outcome where the unshown text starts, so a
    continuing read neither repeats the beginning nor guesses. The offset is
    found by encoding the shown prefix the way the stored outcome encodes it;
    when it cannot be found the note says to read from the start.
  - The ceiling is the same per-call output bound the runtime applies to the
    rendered result, so an inline page is never cut by that later bound.
  - An owner who raised `tool_output_inline_bytes` to 64 KiB already had
    inline pages; the gain is for default settings and for pages above the
    ceiling.
  - Risk: each inline page stays in the child's context. With no limit on
    fetches, 50 inline pages would fill a window, which is why the source
    budget ships with it.
- **A soft source budget, in guidance.** The web-research section asks the
  model to start from search results, fetch a page when a snippet leaves a
  claim unsettled, prefer the one primary page that settles it, and keep to
  the task's source budget, or about ten pages without one, then report what
  is still open. Root delegation guidance asks the parent to give a research
  child its questions and a budget. There is no enforced limit: Smith's
  defaults stay simple, and the parent or a profile's instructions can set a
  tighter budget.
- **Page text without chrome.** Conversion keeps choosing `<main>`, then
  `<article>`, then the body, and gains these removals, all from the rerun
  pages or the elements Mozilla Readability treats as invisible: a `header`
  outside `article`, `aside`, `main`, `nav` and `section` that holds no `h1`
  (the page banner; a post's own title block often sits in one);
  `button`, `iframe`, `template`, `dialog`, `select`, `input`, `textarea`,
  `canvas`, `object`, `embed`, `video`, and `audio`; elements whose `role` is
  `navigation`, `banner`, `contentinfo`, `complementary`, `search`, `menu`,
  `menubar`, `dialog`, or `alertdialog`; and elements marked `hidden`,
  `aria-hidden="true"`, or `display: none` inline. An image becomes its alt
  text, which also drops `data:` URIs. A link with no text is removed (the
  heading permalinks were 53% of one release page's link bytes), and a link
  that points back into the same page, or that had no safe destination, keeps
  only its text; a link whose only text is a zero-width space counts as empty.
  `form` is kept because some sites wrap the whole page in one.
  - Measured on 15 sample pages (docs, blogs, Wikipedia, GitHub, MDN), the
    converted text shrank 19% in total: 31% on Wikipedia (navigation boxes),
    5–12% on documentation pages, with no body text lost in a review of the
    removed lines.
  - Alternative considered: a Readability port as a new dependency. Rejected
    for now: the measured noise is removed by these rules, and the expensive
    pages were not HTML.
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
- Inline fetched pages make each child request larger. The budget guidance
  is the counterweight; if a model ignores it, the context planner's normal
  compaction still applies.
- Removing `aria-hidden` and `hidden` elements could drop content on a page
  that hides its main content until a script runs. Those pages already need
  JavaScript to render, which is a non-goal.
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
