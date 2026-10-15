## ADDED Requirements

### Requirement: Web fetch tool

Smith SHALL provide a `web_fetch` tool that GETs one `http` or `https` URL and
returns its content as text. HTML SHALL be reduced to its main content and
converted to Markdown with links preserved. `text/*`, JSON, and XML SHALL be
returned as is. `image/*` SHALL be returned as an image part. Any other
content type SHALL be a tool error naming the type. The tool SHALL bound
each fetch at 5 MiB, 30 seconds, and 5 redirects. It SHALL send no cookies or
credentials, and SHALL use a fixed Smith User-Agent. The result SHALL begin
with the final URL, status, content type, and page title, followed by a
line marking the body as external, untrusted content from that URL.

#### Scenario: Read a documentation page

- **WHEN** the agent calls `web_fetch` with `https://docs.rs/serde/latest/serde/`
- **THEN** the result begins with the final URL, `200`, and `text/html`
- **AND** the body is Markdown of the page's main content with its links

#### Scenario: Oversized download

- **GIVEN** a URL serving 40 MiB
- **WHEN** the agent fetches it
- **THEN** Smith stops reading at 5 MiB and returns the converted prefix
  with a note that it was truncated

#### Scenario: Unsupported type

- **WHEN** the URL serves `application/zip`
- **THEN** the tool returns an error naming `application/zip`

### Requirement: Web fetch refuses private destinations

Smith SHALL resolve the destination of every request and redirect hop before
connecting. It SHALL refuse loopback, private, carrier-grade NAT,
link-local, unique-local, and unspecified addresses unless
`tools.web_fetch.allow_private_networks` is true. It SHALL connect to the
address it vetted. Only `http` and `https` URLs without userinfo SHALL be
accepted.

#### Scenario: Metadata address

- **WHEN** the agent fetches `http://169.254.169.254/latest/meta-data/`
- **THEN** the tool returns an error that the destination is a private
  address
- **AND** no connection is made

#### Scenario: Redirect into a private network

- **GIVEN** a public URL that redirects to `http://10.0.0.5/admin`
- **WHEN** the agent fetches it
- **THEN** the redirect is refused at the second hop

#### Scenario: Local docs server allowed

- **GIVEN** `[tools.web_fetch] allow_private_networks = true`
- **WHEN** the agent fetches `http://localhost:3000/docs`
- **THEN** the page is returned

### Requirement: Web search through a configured backend

When the search route is a backend, Smith SHALL provide a `web_search` tool
that takes `query` (1 to 400 characters) and an optional `max_results`
(1 to 10, default 5). It SHALL query the selected backend (DuckDuckGo, Brave,
Tavily, or SearXNG) and return each result's title, URL, and snippet,
labelled as external, untrusted content. Backend errors SHALL be tool errors
that carry the backend name and HTTP status, without credentials. DuckDuckGo
needs no key. Smith SHALL query DuckDuckGo by POSTing the search form to its
HTML endpoint without a User-Agent header, and SHALL retry once on its lite
endpoint when the HTML endpoint answers with a challenge page, a non-200
status, or a transport failure. Results that point back at DuckDuckGo (ads
and internal links) SHALL be dropped. A rate-limit or challenge from both
endpoints SHALL be reported as a tool error that names the keyed backends.

#### Scenario: Brave backend on a text-only provider

- **GIVEN** a GLM main model and `[tools.web_search] backend = "brave"` with a
  stored key
- **WHEN** the agent searches `tokio select cancellation safety`
- **THEN** Smith calls the Brave web search API with the key header
- **AND** returns up to 5 titled results with URLs and snippets

#### Scenario: Free search with no configuration

- **GIVEN** a GLM main model and no `[tools.web_search]` settings
- **WHEN** the agent searches `ratatui scrollbar example`
- **THEN** Smith queries DuckDuckGo with no key
- **AND** returns titled results with unwrapped target URLs

#### Scenario: DuckDuckGo lite fallback

- **GIVEN** DuckDuckGo's HTML endpoint answers HTTP 202 with its challenge page
- **WHEN** the agent searches
- **THEN** Smith POSTs the query to the lite endpoint
- **AND** returns the lite results with their direct URLs and snippets

#### Scenario: DuckDuckGo rate-limits

- **GIVEN** both DuckDuckGo endpoints answer with the challenge page
- **WHEN** the agent searches
- **THEN** the tool returns an error that DuckDuckGo rate-limited the
  request and names `brave`, `tavily`, and `searxng` as alternatives

#### Scenario: Search turned off

- **GIVEN** `[tools.web_search] mode = "off"`
- **WHEN** the tool list is built
- **THEN** `web_search` is absent and no hosted search is declared

### Requirement: User-chosen network modules in read-only posture

Smith SHALL let read-only postures keep tools from the reviewed modules
listed in `[agent] read_only_network_modules`. The list defaults to
`["web", "vision"]`. A tool qualifies only if its effects are network read
and data egress, with no filesystem or external write. Smith SHALL still
apply the approval policy to these tools. Unreviewed MCP and plugin tools
SHALL remain excluded from read-only posture whatever the list says.

#### Scenario: Plan posture reads documentation

- **GIVEN** a profile in `plan` posture and the default list
- **WHEN** the tool list is built
- **THEN** `web_fetch` is present
- **AND** `edit` and unreviewed MCP tools are absent

#### Scenario: Vision while planning

- **GIVEN** a text-only main model, a vision model, `plan` posture, and the
  default list
- **WHEN** the tool list is built
- **THEN** `view_image` is present

#### Scenario: User restores the strict filter

- **GIVEN** `[agent] read_only_network_modules = []` in user configuration
- **WHEN** a `plan` profile builds its tool list
- **THEN** neither `web_fetch` nor `view_image` is present

### Requirement: Host-scoped web approval

`web_fetch` SHALL prepare each call with the resource `web:<host>`, and
`web_search` with `web-search:<backend>`, together with network and
data-egress effects scoped to that destination. Prepared-call-scoped
automatic approval rules SHALL therefore be able to approve one host without
approving others.

#### Scenario: Auto-approve one documentation host

- **GIVEN** a user rule approving `smith/web_fetch` for resource `web:docs.rs`
- **WHEN** the agent fetches a `docs.rs` page and then a `example.com` page
- **THEN** the first call runs without a prompt
- **AND** the second call asks for approval
