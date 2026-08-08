## ADDED Requirements

### Requirement: Connected MCP servers contribute registry abilities

Smith SHALL register a trusted, connected server's advertised tools through the
shared ability and tool registry, applying the same approval, attribution, and
display rules as built-in tools. Remote tools MUST be namespaced by their
server so two servers advertising the same tool name coexist, and a remote tool
MUST NOT replace or shadow a built-in.

#### Scenario: Two servers advertise the same tool name

- **GIVEN** two trusted servers each advertising a tool named `search`
- **WHEN** their tools are registered
- **THEN** both are addressable under distinct server-namespaced names
- **AND** neither registration is rejected as a conflict

#### Scenario: Server advertises a built-in's name

- **GIVEN** a trusted server advertising a tool named `shell`
- **WHEN** its tools are registered
- **THEN** the built-in `shell` tool remains the one addressed by that name
- **AND** the remote tool is addressable only under its namespaced name

#### Scenario: Remote tool requires approval

- **GIVEN** an approval policy requiring confirmation for a remote tool's
  permission set
- **WHEN** the agent calls it
- **THEN** Smith requests approval before invocation
- **AND** the request attributes the call to its server

### Requirement: Server connection does not gate session start

Smith SHALL start a session without waiting for MCP servers to connect. A
server that is slow, unreachable, untrusted, or failing MUST NOT delay the first
prompt or fail the session, and its tools SHALL become available at a safe
activation boundary once it connects.

#### Scenario: Server is slow to start

- **GIVEN** a trusted server whose command takes a long time to become ready
- **WHEN** the user opens a session
- **THEN** the prompt is available before the server finishes connecting
- **AND** the server's tools appear at a later safe boundary

#### Scenario: Server never connects

- **GIVEN** a trusted server whose command exits immediately
- **WHEN** the user opens a session
- **THEN** the session is fully usable with the remaining capabilities
- **AND** the failure is retained and reportable rather than silently discarded

### Requirement: Untrusted servers are inert in non-interactive runs

In a non-interactive run Smith MUST NOT prompt for MCP execution trust and MUST
NOT spawn an untrusted server. The server SHALL contribute no tools and the run
MUST report that trust is required, consistent with fail-closed headless
approval.

#### Scenario: Headless run encounters an untrusted server

- **GIVEN** a declared server with no trust record
- **WHEN** Smith runs non-interactively
- **THEN** the server is not spawned and no prompt is shown
- **AND** the run reports that the server requires interactive trust
