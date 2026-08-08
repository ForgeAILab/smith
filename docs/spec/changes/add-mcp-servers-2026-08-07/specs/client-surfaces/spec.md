## ADDED Requirements

### Requirement: MCP server visibility and trust command

Smith SHALL provide a built-in command that lists every configured MCP server
with its connection state, and — where applicable — its tool count, its
configuration source, and a bounded failure reason. The command MUST offer a way
to grant execution trust to a server awaiting confirmation, and MUST NOT
reproduce credential values in any state it displays.

#### Scenario: Inspect configured servers

- **GIVEN** one connected server, one still connecting, and one that failed to
  start
- **WHEN** the user runs the MCP command
- **THEN** each server is listed with its state
- **AND** the connected server shows its tool count
- **AND** the failed server shows a bounded reason

#### Scenario: Grant trust from the command

- **GIVEN** a declared server awaiting execution confirmation
- **WHEN** the user grants trust through the command
- **THEN** Smith displays the resolved command and content identity before
  recording the decision
- **AND** the server connects without restarting the session

#### Scenario: Server environment references a credential

- **GIVEN** a server whose environment references a stored credential
- **WHEN** the user inspects it
- **THEN** the display names the credential
- **AND** never shows its value

### Requirement: Connection state in operational status

While any MCP server is connecting or has failed, Smith SHALL reflect that in
local operational status without obscuring session state. Status MUST resolve to
a quiet state once every configured server has settled.

#### Scenario: Servers settle during a session

- **GIVEN** two servers connecting at session start
- **WHEN** both finish connecting
- **THEN** status no longer reports connection activity
- **AND** the transcript is not interrupted by their transitions
