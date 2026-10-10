---
created_at: 2026-10-10T02:49:20Z
updated_at: 2026-10-10T23:04:33Z
---

## Why

Adding anything to Smith today means writing Rust and shipping a release,
which makes it hard for other people to extend it or share what they built.
Skills, MCP servers, and command providers already load without compiling,
but slash commands are a compiled-in `&'static` registry, and a slash command
is the cheapest thing a user would want to author and pass around.

This change is the first block of the extension path: a markdown file becomes
a slash command. It needs no protocol, no subprocess, and no new authority,
and it gives the later blocks (shell hooks, then the subprocess protocol in
`extension-system`) real content to carry.

## What Changes

- Discover file commands from `commands/<name>.md` under the user state root
  and under the project's `.smith/`. The file stem names the command. The
  layout is fixed, like skills.
- A file command is a prompt template. Optional frontmatter carries
  `description` and `argument-hint`; the body is the prompt. `$ARGUMENTS` in
  the body is replaced by the text after the command name; with no
  placeholder, non-empty arguments are appended.
- Invoking `/name args` submits the expanded text as an ordinary user prompt
  under the same rules as typing it. The composer and queue preview show what
  the user typed; the committed transcript row shows the expanded prompt, the
  same split pasted text already has.
- File commands appear in slash completion, `Ctrl+P`, and `/help` beside the
  built-ins, labelled with their layer.
- Built-in names are reserved. A file command that collides with one is
  refused and reported, never shadowing it. A trusted project command shadows
  a user command of the same name.
- Project commands are gated by hash-bound project trust under a new
  `slash_command` kind, the same way project skills are. An untrusted or changed
  project command is listed but does not run.
- Add `/commands` (list by layer, discovery problems, shadowing),
  `/commands trust <name>`, and `/commands reload`.
- `smith -p "/name args"` expands a discovered file command. Headless mode
  intercepts no slash input today, so this is its only slash handling, and the
  existing `//` escape is honoured there too. An untrusted project command
  fails closed in non-interactive runs.
- Record file commands as content-only contributions in `extension-system`:
  they grant no capability.

## Out of Scope

- Shell hooks and the subprocess extension protocol. They are the next two
  blocks and get their own proposals.
- Shell execution, file embedding, or tool calls inside a template (`!cmd`,
  `@file` expansion at template level). A template is text only.
- Per-command `model`, `allowed-tools`, or profile frontmatter.
- Positional placeholders (`$1`, `$2`) and subdirectory namespaces
  (`commands/git/commit.md` as `/git:commit`).
- Invoking skills as slash commands. Skills stay model-activated.
- A package format or installer for sharing sets of commands and skills.

## Impact

- Affected specs: `client-surfaces`, `configuration`, `extension-system`.
- Affected code: `crates/smith-config/src/trust.rs` (new kind),
  `crates/smith-client/src/commands.rs` plus a new discovery module,
  `crates/smith-tui/src/app/input.rs` and `render/modal.rs` (menu and
  dispatch), `crates/smith-cli` (headless expansion, `/commands` executor).
- Overlap with `add-feature-toggles`: both add requirements to the same three
  specs and both record contributions through the existing `ModuleSpec` shape
  (`HarnessSpec::with_module`, as MCP servers do today). Neither defines that
  shape, the added requirements do not share names, and either can land first.
