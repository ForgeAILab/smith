## ADDED Requirements

### Requirement: Built-in task guides

Smith SHALL ship built-in task guides as skills named `smith.guide.<topic>`,
each embedded at compile time from `docs/guides/<topic>.md` and registered
as one whole-body skill in the built-in layer. The initial topics SHALL be
`overview`, `setup`, `models-and-thinking`, `vision`,
`profiles-and-advisor`, `sessions`, `extending`, `headless`, and
`troubleshooting`. A guide body MUST NOT exceed 1,200 estimated tokens, and
`overview` MUST NOT exceed 600. Each guide MUST name the exact commands and
keys for its task, state for each step whether the user or the agent
performs it, give a verification step, and state that configuration changes
apply when the session is next rebuilt (`/profile`, `/model`, `/resume`, or a
restart). Each guide SHALL carry an authored list of distinctive pre-activation
keywords in place of terms derived from its name and description, so an
everyday coding prompt does not load a guide; its description remains
searchable through `registry.search`. Guides MUST NOT instruct the agent to
read, write, or relay a credential. Guides carry no authority, under the same rule
as the reference skills.

#### Scenario: Agent finds how to enable vision
- **GIVEN** a Smith agent in a workspace without Smith documentation
- **WHEN** the user asks how to let the agent see images
- **THEN** descriptor retrieval offers `smith.guide.vision`
- **AND** activating it returns the shipped `docs/guides/vision.md` bytes
  whole, with no section lookup

#### Scenario: Ordinary prompt loads no guide
- **GIVEN** the headless prompt `edit the file`
- **WHEN** the turn's capabilities are pre-activated
- **THEN** no `smith.guide.*` skill is bound

#### Scenario: Guide size is enforced
- **GIVEN** a guide whose body estimates above its bound
- **WHEN** the test suite runs
- **THEN** a test fails, naming the guide and its estimate

#### Scenario: Guide names only real surfaces
- **GIVEN** a guide that names a `smith` subcommand, a slash command, or a
  configuration key
- **WHEN** the test suite runs
- **THEN** each one named resolves against the CLI help, the slash-command
  registry, or the configuration schema
- **AND** an unknown name fails the test

#### Scenario: Credential steps stay with the user
- **GIVEN** the `setup` guide is activated
- **WHEN** the agent follows it to add a provider
- **THEN** the guide directs the user to run `smith setup credential` or
  `/connect`
- **AND** it gives the agent no step that reads or writes a secret

#### Scenario: Same guides on every host
- **GIVEN** the same resolved configuration
- **WHEN** a session is composed interactively and through `smith -p`
- **THEN** both index the identical `smith.guide.*` set
