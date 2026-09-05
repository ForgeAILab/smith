## ADDED Requirements

### Requirement: Layered command-provider declaration

Smith SHALL accept a `command-jsonl` provider with a strict namespaced
`[providers.<name>.command]` declaration containing an absolute executable,
bounded fixed arguments, a workspace-or-absolute working directory, and an
explicit environment map. Every field MUST retain ordinary layer and path
provenance, and settings for HTTP provider transports MUST remain invalid for
this provider kind.

#### Scenario: Resolve a user-declared command provider

- **GIVEN** owner-controlled user configuration declares a `command-jsonl`
  provider with an absolute executable, fixed arguments, workspace cwd, and an
  environment credential reference
- **AND** the selected model has complete enforceable limits
- **WHEN** Smith resolves the run configuration
- **THEN** it produces one typed command-provider declaration with source
  provenance for every process field
- **AND** it does not resolve the environment credential or start the process
  during declarative resolution

#### Scenario: Command provider declares HTTP settings

- **GIVEN** a `command-jsonl` provider also declares a base URL, top-level
  provider credential, credential pool, rotation threshold, headers, or HTTP
  response normalization
- **WHEN** Smith resolves configuration
- **THEN** it rejects the incompatible option before credential or process I/O
- **AND** it does not silently ignore or reinterpret the setting

#### Scenario: Native provider declares a command table

- **GIVEN** a native HTTP provider contains a `command` table
- **WHEN** Smith resolves configuration
- **THEN** it rejects the table as incompatible with that adapter kind
- **AND** it does not execute or probe the declared program

#### Scenario: Executable or working directory is relative

- **GIVEN** a command provider declares a relative executable or a relative cwd
  other than the exact `workspace` token
- **WHEN** Smith validates the selected provider
- **THEN** resolution fails with the field and source that supplied it
- **AND** Smith performs no PATH lookup, credential access, or process spawn

### Requirement: Command-provider process authority

Smith MUST accept process-bearing command-provider settings only from
owner-controlled user configuration or an explicit higher-precedence host
authority. Repository-controlled configuration MAY select a provider/model
whose complete command declaration is already user-owned, but MUST NOT define
or override its adapter kind, executable, arguments, working directory, or
environment.

#### Scenario: Project selects a user-owned command provider

- **GIVEN** user configuration completely declares provider `local-bridge`
- **AND** project configuration selects `local-bridge/local-model` without
  supplying any process field
- **WHEN** Smith resolves the project run
- **THEN** the selection succeeds with user provenance on the whole command
  declaration
- **AND** project text gains no process or environment authority

#### Scenario: Project changes fixed arguments

- **GIVEN** user configuration declares a command provider
- **AND** project or project-local configuration replaces or appends one fixed
  argument, cwd, executable, environment value, or command adapter kind
- **WHEN** Smith preflights configuration
- **THEN** startup fails before credential access, process spawn, session
  creation, or terminal entry
- **AND** the diagnostic identifies the unauthorized field and source without
  rendering any environment value

### Requirement: Explicit command-provider environment

Smith SHALL clear the child process's ambient environment and pass only the
bounded names and values declared in the resolved command provider. Credential
references MUST resolve through Smith's existing secret boundary immediately
before provider construction, and all resolved values MUST remain absent from
configuration explanation, Debug, errors, events, journals, and terminal
output.

#### Scenario: Bridge receives an environment credential

- **GIVEN** a command environment entry maps `BRIDGE_TOKEN` to a valid
  credential reference
- **WHEN** Smith constructs the command provider
- **THEN** it resolves and redaction-registers the secret before adding that
  exact variable to the process configuration
- **AND** no other ambient variable is inherited
- **AND** no visible or persisted surface contains the resolved value

#### Scenario: Environment reference is unusable

- **GIVEN** a command environment credential reference is missing, malformed,
  locked, or exceeds its bounded lookup deadline
- **WHEN** Smith preflights the provider
- **THEN** startup fails before probing or starting the executable
- **AND** the diagnostic names the variable and reference source without the
  secret value
