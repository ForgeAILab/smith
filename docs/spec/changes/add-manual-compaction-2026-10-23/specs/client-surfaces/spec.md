## ADDED Requirements

### Requirement: Explicit conversation compaction command

Smith SHALL expose `/compact` through the shared
command registry. Running it without arguments SHALL request one
protected semantic-compaction attempt at an idle session boundary. It SHALL
reuse the existing summary policy, persistence, usage attribution, and
safe-boundary admission, without submitting a conversational turn. An explicit
request MAY retry an idle interval that already had an automatic or manual
attempt, in this process or an earlier one; automatic idle compaction SHALL
keep its once-per-interval gate. When compaction cannot run, Smith SHALL name
the cause and when retrying can succeed.

#### Scenario: A user compacts eligible history

- **WHEN** an idle user runs `/compact` with eligible completed history
- **THEN** Smith attempts semantic compaction and reports its outcome
- **AND** recent canonical turns remain available under the existing retention policy
- **AND** the command is discoverable in help, completion, and the command palette

#### Scenario: Compaction cannot run

- **WHEN** the session is busy, shutting down, or lacks eligible history
- **THEN** Smith reports the condition without creating a conversation turn or automatically retrying
- **AND** each cause has its own message: a running turn, a busy runtime, a pending request, missed session events, shutdown, or no eligible history
- **AND** history that the committed summary already covers is reported as no eligible history, not as a failure, and leaves the previous summary in place

#### Scenario: An explicit request retries within the interval

- **WHEN** an earlier automatic or manual attempt in the current idle interval failed and the user runs `/compact` before another turn
- **THEN** Smith makes a new attempt for that interval
- **AND** a repeat with nothing new to summarize reports no eligible history and sends no summary request
- **AND** the automatic idle timer still makes no further attempt in that interval

#### Scenario: An explicit request retries after a restart

- **WHEN** a resumed session's durable gate shows the interval was attempted in an earlier process and the user runs `/compact`
- **THEN** Smith makes one new attempt for that interval
- **AND** the resumed automatic idle timer still makes no attempt
- **AND** a request that is refused at the boundary restores the gate, so automatic compaction stays gated

#### Scenario: A summary attempt fails

- **WHEN** manual semantic compaction fails
- **THEN** Smith reports the failure and preserves original canonical history and the previously committed summary
