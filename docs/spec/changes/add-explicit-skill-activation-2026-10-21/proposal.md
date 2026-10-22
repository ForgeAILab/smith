---
created_at: 2026-10-21T20:59:02Z
updated_at: 2026-10-21T21:47:56Z
---

## Why

Smith discovers skills and lists their trust status in `/skills`, but the user
cannot directly activate one through slash completion. They have to describe
the skill in ordinary text and depend on the agent's capability discovery.

## What Changes

- Add detected skill names to the shared slash menu and command palette as
  `/<skill-name>`, with the authored description and source. The user chose
  direct skill commands instead of a separate `/skill` picker.
- `/<skill-name>` activates that skill locally for the current session and
  returns the composer ready for the next prompt. It sends no model request.
- `/<skill-name> <task>` activates the skill before submitting the task through
  the ordinary input, attachment, and history pipeline.
- Keep built-ins and existing file commands ahead of skill aliases. Extend
  `/skills` with `/skills use <name> [task]` so a colliding skill remains usable.
- Resolve the existing winning skill declaration, then enforce its trust,
  pinned bytes, capability scope, and activation budget. Report a named refusal
  when activation cannot complete, and acknowledge activation only after it
  succeeds.
- Reuse the current terminal completion and picker components, keyboard
  behavior, and transcript notices.
- Initial delivery targets Smith's provider-backed turns. Installed CLI
  backends retain their existing skill MCP bridge; direct host activation is
  shown as unavailable there until delivery into that backend's context can be
  guaranteed. It must never claim success while the CLI has no instructions.

## Impact

- Affected specs: `client-surfaces`, `harness-policy`.
- Affected code: `smith-client` command parsing/completion and skill metadata;
  `smith-tui` command routing, resources, input preparation, and notices;
  `smith-cli` skill context, resource assembly, and safe session rebind;
  `smith-runtime` explicit skill activation through its one factory.
- Documentation: `docs/skills.md`, slash-command documentation, and README.
- Compatibility: a skill cannot replace an existing built-in or file command;
  `/skills` and `/skills trust <name>` retain their current behavior.
- The active `add-smith-guides-and-capability-hints-2026-10-19` change adds more
  built-in skills. This feature consumes their resolved descriptors and does
  not alter their bodies or discovery.
