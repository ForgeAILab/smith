---
created_at: 2026-10-15T00:44:35Z
updated_at: 2026-10-15T09:15:10Z
---

## Why

A Smith agent cannot search the web or read an online page. No built-in tool
reaches the network except `generate_image`. The runtime rejects every
provider-hosted tool item (`responses.rs` `unsupported_hosted_item`) and
drops citations. The only routes are a user-declared MCP server or `curl`
through `shell`, which has no bounds, no HTML handling, and no
per-destination approval. An agent that cannot read current documentation
guesses at APIs.

Owner direction on 2026-10-14: web access is required, and broad support
matters more than a single best path. The user chooses the route and the
backend. A free option must work with no key: plain `web_fetch` always, and
keyless search through DuckDuckGo. Provider-hosted search and keyed APIs
(Brave, Tavily, SearXNG) are choices on top of that.

## What Changes

- **`web_fetch`** (built-in, in the compiled-in module `web`, default on):
  - GETs one `http(s)` URL and returns readable text. HTML becomes Markdown
    of the main content. Text, JSON, and XML pass through. Images become
    image parts, which the visual-results projection then handles.
  - Bounded: 5 MiB download, 30 s, at most 5 redirects, each hop
    re-validated.
  - Refuses loopback, private, link-local, and metadata addresses unless
    `[tools.web_fetch] allow_private_networks = true`.
  - Large pages go through the existing artifact offloader.
  - Declares network and data-egress effects scoped to the destination host,
    so approval and auto-approval rules can match by host.
- **`web_search`** returns titles, URLs, and snippets. Routing, controlled by
  `[tools.web_search] mode = "auto" | "hosted" | "backend" | "off"`
  (default `auto`):
  - **Hosted**, when the active provider and model support it: OpenAI and
    ChatGPT Responses `web_search`, xAI `web_search`, Anthropic
    `web_search_20250305`, Gemini `google_search`. The provider runs the
    search inside the response. Smith declares the hosted tool and records
    each search and its sources as display events. Citations are rendered
    into the assistant text as links. Provider-opaque blocks are never
    replayed, so history stays cross-provider safe.
  - **Backend** otherwise: a `web_search` function tool backed by the user's
    choice of `duckduckgo` (free, no key, best-effort), `brave`, `tavily`,
    or `searxng`, with keys through the existing secret path.
  - In `auto` the order is hosted, then the configured backend, then
    `duckduckgo`, so search works out of the box on every provider. The
    user picks anything else with `mode` and `backend`.
- **Untrusted content.** Fetched pages and search results are labelled with
  their source URL as external content. They never become instructions.
- **The user chooses which network modules work in read-only postures.**
  `[agent] read_only_network_modules` is user scope only. It defaults to
  `["web", "vision"]`, so research and `view_image` work while planning,
  still subject to approval; `[]` restores today's strict behavior. This
  supersedes the visual-results scenario that keeps `view_image` out of
  `plan`.
- **Display.** `Web Fetch(host/path)` and `Web Search("query")` rows with
  result counts. Hosted searches render as `Web Search("query") · hosted`
  rows with the sources found.

## Impact

- Affected specs: `tool-execution`, `configuration`, `provider-runtime`,
  `tool-call-display`
- Affected code:
  - Smith: a new `crates/modules/web` (fetch, HTML to Markdown, backends),
    `smith-config` (`tools.web_fetch`, `tools.web_search`), posture
    filtering in `smith-runtime/src/factory/capabilities.rs`, the ChatGPT
    adapter's hosted tool declaration and parsing, `smith-tools/src/display.rs`,
    and `smith-tui` hosted-search rows.
  - Agent Runtime (a new branch on top of the visual-results branch, with its
    own spec change): a canonical hosted-tool declaration on
    `ProviderRequest`, hosted search events plus citation annotations, and
    adapter support in `responses.rs`, `anthropic.rs`, and `gemini.rs`.
    Then the pins are bumped.
  - Dependencies: an HTML-to-Markdown crate (choose a maintained,
    pure-Rust one; see `design.md`).
- Compatibility: additive. Without configuration, every session gains
  `web_fetch` and search: hosted on ChatGPT, OpenAI, xAI, Anthropic, and
  Gemini, and DuckDuckGo elsewhere.
- Cost: hosted searches are billed per search by the provider (Anthropic
  publishes $10 per 1,000). `mode = "backend"` with `backend = "duckduckgo"`
  is free.
- DuckDuckGo has no official search API. Smith reads its HTML results page,
  so it can rate-limit or change layout. Smith reports that plainly and
  names the keyed backends.
- Security: SSRF guard on every hop, a GET-only verb, host-scoped approval,
  content labelled as untrusted, no cookies or credentials sent, and a fixed
  User-Agent.
- Not in scope: PDF text extraction, JavaScript rendering or a headless
  browser, authenticated fetches, POST or form submission, a response cache,
  and hosted search on OpenAI Chat Completions endpoints.
