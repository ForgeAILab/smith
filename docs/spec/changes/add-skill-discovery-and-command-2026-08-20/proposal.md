---
created_at: 2026-08-20T20:28:10Z
updated_at: 2026-08-20T20:28:10Z
---

## Why

Smith's skill machinery is complete except at its two ends. `smith-runtime`
resolves a four-layer catalog — built-in, user, trusted workspace, session —
with name shadowing, digest-pinned activation, and a bounded index that is
built without opening a body. `harness-policy` already requires all of it, and
`docs/skills.md` already documents it.

Nothing fills the middle three layers, and nothing shows the result.

- `SmithSkillSources::with_user` and `with_workspace` are called only from
  tests. No code reads `~/.smith/skills/` or `<project>/.smith/skills/`, so in
  every real session the catalog is exactly the four compiled-in harness
  references. A user cannot add a skill; a project cannot ship one.
- `Smith::skill_index()` exposes every entry — name, description, layer,
  activatable, trust class — and no surface consumes it. `grep -i skill` over
  `crates/smith-tui/src` returns nothing. A user cannot see which skills exist,
  which layer won a name, or that a workspace skill is sitting untrusted.

The second gap makes the first one worse. Progressive trusted disclosure only
protects a user who can find out that something was withheld; a workspace skill
that silently does not activate is indistinguishable from one that does not
exist.

## What Changes

- **Discover skills on disk.** Read `<root>/skills/<name>/SKILL.md` under the
  user state root and under the project's `.smith/`, parse bounded frontmatter
  for a `description`, and declare each into its existing source layer. The
  directory name is the skill's identity; a frontmatter `name` may restate it
  but never contradict it.
- **Pin every discovered body to the bytes that were read.** Discovery digests
  `SKILL.md` and declares the skill through `Skill::from_verified_file`, so a
  body rewritten between indexing and activation fails closed instead of
  activating unreviewed text. This closes the discovery-time TOCTOU window for
  user skills as well as workspace ones.
- **Bind workspace skills to hash-bound project trust.** Add
  `ExecutableKind::Skill` and resolve each workspace declaration's
  `TrustStatus` before it is offered to the resolver. A project skill is
  repository content asking to write privileged instructions into the model's
  context, which is authority even though nothing is spawned.
- **Fail closed per skill, never per session.** A malformed, oversized,
  misnamed, or unreadable `SKILL.md` is not registered and is reported as a
  named discovery problem. A broken skill file must not stop Smith opening in
  that project.
- **Add `/skills`.** List every indexed entry grouped by layer with its
  description, whether it can activate, which layer shadowed it, and every
  discovery problem. Add `/skills trust NAME`, which shows the project-relative
  path and content digest, records the decision, and recomposes the session at
  the next idle boundary through the existing `CapabilitiesChanged` path so the
  newly trusted skill is usable without restarting Smith.

## Impact

- Affected specs: `harness-policy`, `configuration`, `client-surfaces`
- Affected code: `smith-config` (the `Skill` executable kind), `smith-runtime`
  (`skills::discovery`, factory wiring), `smith-cli` (the skill context, the
  `/skills` handler, the recompose flag), `smith-tui` (the command registry)
- Public compatibility: additive. A user with no `skills/` directory and a
  project with no `.smith/skills/` see exactly today's four built-in entries,
  and `/skills` says so.
- Security: this is the first path by which repository-controlled content can
  place instructions into privileged context. The digest pin, the trust gate,
  and the symlink-escape refusal inherited from `Executable::from_file` are the
  load-bearing parts. Activation still contributes instructions only — it
  grants no tool, permission, approval, credential, or wider workspace.
- Performance: discovery reads at most a bounded number of small files at host
  start. Descriptor resolution still opens no body.

## Non-Goals

- No skill installation, registry, marketplace, or update mechanism. Users
  bring their own directories.
- No supporting-file (`SkillFile`) discovery. `SKILL.md` only; a skill's
  auxiliary assets remain a later change.
- No frontmatter schema beyond `name` and `description`. Fields other tools
  write are ignored rather than interpreted, so a skill authored elsewhere
  loads without importing that tool's policy.
- No editing, creating, or disabling skills from the TUI. `/skills` inspects
  and grants trust; a skill is authored in a file.
- No per-skill enable/disable configuration keys.

## Delivery Slices

1. Trust kind: `ExecutableKind::Skill` and its stable string.
2. Discovery: the on-disk layout, frontmatter parsing, bounds, digest pinning,
   and per-skill diagnostics, with no host wired to it.
3. Composition: the interactive and headless hosts declare discovered user and
   workspace skills, with workspace trust resolved against the project.
4. Surface: `/skills`, `/skills trust NAME`, and idle-boundary recomposition.

Slice 1 must pass before slice 3 declares a workspace skill.

## Approval Boundary

Approval authorizes Stage 2 implementation of the four slices above. It does
not authorize skill installation, supporting-file loading, frontmatter fields
beyond `name` and `description`, or any change to what activation is permitted
to contribute.
