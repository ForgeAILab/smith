# Plugins

A plugin is a directory of skills, prompt commands, and MCP server declarations
in the Claude Code plugin layout. Smith copies it into your user state root and
loads its admitted components through the same catalogs as your own skills,
commands, and MCP configuration. Names are qualified by the plugin:
`smith:smith-profiles`, `/smith:profiles`, or `deploy-tools:server`.

## Install and inspect

```sh
smith plugin add ./deploy-tools
smith plugin add github:owner/repo#main
smith plugin add https://example.com/owner/repo.git
smith plugin list
```

The source may be a local directory, GitHub shorthand, or a git URL. Git sources
use the `git` executable on your PATH; Smith reports an error if it is missing.
A git source may include `#branch-or-tag`. There are no automatic updates.

Before installation Smith prints the name, version, source, resolved git commit,
each skill and command, each MCP server's resolved command line or URL, and
components it will not load with their reasons. Hooks are explicitly listed as
**will not run**. The review also shows the bundle digest. Answer
`Install and trust? [y/N]` to install. An empty answer cancels.

```sh
smith plugin add ./deploy-tools --yes
```

`--yes` prints the same review and confirms it explicitly. Without a terminal,
installation and updates require `--yes`; Smith never reads piped input as an
installation confirmation. Plugin management does not start an agent session.

The SHA-256 digest covers every file in the plugin directory, including scripts,
assets, unsupported components, and raw manifest bytes. Relative paths use `/`,
are sorted, and are framed separately from file bytes. The effective bundle name
is also bound to the digest. `.git`, `.hg`, `.svn`, and `.bzr` metadata is excluded.
The installation's location is not part of the identity. Editing `server.js` is
therefore a trust change even if `.mcp.json` is unchanged.

Installation copies content under
`~/.smith/plugins/store/<name>/<digest-prefix>/` and records it in
`~/.smith/plugins/installed.toml`. It never runs from the source checkout.
Confined internal symlinks are copied as files or directories; escaping symlinks
and cycles are refused. Persistent data lives separately under
`~/.smith/plugins/data/<name>/`.

Trust is one decision about that bundle digest. Its skills, commands, and servers
are not prompted for separately. If installed files change, Smith reports the
plugin as changed and withholds all of it. Headless sessions still start without
its content. Installation grants no new capability or tool authority; MCP tool
calls retain the ordinary approval rules.

## Marketplaces

A marketplace is a directory containing `.claude-plugin/marketplace.json`.
Register it, then install an entry by name:

```sh
smith plugin marketplace add ./catalogue
smith plugin marketplace add github:owner/catalogue
smith plugin marketplace list
smith plugin add deploy-tools@catalogue
smith plugin marketplace update catalogue
smith plugin marketplace update
smith plugin marketplace remove catalogue
```

Entries may use relative plugin paths inside the stored catalogue, GitHub
sources, or git URLs (including a confined subdirectory). npm entries are
reported as unsupported. Updating a marketplace refreshes its catalogue copy;
it does not update installed plugins. Removing a marketplace leaves installed
plugins intact.

## Enable, update, and remove

`smith plugin list` and `/plugins` show on, off, changed, untrusted, or failed
states, source/version, admitted components, and anything not loaded with its
reason. Settings naming plugins that are no longer installed are also reported.

```toml
[plugins.deploy-tools]
enabled = false

[profiles.work.plugins.deploy-tools]
enabled = true
```

Enablement follows ordinary configuration precedence. The manifest's
`defaultEnabled` is the default, otherwise the plugin starts enabled. Plugin
skills and commands sit above built-in content and below user, trusted workspace,
and session layers. Their namespaced names coexist with unprefixed user content.

In a session, `/plugins deploy-tools on` or `/plugins deploy-tools off` previews
an edit to the user configuration file. Confirmation has no default action. The
change applies at the next safe boundary, with rollback if the rebuilt session
cannot start. If a higher layer overrides the user file, Smith reports that
layer instead of claiming the requested change took effect. Unknown names are
refused with the installed names listed. Installing, updating, and removing are
CLI operations; the TUI directs you to `smith plugin …`.

```sh
smith plugin update deploy-tools
smith plugin update --yes
smith plugin remove deploy-tools
smith plugin remove deploy-tools --delete-data
```

Updates fetch the recorded source again and show added, removed, and changed
components, including old and new MCP command lines and the new digest. The old
installation is retained until confirmation. An unchanged trusted installation
reports `Already up to date`. Locally edited installations can be updated: the
review says those edits will be discarded on confirmation. Re-running
`smith plugin add` for an installed name uses the same update review, and can
replace its recorded source. Changes made after review invalidate the commit.

Removal deletes installed content, its install record, and its trust record.
Data survives unless `--delete-data` is supplied. A later install must be trusted
again.

## Author a bundle

Use the [shipped Claude Code plugin](../plugins/claude-code/plugins/smith/) as a
working example:

```text
.claude-plugin/plugin.json     # optional name, version, component declarations
.smith-plugin/plugin.json      # optional Smith-specific overlay
skills/<name>/SKILL.md
commands/<name>.md
.mcp.json
```

The `.smith-plugin/plugin.json` overlay replaces Claude manifest keys one key at
a time. Put Smith-specific keys there. Unknown manifest keys are ignored.
Without a manifest, the source directory or marketplace entry supplies the name.
Manifest paths must be `./`-relative, contained within the plugin, and exist.
Invalid component declarations are listed with a reason; other components remain
usable. A root `SKILL.md` is supported for a single-skill bundle.

Skills use the existing descriptor-first, digest-pinned skill path. Commands are
prompt templates: `$ARGUMENTS` expands to the supplied arguments, or arguments
are appended when no placeholder exists. Shell-looking text in a template is
ordinary prompt content; Smith does not execute it as a hook or preprocessor.
MCP declarations come from `.mcp.json` or manifest `mcpServers` and support stdio
and streamable HTTP through the existing MCP resolver.

`${CLAUDE_PLUGIN_ROOT}`, `${CLAUDE_PLUGIN_DATA}`, and `${CLAUDE_PROJECT_DIR}`
expand in skill and command bodies, and MCP command, arguments, environment,
URL, and headers. Stdio MCP processes also receive `CLAUDE_PLUGIN_ROOT` and
`CLAUDE_PLUGIN_DATA` in their environment. Other variables stay literal.
`${user_config.*}` is not supported: a component requiring it is listed as
needing configuration and is not loaded.

Hooks do not run. Agents, LSP servers, output styles, workflows, themes, monitors,
channels, `bin/`, settings, user configuration prompts, and MCP bundle files
(`.mcpb`/`.dxt`) are listed but not loaded. `bin/` is not added to the tool PATH.

A plugin contributes existing content catalogs without native code in Smith.
A [module](modules.md) is a compiled Rust component with explicit build and mount
contracts. Use a plugin for portable content and MCP declarations; use a module
for native tools or pipeline behavior.
