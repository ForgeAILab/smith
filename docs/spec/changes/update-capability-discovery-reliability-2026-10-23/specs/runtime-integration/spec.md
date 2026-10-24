## MODIFIED Requirements

### Requirement: Agent-driven capability discovery

The agent SHALL be able to enumerate the capabilities its session
authorizes, search them by name, summary, and description text, and activate
chosen ones by registry id for the next provider request, within the
capability budget. A search or listing SHALL return only authorized entries.
A search with no match SHALL say so and report how many capabilities exist
per domain. Every session SHALL be told, in one developer instruction, which
registered tools outside its pinned core it may activate. Two registered tools
SHALL NOT be treated as redundant only because they share permissions.

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

#### Scenario: Activatable tools are named up front
- **GIVEN** a session that registers `agent` and `web_search` but does not
  pin them, and whose limits deny `tool:web_search`
- **WHEN** the first provider request is planned
- **THEN** its instructions name `agent` as activatable
- **AND** they do not name `web_search` or any pinned core tool

#### Scenario: Web tools with equal permissions both stage
- **GIVEN** `web_fetch` and `web_search` registered with the same permissions
- **WHEN** the agent searches for "web search fetch"
- **THEN** both tools can bind, and neither is rejected as redundant
