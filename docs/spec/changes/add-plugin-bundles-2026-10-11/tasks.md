---
created_at: 2026-10-11T01:38:05Z
updated_at: 2026-10-11T01:44:10Z
completed_at:
---

## 1. Plugin model

- [ ] 1.1 Add `crates/smith-plugin`: manifest parsing for
  `.claude-plugin/plugin.json` with a `.smith-plugin/plugin.json` overlay,
  unknown keys ignored and path rules (`./` prefix, containment, existence).
- [ ] 1.2 Standard-layout scan producing an inventory: loaded components
  (skills, commands, MCP servers) and recognised-but-not-loaded components
  with a reason each.
- [ ] 1.3 Content digest over the manifest, loaded component files, and
  resolved MCP declarations; stable across platforms.
- [ ] 1.4 Variable substitution for the three path variables; detection of
  `${user_config.*}` references.
- [ ] 1.5 Unit tests against `plugins/claude-code/plugins/smith` and fixture
  plugins covering every inventory outcome.

## 2. Store and sources

- [ ] 2.1 Plugin store under the user state root: versioned install
  directories, data directories, `installed.toml`, atomic install and
  remove, symlink-escape refusal.
- [ ] 2.2 Sources: local directory; `github:owner/repo[#ref]` and git URLs
  through the user's `git`, with a clear error when `git` is missing.
- [ ] 2.3 Marketplaces: add, list, update, remove; parse
  `.claude-plugin/marketplace.json`; resolve `name@marketplace` for
  relative-path, GitHub, and git-URL sources; report npm sources.
- [ ] 2.4 Update: fetch, compute the new digest, present the inventory
  difference, keep the old version until confirmed.

## 3. Configuration and trust

- [ ] 3.1 `[plugins.<name>] enabled` in the config model with layered
  resolution and provenance; unknown names reported, not fatal.
- [ ] 3.2 Bind install trust to the digest under the existing extension
  trust kind; a changed digest withholds the plugin until confirmed.
- [ ] 3.3 Non-interactive rules: enabled trusted plugins load; nothing is
  installed, updated, or trusted without confirmation or `--yes`.

## 4. Runtime wiring

- [ ] 4.1 Plugin skill layer with `plugin:name` names, between built-in and
  user, through the existing skill source resolver.
- [ ] 4.2 Plugin command layer with `plugin:name` names through the file
  command discovery.
- [ ] 4.3 Plugin MCP servers as `plugin:server` declarations through the
  existing MCP path, with path variables in the process environment.
- [ ] 4.4 Record each enabled plugin as modules with user-manifest
  provenance; rebuild at the safe boundary on enable, disable, install,
  update, and remove.

## 5. Surfaces

- [ ] 5.1 `smith plugin add|list|update|remove` and `smith plugin
  marketplace add|list|update|remove`, with the install confirmation and
  `--yes`.
- [ ] 5.2 `/plugins` listing and `/plugins <name> on|off` through the
  previewed user-config edit.
- [ ] 5.3 Plugin entries in `/skills`, `/commands`, `/mcp`, and `/help`
  labelled with their plugin.
- [ ] 5.4 Reducer, render, command, and fixture tests for every state.

## 6. Verification

- [ ] 6.1 End-to-end test: install the in-repo Claude Code plugin fixture
  from a local path and from a local git repository, run one of its
  commands, activate one of its skills.
- [ ] 6.2 Architecture tests for the new crate's boundaries.
- [ ] 6.3 `docs/plugins.md` (install, trust, what is and is not loaded,
  authoring) and README.
- [ ] 6.4 Full gate with `--no-fail-fast`, including the minimal build.
