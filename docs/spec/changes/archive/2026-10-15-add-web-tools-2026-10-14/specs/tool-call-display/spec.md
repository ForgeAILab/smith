## ADDED Requirements

### Requirement: Web tool rows

The TUI SHALL render `web_fetch` as `Web Fetch(<host><path>)` and, once
complete, the final status and content type. It SHALL render backend
`web_search` as `Web Search("<query>")` with the number of results. A hosted
search event SHALL render as `Web Search("<query>") · hosted` with the
source hosts found, or `Web Search · hosted` when the provider reported no
query. A hosted page-open step SHALL render as `Open Page(<url>) · hosted`,
and a hosted find-in-page step as `Find in Page("<pattern>") · hosted` with
the page URL beneath it. Live and resumed rows SHALL agree.

#### Scenario: Hosted search row

- **GIVEN** a ChatGPT turn whose response ran one hosted search with three
  sources
- **WHEN** the transcript renders
- **THEN** a `Web Search("<query>") · hosted` row lists the three source hosts

#### Scenario: Hosted page-open row

- **GIVEN** a ChatGPT turn whose hosted search opened
  `https://blog.rust-lang.org/2026/10/01/Rust-1.99.0/`
- **WHEN** the transcript renders, live or after resume
- **THEN** the row reads
  `Open Page(https://blog.rust-lang.org/2026/10/01/Rust-1.99.0/) · hosted`
- **AND** no `Web Search("")` row appears

#### Scenario: Fetch row

- **WHEN** a `web_fetch` of `https://docs.rs/serde/` completes
- **THEN** the row reads `Web Fetch(docs.rs/serde/)` with `200 text/html`
