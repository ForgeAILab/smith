## MODIFIED Requirements

### Requirement: Smith built-ins use ability activation

Smith SHALL register its built-in coding tools and standard harness components
through Agent Runtime abilities with accurate affordances, typed permission
upper bounds, risk, context cost, readiness, provenance, and revision. The
provider tool surface MUST be materialized from a frozen activation epoch.
Every epoch SHALL contain the core tools the session's posture and capability
limits authorize (`read`, `list`, `search`, `edit`, `shell`, `task_output`,
`task_stop`) from the first provider request, without depending on
retrieval. Capabilities outside the core SHALL enter an epoch only through
pre-activation or the agent's own discovery.

#### Scenario: Core tools on the first request
- **GIVEN** a build-posture session with no capability limits
- **WHEN** the first provider request is sent, whatever the prompt says
- **THEN** the tool list contains `read`, `list`, `search`, `edit`, `shell`,
  `task_output`, and `task_stop`

#### Scenario: Read-only posture
- **GIVEN** a profile whose posture is read-only
- **WHEN** the first provider request is sent
- **THEN** the tool list contains `read`, `list`, and `search`
- **AND** `edit` and `shell` are not advertised

#### Scenario: Core set does not fit
- **GIVEN** a capability budget smaller than the authorized core tools
- **WHEN** the session starts
- **THEN** startup fails and names the budget
- **AND** no core tool is silently dropped

## ADDED Requirements

### Requirement: Agent-driven capability discovery

The agent SHALL be able to enumerate the capabilities its session
authorizes, search them by name, summary, and description text, and activate
chosen ones by registry id for the next provider request, within the
capability budget. A search or listing SHALL return only authorized entries.
A search with no match SHALL say so and report how many capabilities exist
per domain.

#### Scenario: Browse without a query
- **WHEN** the agent calls `registry.search` with an empty query
- **THEN** the result lists authorized capabilities by id and one-line summary
- **AND** nothing is activated by the listing

#### Scenario: Activate by name
- **GIVEN** the agent has seen `skill:release-notes` in a listing
- **WHEN** it calls `registry.activate` with that id
- **THEN** the capability is active on the next provider request

#### Scenario: No match is not no capability
- **GIVEN** a query that matches nothing
- **WHEN** the search returns
- **THEN** the result states that nothing matched
- **AND** reports the number of available capabilities per domain

#### Scenario: Description text matches
- **GIVEN** a capability whose description mentions "port scan" and whose
  keyword list does not
- **WHEN** the agent searches for "port scan"
- **THEN** the capability is among the candidates

#### Scenario: Unknown or unauthorized id
- **WHEN** the agent calls `registry.activate` with an id outside its
  authorized view
- **THEN** the call returns an error and nothing is activated
