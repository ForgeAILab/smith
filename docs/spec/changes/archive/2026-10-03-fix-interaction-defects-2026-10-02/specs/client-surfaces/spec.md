## ADDED Requirements

### Requirement: Every offered setup entry is actionable

Guided setup SHALL offer only entries that start a flow. Selecting any listed
entry and confirming it MUST advance to that entry's next step or report why
it cannot proceed; a confirmation MUST NOT be silently ignored.

#### Scenario: A listed provider path is confirmed

- **GIVEN** guided setup lists a provider or action entry
- **WHEN** the user selects it and presses Enter
- **THEN** setup shows that entry's next step
- **AND** the selection screen does not remain unchanged

#### Scenario: An entry has no first-run flow

- **GIVEN** a provider descriptor has no flow available in guided setup
- **WHEN** Smith builds the setup choices
- **THEN** the entry is omitted, or shown as unavailable with the command
  that does support it
- **AND** an automated check fails when an offered entry has no handler

### Requirement: Setup text names the values it writes

Labels and review text in guided setup SHALL be derived from the provider,
model, limits, and catalog revision that setup will write. Smith MUST NOT show
a model name or catalog revision that differs from the published value.

#### Scenario: Quick-start review

- **GIVEN** the user chooses a quick-start provider path
- **WHEN** setup shows its menu label and its review
- **THEN** both name the model that will be written
- **AND** the catalog revision shown equals the revision recorded in
  configuration

### Requirement: Usage help at every command level

The command line SHALL print usage and exit successfully for `smith help`,
and for `-h` or `--help` after any subcommand. A parse error SHALL name the
problem and print exactly one recovery hint.

#### Scenario: Help after a subcommand

- **WHEN** the user runs `smith setup --help`, `smith config --help`, or
  `smith sessions --help`
- **THEN** Smith prints usage covering that subcommand to stdout
- **AND** exits with status 0 without opening setup or a session

#### Scenario: Unknown argument

- **WHEN** the user runs `smith` with an unrecognised option or subcommand
- **THEN** stderr names the argument and gives one hint to run help
- **AND** the exit status is the documented usage-error status

### Requirement: Interactive launch without a terminal is refused clearly

When configuration is ready and no prompt is supplied, Smith SHALL verify
that it has an interactive terminal before entering the alternate screen. If
it does not, Smith MUST exit with a message that names the cause and the
headless alternative, and MUST NOT surface a raw operating-system error.

#### Scenario: Standard input is a pipe

- **GIVEN** configuration is ready
- **WHEN** the user runs `echo hi | smith`
- **THEN** stderr explains that the interactive surface needs a terminal and
  that `smith -p` accepts a prompt on standard input
- **AND** no session is created and no provider request is sent

### Requirement: Configured background exit policy is honoured

A headless run SHALL resolve its background-exit policy from the
`--background-exit` flag, then the resolved `background.exit_policy`
configuration value, then the default `error`.

#### Scenario: Policy set only in configuration

- **GIVEN** configuration sets `background.exit_policy = "wait"`
- **AND** the caller passes no `--background-exit` flag
- **WHEN** a headless turn finishes with a running background task
- **THEN** Smith waits for the task's terminal state before exiting

#### Scenario: Flag overrides configuration

- **GIVEN** configuration sets `background.exit_policy = "wait"`
- **WHEN** the caller passes `--background-exit stop`
- **THEN** Smith applies `stop`

### Requirement: Session listing is readable on a terminal

`smith sessions list` SHALL print a header row, aligned columns, and the
last-updated time in local time when standard output is a terminal. When
standard output is not a terminal it MUST keep the existing tab-separated
row format.

#### Scenario: Listing on a terminal

- **GIVEN** the project has saved sessions
- **WHEN** the user runs `smith sessions list` in a terminal
- **THEN** each column has a heading
- **AND** the update time is a local date and time, not a millisecond count

#### Scenario: Listing through a pipe

- **WHEN** the output of `smith sessions list` is piped to another program
- **THEN** each session is one tab-separated row with the fields and order
  documented for the Claude Code plugin
