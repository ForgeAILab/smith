---
created_at: 2026-10-16T20:40:00Z
updated_at: 2026-10-16T20:40:00Z
completed_at:
---

## 1. Implementation
- [x] 1.1 `discover_into` also reads `<user_root parent>/.agents` for the
      user layer and `<project>/.agents` for the workspace layer, using the
      existing `discover(root)`. For each layer, merge the two locations by
      name: `.smith` wins, and the `.agents` copy becomes a `SkillProblem`
      naming both paths. Workspace `.agents` skills go through
      `workspace_trust` exactly as `.smith` ones do.
- [x] 1.2 `SkillContext::compose` (`crates/smith-cli/src/skills.rs`) builds
      its workspace name→path map from the same merged result, so
      `/skills trust NAME` shows the right project-relative path for an
      `.agents` skill. Do not discover a second time.
- [x] 1.3 `docs/skills.md`: "Where skills live" lists all four locations,
      the `.smith`-wins rule, and that `npx skills add` installs work.

## 2. Tests
- [x] 2.1 Discovery: a user `.agents` skill lands in the user layer; a
      project `.agents` skill lands in the workspace layer with trust status;
      a same-name clash in each layer declares the `.smith` copy and reports
      the `.agents` one; a symlink escape is refused; nothing is created when
      no directory exists.
- [x] 2.2 Composition (`crates/smith-runtime/tests/composition/skills_and_memory.rs`):
      a session with skills in both `.agents` locations resolves without a
      duplicate-name error.
- [x] 2.3 `/skills trust NAME` for a project `.agents` skill records a
      decision for `.agents/skills/NAME/SKILL.md`.

## 3. Verification
- [x] 3.1 Full gate.
- [x] 3.2 Live: `npx skills add` (or a hand-made `~/.agents/skills/x/SKILL.md`)
      shows up in `/skills`.
