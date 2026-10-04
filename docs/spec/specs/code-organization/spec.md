# code-organization Specification

## Purpose
Ownership and boundary rules for Smith's crates and modules, and how those boundaries are checked.
## Requirements
### Requirement: Large Rust modules have stable responsibility boundaries

Smith SHALL preserve existing module paths and behavior while decomposing
large Rust source files into cohesive private modules. A child module SHALL
own a complete responsibility or stage, and implementation visibility SHALL
be limited to the narrowest scope needed for coordination.

#### Scenario: Cache lifecycle work is decomposed without changing scheduling

- **GIVEN** the cache controller's scheduler, idle-compaction lane,
  handoff-persistence path, and resume-capsule event projection
- **WHEN** idle compaction, handoff persistence, and capsule event projection
  move into private child modules
- **THEN** `smith_runtime::cache_controller` retains its existing public
  exports and controller orchestration
- **AND** admission boundaries, cancellation, Runtime event order, capsule
  persistence rollback, and synthetic-attempt accounting remain unchanged

#### Scenario: Headless output and event flow are decomposed without contract changes

- **GIVEN** a headless run using text, JSON, or stream JSON output and any
  configured background-exit policy
- **WHEN** output projection, background-exit handling, and event-stream flow
  move into private child modules
- **THEN** the existing `headless::run` entry point remains available to the
  CLI
- **AND** serialized fields, output ordering, diagnostics, background task
  states, and exit codes remain unchanged

#### Scenario: Extracted modules do not widen internal APIs

- **GIVEN** sibling orchestration calls an extracted stage
- **WHEN** private child modules are introduced
- **THEN** only the visibility required between the parent and those children
  is added
- **AND** no item becomes public solely to support the refactor

### Requirement: Factory and application state have physical ownership boundaries

Smith SHALL place the implementation of a factory stage in the private module
that names that stage, and SHALL keep TUI child-session presentation methods
in a cohesive private application module. The public factory and `App` paths
and signatures MUST remain stable. The extraction MUST preserve observable
runtime and TUI behavior.

#### Scenario: Factory stage implementation is owned by its module

- **GIVEN** provider, credential, and authority preparation is performed during
  `factory::build`
- **WHEN** factory source is decomposed
- **THEN** the corresponding stage modules contain the implementation rather
  than only forwarding to functions in `factory.rs`
- **AND** validation, credential, tool, and composition order is unchanged

#### Scenario: Child presentation moves without changing App behavior

- **GIVEN** the TUI tracks child conversations, counts, inspection, and
  dismissal in `App`
- **WHEN** those methods move to a private child module
- **THEN** existing `App` imports and method calls compile
- **AND** live and replayed events produce the same child presentation

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

### Requirement: Crate boundaries are checked by structure

Smith's architecture tests SHALL check crate boundaries from manifests and
source structure rather than file names or literal strings: `smith-tui` has
no `smith-config` dependency, `smith-client` has no terminal-library
dependency, headless code does not import `smith-tui`, production
`smith-cli` code has no glob imports, and `smith-runtime` exposes only an
allow-listed set of modules. Modules of `smith-runtime` with no consumer
outside the crate MUST be crate-private, and `unreachable_pub` SHALL be
denied for that crate.

#### Scenario: A new public module

- **GIVEN** a contributor adds a `pub mod` to `smith-runtime` that is not on
  the allow-list
- **WHEN** the tests run
- **THEN** the architecture test fails and names the module

### Requirement: One definition per listed key binding

Smith SHALL define each key binding that `/help` or the shortcuts panel
lists once, in a typed table that supplies its chord, the context it applies
in, its effect, and its help text. `/help` and the shortcuts panel MUST
render from that table, and a test MUST check every binding against the
TUI's key handling.

#### Scenario: A listed key does nothing

- **GIVEN** a binding in the table whose chord, in its context, does not
  produce its effect in the TUI's key handling
- **WHEN** the workspace tests run
- **THEN** the binding check fails and names the binding

#### Scenario: Help text is unchanged

- **WHEN** `/help` or the shortcuts panel renders from the table
- **THEN** its rows match the previous hand-written list byte for byte

### Requirement: The interactive loop is split by event source

The interactive TUI loop SHALL keep its state in one value and handle each
event source and each user action in its own function, so that no single
function holds the whole loop.

#### Scenario: Changing one action

- **WHEN** a developer changes how one user action is handled
- **THEN** the change is inside that action's function
- **AND** the function that runs the loop only selects the next event and
  hands it on
