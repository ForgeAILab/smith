## Context

`add-module-kernel` gave Smith compile-time modules. Runtime-installable
code is `add-wasm-modules`. This change is the tier between them: content
that needs no code loaded into Smith. Claude Code's plugin layout is the
de-facto format (Codex reads a near-identical one), and Smith already ships
itself as a Claude Code plugin under `plugins/claude-code/`.

Smith's skills already match that layout (`skills/<name>/SKILL.md`, unknown
frontmatter keys ignored). File commands landed with `add-file-commands`.
MCP servers have declarative config, hash-bound trust, and lazy connection.

## Goals / Non-Goals

- Goals:
  - Install an unmodified Claude Code plugin and get its skills, commands,
    and MCP servers working.
  - One trust decision per plugin version, made with the content in view.
  - Reuse the existing skill, command, and MCP paths; add no second loader.
- Non-Goals:
  - Feature parity with Claude Code's plugin system.
  - Running any plugin-supplied program other than its declared MCP servers.

## Decisions

### Read the Claude Code layout, do not define a new one

Smith reads `.claude-plugin/plugin.json` and the standard directories. A
`.smith-plugin/plugin.json`, when present, is read instead, so an author can
ship Smith-specific fields without breaking other tools. Unknown manifest
keys are ignored, as Claude Code does.

- Why: an existing catalogue on day one, and authors target one format.
- Cost: Smith inherits names it does not control. Mitigated by listing
  every recognised-but-unsupported component instead of failing.

### A plugin is a source of existing things, not a new runtime object

A plugin contributes to the catalogs Smith already has: a skill layer, a
command layer, and MCP server declarations. Precedence: built-in < plugin <
user < trusted workspace < session for skills and commands. Plugin entries
are always namespaced (`plugin:name`), so they never shadow or get shadowed
by an unprefixed name. MCP servers are named `plugin:server`.

### Trust is per plugin version, bound to a content digest

The digest covers the manifest, every loaded component file, and the
resolved MCP declarations. It is recorded under the existing
`ExecutableKind::Extension` kind. Component-level trust prompts (per skill,
per MCP server) are not shown again for plugin content: the plugin
confirmation already displayed them. A file changed on disk after install
makes the digest differ; the plugin is then treated as changed and does not
load until confirmed, exactly like a changed project skill.

- Alternative considered: reuse per-component trust (`/skills trust`,
  `/mcp trust`) for each item. Rejected: a plugin with twenty skills would
  need twenty confirmations, and users would stop reading them.

### Installing copies; nothing runs from the source checkout

`smith plugin add` resolves the source, copies the plugin directory to
`<state>/plugins/store/<name>/<digest-prefix>/`, and records source, ref,
resolved commit, version, and digest in `<state>/plugins/installed.toml`.
`${CLAUDE_PLUGIN_DATA}` is `<state>/plugins/data/<name>/`. Git sources are
fetched by running the user's `git` with a shallow clone into a temporary
directory; Smith adds no git library. Symlinks that escape the plugin root
are refused, as skill discovery already does.

Marketplaces are stored the same way under `<state>/plugins/marketplaces/`
and refreshed only on `smith plugin marketplace update`.

### No hooks here

Hooks are the one bundle component that executes arbitrary programs inside
the turn loop, and they need an interception seam (before a tool runs,
before a prompt is submitted) that Smith does not have yet. `add-file-commands`
already names shell hooks as its next block. Keeping them out lets this
change ship on existing seams only. The install confirmation and `/plugins`
say plainly that the plugin's hooks will not run.

### Variable substitution

`${CLAUDE_PLUGIN_ROOT}`, `${CLAUDE_PLUGIN_DATA}`, `${CLAUDE_PROJECT_DIR}`
are substituted in MCP `command`, `args`, `env`, `url`, `headers`, and in
skill and command bodies. Stdio MCP servers also receive the first two as
environment variables. Any other `${...}` is left as written. A component
referencing `${user_config.*}` is not loaded and is listed as needing
configuration.

### Namespaced skills and discovery

Plugin skills are model-activated like any other skill and enter the same
descriptor-first retrieval. The `add-capability-discovery` limits apply to
them unchanged.

## Risks / Trade-offs

- Supply chain: a plugin's MCP server is a program the user runs. The
  confirmation shows the exact command line; this is the same risk and the
  same control as a hand-written MCP entry.
- Format drift: Claude Code may change its layout. Smith ignores what it
  does not know and lists it.
- Many plugins mean many skills competing for retrieval. Existing capability
  limits bound it; no new mechanism here.

## Migration Plan

1. `smith-plugin` crate: manifest parse, layout scan, digest, inventory.
   Pure functions, tested against `plugins/claude-code/plugins/smith` and
   fixture plugins.
2. Store and sources: local path first, then git, then marketplaces.
3. Wire plugin layers into skills, commands, and MCP.
4. Surfaces: `smith plugin …`, `/plugins`.

## Open Questions

- Should `smith plugin add` accept `--yes` in headless use, or should
  installation always need a terminal? Proposed: accept `--yes`, print the
  inventory first.
- Whether to read Codex's `.codex-plugin/plugin.json` too. Proposed: not in
  this change.
