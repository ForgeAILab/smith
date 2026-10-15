## MODIFIED Requirements

### Requirement: CLI-owned tools are off by default

A harness SHALL run without its own tools unless
`harness.<name>.allow_own_tools` is explicitly enabled in owner-controlled
configuration. When disabled, Smith MUST pass the CLI's read-only or
no-tool mode rather than relying on the CLI's default.

Enabling it means the CLI executes reads, writes, and commands that Smith did
not approve, did not scope to the workspace, and cannot record as tool history.

This setting governs only the tools the CLI implements and runs itself.
Tools Smith offers through the capability bridge are Smith tool calls, not
the CLI's own tools: they are offered whether or not this setting is
enabled, and no configuration key is needed to offer them. Pre-allowing the
`smith` server in the CLI invocation MUST NOT allow any other tool.

#### Scenario: Default harness turn runs without CLI tools

- **GIVEN** a harness with no `allow_own_tools` setting
- **WHEN** a turn runs
- **THEN** the CLI is invoked in its no-own-tools mode

#### Scenario: Project cannot enable CLI tools

- **GIVEN** a project file setting `harness.claude-code.allow_own_tools = true`
- **WHEN** configuration resolves
- **THEN** resolution fails naming the project-layer key

#### Scenario: Bridge with CLI tools off

- **GIVEN** a harness with no `allow_own_tools` setting and a non-empty
  bridged set
- **WHEN** a turn runs
- **THEN** the CLI is invoked in its no-own-tools mode
- **AND** the only tools pre-allowed are those of the `smith` server

#### Scenario: Denying every capability turns the bridge off

- **GIVEN** a harness profile whose capability limits deny every tool and
  skill
- **WHEN** a turn runs
- **THEN** no endpoint is started
- **AND** the CLI is invoked as it was before the bridge existed
