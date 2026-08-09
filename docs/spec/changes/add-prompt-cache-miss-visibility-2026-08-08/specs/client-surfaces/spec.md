## ADDED Requirements

### Requirement: Consistent prompt-cache visibility

Smith SHALL project the same canonical cache state and derived missed-token
facts through the interactive footer, `/status`, exit/session summary, final
JSON, and streaming JSON. The footer's `CH` value SHALL represent the latest
completed root turn's provider-reported cache-read share of total prompt input,
including billed failed attempts. Explicit zero MUST render as `0%` and absent
evidence MUST render as unknown.

#### Scenario: Completed turn reports a zero cache read

- **GIVEN** a root turn has reported prompt-input usage
- **AND** its provider explicitly reports zero cache-read tokens
- **WHEN** the turn completes
- **THEN** the footer renders `CH 0%`
- **AND** `/status` and machine output retain the matching canonical state

#### Scenario: Cache-read evidence is absent

- **GIVEN** a root turn reports input usage but no cache-read observation
- **WHEN** the turn completes
- **THEN** the footer renders cache hit rate as unknown
- **AND** no surface turns the omission into zero or a miss

#### Scenario: TUI and final JSON consume the same events

- **GIVEN** a deterministic turn includes a partial cache miss and one failed
  retry
- **WHEN** it is reduced by the TUI and headless hosts
- **THEN** both report equivalent state, expected, observed, missed, and
  cache-read percentage values
- **AND** stream JSON retains the attempt-level canonical events

### Requirement: Cache notices remain local presentation

An interactive cache-miss notice SHALL be a bounded local transcript block and
MUST NOT enter canonical conversation history or provider context. Human
headless mode SHALL write an enabled significant miss notice to stderr while
keeping answer stdout unchanged.

#### Scenario: User sends another prompt after a miss notice

- **GIVEN** a cache-miss notice is visible in the transcript
- **WHEN** the user sends the next prompt
- **THEN** the notice is absent from the provider request
- **AND** the canonical user and assistant history is unchanged by the notice

#### Scenario: Headless text output reports a miss

- **GIVEN** notices are enabled for a headless text run
- **AND** its completed turn crosses a significance threshold
- **WHEN** Smith exits successfully
- **THEN** stdout contains only the requested answer
- **AND** stderr may contain the factual cache-miss diagnostic
