## ADDED Requirements

### Requirement: Web research instruction section

Smith SHALL contribute a web-research instruction section only when
`web_fetch`, `web_search`, or provider-hosted search is on the run's surface.
The section SHALL tell the model to check claims against primary sources,
starting from search results and fetching a page when a snippet leaves a claim
unsettled; to keep to the source budget its task sets, or about ten pages when
the task sets none, and then report what is still open; to cite the page URLs
it read; to continue a page shown only in part through `artifact.read` at the
offset its result names; and to keep reports concise with one source list. The
section SHALL NOT tell the model to fetch every source it cites. It SHALL be positioned after the unconditional sections and
SHALL NOT change any unconditional section's identity or revision.

#### Scenario: A run with web tools

- **GIVEN** a run whose surface includes `web_fetch`
- **WHEN** the context is assembled
- **THEN** the web-research section is present after every unconditional
  section
- **AND** it names a source budget of about ten pages for a task that sets
  none

#### Scenario: A run without web tools

- **GIVEN** a run with `[tools.web_search] mode = "off"` and `modules.web`
  disabled
- **WHEN** the context is assembled
- **THEN** no web-research section is present
- **AND** every unconditional section carries the revision it carried before
  this change
