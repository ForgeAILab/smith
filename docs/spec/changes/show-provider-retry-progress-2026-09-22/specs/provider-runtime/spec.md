## ADDED Requirements

### Requirement: Provider retry decisions are observable

Agent Runtime SHALL expose the provider loop's actual retry decision on the
finished-attempt event using redaction-safe, serde-compatible metadata: the
finished attempt index, configured maximum attempts, and the effective delay
before the next attempt. A delay MUST be present only when policy and the
remaining turn deadline have admitted another attempt; error retryability by
itself MUST NOT be presented as proof that a retry will occur.

#### Scenario: Retry is admitted with exponential backoff

- **GIVEN** the first provider attempt fails with a retryable server error
- **AND** the configured attempt and turn-time budgets admit another attempt
- **WHEN** the provider loop finishes the failed attempt
- **THEN** its finish event identifies attempt 1 of the configured total
- **AND** carries the effective delay before attempt 2
- **AND** the next attempt begins only after that cancellable delay

#### Scenario: Provider retry hint lengthens the delay

- **GIVEN** a retryable provider response carries a valid `Retry-After` longer
  than the local exponential backoff
- **WHEN** the runtime schedules the next attempt
- **THEN** the finish event carries the provider-directed effective delay
- **AND** no raw response header or credential material enters the event

#### Scenario: Attempt budget is exhausted

- **GIVEN** the final configured attempt fails with an otherwise retryable
  provider error
- **WHEN** the provider loop applies the retry policy
- **THEN** the finish event identifies the final attempt and configured total
- **AND** carries no scheduled retry delay
- **AND** the turn ends through the existing provider-attempt limit outcome

#### Scenario: Legacy attempt event is replayed

- **GIVEN** a journal contains a provider-attempt finish event written before
  retry decision metadata existed
- **WHEN** a current runtime or Smith client deserializes it
- **THEN** the optional retry metadata is absent
- **AND** the event's existing finish, retryability, and error fields remain
  readable
