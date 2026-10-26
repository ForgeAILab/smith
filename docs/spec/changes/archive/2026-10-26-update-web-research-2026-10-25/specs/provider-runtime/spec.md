## ADDED Requirements

### Requirement: Hosted citations link each source once with its destination

An adapter SHALL link each hosted-search source at most once per assistant
message, at its first cited span, in addition to skipping a URL the cited text
already contains. The Gemini adapter SHALL resolve each distinct grounding
redirect URL on the provider's redirect host to its destination before
rendering, by one request that does not follow the redirect or read a body,
under a per-request timeout and a total budget per response. It SHALL use the
destination only when the response is a redirect to an absolute `http` or
`https` URL, and SHALL keep the redirect URL otherwise. The resolved URL SHALL
be used in both the rendered link and the hosted-search event. The adapter
MUST NOT send any request to a destination host while resolving.

#### Scenario: A Gemini citation names the real page

- **GIVEN** a Gemini response citing
  `https://vertexaisearch.cloud.google.com/grounding-api-redirect/<token>`
  that redirects to `https://releases.rs/`
- **WHEN** the adapter renders the response
- **THEN** the assistant text links `https://releases.rs/`
- **AND** the hosted-search event's source URL is `https://releases.rs/`

#### Scenario: Resolution fails

- **GIVEN** a grounding redirect that times out or answers without a redirect
- **WHEN** the adapter renders the response
- **THEN** the citation links the original redirect URL
- **AND** the turn completes without an error

#### Scenario: A page cited in several blocks is linked once

- **GIVEN** an assistant message whose first and third text blocks cite the
  same source URL
- **WHEN** the adapter renders the message
- **THEN** the URL is linked after the first block's cited span only
