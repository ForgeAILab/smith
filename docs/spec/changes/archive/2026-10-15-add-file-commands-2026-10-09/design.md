## Context

Three things extend Smith without compiling today: skills
(`skills/<name>/SKILL.md`, model-activated), MCP servers (tools), and command
providers (models). Slash commands are a static registry in
`crates/smith-client/src/commands.rs`: `COMMANDS: &[CommandSpec]`, with
`matches`, `parse`, and `has_exact_name` all returning or testing
`&'static CommandSpec`.

The `extension-system` spec already names commands as a contribution point of
the subprocess protocol. That protocol is unbuilt and was deferred on
2026-10-05 because it had no consumers. File commands are the content-only
form of the same contribution and need none of it.

## Goals / Non-Goals

- Goals:
  - A user writes one markdown file and gets a working slash command.
  - A project can ship commands, and running one is a reviewed decision.
  - Reuse skill discovery's shape: fixed layout, name from the path, bounded,
    fail closed per file, problems reported.
- Non-Goals:
  - Any execution inside a template.
  - Hooks, the extension protocol, or a package format.

## Decisions

- Decision: the file stem is the command name; frontmatter cannot rename it.
  Names are 1..=64 lowercase ASCII letters, digits, and `-`.
  - Why: same reason as skills. The name must be readable from a directory
    listing, and a project file must not be able to aim at a name the reader
    cannot see. Lowercase-only avoids case-insensitive filesystem collisions.
  - Alternative considered: `name` in frontmatter. Rejected for the above.

- Decision: built-in command names are reserved; a colliding file is refused
  and reported.
  - Why: `/quit`, `/model`, and `/skills trust` must mean the same thing in
    every project. Shadowing a built-in is the "trusted tool replacement"
    tier of `extension-system` and stays out of a content-only feature.
  - Consequence: a future built-in can collide with an existing user file.
    The file is then reported by `/commands`, not silently dropped.

- Decision: project commands need hash-bound trust under a new
  `ExecutableKind::SlashCommand` (`"slash_command"`).
  - Why: the user types `/deploy`, not the text the repository put behind
    it. That text is submitted as the user's own prompt, which is the same
    "text Smith would adopt" reasoning that gates project skills.
  - Alternative considered: no gate, since invocation is explicit. Rejected:
    explicit invocation proves the user chose the name, not that they read
    the body. A cloned repository could otherwise put instructions behind a
    harmless-looking name.
  - Alternative considered: reuse the `skill` kind. Rejected: a decision
    should say what was approved, and the persisted file already
    distinguishes kinds.
  - Named `SlashCommand`, not `Command`, because "command provider" and
    "command-jsonl" already mean something else in this codebase.

- Decision: `$ARGUMENTS` only. With no placeholder and non-empty arguments,
  the arguments are appended after a blank line. Substitution is single-pass;
  placeholder text inside the arguments is not expanded again.
  - Why: covers nearly every real template, and append-by-default means a
    template author who forgot the placeholder still gets a working command.

- Decision: expansion happens in the client and produces an ordinary prompt
  submission. The runtime sees a user message and nothing else.
  - Why: no runtime change, no new event type, and the pinned
    `agent-runtime` compat worktree is untouched. Queueing while a turn runs,
    `@` references, and paste expansion all behave as for typed text.
  - Shown versus sent text reuses `PreparedSubmission`: `display_text` is the
    typed `/name args` (composer, queue preview, input history), and
    `committed_text` and `expanded_text` are the expanded prompt. The committed
    transcript row therefore shows the expanded prompt, live and after resume,
    exactly as a pasted chunk does. Nothing new is persisted.
  - Alternative considered: keep `/name args` in the committed row. Rejected:
    it needs a display field persisted beside the user message so resume can
    rebuild it, which is new session state for a cosmetic gain.

- Decision: the catalog is built at host start and by `/commands reload`.
  Each invocation re-reads that one file. A user command runs with its
  current bytes; a project command runs only if the bytes still match the
  trusted digest, otherwise it is reported as changed. The description and
  argument hint shown in completion come from the catalog, so an edit to
  those shows after `/commands reload`.
  - Why: editing a template and running it again should work without a
    restart, while a project file rewritten mid-session must not run
    unreviewed.
  - Alternative considered: pin all bodies at discovery like skills.
    Rejected for user commands: it makes authoring need a restart per edit.

- Decision: discovery and expansion live in `smith-client`; the trust kind
  lives in `smith-config`.
  - Why: `smith-client` already depends on `smith-config` and owns the
    command registry, and both the TUI and `smith -p` go through it.
  - The static registry stays as is. Lookup becomes two-stage: built-in
    `COMMANDS` first, then the discovered catalog. A menu row type covers
    both so the TUI renders one list.

- Decision: bounds are 256 commands per layer, 64 KiB per file, 64
  frontmatter lines, 256 characters of description.

## Risks / Trade-offs

- A trust prompt on a project's commands adds friction for the common case
  of a team's own repository. Accepted: it is one confirmation per file
  content, and it matches skills and MCP servers.
- `smith -p "/name ..."` changes meaning when a file named `name` exists.
  Bounded by the name grammar: a path such as `/usr/bin/x` can never match,
  and `//name` sends the text literally. Built-in commands stay TUI-only;
  headless runs still pass `/model` through as prompt text.
- Two lookup stages mean `matches`/`parse` signatures change, which touches
  the TUI call sites listed in the proposal.

## Migration Plan

None. No existing file, key, or command changes meaning. Trust files written
before this change load unchanged; project commands start undecided.

## Open Questions

- Should a `description` be required, as it is for skills? Proposed: optional,
  falling back to the first non-empty body line, because a command is picked
  by name and a missing description should not make it vanish.
- Should `/commands new <name>` scaffold a file? Left out; easy to add later.
