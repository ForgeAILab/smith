---
created_at: 2026-08-07T00:00:00Z
updated_at: 2026-08-07T20:19:28Z
completed_at: 2026-08-07T00:00:00Z
---

## 1. Collapse and retire the anchored todo pane

- [x] 1.1 Partition the public plan projection into open items (pending, in
  progress, cancelled) and completed items, keeping authored order within the
  open group.
- [x] 1.2 Render at most one collapsed completed row below the open items:
  the most recently completed item's text, struck through and dim, with
  `(+N done)` only when more than one item is complete.
- [x] 1.3 Retire the pane when every item is completed and the activity is
  neither working nor interrupting; keep it while the turn still runs.
- [x] 1.4 Charge the collapsed row against `MAX_VISIBLE_TODOS` in
  `desired_todo_rows` so the anchored budget is unchanged.
- [x] 1.5 Leave the sensitive-plan path untouched — counts only, no item text,
  no collapse row.

## 2. Reviewed redundant-row suppression

- [x] 2.1 Add an explicit suppression predicate in the transcript renderer
  keyed on tool name plus reviewed display projection, applied only to
  `ToolStatus::Ok`.
- [x] 2.2 Suppress successful `write_todos` and `registry.search` rows,
  including their result previews.
- [x] 2.3 Suppress successful `agent` rows for `spawn`'s follow-on lifecycle
  actions — `wait`, `result`, `resume`, `stop` — and keep `follow_up` and
  `list`.
- [x] 2.4 Keep the suppressed call in canonical history, the journal, and
  machine output; assert no row-adjacent blank line survives a suppression.
- [x] 2.5 Apply the same predicate to the resumed-history path and to a child's
  inspected transcript, so live and replay stay identical.

## 3. The agent row

- [x] 3.1 Add a reviewed `agent` projector to `smith-tools/src/display.rs`
  covering action, child id, tool scope, workspace posture, and a bounded task
  excerpt; register it in `has_tool_call_display_schema`.
- [x] 3.2 Correlate `ChildSpawned` to the originating spawn call id and enrich
  that row in place with the child id, workspace, and turn ceiling, omitting
  the ceiling for an unbounded child.
- [x] 3.3 Name the child's agent profile on the spawn row, labelled inherited
  when the spawn selected none.
- [x] 3.4 Drop the `sub-agent · <child> started` transcript notice; keep the
  per-child log entry and the panel row.
- [x] 3.5 Keep every terminal child line — completed, needs input, interrupted,
  stopped, failed — exactly where it lands.

## 4. Child profile selection

- [x] 4.1 Add an optional `profile` argument to the `agent` tool's `spawn`
  action, with the registered child-enabled profile names enumerated in the
  schema and named in the description.
- [x] 4.2 Give `AgentTool` a directory of child-enabled profiles (name,
  revision, provider, model) built from the same `request.child_profiles` that
  produced the preflighted routes.
- [x] 4.3 Resolve a named profile to `ChildModelSelection::Explicit` through
  `profile_route_key`, exactly as `start_agent` does for `/agent <preset>`.
- [x] 4.4 Fail an unknown, non-child-enabled, or unrouted profile with a tool
  error naming the available profiles, creating no child and no lifecycle
  event.
- [x] 4.5 Leave an absent `profile` on `ChildModelSelection::Inherit` so every
  existing call behaves identically.

## 5. Child write access from profile posture

- [x] 5.1 Derive `SmithChildRoute.read_only` from the route's own agent-profile
  posture on both the default route (`factory.rs:1530`) and every profile route
  (`factory.rs:1660`), replacing the hardcoded `true`.
- [x] 5.2 Enforce the three-key rule in `child_builder`: write tools require a
  non-read-only posture, the spawn's `tools: "all"` scope, and a workspace
  policy that is not the read-only view, together. The workspace key is load
  bearing because `ReadOnlyView` resolves to the same shared workspace handle
  `SharedProject` does and is what an unnamed workspace defaults to.
- [x] 5.3 Confirm a writing child uses the parent's approval policy and
  workspace unchanged, and add no permission the root does not hold.
- [x] 5.4 Update the `agent` tool description so the model knows a build
  profile still needs `tools: "all"` to write.

## 6. Delegated-work panel detail

- [x] 6.1 Store the reviewed display projection for a child's current call in
  its panel detail instead of the bare tool name.
- [x] 6.2 Carry the child's profile on `ChildSummary` and render it on the row.
- [x] 6.3 Extend the existing poll-on-redraw to refresh coordinator turn and
  token counts for every visible child, not only the inspected one.
- [x] 6.4 Keep the row to one clipped line with the clock docked right, and
  fall back to the tool name with an honest unavailable label when no reviewed
  projection exists.

## 7. Delegated usage accounting

- [x] 7.1 Accumulate per-counter delegated usage from `RuntimeEvent::Usage` on
  the child streams in `App::apply_child`, keyed so each contributing child is
  counted once.
- [x] 7.2 Extend `SessionUsage` with the delegated totals and contributor count
  without blending them into the root counters.
- [x] 7.3 Render the merged total line plus indented `root` and `agents`
  sub-lines; omit the breakdown entirely when nothing was delegated.
- [x] 7.4 Carry the same shape into `/status` and the session usage log record.
- [x] 7.5 Count only what this process observed — a recovered dormant child
  contributes nothing and is not a contributor.

## 8. Catalog price reference

- [x] 8.1 Add an optional `CatalogModelCost` (input, output, cache read, cache
  write; USD per million tokens) to `CatalogModel`, each field individually
  optional.
- [x] 8.2 Normalize Models.dev `cost` in `model_catalog.rs`, dropping
  individually invalid values without rejecting or disabling the entry.
- [x] 8.3 Bump `CATALOG_SCHEMA_REVISION` to 2 and confirm a revision-1 cache
  falls back to the seed and refreshes.
- [x] 8.4 Teach `scripts/generate-model-catalog.py` the same normalization and
  regenerate `crates/smith-runtime/data/models-dev-seed.json`.

## 9. Session cost

- [x] 9.1 Compute session cost from the active model's catalog price and the
  accumulated counters, root and delegated priced by the same reference.
- [x] 9.2 Label the figure exact when every contributing counter is
  provider-reported and priced, estimated otherwise.
- [x] 9.3 Print no cost line at all when the catalog does not price the model.
- [x] 9.4 Keep cost out of routing, approval, context, and budget paths, and
  out of anything the model can read.

## 10. Docs and truth specs

- [x] 10.1 Update `DESIGN.md` for the anchored pane, row suppression, child
  profile selection, the panel row, and the exit report shape.
- [x] 10.2 Land the `client-interaction`, `tool-call-display`, `child-agents`,
  `usage-accounting`, and `configuration` deltas.

## 11. Verification

- [x] 11.1 `cargo test -p smith-tui` covering: collapse ordering, the
  single-item no-count case, cancelled excluded from the collapse,
  retire-on-terminal, retained anchored budget, suppression of successful rows,
  a failed suppressed call still rendering, live/replay suppression parity,
  spawn-row enrichment, panel projection and clock docking, and the delegated
  usage breakdown.
- [x] 11.2 `cargo test -p smith-tools` for the `agent` projector, including
  control normalization and the unbounded-turn case.
- [x] 11.3 `cargo test -p smith-runtime` for profile-selected spawns, the
  refusal path for an unavailable profile, inherit-by-default parity, and the
  posture/scope matrix for child write access.
- [x] 11.4 `cargo test -p smith-config -p smith-runtime` for price
  normalization, partial/ill-typed prices, and the schema-revision bump.
- [x] 11.5 `cargo test --workspace`, `cargo clippy --workspace --all-targets`,
  and `cargo fmt --check`.
