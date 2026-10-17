## ADDED Requirements

### Requirement: MCP server stderr never reaches the terminal

Smith SHALL capture the stderr of every stdio MCP server it starts, including
the watchdog's own messages and anything a descendant of the server writes,
and MUST NOT let any of it reach Smith's terminal in an interactive or a
headless run. Smith SHALL drain that output continuously so a server never
blocks on a full pipe, and SHALL keep only a bounded tail per server with
terminal escape sequences removed and every environment value it passed to
the server redacted. Smith SHALL write that output to the session log. When
a server fails to start or exits on its own, Smith SHALL report the failure
with the last lines of that tail where the user already sees MCP status, and
normal output from a healthy server MUST NOT produce any notice. Found by the
owner on 2026-10-16: `npm notice` lines from `npx` were painted over the
composer and could not be cleared.

#### Scenario: A healthy server writes to stderr

- **GIVEN** a trusted stdio MCP server that writes `npm notice` lines to
  stderr and then connects
- **WHEN** an interactive session is open
- **THEN** nothing the server wrote appears on the terminal
- **AND** the server is listed as connected with no stderr detail
- **AND** the lines are written to the session log at `info` level

#### Scenario: A server fails to start

- **GIVEN** a trusted stdio MCP server that writes
  `cannot find module` to stderr and exits with status 1
- **WHEN** Smith tries to connect it
- **THEN** `/mcp` lists the server as failed
- **AND** the failure includes `cannot find module`

#### Scenario: A headless run with a failing server

- **GIVEN** the same failing server
- **WHEN** `smith -p` runs with `--output-format json`
- **THEN** Smith's stdout carries only the JSON envelope and its stderr does
  not contain the server's output
- **AND** the result's `diagnostics` name the server and include its last
  stderr line

#### Scenario: A connected server exits

- **GIVEN** a connected stdio MCP server
- **WHEN** the server exits on its own with status 3 after writing an error
  to stderr
- **THEN** the server is reported as failed with its exit status and that
  error
- **AND** its tools are withdrawn at the next safe boundary

#### Scenario: A server exits while a descendant holds its output

- **GIVEN** a connected stdio MCP server that started a background process
  sharing its stdout and stderr
- **WHEN** the server exits with status 4
- **THEN** the server is reported as failed with that status within about two
  seconds

#### Scenario: Output before the session log exists

- **GIVEN** `SMITH_LOG=info` and a server that writes to stderr while Smith is
  still starting the session
- **WHEN** the session's log file is created
- **THEN** those lines are in it

#### Scenario: Smith stops a server

- **GIVEN** a connected stdio MCP server
- **WHEN** Smith closes the connection on reconfigure or exit
- **THEN** the server is not reported as failed

#### Scenario: A server prints its own credential

- **GIVEN** a server started with `API_TOKEN` resolved from a credential
- **WHEN** the server writes that token's value to stderr
- **THEN** the session log and the reported failure show it redacted
