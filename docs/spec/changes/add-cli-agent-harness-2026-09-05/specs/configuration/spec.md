## ADDED Requirements

### Requirement: CLI agent harness selection

A profile MAY select an installed coding agent with `harness = "claude-code"`
or `harness = "codex"`. Such a profile SHALL run its turns on that CLI instead
of a model provider, and MUST be selectable as the main agent or as a delegated
child through the ordinary `use` list.

A harness profile MUST NOT also carry provider-model settings whose semantics
the CLI would silently ignore. Smith SHALL reject that combination at
resolution rather than accept a model choice with no effect.

#### Scenario: Profile selects a harness

- **GIVEN** a profile with `harness = "claude-code"`
- **WHEN** configuration resolves
- **THEN** the profile is valid without a provider or model
- **AND** `smith config explain` reports the harness and its source

#### Scenario: Harness profile carries provider-only settings

- **GIVEN** a harness profile that also sets a provider credential or endpoint
- **WHEN** configuration resolves
- **THEN** resolution fails naming the conflicting key

### Requirement: Harness process settings

Per-harness settings SHALL be declared under `[harness.<name>]` with an
executable, an optional model, optional fixed arguments, an optional working
directory, and an explicit environment overlay. Values remain layered and
source-explainable.

The executable MUST be resolved without a shell. Project and project-local
configuration MAY select a harness but MUST NOT declare or override its
executable, arguments, working directory, or environment, matching the rule
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
