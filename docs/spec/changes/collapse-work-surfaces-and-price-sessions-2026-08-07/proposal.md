---
created_at: 2026-08-07T00:00:00Z
updated_at: 2026-08-07T20:19:28Z
---

## Why

Three surfaces report work that is already reported somewhere better, and one
number the user is actually spending is reported nowhere at all.

**The todo pane never shrinks.** `draw_todos` renders every item in authored
order for the whole turn, so a five-step plan on its last step still spends
five rows saying four things the user finished minutes ago. When the plan is
finished, `Replay-equivalent anchored todo pane` keeps the reconciled list
pinned above the composer "until the next turn starts" — so a completed plan
is the most persistent thing on screen and the least useful.

**Delegation prints itself twice.** A spawn writes an `agent(action, task ·
arguments hidden) · ok` row from the tool fold at
`crates/smith-tui/src/transcript.rs:322`, and then a `sub-agent · child-1
started · read-only` notice from the reducer at
`crates/smith-tui/src/app/reducer.rs:589`. Two rows, one fact. Neither one says
what the agent was actually told to do: `crates/smith-tools/src/display.rs` has
no projector for `agent`, so the row falls back to a value-free key list and
the task text — the only thing that distinguishes child-1 from child-4 — never
reaches the screen. `write_todos` has the same shape of problem in reverse: the
anchored pane already renders its effect, so its row and its JSON result
preview are pure duplication.

**Every delegated agent is the same agent.** Smith preflights a child route per
child-enabled agent profile and `/agent <preset>` uses them, but the
model-facing tool cannot: `AgentTool::invoke` pins every spawn to
`ChildModelSelection::Inherit`, so a model that wants a planner, an explorer,
and a builder gets three copies of whatever the root happens to be running. The
routes are built and sitting unused. And even reaching them would not be
enough, because `SmithChildRoute.read_only` is a hardcoded `true` on both the
default and every profile route — a build-profile child would arrive with build
posture, build instructions, and no tool that can write.

**The panel says almost nothing.** A child's row detail is
`format!("running {name}")` — `running Read`, for every read, all session. The
reviewed projection that would make it `Read(src/retry.rs)` arrives one line
later in the same function and is thrown away, and the turns and tokens the
coordinator already tracks are fetched only for the one child under inspection.

**Delegated tokens are invisible and nothing is priced.** `apply_child`
(`crates/smith-tui/src/app/state.rs:1146`) deliberately folds a child's stream
against the child's transcript and never touches session status, and the
runtime's child monitor does not mirror child `Usage` onto the parent stream.
So four agents can burn more tokens than the root conversation and the exit
report — `12 turn(s) · input 143k · output 2.7k` — counts none of it. That
report is also the last thing a user sees before quitting, and it is denominated
in a unit nobody budgets in. `usage-accounting` already requires labelled cost
calculation from a versioned price reference; Smith has never had the reference.
Models.dev publishes per-model `cost`, and Smith already fetches and normalizes
that exact document through `scripts/generate-model-catalog.py` and
`smith_config::catalog` — it drops the `cost` block on the floor during
normalization.

## What Changes

### Anchored todo pane

- Render open items (pending, in progress, cancelled) first in authored order,
  then at most one collapsed completed row at the bottom: the most recently
  completed item's text, struck through and dim, with a `(+N done)` count when
  more than one item is complete.
- Retire the pane entirely once every item is completed and the turn is no
  longer running, instead of pinning a finished list until the next turn. A
  fully-completed plan stays visible while the turn is still working, so the
  user sees it land.
- Keep the existing anchored row budget: the collapsed row counts against
  `MAX_VISIBLE_TODOS`, so the pane can never grow past what it costs today.

### Transcript rows

- Suppress the transcript row for a tool call whose effect a better surface
  already reports, when and only when that call succeeded: `write_todos` (the
  anchored pane), `registry.search` (capability bootstrap the user did not ask
  for), and the `agent` actions that delegation's own lifecycle line reports —
  `wait`, `result`, `resume`, `stop`.
- Keep the `agent spawn` row: it is the one row that announces a spawn, and it
  is what the dropped `sub-agent · <child> started` notice is replaced by.
- Keep `agent follow_up` and `agent list` rows, because nothing else reports
  them.
- Never suppress a failed, denied, or unreported call. A hidden row is a
  redundancy claim, and a failure is not redundant with anything.
- Drop the now-duplicated `sub-agent · <child> started` notice, and let the
  reviewed spawn row be the one place a spawn is announced.

### Agent rows

- Add a reviewed `agent` projector to `smith-tools`, so a spawn row reads
  `Agent(spawn · "explore the autoloads and data layer…" · read-only · shared)`
  rather than `agent(action, task · arguments hidden)`, and a
  `follow_up`/`stop`/`result` row names its child.
- Correlate `ChildSpawned` back to the spawn row by tool-call id so the row
  also carries the child id, its workspace posture, and its turn ceiling — the
  facts the dropped notice used to carry.
- Name the child's agent profile on the row, labelled inherited when the spawn
  selected none.

### Child profile selection

The routes already exist. `SmithChildFactory.profile_routes` holds a fully
preflighted provider/model/prompt route per child-enabled agent profile, keyed
by `profile_route_key(name, revision)`, and `route_for` resolves it from
`ChildModelSelection::Explicit`. `/agent <preset>` has spawned through that
path since presets landed. Only the model-facing tool cannot reach it:
`AgentTool::invoke` hardcodes `ChildModelSelection::Inherit`.

- Add an optional `profile` argument to `agent spawn`, validated against the
  registered child-enabled profiles and resolved through the same
  `profile_route_key` lookup `/agent <preset>` uses.
- Enumerate the available profile names in the tool schema so the model can
  choose `plan`, `explore`, or `build` rather than guess a name.
- Reject an unknown or non-child-enabled profile with a tool error naming the
  available ones. An absent `profile` keeps today's inherit behavior exactly.

### Child write access follows profile posture

`SmithChildRoute.read_only` is hardcoded `true` for both the default and every
profile route, so a build-profile child gets build posture, instructions, and
model but no tool that can write. That deferral ends here.

- Derive `read_only` from the route's own agent-profile posture instead of the
  constant `true`.
- Require all three keys before a child can write: the profile's posture must
  not be read-only, the spawn must ask for `tools: "all"`, and the spawn's
  workspace policy must not be the read-only view. Any one of them failing
  leaves the child read-only.
- The workspace key is not redundant with the other two. `child_builder`
  resolves `WorkspacePolicy::ReadOnlyView` to the same shared workspace handle
  `SharedProject` resolves to, so the tool set is the only thing enforcing that
  policy — and the read-only view is what the `agent` tool defaults an unnamed
  workspace to. Without this key, a build-posture spawn asking for
  `tools: "all"` and naming no workspace would silently receive write-capable
  tools against the shared project, contradicting the retained requirement that
  a read-only-workspace child receives read tools it cannot mutate files with.
- Change no approval behavior. A writing child goes through the same approval
  surface as the root, because it is built from the same approval policy.

### Delegated-work panel detail

`apply_child` sets a child's panel detail to `format!("running {name}")` — the
bare tool name — even though the reviewed display projection for that exact
call arrives one line later through `set_child_tool_display`, and the
coordinator poll already carries turns and tokens for the inspected child.

- Show the reviewed projection rather than the bare name, so a row reads
  `Read(src/retry.rs)` instead of `Read`.
- Carry the child's profile and its turn and token counts on the row, refreshed
  from the coordinator on the existing poll-on-redraw for every visible child
  rather than only the inspected one.
- Keep the row one clipped line with the clock docked right, as `panel_row`
  already lays it out; the detail clips first so the clock cannot be pushed off
  screen.

### Usage and cost

- Accumulate per-counter delegated usage from the child event stream the TUI
  already subscribes to, kept separate from root counters rather than blended
  into them, along with the number of children that reported any.
- Report a merged total line, then indented `root` and `agents` sub-lines, at
  exit and in `/status`.
- Normalize Models.dev `cost` into a new optional `CatalogModelCost` on
  `CatalogModel` (input, output, cache read, cache write, USD per million
  tokens), bump `CATALOG_SCHEMA_REVISION` to 2, and regenerate the embedded
  seed.
- Price a session from that reference and the counters it prices, labelled
  `exact` when every contributing counter is provider-reported and priced, and
  `est.` otherwise. Print nothing when the catalog does not price the model, per
  the existing requirement that an unpriced model reports unknown rather than
  assuming a price.

## Impact

- Affected specs: `client-interaction`, `tool-call-display`, `child-agents`,
  `usage-accounting`, `configuration`
- Affected code: `crates/smith-tui` (`render/composer.rs`, `render/layout.rs`,
  `render/transcript.rs`, `transcript.rs`, `app/reducer.rs`, `app/state.rs`,
  `status.rs`, `usage_log.rs`); `crates/smith-tools/src/display.rs`;
  `crates/smith-runtime` (`delegation.rs`, `factory.rs`, `model_catalog.rs`);
  `crates/smith-config/src/catalog.rs`;
  `crates/smith-runtime/data/models-dev-seed.json`;
  `crates/smith-cli` (`runtime_host.rs`, `tui_driver.rs`, `submission.rs`,
  `local_command.rs`); `scripts/generate-model-catalog.py`; `DESIGN.md`
- Compatibility: no shared-runtime, journal, or event-schema change — the same
  events are folded, and only which surface renders them moves. The `agent`
  tool gains one optional argument, so every existing call keeps its meaning.
  The catalog schema revision bump invalidates cached snapshots at revision 1;
  they fall back to the embedded seed and refresh, which is the existing
  behavior for a stale revision.
- Security: honoring profile posture means a build-profile child spawned with
  `tools: "all"` over a shared or explicit-directory workspace can write where
  today it could not. A spawn that names no workspace still cannot: the
  read-only view it defaults to is the third key. It writes through the
  parent's own approval policy and workspace, adds no permission the root does
  not already hold, and cannot exceed the depth-one delegation invariant.
- Pricing is presentation only. It never gates a request, never enters an
  approval decision, and never reaches the model.
- Out of scope: forking a conversation so a branch reuses the parent's cached
  prefix. That needs a shared-runtime primitive Smith does not have and is
  proposed separately as `fork-conversation-branches`.

## Approval Boundary

Approval authorizes collapsing completed todos and retiring a finished plan,
suppressing successful tool rows whose effect another surface reports, adding a
reviewed `agent` display projector, letting the model select a registered child
profile per spawn, deriving a child's write access from its profile posture
plus an explicit `tools: "all"` plus a workspace policy that is not the
read-only view, enriching the delegated-work panel rows,
accumulating delegated usage separately from root usage, and adding a
catalog-sourced price reference with a labelled session cost.

It does not authorize suppressing any failed, denied, or unreported tool row;
dropping any terminal child event; hiding sensitive plan item text that is
already withheld; letting a child reach a profile that is not registered and
preflighted for child use; granting a child any permission the root does not
hold, bypassing the parent's approval policy, or writing outside the spawn's
declared workspace; relaxing the depth-one delegation invariant; blending child
counters into the root counters so the two cannot be told apart; presenting a
computed price as anything but a labelled estimate; assuming a price for a
model the catalog does not price; or letting cost influence routing, approval,
or context decisions.
