## MODIFIED Requirements

### Requirement: Child management controls

Smith SHALL expose root-only operations to spawn, list, send or follow up, wait,
fetch result, resume, and stop direct children. Operations MUST be addressed by
stable child ID and return structured lifecycle or error results.

The `agent.wait` operation SHALL accept an optional `timeout_ms`. Its resolved
configuration paths SHALL be
`profiles.<name>.child_agents.wait_default_timeout_ms` and
`profiles.<name>.child_agents.wait_max_timeout_ms`. The default and maximum
ranges SHALL be `0..=300_000` and `1..=300_000` milliseconds respectively, and
the built-in values SHALL both be 300,000 milliseconds. The resolved default
MUST NOT exceed the resolved maximum. A requested timeout of zero SHALL be an
immediate status check; a requested timeout above the resolved maximum SHALL be
rejected before waiting.

When a foreground wait expires before terminal child delivery, `agent.wait`
SHALL return a successful structured `running` result with a timeout marker and
MUST release the parent tool call. This is a soft foreground boundary only: the
child MUST remain active, its lifecycle and exact state MUST be unchanged, and
its terminal outcome MUST remain must-deliver for automatic delivery at the
next safe boundary. The model-facing description SHALL state that terminal
outcomes are delivered automatically and that a timed-out wait does not stop
the child.

#### Scenario: Foreground wait expires without stopping the child

- **GIVEN** the root waits for a child that remains active
- **AND** the foreground wait reaches its configured five-minute boundary
- **WHEN** no terminal child outcome is available
- **THEN** `agent.wait` returns a successful `running` result marked as timed out
- **AND** the parent tool call is released for normal completion or new work
- **AND** the child continues running with its original lifecycle and limits

#### Scenario: Child completes during the foreground wait

- **GIVEN** the root waits for a child that completes before the boundary
- **WHEN** the wait observes the terminal state
- **THEN** `agent.wait` returns the terminal/idle child status without a timeout
  marker
- **AND** the result remains eligible for the existing automatic completion
  delivery path

#### Scenario: Explicit shorter wait remains available

- **GIVEN** the root requests a valid `timeout_ms` shorter than five minutes
- **WHEN** the child remains active until that duration expires
- **THEN** Smith returns the same successful running/timeout projection
- **AND** the child remains active rather than being stopped
