## ADDED Requirements

### Requirement: Web tool settings

Smith SHALL resolve these keys through the existing layers with provenance:

- `[tools.web_fetch]`: `allow_private_networks` (bool, default false).
- `[agent] read_only_network_modules`: a list of reviewed module ids,
  default `["web", "vision"]`, user scope only.
- `[tools.web_search]`:
  - `mode`: `auto`, `hosted`, `backend`, or `off`; default `auto`.
  - `backend`: `duckduckgo`, `brave`, `tavily`, or `searxng`; default
    `duckduckgo`.
  - `api_key`: a credential reference resolved through the existing secret
    path.
  - `searxng_url`: an absolute `http(s)` URL.

Repository-controlled layers MUST NOT set `allow_private_networks`,
`read_only_network_modules`, `backend`, `api_key`, or `searxng_url`. An
unknown module id in `read_only_network_modules` SHALL be rejected with the
list of known ids. A `brave` or `tavily` backend
without a resolvable key, or `searxng` without a URL, SHALL be reported at
the key and SHALL leave backend search unavailable without failing the
session.

#### Scenario: Repository tries to open private networks

- **GIVEN** a project `.smith/config.toml` that sets
  `[tools.web_fetch] allow_private_networks = true`
- **WHEN** configuration loads
- **THEN** Smith rejects the key at the repository layer

#### Scenario: Missing backend key

- **GIVEN** `[tools.web_search] backend = "tavily"` and no stored key
- **WHEN** a session starts
- **THEN** the session starts
- **AND** `smith config explain tools.web_search` reports the missing key

### Requirement: Web search routing

Smith SHALL choose the search route when each runtime is composed:

- `hosted` requires hosted search support on the active provider and model,
  and otherwise offers no search, with a reported reason.
- `backend` uses the configured backend, which defaults to `duckduckgo`.
- `off` offers no search.
- `auto` uses hosted search when the active provider and model support it,
  otherwise the configured backend, otherwise `duckduckgo`.

Each composition SHALL re-decide the route, so a `/model` switch follows it,
and a child SHALL decide its own route.

#### Scenario: Auto on ChatGPT

- **GIVEN** `mode = "auto"` and a ChatGPT main model
- **WHEN** the session starts
- **THEN** requests declare hosted web search
- **AND** the `web_search` function tool is absent

#### Scenario: User prefers the free route everywhere

- **GIVEN** `mode = "backend"` and no `backend` key
- **WHEN** a ChatGPT session starts
- **THEN** no hosted search is declared
- **AND** `web_search` uses DuckDuckGo

#### Scenario: Auto after switching to GLM

- **GIVEN** `mode = "auto"`, a Brave backend, and a ChatGPT session
- **WHEN** the user switches with `/model` to a GLM model
- **THEN** after the rebuild, `web_search` is the backend function tool
