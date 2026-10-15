# Live run: web tools (2026-10-15)

Release build of `feat/web-tools` (`1254ee5`) in a private tmux server, using
the owner's real `~/.smith` config and an empty scratch project with no web
settings (so `mode = "auto"`, backend DuckDuckGo). Approvals were `ask`. The
task: find the latest stable Rust release, fetch the official announcement,
quote its first sentence, and cite sources.

| Run | Main model | Route | Result |
|---|---|---|---|
| W1 | `chatgpt/gpt-6.1-sol` | Hosted search (ChatGPT) | **Correct** in 10 s: Rust 1.99.0 (Oct 1, 2026). Hosted rows showed the query and source hosts, and citations were rendered as links. |
| W2 | `zai/glm-5.3` | Backend `web_search` (DuckDuckGo) + `web_fetch` | **Correct** in 1m47s. DuckDuckGo returned its challenge (HTTP 202) on both searches, and the tool error named the keyed backends. The agent fell back to `web_fetch` on blog.rust-lang.org (redirect followed, Markdown with the untrusted header, large page offloaded and read back via `artifact.read`). |
| W3 | `zai/glm-5.3` | Backend `web_search` (DuckDuckGo), after the fixes below | **Works.** The first search returned 8 results, including the 1.99.0 announcement and the `/releases/latest` redirect; a quoted follow-up query returned 0 results without a challenge. The answer cited only search results. [Capture](captures/w3-glm-duckduckgo.txt). |
| W4 | `chatgpt/gpt-6.1-sol` | Hosted search, after the fixes below | **Correct** in 14 s. Rows read `Web Search("site:blog.rust-lang.org …") · hosted` and `Open Page(https://blog.rust-lang.org/releases/latest) · hosted`; each citation appears once. [Capture](captures/w4-chatgpt-hosted.txt). |

## Findings

All three were fixed on `fix/web-followups` (Smith) and agent-runtime
`49e8197`, then rechecked in W3 and W4.

1. **DuckDuckGo blocked the free default.** The cause was the
   User-Agent, not only the method. Probes from this machine, same query:
   a GET got HTTP 202 and the challenge with any User-Agent; a POST that
   identified itself (`Smith/0.4.0 (+web_fetch)` or curl's default) also
   got 202; a POST with no User-Agent got 200 and 9 to 10 results from both
   `html.duckduckgo.com/html/` and `lite.duckduckgo.com/lite/`. Search now
   POSTs the form with no User-Agent (it does not pretend to be a browser;
   `web_fetch` still identifies as Smith), falls back once to the lite
   page, and drops results that point back at DuckDuckGo. This stays
   best-effort: DuckDuckGo can change what it blocks.
2. **Hosted page-open steps rendered as `Web Search("")`.** Responses
   `web_search_call` items whose action is `open_page` or `find_in_page`
   carry no query. Hosted-search events now carry the action, its URL and
   pattern; the rows read `Open Page(<url>) · hosted` and
   `Find in Page("<pattern>") · hosted`. The query also falls back to
   `queries`, which replaced the deprecated `query`.
3. **Citations were repeated.** The model writes its own link,
   `([blog.rust-lang.org](https://blog.rust-lang.org/releases/))`, and the
   adapter appended ` [The Rust Release Announcements](…)` for the same
   URL. Adapters (runtime and Smith's ChatGPT copy) no longer append a link
   to a URL the text block already contains.

## Observations (not fixed)

- In W3, GLM first said it had no `web_search` tool, then activated it and
  got `already_available`. The tool was in the request; the model misread
  it. W2 showed the same hesitation.
