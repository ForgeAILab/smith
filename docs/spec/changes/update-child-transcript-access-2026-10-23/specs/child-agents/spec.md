## ADDED Requirements

### Requirement: Root agents can inspect a child's committed conversation

Smith SHALL expose a root-only `agent` action `transcript` for the parent's
own direct children. The action SHALL return ordered canonical conversation
content with message roles, child/session identity, tool call/result pairing,
current lifecycle state, and stable pagination. Accessible content SHALL
include the delegated task, assistant messages, reasoning permitted by existing
visibility policy, tool invocations/results, and final replies. Internal
instructions, credentials, and protected reasoning MUST remain excluded.

#### Scenario: The parent diagnoses a running child

- **GIVEN** a child has committed assistant messages and tool results
- **WHEN** its parent requests `agent transcript` for that child
- **THEN** the parent receives those messages in order with their roles and
  tool identities, rather than only a status summary
- **AND** an unfinished model request is not reported as a completed result

#### Scenario: A large entry requires more than one context insertion

- **GIVEN** a child tool result exceeds the ordinary inline output budget
- **WHEN** the parent reads the transcript page containing it
- **THEN** the response remains bounded and provides a parent-readable
  reference to the complete accessible entry
- **AND** reading successive pages/references does not silently omit content

#### Scenario: A live child appends while the parent pages history

- **GIVEN** the parent is reading a transcript snapshot over multiple pages
- **WHEN** the child commits additional messages
- **THEN** existing page cursors continue to identify the same ordered snapshot
- **AND** later content is reachable through an explicit subsequent read

#### Scenario: Transcript access preserves ownership and visibility

- **GIVEN** a request names an unknown child or a child owned by another parent
- **WHEN** the transcript action resolves the target
- **THEN** it returns a clear error without reading the unrelated history
- **AND** successful reads apply existing credential and reasoning redaction

### Requirement: Transcript inspection does not execute or consume child work

Reading child history SHALL be a read-only operation available for running,
completed, and retained dormant children. It MUST NOT start, resume, stop, or
follow up a child, issue a provider request, or consume protected completion
readiness. Missing retained history SHALL be reported explicitly.

#### Scenario: Inspect a durable child after restarting the parent

- **GIVEN** a retained child's history exists in protected persistence
- **AND** the child has no live execution handle in the current process
- **WHEN** the parent requests its transcript
- **THEN** Smith reads the retained conversation without starting the child
- **AND** missing segments, if any, are identified as unavailable

#### Scenario: Completion follows a diagnostic transcript read

- **GIVEN** the parent inspected a running child's transcript and then parked
- **WHEN** that child completes
- **THEN** its terminal outcome remains eligible for automatic delivery
- **AND** existing `wait`/`result` acknowledgement behavior is unchanged

### Requirement: Delegation guidance favors completed outcome delivery

Smith's root delegation guidance SHALL tell the model to continue independent
work or park while a child runs, receive completed outcomes through automatic
delivery, and use transcript inspection on demand for understanding or
diagnosis. It SHALL discourage repeated status/transcript polling as a way to
obtain completion and SHALL distinguish child control from shell-task control.

#### Scenario: The parent has no independent work remaining

- **GIVEN** a child is still running and no independent parent work remains
- **WHEN** Smith presents delegation guidance and a foreground wait expires
- **THEN** the guidance directs the parent to park and await automatic delivery
- **AND** it does not direct repeated reads of partial child output

#### Scenario: The parent needs to stop a child

- **GIVEN** the parent is diagnosing a child rather than a background shell task
- **WHEN** Smith describes the available controls
- **THEN** it names `agent` action `stop` with the child ID
- **AND** it identifies `task_stop` as a shell-task operation
