---
created_at: 2026-10-15T00:44:35Z
updated_at: 2026-10-15T09:15:10Z
completed_at:
---

Depends on `add-visual-tool-results` (image parts from `web_fetch`, runtime
branch base). The runtime branch lands and is pushed before Smith bumps pins.

## 1. Smith `web` module: fetch (no runtime dependency)

- [x] 1.1 Choose the HTML parsing and Markdown crate (license per
      `deny.toml`, maintenance, size); record it in `design.md`.
- [x] 1.2 `crates/modules/web`: crate, cargo feature, compiled-in list,
      default on, inactive reasons.
- [x] 1.3 `web_fetch`: URL validation, vetted DNS with a private-range guard,
      manual redirects, bounds, content-type conversion, untrusted header,
      `web:<host>` resource and effects.
- [x] 1.4 Tests with a local HTTP fixture server: HTML to Markdown,
      truncation, redirect into a private range, metadata IP, unsupported
      type, image part, and `allow_private_networks`.

## 2. Configuration

- [x] 2.1 `tools.web_fetch.allow_private_networks`; `tools.web_search`
      `mode`, `backend`, `api_key` (credential reference), and
      `searxng_url`; repository-layer refusal; `config explain`.
- [x] 2.2 Tests for every scenario in `specs/configuration/spec.md`.

## 3. Backend search

- [x] 3.1 DuckDuckGo (keyless HTML, redirect unwrapping, challenge
      detection), Brave, Tavily, and SearXNG clients normalized to
      `{title, url, snippet}`, with errors that never echo credentials.
      Add an opt-in live DuckDuckGo drift test.
- [x] 3.2 `web_search` function tool and the `web-search:<backend>`
      resource.
- [x] 3.3 Tests against fixture servers for each backend.

## 4. Posture and approval

- [x] 4.1 `[agent] read_only_network_modules` (user scope, default
      `["web", "vision"]`); the posture filter admits only the listed
      reviewed modules' network-read tools.
- [x] 4.2 Tests: plan posture includes `web_fetch` and excludes `edit` and
      unreviewed MCP tools; a host-scoped auto-approval rule matches one host
      only.

## 5. Agent Runtime: hosted search (new branch on the visual-results branch)

- [x] 5.1 Runtime spec change.
- [x] 5.2 Canonical hosted-search declaration on `ProviderRequest`, capability
      flag, hosted-search stream and runtime events, and a usage count.
- [x] 5.3 Responses (`web_search`, `url_citation`), xAI included.
- [x] 5.4 Anthropic (`web_search_20250305`, `server_tool_use`,
      `web_search_tool_result`, citations, usage count, organization-disabled
      classification).
- [x] 5.5 Gemini (`google_search`, `groundingMetadata`).
- [x] 5.6 No opaque block reaches canonical history; cross-provider
      continuation tests.
- [x] 5.7 Runtime gate; push; bump the Smith pins.

## 6. Smith hosted wiring

- [x] 6.1 The ChatGPT adapter (`smith-runtime/src/chatgpt`) maps the
      declaration and parses `web_search_call` and annotations.
- [x] 6.2 Routing per composition (`auto`, `hosted`, `backend`, `off`),
      fallback on organization-disabled, and children.
- [x] 6.3 Fake-provider tests: ChatGPT auto declares hosted with no function
      tool; switching to GLM yields the backend tool.

## 7. Display and docs

- [x] 7.1 `Web Fetch` and `Web Search` rows, hosted rows with sources, live
      and resumed.
- [x] 7.2 `docs/configuration.md` section (its own `##`, under the
      built-in-skill size ceiling), including hosted search cost.

## 8. Gate and live check

- [x] 8.1 Smith gate (`--no-fail-fast`, clippy 1.88, `--no-default-features`
      steps).
- [x] 8.2 Live: docs lookup on ChatGPT (hosted) and GLM (DuckDuckGo, then
      `web_fetch`); see `docs/qa/live-2026-10-15-web/`. Anthropic hosted and
      Brave were not run live (no native Anthropic provider configured, no
      Brave key); their fixture tests cover them.

## 9. Live-run follow-ups (2026-10-15)

- [x] 9.1 DuckDuckGo: POST the form without a User-Agent, fall back once to
      the lite endpoint, drop results that point at DuckDuckGo, update the
      live drift canary; fixture tests for both endpoints and the fallback.
- [x] 9.2 Runtime: hosted-search events carry the action (search, open
      page, find in page) with URL and pattern; the query falls back to
      `queries`; citations are not appended when the cited span already
      links the URL. Runtime gate; push; bump the Smith pins.
- [x] 9.3 Smith ChatGPT adapter mirrors 9.2; the Smith event, live and
      resumed transcript blocks, and headless output carry the action;
      `Open Page(<url>) · hosted` and `Find in Page("<pattern>") · hosted`
      rows.
- [x] 9.4 Smith gate; live: free search on GLM returns results and the
      ChatGPT hosted run shows page-open rows and no duplicated citation.
