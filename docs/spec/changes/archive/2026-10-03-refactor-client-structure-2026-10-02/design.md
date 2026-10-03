## Context

Six crates, an acyclic graph, one composition path. The runtime side is in
good order. The client side grew feature by feature, and three habits now
account for most of its cost:

1. A fact has several owners (prices, limits, endpoints, state labels).
2. Data crosses a crate boundary as prose and is parsed back.
3. Each extensible thing has several hand-synced lists.

Current flow for `/status`:

```text
smith-tui  parse "/status" -> CommandAction::Status
           -> reverse name map -> Action::Command
smith-cli  handle_local_command -> format!() ~200 lines -> String
smith-tui  push_local(title, String)
           render: match title == "status" -> split "label: value" -> restyle
```

Target flow:

```text
smith-client   CommandSpec table -> HostCommand::Status
smith-cli      status::report(&host) -> StatusReport
smith-tui      Block::Local(LocalResult::Status(report)) -> draw
headless       smith_client::plain::render(&report) -> text
```

## Goals / Non-Goals

- Goals:
  - Headless compiles and runs without importing anything from `smith-tui`.
  - Adding a slash command is one table entry and one handler.
  - Adding or changing a provider is an edit in `smith-config` only.
  - No user-visible change.
- Non-Goals:
  - Restructuring `App`, the reducer, overlays, or key handling.
  - Improving `/help`, `/status`, or `/diagnostics` wording or layout.
  - Finishing or removing the unused half of client protocol v1.
  - Changing the `agent-runtime` pin or the summary baseline.

## Decisions

- Decision: a new crate rather than a module in `smith-runtime`.
  `smith-runtime` is 47k lines and already exports 40 modules; reports and
  command definitions are client concerns, not composition. The new crate
  depends on `smith-runtime` (client facade), `smith-config`, and
  `smith-host`; `smith-tui` and `smith-cli` depend on it.
  - Alternatives considered: a `headless` feature flag on `smith-tui`
    (keeps the wrong owner); moving everything into `smith-cli` (the TUI
    cannot depend on the binary crate).
- Decision: reports are plain data with a `plain` renderer beside them.
  The terminal renderer is in `smith-tui`. Both render the same value, so
  the two surfaces cannot disagree on a field.
  - Alternatives considered: keep strings and add a structured side
    channel (two sources of truth).
- Decision: migrate reports one command at a time behind
  `LocalResult::Text { title, body }`, and delete that variant when the last
  command is converted. Each step is shippable.
- Decision: the command type encodes the route.
  `Command::Ui(UiCommand) | Host(HostCommand) | Session(SessionControl) |
  Confirm(ConfirmCommand)`. The TUI handles `Ui`; the host matches
  `HostCommand` exhaustively. A misrouted command no longer compiles.
- Decision: `smith-tui` does not gain a `smith-config` dependency. The CLI
  converts descriptors into `SetupEntry { id, label, detail, flow }` values
  and passes them to `SetupApp`. `flow` is an enum (`QuickKey`,
  `CustomEndpoint`, `OAuth`, `AddModel`, `ChangeDefault`), so an entry
  without a flow cannot be constructed and the `match id.as_str()` with a
  `_ => {}` arm is removed.
- Decision: glob imports of the parent module are disallowed in `smith-cli`
  production code and enforced by Clippy's `wildcard_imports` for that
  crate. Tests may keep them.

## Risks / Trade-offs

- A sixth workspace crate adds build and release surface. Mitigation: it
  contains only moved code and plain data; no new third-party dependency.
- Byte-identical output is a strict bar while moving 1,000 lines of
  formatting. Mitigation: fixtures recorded first; one command per commit.
- `render/transcript.rs` and `app/state.rs` are being edited elsewhere.
  Mitigation: stages 1, 2, 4, and 5 do not touch them; stage 3 waits.
- The report types become a second place to update when a field is added.
  That is the intended single place; the string path it replaces had two.

## Migration Plan

1. Record fixtures for every local command and for headless text, JSON, and
   stream JSON.
2. Create the crate; move accounting; update imports.
3. Introduce the command table and routed command type.
4. Convert local results command by command.
5. Move provider descriptors to a single source and pass setup entries as
   data.
6. Make CLI imports explicit and split `handle_local_command`.
7. Remove `LocalResult::Text`, the reverse name map, and the mirrored price
   table.

Each numbered step is independently releasable. Rollback is a revert of the
step; no persisted format changes.

## Open Questions

- Resolved (owner, 2026-10-02): the crate is `smith-client`. The module
  `smith_runtime::client` keeps its name; `smith-client` is the layer above
  it.
- Should `/diagnostics` keep one free-text report in this change and be
  given real structure in the wording change that follows?
- Should child-state labels be unified here for the CLI surfaces only, with
  the reducer adopting the enum in `refactor-tui-state`? Proposed: yes.
