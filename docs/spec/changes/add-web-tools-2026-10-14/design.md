## Context

No Smith tool reaches the web. `generate_image` is the only built-in tool
with a network effect, and it calls a fixed provider endpoint. In Agent
Runtime pin `db8dff2`:

- Responses rejects `web_search_call`, `x_search_call`, `mcp_call`, and
  `computer_call` items as unsupported hosted items.
- Anthropic does not parse `server_tool_use` or `web_search_tool_result`.
- Gemini ignores grounding metadata.
- No adapter keeps citation annotations.

Read-only posture keeps only tools whose permissions are `FsRead` or
`ClockRead` (`read_only_extension`), so any network tool is removed while
planning.

## Goals / Non-Goals

- Goals:
  - Search and read the web from every provider.
  - Use the provider's own search where it exists.
  - Safe by default: SSRF-guarded, GET-only, host-scoped approval, content
    labelled untrusted.
  - Available in `plan` posture.
- Non-Goals:
  - A browser, JavaScript, PDF text extraction, authenticated or POST
    requests, and a fetch cache.
  - Hosted search on Chat Completions.
  - Price accounting for hosted searches (counts only).

## Decisions

### D1. One `web` module, two tools

The module is compiled in and default on. It contributes `web_fetch` always,
and the `web_search` function tool only in backend mode. Hosted search is not
a Smith tool, because the provider executes it, so in hosted mode the module
contributes nothing for search and the factory sets the request's hosted-tool
declaration instead.

### D2. Fetch pipeline

1. Validate the URL: `http`/`https` only, no userinfo, length ≤ 2,048.
2. Resolve DNS. Reject loopback, RFC 1918, CGNAT, link-local (including
   169.254.169.254), unique-local IPv6, and unspecified addresses unless
   `allow_private_networks` is set. Connect to the vetted address, so a
   second DNS answer cannot rebind it.
3. GET with a fixed `User-Agent: Smith/<version> (+web_fetch)`. Send no
   cookies or credentials. Accept `text/html`, `text/*`, `application/json`,
   `application/xml`, and `image/*`.
4. Follow redirects manually. `ModuleHttp` already refuses automatic ones.
   Allow at most 5, repeating step 2 on every hop.
5. Stop reading at 5 MiB. Time out at 30 s overall.
6. Convert by type:
   - HTML: extract the main content, dropping `script`, `style`, `nav`,
     `footer`, and `aside`, then convert to Markdown, keeping links.
   - Text, JSON, and XML: unchanged.
   - Images: an image part, up to the 5 MiB bound.
   - Anything else: a tool error naming the type.
7. Output starts with a header: final URL, status, content type, and title.
   Then `Content from <final URL> (external, untrusted):`, then the body. The
   existing artifact offloader bounds what stays inline.

Crate choice for step 6: `htmd = 0.3.2` (Apache-2.0), with direct
`html5ever = 0.35.0` (MIT) and `markup5ever_rcdom = 0.35.0+unofficial`
(MIT / Apache-2.0) dependencies matching the parser and DOM already used by
htmd. Smith walks that DOM for main-content extraction and DuckDuckGo
results, then htmd converts the selected content to Markdown. This removes
Scraper and its unmaintained `fxhash` dependency without adding a browser,
CSS selector engine, or readability framework. The versions are pinned to
the Rust 1.88-era releases rather than requiring newer compiler features.
Sources: https://docs.rs/crate/htmd/0.3.2,
https://docs.rs/crate/html5ever/0.35.0, and
https://docs.rs/crate/markup5ever_rcdom/0.35.0+unofficial (published manifests
and source reviewed). Readability-style extraction is a small Smith-owned
pass over the parsed tree, not a port.

### D3. Hosted search in the runtime

The runtime gains a canonical `HostedTool::WebSearch { max_uses }` on
`ProviderRequest`. Each adapter maps it, or reports it unsupported through
capabilities, so routing is decided from capabilities rather than by
provider name:

| Adapter | Wire |
|---|---|
| Responses (OpenAI, xAI) and Smith's ChatGPT adapter | `{"type":"web_search"}`; parse `web_search_call` items; `url_citation` annotations |
| Anthropic | `{"type":"web_search_20250305","name":"web_search","max_uses":N}`; parse `server_tool_use` and `web_search_tool_result`; text-block `citations`; `usage.server_tool_use.web_search_requests` |
| Gemini | `{"google_search":{}}`; parse `groundingMetadata` (queries, chunks) |
| Chat Completions, command | unsupported, so the backend tool is used |

Stream output adds `ProviderStreamEvent::HostedToolActivity { kind, query,
sources }`, which becomes a canonical runtime event. Citations become
`ContentPart::Text` with the cited source appended as a Markdown link.
Opaque provider blocks (`encrypted_content`, call ids) are **not** stored in
canonical history and are never replayed. Each provider accepts later
requests without them: Responses with `store=false` needs no
`web_search_call` items, and Anthropic accepts an assistant turn that keeps
only its text. This keeps history portable across providers, and Smith has
already paid once for cross-provider history bugs. The hosted-search count is
recorded on usage.

### D4. Routing in `auto` mode

The decision is made at composition from the active model's capability:
hosted when supported, otherwise the configured backend, otherwise
`duckduckgo`. It is re-decided at each composition, so `/model` switches
follow it. A child resolves its own.

| `mode` | Route |
|---|---|
| `auto` (default) | hosted → configured `backend` → `duckduckgo` |
| `hosted` | hosted, else none (with a reason) |
| `backend` | configured `backend`, default `duckduckgo` |
| `off` | none |

If the hosted route fails at runtime (for example, Anthropic search not
enabled for the organization), `auto` falls back for the rest of the session.

### D5. Backends

| Backend | Request | Auth |
|---|---|---|
| DuckDuckGo (free) | `GET https://html.duckduckgo.com/html/?q=` and parse `a.result__a` (unwrapping the `uddg` redirect) and `.result__snippet` | none |
| Brave | `GET https://api.search.brave.com/res/v1/web/search?q=&count=` | `X-Subscription-Token` |
| Tavily | `POST https://api.tavily.com/search` `{query, max_results}` | `Authorization: Bearer` |
| SearXNG | `GET <url>/search?q=&format=json` | none (the instance must enable JSON) |

All four normalize to `[{title, url, snippet}]`, at most 10 results.
DuckDuckGo is best-effort: Smith detects its anomaly/challenge page and
non-200 answers and returns a tool error that says it was rate-limited and
names the keyed backends. It gets a fixture-based parser test, plus an opt-in
live test that catches layout drift, like the CLI drift canaries. Keys
use the existing credential-reference path. A repository-layer config cannot
set `searxng.url` or the backend, by the same rule that keeps repository
configuration from redirecting user state.

### D6. Posture

The user chooses. `[agent] read_only_network_modules` (user scope only,
because it bears authority) lists the reviewed module ids whose tools
read-only postures (`plan`, `review`) may keep. It defaults to
`["web", "vision"]`. The posture filter admits such a tool when its effects
are network read plus data egress, with no filesystem write or external
write; ordinary approval still applies. `[]` restores the strict filter.
Unreviewed MCP and plugin tools are never admitted through this key. Hosted
search follows the `web` entry, because it is part of the provider response.

### D7. Approval and resources

`web_fetch` prepares with the resource `web:<host>` and `web_search` with
`web-search:<backend>`, so the existing prepared-call-scoped auto-approval
rules can allow `web:docs.rs` without allowing every host.

## Risks / Trade-offs

- **Prompt injection through fetched content.** It is mitigated by the
  untrusted label and the separate tool result, not prevented. The agent's
  instructions already treat tool output as data.
- **Hosted search cost** is easy to miss. The docs say it, and
  `mode = "backend"`/`"off"` exist.
- **HTML extraction quality** varies by site. A raw-text fallback is used
  when extraction yields almost nothing.
- **Anthropic's web search must be enabled for the organization.** A 4xx
  that names the server tool is reported once, and that session falls back
  to backend mode.

## Migration Plan

1. Runtime branch (on top of `feat/smith-visual-tool-results`): hosted-tool
   declaration, events, and the three adapters, with its own spec change.
2. Smith: configuration, the `web` module (`web_fetch` first, then
   backends), posture, ChatGPT adapter hosted search, and display.
3. Live check: docs lookup on ChatGPT (hosted), on Anthropic (hosted), and
   on GLM (Brave backend).

## Open Questions

- None blocking. The HTML-to-Markdown crate is chosen in task 1.1 and
  recorded here.
