## ADDED Requirements

### Requirement: Declarative MCP server configuration

Smith SHALL resolve Model Context Protocol server declarations through the same
layered precedence and source attribution as every other setting. A declaration
MUST identify its transport, and environment values referencing credentials MUST
resolve through the existing secret path rather than being read as literals.
A raw secret literal in repository-controlled configuration MUST be rejected.

#### Scenario: Explain where a server came from

- **GIVEN** a server declared in user configuration and overridden in project
  configuration
- **WHEN** the user inspects resolved configuration
- **THEN** the effective declaration reports its winning layer
- **AND** the overridden layer remains visible as an explanation

#### Scenario: Project configuration embeds a literal secret

- **GIVEN** project configuration declares a server whose environment contains a
  literal token value
- **WHEN** Smith resolves configuration
- **THEN** resolution fails with a diagnostic naming the offending key
- **AND** the diagnostic does not reproduce the value

### Requirement: Hash-bound MCP server execution trust

Smith MUST obtain user confirmation before spawning a declared MCP server, in
every configuration layer. Trust SHALL bind the canonical project path to a
digest of the fully resolved invocation — command, arguments, and environment
variable names — and MUST NOT include environment values. A change to the
command, its arguments, or the set of environment variable names MUST invalidate
the prior decision.

#### Scenario: First connection to a declared server

- **GIVEN** a declared MCP server with no matching trust record
- **WHEN** Smith would connect to it
- **THEN** Smith displays the server name, its resolved command and arguments,
  and its content identity
- **AND** does not spawn it until the user confirms

#### Scenario: Server arguments change

- **GIVEN** the user trusted a server
- **WHEN** its resolved arguments change
- **THEN** the old trust record no longer authorizes execution
- **AND** Smith requests confirmation for the new digest

#### Scenario: Credential rotates behind a trusted server

- **GIVEN** the user trusted a server whose environment references a credential
- **WHEN** that credential's value changes but its name does not
- **THEN** the existing trust record still authorizes execution
- **AND** Smith does not re-prompt

### Requirement: Remote MCP servers authenticate through the existing secret path

A declared remote server SHALL send credentials resolved through the same
secret path as a provider's, never values written in configuration. A bearer
credential MUST be declared as a reference and sent under an authorization
header Smith composes; an authorization-bearing header written as a literal
MUST be rejected. Execution trust for a remote server SHALL bind its endpoint
and the names of the headers it would send, and MUST NOT include their values.

#### Scenario: Authorization written in plain text

- **GIVEN** a declared remote server whose headers contain a literal
  authorization value
- **WHEN** Smith resolves configuration
- **THEN** resolution fails with a diagnostic naming the offending header
- **AND** the diagnostic does not reproduce the value

#### Scenario: Bearer credential rotates behind a trusted endpoint

- **GIVEN** the user trusted a remote server declaring a bearer credential
- **WHEN** that credential's value changes but the endpoint and header names do
  not
- **THEN** the existing trust record still authorizes the connection

#### Scenario: The endpoint changes

- **GIVEN** the user trusted a remote server
- **WHEN** its declared endpoint changes
- **THEN** the old trust record no longer authorizes the connection
- **AND** Smith requests confirmation for the new content identity

#### Scenario: An option the transport cannot use

- **GIVEN** a declaration naming a local command together with a credential, or
  a remote endpoint together with command arguments
- **WHEN** Smith resolves configuration
- **THEN** resolution fails naming the option the chosen transport cannot use

### Requirement: Repository configuration cannot self-authorize MCP servers

An approval mode or auto-approval list MUST NOT authorize spawning an MCP
server; execution trust is a separate decision asked per content digest.
Repository-controlled configuration that attempts to auto-approve MCP tools MUST
fail preflight, consistent with the existing prohibition on repository
self-authorization of tools.

#### Scenario: Allow-all does not spawn an untrusted server

- **GIVEN** user configuration selects an allow-all approval mode
- **AND** a project declares an MCP server with no trust record
- **WHEN** Smith starts the session
- **THEN** the server is not spawned
- **AND** Smith still requests execution confirmation

#### Scenario: Project auto-approves a remote tool

- **GIVEN** project or project-local configuration auto-approves a tool
  belonging to an MCP server
- **WHEN** Smith preflights the session
- **THEN** startup fails before creating session state
- **AND** the diagnostic says to move the policy to user configuration
