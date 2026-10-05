---
created_at: 2026-10-05T20:06:57Z
updated_at: 2026-10-05T20:06:57Z
---

## Why

Every Smith session starts knowing nothing about earlier ones. The user wants
memory like Claude Code's, so Smith knows what happened before: who the user
is, corrections they gave, project decisions that are not in the code, and
where to look for things. The runtime already has a bounded memory lane and
Smith already has a `SmithMemorySource` for it, but nothing stores memory
between sessions.

This replaces `add-file-backed-project-memory-2026-08-02` (deleted with this
proposal; it remains in git history), which was written
before the 0.3.x host and client refactors and planned ranking, automatic
model capture, and a 38-task store. This version follows Claude Code's
simpler model: Markdown files, an index the agent always sees, and the agent
writing memories through a tool.

## What Changes

- Store project memory at `~/.smith/memory/<project-id>/`, using the same
  project identity as sessions: one Markdown file per memory with frontmatter
  (`name`, `description`, `type` of `user`, `feedback`, `project`, or
  `reference`) and a body. Files are owner-only. The user can read and edit
  them by hand.
- Smith generates `MEMORY.md` from the files' frontmatter: a header naming the
  workspace path, then one line per memory (title, file, one-line
  description). Smith rewrites it; hand edits to topic files are picked up.
- Add a file-backed memory source. At session start, resume, and host rebuild
  it snapshots the index into the runtime memory lane as always-on records,
  within the runtime's bounds (4,096 characters per record, 16 records, 16,384
  characters total) and Smith's 8,192-character policy. When the index does
  not fit, it ends with a line saying how many entries were left out. The
  snapshot does not change mid-session, so the start of the prompt stays
  stable and the provider's prompt cache keeps working; memories written
  during a session are in the index from the next session.
- Add one `memory` tool with `list`, `read`, `write`, and `delete`. `write`
  creates or replaces one memory by name; `delete` removes one by name. Names
  are single safe path components; the tool cannot address any other path.
  Content containing a registered secret value is refused.
- Writes go to Smith's own owner-only store, not the workspace, so `write`
  and `delete` do not prompt under the default `build` posture; each call is a
  visible tool row. `plan` and `review` postures, and child agents, get
  `list` and `read` only.
- Add prompt guidance for the main agent: what to save (the user's role and
  preferences, corrections and confirmed approaches with the reason, project
  facts not derivable from the code or history, pointers to external
  resources), what not to save (code structure, git history, task progress,
  secrets), to update or delete rather than duplicate, and to check a recalled
  memory against the current state before acting on it.
- Add `/memory`: shows the memory directory and lists entries.
- Add a `memory` entry to the feature registry from `add-feature-toggles`,
  controlled by `memory.enabled` (default on). Off means no index in context
  and no tool; files are kept. Only the user layer, environment, and command
  line may set it; a project file cannot turn memory on or off.
- Headless runs (`smith -p`) read and write memory the same way.

## Out of Scope

- Automatic memory capture by a hidden model call after each turn.
- Ranking, embeddings, or search beyond `list` with a substring filter.
- Global or team memory, sync, and semantic consolidation.
- Changing project instructions (`AGENTS.md`), which stay required developer
  instruction; memory is optional context and never overrides them.

## Open Question

Should Smith also record a short note of what happened when a session ends,
so the next session can see it without the agent choosing to save it? That
is a hidden model call or a mechanical summary. This proposal leaves it out
unless the owner wants it.

## Impact

- Affected specs: new `project-memory`; `configuration`; `client-surfaces`.
- Affected code: `crates/smith-runtime/src/memory.rs` (file-backed source) and
  host composition; a new memory store module; `crates/smith-tools` or
  `crates/smith-runtime` (`memory` tool); prompt fragments;
  `crates/smith-client/src/commands.rs` (`/memory`); `crates/smith-config`
  (`memory.enabled`); documentation.
- Compatibility: additive. The store starts empty, so nothing enters context
  until the agent writes a memory.
- Security: memory is user state outside the workspace, owner-only, never
  copied into canonical history or audit metadata, refused when it contains a
  registered secret, and never grants tool authority.
- Depends on `add-feature-toggles` for the registry entry; everything else
  stands alone.

## Approval Boundary

Approval covers the store, generated index, session-start snapshot, the
`memory` tool, prompt guidance, `/memory`, `memory.enabled`, and tests and
documentation. It does not cover automatic capture, ranking, global memory,
or an end-of-session note unless the owner adds it.
