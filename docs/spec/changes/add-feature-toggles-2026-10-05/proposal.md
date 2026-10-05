---
created_at: 2026-10-05T20:06:57Z
updated_at: 2026-10-05T20:06:57Z
---

## Why

The owner wants Smith's features to be configurable like building blocks, the
way pi treats everything as an extension. Smith already has switches for most
optional behavior, but they are scattered (`cache.idle_compaction`,
`cache.resume_capsule`, `tools.image_generation.enabled`, ...), and nothing
lists which features exist, which are on, or where that choice came from. The
pi-style extension runtime (`extension-system`: brokered subprocess plus a
TypeScript host) is specified but not built, and it is the expensive part.

This change is the cheap first block: one list of built-in features, each
switchable in one place, recorded the same way a future extension would be.
The three planned features (idle compaction summary, project memory, session
fork) then plug into it instead of adding three more scattered switches.

## What Changes

- Add a built-in feature registry in `smith-config`. Each entry has a stable
  id, a one-line description, the existing configuration key that controls
  it, its default, and whether a change applies at the next safe boundary or
  only to new sessions. No existing key moves or is renamed.
- First entries, all global booleans that already exist: `idle-compaction`
  (`cache.idle_compaction`), `resume-capsule` (`cache.resume_capsule`),
  `handoff-checkpoint` (`cache.handoff_checkpoint`), and `image-generation`
  (`tools.image_generation.enabled`). Later changes add their own entries
  (project memory adds `memory`).
- Add `/features` to the TUI: one row per feature with on/off, the controlling
  key, and its provenance (built-in default, user file, project file, ...).
- Add `/features <id> on|off`: prepares a user-config edit through the existing
  previewed, rollback-capable `prepare_user_config_edit` path, asks for
  confirmation, writes `~/.smith/config.toml`, and applies through the
  existing safe-boundary reconfigure used by `/model`. A value that a
  higher-precedence layer overrides is reported, not silently written.
- Add `smith config features` as the read-only headless listing.
- Record each enabled built-in feature as a `ModuleSpec` with `BuiltIn`
  provenance and its contributions, so composition evidence names it the same
  way it names MCP servers today, and a future extension can appear in the
  same list.

## Out of Scope

- The pi-style extension runtime, user-installed plugins, and the TypeScript
  host. They stay specified in `extension-system` and are not started here.
- Per-profile settings such as `delegation`. The first registry holds global
  booleans only; a scope field can be added when a profile-scoped entry is
  wanted.
- `persistence.enabled`, which changes where state lives and is never toggled
  from inside a running session.
- MCP servers, which already have per-server `enabled` and `/mcp`.

## Impact

- Affected specs: `configuration`, `client-surfaces`, `extension-system`.
- Affected code: `crates/smith-config` (registry and resolution),
  `crates/smith-client/src/commands.rs` (`/features`), `crates/smith-tui`
  (list and confirmation), `crates/smith-cli` (`smith config features`,
  reconfigure wiring), `crates/smith-runtime/src/harness.rs` (built-in module
  records).
- Compatibility: additive. Existing configs mean the same thing. Adding module
  records changes the harness composition revision, so resuming a session
  saved by 0.3.9 after upgrade is covered by a test.
- Security: `/features` writes only the user file, through the same reviewed
  edit path as setup. It cannot write project files or enable anything a
  project could not already enable.

## Approval Boundary

Approval covers the registry, the four initial entries, `/features` with its
toggle, `smith config features`, and built-in module records. It does not
cover moving or renaming existing keys, profile-scoped entries, or any part of
the extension runtime.
