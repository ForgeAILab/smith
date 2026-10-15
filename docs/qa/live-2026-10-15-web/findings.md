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

## Findings (follow-ups, not blockers)

1. **DuckDuckGo blocks the free default quickly.** A plain GET to
   `html.duckduckgo.com/html/` got the challenge page on the first try.
   Widely used clients POST the query as form data and fall back to
   `lite.duckduckgo.com/lite/`. Without that, the free route rarely works.
2. **Hosted page-open steps render as `Web Search("")`.** Responses
   `web_search_call` items whose action is `open_page` or `find_in_page`
   carry no query. The row should name the action and its URL.
3. **Citation text duplicates the host.** The model's own source mention
   plus the rendered link reads "(blog.rust-lang.org (https://…))".
   Cosmetic.
