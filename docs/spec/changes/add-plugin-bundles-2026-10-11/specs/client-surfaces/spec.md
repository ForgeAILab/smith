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
