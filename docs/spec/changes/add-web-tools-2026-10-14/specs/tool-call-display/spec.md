## ADDED Requirements

### Requirement: Web tool rows

The TUI SHALL render `web_fetch` as `Web Fetch(<host><path>)` and, once
complete, the final status and content type. It SHALL render backend
`web_search` as `Web Search("<query>")` with the number of results. A hosted
search event SHALL render as `Web Search("<query>") · hosted` with the
source hosts found. Live and resumed rows SHALL agree.

#### Scenario: Hosted search row

- **GIVEN** a ChatGPT turn whose response ran one hosted search with three
  sources
- **WHEN** the transcript renders
- **THEN** a `Web Search("<query>") · hosted` row lists the three source hosts

#### Scenario: Fetch row

- **WHEN** a `web_fetch` of `https://docs.rs/serde/` completes
- **THEN** the row reads `Web Fetch(docs.rs/serde/)` with `200 text/html`
