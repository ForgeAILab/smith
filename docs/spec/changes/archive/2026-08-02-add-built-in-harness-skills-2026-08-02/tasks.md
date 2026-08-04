---
created_at: 2026-08-02T02:44:11Z
updated_at: 2026-08-02T03:05:00Z
completed_at: 2026-08-02T03:05:00Z
---

## 1. Implementation

- [x] 1.1 Add `crates/smith-runtime/src/built_in_skills.rs` declaring the
      reference set (`smith.configuration`, `smith.headless`,
      `smith.persistence`, `smith.security`) with `include_str!` bodies from
      `docs/` and authored task-oriented descriptions.
- [x] 1.2 Seed the factory's default `SmithSkillSources` with the built-in
      set so TUI and `smith -p` compose one identical index; explicit
      embedder-supplied sources replace it entirely.
- [x] 1.3 Tests: names validate; built-ins resolve as activatable
      host-policy entries; user/session declarations shadow by name while
      the built-in stays indexed; descriptor resolution performs no body
      I/O; activation returns the exact embedded document.
- [x] 1.4 Write `docs/skills.md` documenting the skill source layers, trust
      gating, and the shipped built-in reference set.
- [x] 1.5 Validate: `cargo fmt --check`, `cargo clippy` (warnings as
      errors), workspace tests. Ran in a clean worktree at HEAD plus only
      this change, because unrelated in-progress reasoning-controls work
      breaks the primary working tree's build.
