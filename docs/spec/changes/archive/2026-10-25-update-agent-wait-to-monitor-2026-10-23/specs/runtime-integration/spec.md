## ADDED Requirements

### Requirement: Wait timeout uses host internal-turn admission

Smith SHALL admit `delegation.wait-timeout` continuations through the session's
existing idle-only internal-turn admission with an `InternalTurnSource` of
kind `delegation.wait-timeout`, and SHALL NOT require a new Agent Runtime
primitive for the timer. Before admission Smith SHALL re-check the child's
coordinator status and SHALL drop the wake when the child is terminal. An
accepted turn SHALL use the parent runtime composition applicable at
admission, including a harness backend when the session runs on one, and
SHALL observe the same provider, context, tool, approval, workspace,
checkpoint, cancellation, retry, and global-limit contracts as an ordinary
turn.

#### Scenario: Timeout wake on an idle parent

- **GIVEN** a parked parent whose armed timer expired for a running child
- **WHEN** Smith requests idle-only internal admission
- **THEN** runtime accepts one turn attributed `delegation.wait-timeout`
- **AND** no user-role message is appended

#### Scenario: Timeout wake on a harness session

- **GIVEN** a root session running on a harness with an expired wait timer
- **WHEN** the wake is admitted
- **THEN** the turn runs on the harness as other internal turns do
