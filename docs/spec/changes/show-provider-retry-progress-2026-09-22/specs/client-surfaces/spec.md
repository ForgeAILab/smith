## ADDED Requirements

### Requirement: Interactive provider retry progress is explicit

The interactive TUI SHALL distinguish an initial provider attempt, a scheduled
retry backoff, an in-flight retry, and final retry exhaustion using words and
attempt counts rather than color alone. Attempt totals and delays MUST come
from the runtime's retry decision metadata; when exact metadata is absent,
Smith MUST use bounded generic wording instead of fabricating progress.

#### Scenario: TUI waits through retry backoff

- **GIVEN** attempt 1 of 3 fails and the runtime schedules attempt 2 after a
  positive delay
- **WHEN** the TUI receives the finished-attempt event
- **THEN** the live row reads `Retrying 2/3` and shows the remaining backoff
- **AND** an informational notice leads with retry 2 of 3 before the bounded
  redaction-safe provider cause

#### Scenario: Retried provider request is still waiting

- **GIVEN** the runtime has started attempt 2 of 3 after a failed first attempt
- **AND** the provider has not produced output yet
- **WHEN** the TUI redraws the live row
- **THEN** it continues to read `Retrying 2/3`
- **AND** shows the existing sending-phase elapsed time so the wait is not
  confused with backoff or generic work

#### Scenario: Retry succeeds

- **GIVEN** a later provider attempt succeeds
- **WHEN** its output commits and the turn completes
- **THEN** the TUI clears retry progress
- **AND** does not render the transient attempt failure as a terminal error

#### Scenario: Retry budget is exhausted

- **GIVEN** attempt 3 of 3 fails with a redaction-safe provider error
- **AND** the runtime schedules no further attempt
- **WHEN** the turn reaches its terminal outcome
- **THEN** the TUI renders one attributed `failed after 3/3 attempts` error
- **AND** no row or notice claims another retry is pending

#### Scenario: Exact retry metadata is unavailable

- **GIVEN** the TUI replays an older event that classifies an error retryable
  but carries no attempt total or scheduled delay
- **WHEN** it renders that event
- **THEN** it does not invent `x/x`, a delay, or an admitted retry
- **AND** preserves a bounded generic provider diagnostic
