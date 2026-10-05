---
created_at: 2026-10-05T20:06:57Z
updated_at: 2026-10-05T20:06:57Z
completed_at:
---

## 1. Store

- [ ] 1.1 Resolve `~/.smith/memory/<project-id>/` from the host's `ProjectId`;
  create it owner-only on first write, never fall back to the workspace.
- [ ] 1.2 Parse and render memory files: frontmatter `name`, `description`,
  `type`, then body; bounded sizes; invalid files reported by name and skipped.
- [ ] 1.3 Atomic same-directory writes and deletes under a project-scoped
  lock, so two Smith processes cannot interleave.
- [ ] 1.4 Generate `MEMORY.md` deterministically: workspace header, one line
  per memory sorted by type then name, omission line when bounded out.
- [ ] 1.5 Unit tests: create, replace, delete, hand edit, malformed file,
  unsafe name, symlink escape, concurrent writers, secret refusal.

## 2. Context

- [ ] 2.1 File-backed memory source that snapshots the index at session start,
  resume, and host rebuild, split into always-on records within the runtime
  and Smith bounds.
- [ ] 2.2 Test that a write mid-session does not change the next request's
  memory records, and does change them in the next session.
- [ ] 2.3 Test that memory never enters canonical history and that children
  see the same index.

## 3. Tool and prompt

- [ ] 3.1 `memory` tool with `list` (optional substring filter), `read`,
  `write`, `delete`; names are safe single components.
- [ ] 3.2 `write`/`delete` registered for the main agent in `build`; `plan`,
  `review`, and children get `list`/`read` only; no approval prompt under the
  default posture.
- [ ] 3.3 Prompt guidance fragment: what to save and not save, update instead
  of duplicate, verify recalled memory before acting.
- [ ] 3.4 Tool and posture tests, including headless.

## 4. Settings and surfaces

- [ ] 4.1 `memory.enabled` (default true), owner layers only; off removes the
  index and tool and keeps files.
- [ ] 4.2 `memory` entry in the feature registry.
- [ ] 4.3 `/memory` shows the directory and lists entries.
- [ ] 4.4 Configuration, command, and render tests.

## 5. Documentation and verification

- [ ] 5.1 Document memory files, the tool, `/memory`, and `memory.enabled`.
- [ ] 5.2 Live check: save a memory in one session, start a new session in the
  same project and see it used; a different project does not see it.
- [ ] 5.3 `cargo fmt --all --check`, workspace Clippy with `-D warnings`
  (stable and 1.88), `cargo test --workspace --locked --no-fail-fast`.
