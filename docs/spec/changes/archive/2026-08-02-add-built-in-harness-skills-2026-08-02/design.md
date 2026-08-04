# Design: Built-in harness reference skills

## Context

`SmithSkillSources` already resolves `built-in < user < trusted workspace <
session` with fail-closed naming, duplicate detection, and descriptor-first
indexing, and the factory registers resolved abilities end to end. The
built-in constructor `with_built_in` has zero callers, so the entire change
is content plus one wiring decision.

## Goals / Non-Goals

- Goals:
  - A Smith agent can activate an authoritative harness reference in any
    workspace, offline, with zero filesystem dependence.
  - The reference cannot drift from the shipped documentation of the same
    binary revision.
- Non-Goals:
  - No new configuration surface (no toggle to disable built-ins; shadowing
    by name already provides an override path).
  - No user-facing skill management UX (`/skills` listing, install flows).
  - No workspace or user skill discovery; those layers remain host-supplied.

## Decisions

- Decision: bodies are `include_str!` of the shipped `docs/*.md` files.
  - Alternatives considered: hand-authored skill bodies (drifts from docs);
    reading docs at runtime (docs are absent from user machines); a build
    script generating condensed bodies (extra machinery, same content).
- Decision: names use the `smith.` prefix (`smith.configuration`,
  `smith.headless`, `smith.persistence`, `smith.security`), valid under the
  existing 1..=96 ASCII name rule and clearly harness-owned.
- Decision: seeding happens in the `smith-runtime` factory default so the
  TUI and `-p` share one composition path; an embedder that sets `skills`
  explicitly replaces the whole set, mirroring the `system_prompt` override
  contract.
- Decision: descriptions are authored, task-oriented sentences ("Load before
  editing any .smith/config.toml …") because descriptor keywords drive
  retrieval; doc titles alone are too weak.

## Risks / Trade-offs

- Full docs are a few thousand tokens each at activation; acceptable because
  descriptor-first retrieval defers the cost until an agent opts in, and the
  descriptor advertises the estimated instruction cost.
- Docs written for humans double as model instructions; the docs are already
  normative and terse, and single-sourcing beats a second copy that rots.

## Migration Plan

Purely additive. No configuration, persistence, or protocol changes; no
transition release required.

## Open Questions

- None blocking. A future `docs/skills.md` (authored by task 1.4) may itself
  join the built-in set in a later change.
