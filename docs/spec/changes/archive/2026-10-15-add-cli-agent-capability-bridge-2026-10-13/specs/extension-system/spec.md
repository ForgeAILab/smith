## ADDED Requirements

### Requirement: Smith-declared MCP servers reach a harness only through Smith

Smith SHALL offer a harness session the tools of each trusted, connected MCP
server declared in Smith's configuration or contributed by an active plugin
through the bridge, and SHALL make each call itself on the CLI's behalf
under the server's existing trust, credential, approval, attribution, and
conservative-authority rules. Tools contributed by an active module SHALL be
offered the same way under that module's grants.

Smith MUST NOT write a Smith-declared server, its command, its URL, or its
credentials into the CLI's own MCP configuration or argument list. A server
that is untrusted, disconnected, or withheld MUST contribute nothing to the
bridge.

#### Scenario: A remote tool called from a harness

- **GIVEN** a trusted, connected server `docs` declared in Smith's
  configuration
- **WHEN** the CLI calls its `lookup` tool through the bridge
- **THEN** Smith makes the call with the server's configured credential
- **AND** the approval request attributes the call to `docs`

#### Scenario: The CLI's invocation is inspected

- **GIVEN** a Smith-declared server that authenticates with a stored secret
- **WHEN** the harness is invoked
- **THEN** the invocation names only the `smith` server
- **AND** neither the server's address nor its secret appears in it

#### Scenario: An untrusted server

- **GIVEN** a declared server whose trust is not recorded
- **WHEN** a harness turn runs
- **THEN** none of its tools are bridged

#### Scenario: A module tool

- **GIVEN** an active module contributing `generate_image`
- **WHEN** the CLI calls it through the bridge
- **THEN** the call runs under that module's granted capabilities

### Requirement: Interactive sessions name untrusted servers

In an interactive session Smith SHALL tell the user, once when the session
opens, which declared MCP servers are untrusted and how to trust each one.
The notice MUST NOT spawn the server, prompt for trust, or delay the first
prompt. Found by the 2026-10-14 live check: a harness turn searched the
registry, found nothing, and could only tell the user to check the server.

#### Scenario: Session opens with an untrusted project server

- **GIVEN** a project declares an MCP server `docs` with no trust record
- **WHEN** the user opens an interactive session
- **THEN** the transcript shows one notice naming `docs` and `/mcp trust docs`
- **AND** `docs` is not spawned until the user trusts it

#### Scenario: Every declared server is trusted

- **GIVEN** every declared MCP server has a trust record
- **WHEN** the user opens an interactive session
- **THEN** no untrusted-server notice is shown
