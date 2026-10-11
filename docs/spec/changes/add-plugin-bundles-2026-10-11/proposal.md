---
created_at: 2026-10-11T01:38:05Z
updated_at: 2026-10-11T01:38:05Z
---

## Why

Smith can now be composed from modules, but only by people who compile it.
The community tier is still missing: there is no way to install something
another person wrote. Claude Code and Codex built their ecosystems on
installable bundles of content (skills, commands, MCP servers) fetched from
git, and a large catalogue in that format already exists.

Smith already loads each of those pieces on its own: skills from
`skills/<name>/SKILL.md`, file commands from `commands/<name>.md`
(`add-file-commands`), and MCP servers from configuration. This change adds
the package around them: a plugin is a directory in the Claude Code layout
that Smith can install, trust, enable, update, and remove as one unit.

## What Changes

- A plugin is a directory with an optional `.claude-plugin/plugin.json`
  manifest and components in the Claude Code standard layout. Smith reads
  the same files, so an existing Claude Code plugin installs unchanged.
- Components loaded in this change: `skills/` (and a root `SKILL.md`),
  `commands/*.md`, and MCP servers from `.mcp.json` or the manifest's
  `mcpServers`. Skills and commands are namespaced `plugin:name`.
- Components recognised but not loaded yet are listed per plugin with the
  reason: `hooks`, `agents`, `lspServers`, `outputStyles`, `workflows`,
  `themes`, `monitors`, `channels`, `bin/`, `settings`, and `.mcpb` bundles.
  A plugin never fails to install because it carries one.
- `${CLAUDE_PLUGIN_ROOT}`, `${CLAUDE_PLUGIN_DATA}`, and
  `${CLAUDE_PROJECT_DIR}` resolve in MCP server `command`, `args`, and
  `env`, and in skill and command bodies. A component that references
  `${user_config.*}` is listed as needing configuration and is not loaded.
- `smith plugin add <source>` installs from a local directory or a git
  source (`github:owner/repo[#ref]` or a git URL). `smith plugin
  marketplace add <source>` registers a catalogue that carries a
  `.claude-plugin/marketplace.json`; `smith plugin add name@marketplace`
  installs from it. Relative-path, GitHub, and git-URL plugin sources are
  supported; npm sources are reported as unsupported.
- Installing is a trust decision. Smith shows what the plugin would load
  (each skill and command, each MCP server's resolved command line or URL,
  and what it carries but Smith will not load) with a digest of the content,
  and installs only after confirmation. The decision is bound to that
  digest. An update that changes content is shown as a change and confirmed
  again; until then the installed version keeps running.
- Installed plugins live under the user state root, one directory per
  plugin and version, with a separate data directory that survives updates.
  `[plugins.<name>] enabled` switches a plugin through the ordinary layers.
- `/plugins` and `smith plugin list` show each plugin, its state, its
  source and version, its components, and anything not loaded. `/plugins
  <name> on|off`, `smith plugin update`, and `smith plugin remove` manage
  them. A change applies at the next safe boundary.
- Each enabled plugin is recorded as modules in composition evidence with
  user-manifest provenance: content-only for skills and commands, trusted
  MCP for its servers. A plugin grants no capability by being installed.
- In non-interactive runs an installed, enabled plugin loads; nothing is
  ever installed, updated, or trusted without an interactive confirmation
  or an explicit `--yes` on the `smith plugin` command.

## Out of Scope

- Hooks. A plugin's hooks are listed and not run. `add-shell-hooks` adds the
  hook engine and turns them on for plugins.
- Plugin-provided agents, LSP servers, output styles, workflows, themes,
  monitors, channels, `bin/` on the tool PATH, and `.mcpb` bundles.
- `userConfig` prompting and secret storage for plugin options.
- Project-scoped or managed plugin installs, plugin dependencies, npm
  sources, and automatic background updates.
- WASM modules inside a plugin (`add-wasm-modules`, which builds on this).
- A Smith-hosted directory or publishing flow.

## Impact

- Affected specs: `extension-system`, `configuration`, `client-surfaces`.
- Affected code: new `crates/smith-plugin` (manifest, layout scan, sources,
  store, digest); `crates/smith-config` (`plugins` table, trust binding);
  `crates/smith-runtime` (skill, command, and MCP sources from plugins,
  module records); `crates/smith-client`, `crates/smith-tui` (`/plugins`);
  `crates/smith-cli` (`smith plugin` commands).
- Depends on `add-module-kernel` (module records, safe-boundary rebuild) and
  on `add-file-commands` (command discovery and the `slash_command` trust
  kind), which is on local `main` and conflicts with `add-module-kernel` in
  two files until one is merged over the other.
- New dependency risk: fetching git sources. The design shells out to the
  user's `git` rather than adding a git library.
