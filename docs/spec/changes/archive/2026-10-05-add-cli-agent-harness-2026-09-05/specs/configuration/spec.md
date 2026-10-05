## ADDED Requirements

### Requirement: CLI agent harness selection

Smith SHALL let a profile select an installed coding agent by model id
`cli/<kind>/<model>`, where `<kind>` is `claude-code` or `codex` (for example
`cli/claude-code/sonnet`). Such a profile SHALL run its turns on that CLI
instead of a model provider, and MUST be selectable as the main agent or as a
delegated child through the ordinary `use` list. Selecting an agent MUST NOT
require a `[harness]` declaration; installed-agent model ids SHALL be offered
wherever models are listed.

The `<model>` segment is what the CLI is told to use. Smith plans against
fixed installed-agent context, input, and output limits before any work runs.
The harness replaces how the turn is *executed*, not how the profile resolves;
the resolved provider is never called to produce a harness turn.

#### Scenario: Profile selects an installed agent by model id

- **GIVEN** a profile whose model is `cli/claude-code/sonnet`
- **WHEN** configuration resolves
- **THEN** the profile is valid without any `[harness]` table
- **AND** the CLI is asked to use `sonnet`
- **AND** the resolved provider is never called to produce a turn

#### Scenario: A provider model is not mistaken for an installed agent

- **GIVEN** a profile whose model id does not start with `cli/`
- **WHEN** configuration resolves
- **THEN** no harness is resolved and the provider runs the turn

### Requirement: Harness process settings

Optional per-machine settings SHALL be declared under `[harness.<kind>]`: an
absolute executable path, fixed arguments, an environment overlay, and
`allow_own_tools`. Absent settings mean the program is found on `PATH` and run
without its own tools. Values remain layered and source-explainable.

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

### Requirement: CLI-owned tools are off by default

A harness SHALL run without its own tools unless
`harness.<name>.allow_own_tools` is explicitly enabled in owner-controlled
configuration. When disabled, Smith MUST pass the CLI's read-only or
no-tool mode rather than relying on the CLI's default.

Enabling it means the CLI executes reads, writes, and commands that Smith did
not approve, did not scope to the workspace, and cannot record as tool history.

#### Scenario: Default harness turn runs without CLI tools

- **GIVEN** a harness with no `allow_own_tools` setting
- **WHEN** a turn runs
- **THEN** the CLI is invoked in its no-own-tools mode

#### Scenario: Project cannot enable CLI tools

- **GIVEN** a project file setting `harness.claude-code.allow_own_tools = true`
- **WHEN** configuration resolves
- **THEN** resolution fails naming the project-layer key

### Requirement: Harness environment is inherited with explicit overrides

A harness child process SHALL inherit the ambient environment, with
`[harness.<name>.env]` applied over it. An installed coding CLI depends on its
own login, `PATH`, and home directory; clearing the environment prevents it
from authenticating at all.

This differs deliberately from the command-provider rule, which clears the
environment because that executable is a Smith-specific bridge rather than an
independently configured program.

#### Scenario: Harness reaches its own credentials

- **GIVEN** an installed CLI authenticated for the current user
- **WHEN** a harness turn runs
- **THEN** the child inherits the environment that authentication depends on
