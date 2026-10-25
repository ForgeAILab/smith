## MODIFIED Requirements

### Requirement: Detected skills are directly invokable slash commands

Smith SHALL expose the effective detected skill names as `/<skill-name>` in
the shared slash completion, command palette, help, and parser. Rows SHALL
carry bounded description, source, and availability metadata. Built-in and
existing file commands MUST retain name precedence, and `/skills use <name>
[task]` SHALL provide an explicit skill route for collisions. Existing
`/skills` listing and trust commands SHALL retain their behavior.

#### Scenario: Discover and complete a skill

- **GIVEN** an indexed skill named `rust-review` with a description
- **WHEN** the user opens slash completion or the command palette
- **THEN** `/rust-review` is discoverable with that description and its source
- **AND** Tab completes the name without loading instructions or executing it

#### Scenario: Invoke a skill without task text

- **GIVEN** an idle provider-backed session with an authorized `rust-review` skill
- **WHEN** the user submits `/rust-review`
- **THEN** Smith activates that exact skill before acknowledging success
- **AND** immediately starts an ordinary user turn with the verified skill
  instructions as its prompt body
- **AND** the invocation does not require a second user prompt

#### Scenario: Invoke with a task

- **GIVEN** an idle provider-backed session with an authorized `rust-review` skill
- **WHEN** the user submits `/rust-review inspect these changes`
- **THEN** Smith activates the skill before the task's first provider request
- **AND** the prompt contains the verified skill instructions followed by a
  blank line and `inspect these changes` through the ordinary prepared
  submission path
- **AND** typed history retains the invocation and attachments remain intact
- **AND** only task text is parsed for composer references; the authored skill
  instructions are preserved verbatim

#### Scenario: Invoke an already active skill

- **GIVEN** an idle provider-backed session where `rust-review` is already active
- **WHEN** the user submits `/rust-review` or `/skills use rust-review`
- **THEN** Smith verifies the selected skill and starts another user turn with
  its instruction body
- **AND** no duplicate active instruction fragment is appended to the session

#### Scenario: A skill name collides with a command

- **GIVEN** a skill and a built-in or file command have the same name
- **WHEN** that slash name is invoked
- **THEN** the existing command keeps its route
- **AND** the skill's availability details identify the collision and its
  `/skills use <name>` alternative
- **AND** the explicit alternative resolves the skill rather than the command

#### Scenario: Activation is unavailable

- **GIVEN** a skill is unknown, withheld, changed, denied, or over budget
- **WHEN** the user invokes it
- **THEN** Smith reports a named local reason and applicable trust guidance
- **AND** no task or provider request is submitted on its behalf
- **AND** the current session remains usable

#### Scenario: A turn is already running

- **WHEN** the user completes a skill name while a turn runs
- **THEN** completion remains available
- **WHEN** the user tries to apply that skill
- **THEN** Smith explains that activation requires an idle session
- **AND** no body is loaded and the in-flight turn is unchanged

#### Scenario: The selected backend is an installed CLI

- **GIVEN** an installed CLI owns the active model context
- **WHEN** the user inspects or invokes a skill shortcut
- **THEN** direct activation is marked unavailable with guidance to the
  existing skill MCP bridge
- **AND** Smith does not claim that a native epoch activated it in the CLI

