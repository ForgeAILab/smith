---
created_at: 2026-10-11T01:38:05Z
updated_at: 2026-10-11T04:04:13Z
completed_at: 2026-10-11T03:59:02Z
---

## 1. Plugin model

- [x] 1.1 Add `crates/smith-plugin`: manifest parsing for
  `.claude-plugin/plugin.json` with a `.smith-plugin/plugin.json` overlay,
  unknown keys ignored and path rules (`./` prefix, containment, existence).
- [x] 1.2 Standard-layout scan producing an inventory: loaded components
  (skills, commands, MCP servers) and recognised-but-not-loaded components
  with a reason each.
- [x] 1.3 Content digest over every file in the plugin directory; stable
  across platforms.
- [x] 1.4 Variable substitution for the three path variables; detection of
  `${user_config.*}` references.
- [x] 1.5 Unit tests against `plugins/claude-code/plugins/smith` and fixture
  plugins covering every inventory outcome.

## 2. Store and sources

- [x] 2.1 Plugin store under the user state root: versioned install
  directories, data directories, `installed.toml`, atomic install and
  remove, symlink-escape refusal.
- [x] 2.2 Sources: local directory; `github:owner/repo[#ref]` and git URLs
  through the user's `git`, with a clear error when `git` is missing.
- [x] 2.3 Marketplaces: add, list, update, remove; parse
  `.claude-plugin/marketplace.json`; resolve `name@marketplace` for
  relative-path, GitHub, and git-URL sources; report npm sources.
- [x] 2.4 Update: fetch, compute the new digest, present the inventory
  difference, keep the old version until confirmed.

## 3. Configuration and trust

- [x] 3.1 `[plugins.<name>] enabled` in the config model with layered
  resolution and provenance; unknown names reported, not fatal.
- [x] 3.2 Bind install trust to the digest under the existing extension
  trust kind; a changed digest withholds the plugin until confirmed.
- [x] 3.3 Non-interactive rules: enabled trusted plugins load; nothing is
  installed, updated, or trusted without confirmation or `--yes`.

## 4. Runtime wiring

- [x] 4.1 Plugin skill layer with `plugin:name` names, between built-in and
  user, through the existing skill source resolver.
- [x] 4.2 Plugin command layer with `plugin:name` names through the file
  command discovery.
- [x] 4.3 Plugin MCP servers as `plugin:server` declarations through the
  existing MCP path, with path variables in the process environment.
- [x] 4.4 Record each enabled plugin as modules with user-manifest
  provenance; rebuild at the safe boundary on enable, disable, install,
  update, and remove.

## 5. Surfaces

- [x] 5.1 `smith plugin add|list|update|remove` and `smith plugin
  marketplace add|list|update|remove`, with the install confirmation and
  `--yes`.
- [x] 5.2 `/plugins` listing and `/plugins <name> on|off` through the
  previewed user-config edit.
- [x] 5.3 Plugin entries in `/skills`, `/commands`, `/mcp`, and `/help`
  labelled with their plugin.
- [x] 5.4 Reducer, render, command, and fixture tests for every state.

## 6. Verification

- [x] 6.1 End-to-end test: install the in-repo Claude Code plugin fixture
  from a local path and from a local git repository, run one of its
  commands, activate one of its skills.
- [x] 6.2 Architecture tests for the new crate's boundaries.
- [x] 6.3 `docs/plugins.md` (install, trust, what is and is not loaded,
  authoring) and README.
- [x] 6.4 Full gate with `--no-fail-fast`, including the minimal build.

## 7. Withheld command invocation follow-up

- [x] 7.1 Retain installed inactive namespaces in command preflight, refuse
  locally with the plugin reason and remedy, and preserve unknown prefixes
  and literal slash escapes.
- [x] 7.2 Verify changed/off headless refusals without provider requests,
  terminal refusals, and unchanged unknown-prefix/escape handling with the
  permitted plugin tests and CLI/client clippy checks.
