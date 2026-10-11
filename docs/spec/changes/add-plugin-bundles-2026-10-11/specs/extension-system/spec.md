## ADDED Requirements

### Requirement: Installable plugin bundles in the Claude Code layout

Smith SHALL treat a directory in the Claude Code plugin layout as an
installable plugin: an optional `.claude-plugin/plugin.json` manifest plus
components in their standard locations. A `.smith-plugin/plugin.json`, when
present, MUST be read instead of the Claude Code manifest. Unknown manifest
keys MUST be ignored. A manifest path that is not `./`-relative, resolves
outside the plugin root, or does not exist MUST make that component fail to
load without failing the plugin.

#### Scenario: Existing Claude Code plugin is read unchanged

- **GIVEN** a directory with `.claude-plugin/plugin.json`, `skills/deploy/SKILL.md`,
  `commands/status.md`, and `.mcp.json`
- **WHEN** Smith inspects it as a plugin
- **THEN** its inventory lists one skill, one command, and the declared MCP
  servers under the manifest's name

#### Scenario: Plugin without a manifest

- **GIVEN** a directory with `skills/` and no manifest
- **WHEN** Smith inspects it as a plugin
- **THEN** it is accepted and named after its directory or marketplace entry

#### Scenario: Component path escapes the plugin root

- **GIVEN** a manifest whose `skills` entry is `./../other`
- **WHEN** Smith inspects the plugin
- **THEN** that entry is reported as escaping the plugin root and is not loaded
- **AND** the plugin's other components are still listed

### Requirement: Plugin components load through existing catalogs

An enabled, trusted plugin SHALL contribute its skills, file commands, and
MCP servers through the same resolvers that load user and project ones, and
Smith MUST NOT add a second loader for them. Plugin skills and commands MUST
be named `<plugin>:<name>` and plugin MCP servers `<plugin>:<server>`, so a
plugin entry never shadows and is never shadowed by an unprefixed name.
`${CLAUDE_PLUGIN_ROOT}`, `${CLAUDE_PLUGIN_DATA}`, and `${CLAUDE_PROJECT_DIR}`
MUST resolve in MCP server command, arguments, environment, URL, and
headers, and in skill and command bodies.

#### Scenario: Plugin command runs

- **GIVEN** an enabled plugin `deploy-tools` with `commands/status.md`
- **WHEN** the user runs `/deploy-tools:status`
- **THEN** the expanded template is submitted as an ordinary user prompt

#### Scenario: Plugin skill does not shadow a user skill

- **GIVEN** a user skill `deploy` and a plugin skill `deploy-tools:deploy`
- **WHEN** the skill catalog resolves
- **THEN** both are present under their own names

#### Scenario: Plugin MCP server finds its own files

- **GIVEN** a plugin MCP server declared with
  `args = ["${CLAUDE_PLUGIN_ROOT}/server.js"]`
- **WHEN** Smith starts the server
- **THEN** the argument is the absolute path inside the installed plugin
- **AND** the process environment carries `CLAUDE_PLUGIN_ROOT` and
  `CLAUDE_PLUGIN_DATA`

### Requirement: Unsupported plugin components are listed, never run

Smith SHALL recognise plugin components it does not load (hooks, agents,
LSP servers, output styles, workflows, themes, monitors, channels, a `bin/`
directory, plugin settings, and MCP bundle files) and MUST list each with
the reason it is not loaded. Such a component MUST NOT prevent installation
and MUST NOT be executed. A component that references `${user_config.*}`
MUST be listed as needing configuration and MUST NOT be loaded.

#### Scenario: Plugin carries hooks

- **GIVEN** a plugin with `hooks/hooks.json`
- **WHEN** the user installs it
- **THEN** the confirmation and the plugin listing state that its hooks will
  not run
- **AND** no hook command is executed in any session

#### Scenario: MCP server needs user configuration

- **GIVEN** a plugin MCP server whose environment references
  `${user_config.api_token}`
- **WHEN** the plugin is enabled
- **THEN** that server is listed as needing configuration and is not started
- **AND** the plugin's skills and commands still load

### Requirement: Plugins are recorded as modules without authority

Each enabled plugin SHALL be recorded in composition evidence with
user-manifest provenance: its skills and commands as content-only
contributions and its MCP servers under the trusted-MCP tier. Installing or
enabling a plugin MUST NOT grant any capability; its MCP tools remain
subject to the existing conservative authority and approval rules.

#### Scenario: Disabled plugin leaves no record

- **GIVEN** an installed plugin that is switched off
- **WHEN** the runtime composition resolves
- **THEN** no module is recorded for it
- **AND** none of its skills, commands, or MCP servers are present

#### Scenario: Plugin MCP tool still needs approval

- **GIVEN** an enabled plugin whose MCP server advertises a mutating tool
- **WHEN** the model calls that tool
- **THEN** the call is subject to the same approval as any other MCP tool
