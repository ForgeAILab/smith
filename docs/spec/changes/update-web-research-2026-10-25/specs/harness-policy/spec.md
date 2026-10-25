## ADDED Requirements

### Requirement: Web research instruction section

Smith SHALL contribute a web-research instruction section only when
`web_fetch`, `web_search`, or provider-hosted search is on the run's surface.
The section SHALL tell the model to check claims against fetched primary
sources, cite the page URLs it read, read a stored page through
`artifact.read` when the excerpt is not enough, and keep reports concise with
one source list. It SHALL be positioned after the unconditional sections and
SHALL NOT change any unconditional section's identity or revision.

#### Scenario: A run with web tools

- **GIVEN** a run whose surface includes `web_fetch`
- **WHEN** the context is assembled
- **THEN** the web-research section is present after every unconditional
  section

#### Scenario: A run without web tools

- **GIVEN** a run with `[tools.web_search] mode = "off"` and `modules.web`
  disabled
- **WHEN** the context is assembled
- **THEN** no web-research section is present
- **AND** every unconditional section carries the revision it carried before
  this change
