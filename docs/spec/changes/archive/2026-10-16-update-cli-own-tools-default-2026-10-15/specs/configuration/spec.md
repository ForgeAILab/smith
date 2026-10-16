## RENAMED Requirements

- FROM: `### Requirement: CLI-owned tools are off by default`
- TO: `### Requirement: CLI-owned tools are on by default`

## MODIFIED Requirements

### Requirement: Harness process settings

Optional per-machine settings SHALL be declared under `[harness.<kind>]`: an
absolute executable path, fixed arguments, an environment overlay, and
`allow_own_tools`. Absent settings mean the program is found on `PATH` and run
with its own tools. Values remain layered and source-explainable.

The executable MUST be an absolute path invoked without a shell. Project and
project-local configuration MAY select an installed agent but MUST NOT declare
or override its executable, arguments, environment, or `allow_own_tools`,
matching the rule
already applied to command providers.

#### Scenario: Project selects but cannot redefine a harness

- **GIVEN** a user-declared `claude-code` harness
- **AND** a project file that sets `harness.claude-code.executable`
- **WHEN** configuration resolves
- **THEN** resolution fails naming the project-layer key

### Requirement: CLI-owned tools are on by default

A harness SHALL run with its own tools unless owner-controlled configuration
sets `harness.<name>.allow_own_tools = false`. A harness CLI runs
non-interactively and cannot ask for permission. So when its own tools are
on, Smith MUST invoke the CLI in its no-prompt mode: Claude Code with
`--permission-mode bypassPermissions`, Codex with
`--dangerously-bypass-approvals-and-sandbox`. When they are off, Claude Code
runs in its default mode with only the `smith` server pre-allowed, so `-p`
denies anything it would ask about and only its read-only tools run. Codex
runs with `--sandbox read-only`.

With its own tools on, the CLI executes reads, writes, commands, and network
requests that Smith did not approve, did not scope to the workspace, and
cannot record as tool history.

Smith SHALL turn the CLI's own tools off, whatever the setting, for a turn
whose session may not write: a root profile with a read-only posture, or a
child not granted write-capable tools. No caller can turn them on when the
owner turned them off.

Owner `args` SHALL follow every flag Smith builds for Claude Code, so an
owner argument can narrow the default permission mode.

This setting governs only the tools the CLI implements and runs itself.
Tools Smith offers through the capability bridge are Smith tool calls, not
the CLI's own tools: they are offered whether or not this setting is
enabled, and no configuration key is needed to offer them. Pre-allowing the
`smith` server in the CLI invocation MUST NOT allow any other tool.

#### Scenario: Default harness turn runs the CLI's own tools without prompts

- **GIVEN** a `cli/claude-code/opus` profile with no `[harness]` table
- **WHEN** a turn runs
- **THEN** Claude Code is invoked with `--permission-mode bypassPermissions`
- **AND** a WebFetch the CLI makes is not denied for want of approval

#### Scenario: Default Codex turn has no sandbox prompts

- **GIVEN** a `cli/codex/gpt-6-astra` profile with no `[harness]` table
- **WHEN** a turn runs
- **THEN** Codex is invoked with `--dangerously-bypass-approvals-and-sandbox`
- **AND** without `--sandbox`

#### Scenario: Owner turns the CLI's own tools off

- **GIVEN** owner configuration setting `harness.claude-code.allow_own_tools = false`
- **WHEN** a turn runs
- **THEN** the CLI is invoked in its no-own-tools mode

#### Scenario: Project cannot set CLI tools

- **GIVEN** a project file setting `harness.claude-code.allow_own_tools`
  to either value
- **WHEN** configuration resolves
- **THEN** resolution fails naming the project-layer key

#### Scenario: Read-only root posture narrows the CLI

- **GIVEN** a root profile with `posture = "plan"` on `cli/claude-code/opus`
  and no `allow_own_tools` setting
- **WHEN** a turn runs
- **THEN** the CLI is invoked in its no-own-tools mode

#### Scenario: Read-only child narrows the CLI

- **GIVEN** a child on `cli/codex/gpt-6-astra` spawned without write-capable
  tools
- **WHEN** its turn runs
- **THEN** Codex is invoked with `--sandbox read-only`

#### Scenario: Owner arguments narrow Claude Code

- **GIVEN** owner configuration with
  `harness.claude-code.args = ["--permission-mode", "acceptEdits"]`
- **WHEN** a turn runs
- **THEN** the owner's `--permission-mode` follows Smith's in the argument list

#### Scenario: Bridge with CLI tools off

- **GIVEN** a harness with `allow_own_tools = false` and a non-empty bridged
  set
- **WHEN** a turn runs
- **THEN** the CLI is invoked in its no-own-tools mode
- **AND** the only tools pre-allowed are those of the `smith` server
