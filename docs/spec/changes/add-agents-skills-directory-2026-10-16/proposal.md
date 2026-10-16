---
created_at: 2026-10-16T20:40:00Z
updated_at: 2026-10-17T00:30:00Z
---

## Why

Skills installed the standard way do not reach Smith. The shared skill
location used by Codex and by skill installers such as `npx skills add` is
`.agents/skills/<name>/SKILL.md`: `$HOME/.agents/skills` for the user and
`<repo>/.agents/skills` for a project. Smith reads only `~/.smith/skills`
and `<project>/.smith/skills`, so the owner has to copy or symlink every
skill. The owner asked on 2026-10-16 for Smith to load skills from the
standard location.

## What Changes

- The user layer also reads `<home>/.agents/skills/<name>/SKILL.md`. Here
  `<home>` is the directory that holds the user state root (`<home>/.smith`).
- The workspace layer also reads `<project>/.agents/skills/<name>/SKILL.md`,
  under the same hash-bound project trust, containment, pinning, bounds, and
  problem reporting as `.smith/skills`.
- Within one layer, a `.smith/skills/<name>` skill wins over
  `.agents/skills/<name>`. The shadowed one is reported by name and path in
  `/skills`. It is not a duplicate-name error, and it does not stop startup.
- Both locations stay fixed, not configurable, and are never created. Layer
  precedence (built-in < plugin < user < workspace < session) is unchanged.
- `docs/skills.md` documents the four locations.
- Amended after the live check: discovery read the owner's 81 skills in
  `~/.agents/skills`, but the frontmatter grammar refused at least 21 of
  them. 26 use a YAML block scalar for `description` (`description: >` or
  `|`). The grammar gains that one YAML form:
  - A `description` value may be a block scalar (`>`, `|`, optionally with
    `-` or `+`) or a plain value continued on indented lines.
  - Indented lines under any other key are skipped instead of refused.
  - A value wrapped in matching quotes is unquoted.
  - An indented line never sets a top-level key.
  - Nothing else from YAML is accepted, and the directory-name rule is
    unchanged.
- The remaining 18 refusals were `chatcut-*` skills whose `name` drops the
  directory's prefix (folder `chatcut-voice` declares `name: voice`). On
  2026-10-16 the owner chose to load these. A skill is still named only by
  its directory, so the frontmatter `name` cannot redirect shadowing. A
  differing `name` is now ignored instead of refused.

## Impact

- Affected specs: `configuration`, `harness-policy`
- Affected code:
  - `crates/smith-runtime/src/skills/discovery.rs`: `discover_into` plus
    per-layer merging
  - `crates/smith-cli/src/skills.rs`: the `SkillContext` workspace path map
    used by `/skills trust`
  - `crates/smith-runtime/src/skills/discovery.rs`: `parse_front_matter`
  - `docs/skills.md`
- No configuration keys, no migration.
