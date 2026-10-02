## ADDED Requirements

### Requirement: Session accounting is client-neutral

Smith SHALL keep prompt-cache projection, usage, pricing, cost, and usage-log
logic in a crate whose dependency closure contains no terminal UI library.
Headless and exit-report code MUST NOT import these from the terminal client
crate.

#### Scenario: Headless reports cost

- **GIVEN** a headless run that reports usage, cache state, and cost
- **WHEN** its imports are inspected
- **THEN** none refers to the terminal client crate
- **AND** the reported fields and values equal those before the move

#### Scenario: The accounting crate stays UI-free

- **WHEN** the workspace dependency graph is evaluated in CI
- **THEN** the accounting crate's closure contains neither ratatui nor
  crossterm

### Requirement: Local command results are typed across the host boundary

Smith SHALL pass the result of a local command from host to client as a typed
report. A renderer MUST choose presentation from the report's type and fields
and MUST NOT recover structure by inspecting a title, heading, glyph, or
label text.

#### Scenario: Status is rendered from data

- **GIVEN** the user invokes `/status`
- **WHEN** the host returns its result
- **THEN** the client receives a status report value
- **AND** the terminal output equals the output before the migration

#### Scenario: Headless and terminal share one report

- **GIVEN** goal or child state is shown in headless output and in a local
  command result
- **WHEN** both are produced for the same session state
- **THEN** both are rendered from the same report type
- **AND** the same state has the same label on both surfaces

### Requirement: One definition per slash command

Smith SHALL define each slash command once, in a table that supplies its
name, summary, argument grammar, group, and route. Completion, the command
palette, `/help`, parsing, and dispatch MUST read that table, and no other
list of command names may exist in production code.

#### Scenario: A command is added

- **WHEN** a developer adds a read-only host command
- **THEN** the change is one table entry and one handler function
- **AND** completion, the palette, and `/help` show it without further edits

#### Scenario: A command reaches the wrong executor

- **GIVEN** the command type separates client-handled from host-handled
  commands
- **WHEN** a host handler omits a host command or a client command is sent
  to the host
- **THEN** the workspace fails to compile
- **AND** no runtime `unreachable!` guards command routing

### Requirement: Provider setup data has one source

Smith SHALL derive guided-setup entries, fixed endpoints, trusted model
limits, and the list of connectable providers from the provider descriptors
in the configuration crate. Other crates MUST receive these as data and MUST
NOT restate them as literals.

#### Scenario: A provider is added

- **WHEN** a provider descriptor is added with a setup flow
- **THEN** guided setup offers it and `/connect` lists it
- **AND** no source file outside the configuration crate changes to name its
  endpoint or limits

#### Scenario: A setup entry has no flow

- **WHEN** code attempts to offer a setup entry
- **THEN** the entry's type requires a flow
- **AND** an entry that does nothing on confirmation cannot be constructed

### Requirement: CLI modules declare their dependencies

Production modules in the CLI crate SHALL import the items they use by name.
A module MUST NOT glob-import its parent to obtain its dependencies.

#### Scenario: A CLI module is read in isolation

- **WHEN** a production module in the CLI crate is opened
- **THEN** every external item it uses appears in its own import list
- **AND** lint configuration rejects a glob import of the parent module
