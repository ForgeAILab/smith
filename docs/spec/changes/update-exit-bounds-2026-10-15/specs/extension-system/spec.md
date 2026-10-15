## ADDED Requirements

### Requirement: Stdio servers do not outlive Smith

Every stdio MCP server Smith starts SHALL be stopped when the Smith process
that started it ends, however it ends, including SIGKILL. A server that is
busy and not reading its input MUST NOT keep running after Smith is gone.
Smith SHALL start each server under a watchdog that never loads
configuration; if no watchdog can be started, Smith starts the server
directly and warns once.

#### Scenario: Smith is SIGKILLed during an MCP call

- **GIVEN** a declared stdio MCP server is executing a long tool call
- **WHEN** the Smith process is killed with SIGKILL
- **THEN** the server process is gone within 5 seconds

#### Scenario: Normal shutdown still goes through Smith

- **GIVEN** a connected stdio MCP server
- **WHEN** Smith exits normally or closes the connection on reconfigure
- **THEN** the server stops under Smith's existing grace and kill bounds
- **AND** the watchdog exits with it
