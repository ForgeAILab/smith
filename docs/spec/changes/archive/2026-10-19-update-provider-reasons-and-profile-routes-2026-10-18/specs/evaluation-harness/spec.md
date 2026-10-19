## ADDED Requirements

### Requirement: Live provider conformance suite

Smith SHALL provide an ignored, opt-in live test that drives the built (or a
named) `smith` binary headlessly against routes from the owner's real
configuration, named in `SMITH_LIVE_CONFORMANCE_MODELS`. For every route it
SHALL run a two-read tool turn, a resumed tool-using continuation, a turn
killed after its first tool result and then resumed, and, for routes named in
`SMITH_LIVE_CONFORMANCE_SEARCH`, a hosted-search-plus-tool turn and a resumed
follow-up; for every ordered pair of distinct routes it SHALL run a tool turn
on the first and a resumed tool-using turn on the second. It MUST NOT move
the owner's home or configuration, MUST remove only the session files its own
runs created, and MUST write `report.json` and `report.md` with each
scenario's status, session, usage, cache-read percentage, and on failure
Smith's full error text and the stderr tail. Cache percentages SHALL be
reported and never asserted.

#### Scenario: A provider rejects a resumed turn
- **GIVEN** a route whose provider rejects the resumed half of a scenario
- **WHEN** the suite finishes
- **THEN** the report marks that scenario failed and quotes Smith's error,
  including the provider's checked reason
- **AND** the remaining scenarios still run and the test fails at the end

#### Scenario: Nothing is configured
- **WHEN** the suite runs without `SMITH_LIVE_CONFORMANCE_MODELS`
- **THEN** it spends no provider quota and does nothing
