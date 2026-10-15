## ADDED Requirements

### Requirement: Hosted web search declaration

A provider request SHALL be able to declare hosted web search with an
optional per-response search cap. Each adapter SHALL either map the
declaration to its provider's hosted search or report it unsupported through
capabilities before any request is sent:

- Responses and ChatGPT: `web_search`.
- xAI: `web_search`.
- Anthropic: `web_search_20250305`.
- Gemini: `google_search`.

Chat Completions and command providers SHALL report it unsupported.

#### Scenario: Anthropic request

- **GIVEN** an Anthropic session with hosted search declared and a cap of 5
- **WHEN** the request is serialized
- **THEN** its tools include `{"type":"web_search_20250305","name":"web_search","max_uses":5}`

#### Scenario: Unsupported adapter

- **GIVEN** an OpenAI-compatible Chat Completions provider
- **WHEN** capabilities are read
- **THEN** hosted web search is reported unsupported

### Requirement: Hosted search results are normalized

An adapter SHALL turn the provider's hosted-search output into canonical
hosted-search events that carry the query and the source titles and URLs. It
SHALL render citations into the assistant text as Markdown links, and SHALL
add the provider-reported search count to usage. Provider-opaque blocks
(encrypted content, hosted call ids, raw grounding payloads) SHALL NOT be
stored in canonical history or replayed in later requests. A session that
used hosted search MUST continue on another provider without errors.

#### Scenario: Responses search with citations

- **GIVEN** a Responses stream with a `web_search_call` item and output text
  carrying a `url_citation` annotation
- **WHEN** the adapter processes it
- **THEN** one hosted-search event is emitted with the query and source URL
- **AND** the assistant text holds the cited span followed by a Markdown link
- **AND** no `web_search_call` item is kept in canonical history

#### Scenario: Continue on another provider

- **GIVEN** a session whose earlier Anthropic turn used hosted search
- **WHEN** the user switches to a GLM model and sends a prompt
- **THEN** the request carries the earlier assistant text and no
  `server_tool_use` or `web_search_tool_result` block

#### Scenario: Organization has not enabled search

- **GIVEN** an Anthropic response rejecting the web search server tool
- **WHEN** the error is classified
- **THEN** Smith reports once that hosted search is unavailable for this
  organization
- **AND** the session falls back to backend routing
