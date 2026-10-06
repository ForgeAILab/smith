## Context

Activation today is monotonic and budgeted: the runtime pre-activates what a
deterministic matcher scores above zero for the prompt, and the model keeps
`registry.search` as the fallback. The matcher compares query terms with a
card's name, tags, and keywords by exact word. Smith supplies those keywords
per tool by hand (`smith-runtime/src/abilities.rs`). Skills derive theirs
from their description, so reference skills match far more prompts than
tools do.

## Goals / Non-Goals

- Goals: a model never concludes a core tool is missing; the model can
  enumerate and choose capabilities itself; restriction is declared, visible,
  and enforced by the authorized view.
- Non-Goals: semantic retrieval, a second approval path, loading every
  skill up front.

## Decisions

- **Pinned core is a host-declared set, enforced by the runtime.** Smith
  passes the core ids to the runtime's initial activation. Pinned entries are
  bound before retrieval and counted against the capability budget first. If
  they do not fit, session start fails with the budget named; nothing is
  silently dropped.
  - Alternative considered: keep widening keywords. Rejected: every miss is
    silent and looks like a missing tool.
  - Alternative considered: activate everything. Rejected: reference skills
    are thousands of tokens each and activation is monotonic.
- **Posture and limits narrow the pinned set; they never fail it.** A
  read-only posture pins `read`, `list`, and `search` only. A profile that
  denies `tool:shell` simply has no shell.
- **Limits apply at the view, not at ranking.** `capabilities.allow` and
  `capabilities.deny` become part of the `ViewFilter`, so a denied id cannot
  be listed, searched, activated by name, or pre-activated. `deny` wins over
  `allow`. With no `allow`, everything not denied is allowed.
- **Browse and activate are separate from search.** Listing returns cards
  without activating anything, so browsing costs no schema tokens.
  `registry.activate` is the only call that binds by name, and it accepts
  only ids present in the authorized view.
- **Session narrowing is one-way.** `/capabilities deny` adds to the session's
  deny set and applies at the next safe boundary by rebuilding the view. An
  already-active capability that becomes denied leaves the next epoch.
  Activation is otherwise still monotonic.

## Risks / Trade-offs

- A larger first request for every session, including ones that only chat.
  Accepted: the core schemas are small and the failure they prevent is total.
- `shell` is advertised to every build-posture session from the first turn.
  Approval policy is unchanged, so a call still prompts as before.
- Description matching raises recall and with it false activations of
  skills. Mitigated by the existing instruction budget and the tool-first
  tie-break.
- The runtime pin moves, which has shifted event timing before. Every CI leg
  is read, not only macOS.

## Migration Plan

1. Runtime: pinned initial activation, browse and activate, description
   scoring, view limits; conformance tests; push the compat branch.
2. Smith: bump the pin, declare the pinned core, add profile limits and
   `/capabilities`, remove the v0.3.10 shell keywords.
3. Existing configurations keep working: with no `capabilities` keys, a
   session has the pinned core and may discover everything it could before.

## Open Questions

- Should the built-in `plan` and `review` postures pin `shell` for read-only
  commands, or stay at `read`, `list`, `search`? Proposed: stay; the agent
  can ask to activate it and approval still applies.
