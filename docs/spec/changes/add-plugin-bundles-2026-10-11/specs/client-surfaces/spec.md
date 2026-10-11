## ADDED Requirements

### Requirement: Plugin management commands

Smith SHALL provide `smith plugin add`, `list`, `update`, and `remove`, and
`smith plugin marketplace add`, `list`, `update`, and `remove`. The TUI
SHALL provide `/plugins`, listing each installed plugin with its state (on,
off, changed, or failed), source, version, loaded components, and components
not loaded with the reason for each. `/plugins <name> on|off` MUST write
only the user configuration file through the previewed, rollback-capable
edit and apply at the next safe boundary. Removing a plugin MUST delete its
installed content and MUST keep its data directory unless the user asks to
delete it.

#### Scenario: List shows what is not loaded

- **GIVEN** an installed plugin with two skills, one command, and hooks
- **WHEN** the user runs `/plugins`
- **THEN** its row lists the skills and the command as loaded
- **AND** lists its hooks as not run

#### Scenario: Switch a plugin off in a session

- **GIVEN** an enabled plugin
- **WHEN** the user runs `/plugins deploy-tools off` and confirms
- **THEN** `plugins.deploy-tools.enabled = false` is written to the user file
- **AND** from the next safe boundary its skills, commands, and MCP servers
  are absent

#### Scenario: Remove keeps data

- **WHEN** the user runs `smith plugin remove deploy-tools`
- **THEN** its installed content and install record are deleted
- **AND** its data directory remains

### Requirement: Plugin content is attributed where it appears

Skills, commands, and MCP servers that come from a plugin SHALL be labelled
with their plugin in `/skills`, `/commands`, `/mcp`, `/help`, and slash
completion, and MUST appear under their namespaced name.

#### Scenario: Plugin command in help

- **GIVEN** an enabled plugin `deploy-tools` with a `status` command
- **WHEN** the user opens `/help`
- **THEN** `/deploy-tools:status` is listed and labelled as from
  `deploy-tools`

### Requirement: Withheld plugin command invocations fail closed

Smith SHALL refuse an invocation in an installed plugin namespace whose
plugin is changed, untrusted, failed, or off through the file-command path and
MUST NOT be submitted as ordinary prompt text. The refusal MUST name the
command and plugin and state the reason and remedy. Changed content MUST
name `smith plugin update <name>`; off plugins MUST name `/plugins <name> on`
or `plugins.<name>.enabled = true`. Unknown uninstalled prefixes and the
literal `//` escape MUST keep their existing behavior.

#### Scenario: Headless withheld plugin invocation

- **GIVEN** an installed plugin `deploy-tools` that is changed or off
- **WHEN** the user runs `smith -p "/deploy-tools:status api"`
- **THEN** Smith exits non-zero with the command, plugin, reason and remedy
- **AND** no provider request is issued

#### Scenario: Terminal withheld plugin invocation

- **GIVEN** an installed plugin that is changed, untrusted, failed, or off
- **WHEN** the user invokes a command in its namespace
- **THEN** Smith shows the same local refusal as headless preflight
- **AND** no input is submitted or queued for the provider

#### Scenario: Unknown plugin prefix and literal escape

- **GIVEN** no installed plugin named `unknown`
- **WHEN** the user submits `/unknown:status` or `//deploy-tools:status`
- **THEN** each follows its existing unknown-input or literal-input route
