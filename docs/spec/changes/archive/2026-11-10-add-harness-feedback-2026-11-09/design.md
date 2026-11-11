## Context

Smith pins a fixed set of tools in every turn: `CORE_TOOL_NAMES` plus
`view_image` when the vision module registers it (`pinned_core` in
`crates/smith-runtime/src/capability_limits.rs`). Every other registered tool,
including `write_todos`, the goal tools and compiled-in module tools such as
`web_fetch`, is in the activatable index, and the agent must call
`registry_activate` before using one.

The CLI bridge (`BridgedSet::new` in `cli_bridge.rs`) serves every tool in
the session's authorized view except the core and harness tools in
`excluded_tool_names`, registry-support tools without authorized skills, and
names that are too long or collide. A module tool therefore reaches `cli/*`
turns without changes.

`write_todos` and the goal tools declare `ToolEffects::default()`, which is
empty, and run without approval. The feedback tool needs the same: it writes
only Smith-owned state outside the workspace.

Module crates depend on `smith-module`, not `smith-runtime`. `ModuleContext`
provides `user_dir`, `is_child_session`, `posture` and `session_history`. It
does not provide profile, provider or model identity, or a way to redact
secrets. Registered-secret redaction lives in
`crates/smith-runtime/src/journal/redaction.rs` (literal rules).

## Goals / Non-Goals

- Goals: the agent can always report without searching for the tool; a report
  never prompts or interrupts; reports carry trustworthy evidence; the owner
  can review, file or dismiss them; a repair agent can read the queue.
- Non-Goals: crash capture, telemetry, automatic upload, Smith-initiated
  reports, headless envelope changes.

## Decisions

- **Pinned like `view_image`.** An agent that just hit friction will not
  search the registry for a feedback tool. `pinned_core` adds
  `harness_feedback` beside `view_image`, pinned only when it is registered.
  This reuses the existing mechanism, so the module contract does not change.
  Alternative considered: a prompt line pointing at an activatable tool. It is
  cheaper, but it relies on the agent activating a tool at the worst moment.
- **CLI agents get it as-is.** The tool is not added to the bridge
  exclusions. CLI-backed turns are where bridge friction is most likely.
  Smith adds no prompt; the CLI's own permission settings still apply to
  bridged calls.
- **Empty effects.** The tool declares `ToolEffects::default()` and writes
  only through the module's store under `user_dir`. Its arguments cannot name
  a path.
- **Compiled-in module.** This follows the feature-blocks direction: one
  module the user can switch off with `[modules.harness-feedback] enabled`.
- **Host-supplied facts.** `ModuleContext` gains three host handles:
  - session identity: Smith version and commit, profile, provider and model;
  - a redactor backed by the journal's registered-secret literal rules;
  - a recent-failures source.
  The tool reads ids from `InvocationContext` and the rest from these
  handles, never from its arguments.
- **Failure recorder.** `session_history` visits canonical history, which
  may not hold the in-flight turn yet. Smith therefore wraps tool invocation
  with a recorder, as the change and read recorders in `smith-tools` do. It
  keeps a ring of the last 5 failed calls per session: tool name, call id,
  turn id, and error text truncated to 500 chars. Bridged calls pass through
  the same wrapper. The CLI's own tools do not, so they are absent from the
  evidence.
- **Store format.** Each report is one `~/.smith/feedback/<id>.md` file
  (`0600`, directory `0700`). YAML frontmatter holds id, created and updated
  times, status (`queued`, `filed` or `dismissed`), `viewed`, kind, area,
  title, count and the evidence fields. The body holds the labeled sections.
  Ids look like `hf-<yyyymmdd>-<6 base32 chars>`. Writes are atomic (temp
  file + rename).
- **Concurrency.** Parallel sessions and children share the store. Creating a
  report, raising a count, marking reports viewed and pruning hold an
  exclusive `fs2` lock on `~/.smith/feedback/.lock`, as `model_catalog.rs`
  does for its refresh. The kernel releases the lock if a process dies.
- **Bounds.** Title ≤ 120 chars. Each text field ≤ 2,000 chars, truncated with
  a marker. File ≤ 16 KiB. At most 500 report files. At most 5 new reports per
  session. When the store is full, filed and dismissed reports older than 30
  days are pruned first. If it is still full, the tool returns a message and
  writes nothing. Queued reports are never deleted automatically.
- **Duplicate handling.** Within one session, a report whose kind, area and
  normalized title match an earlier one raises that report's `count` and
  appends the new evidence (bounded). Across sessions, the store keeps
  separate files; `/feedback` groups equal titles in the list.
- **Startup notice.** The notice counts queued reports with `viewed: false`.
  Listing with `/feedback` or `smith feedback list` sets `viewed: true` on the
  reports it showed.
- **GitHub filing.** The URL is
  `https://github.com/ForgeAILab/smith/issues/new` with `title`, `body` and
  `labels=harness-feedback` set. The body is the labeled sections plus Smith
  version, OS, provider/model and area. When the encoded URL would exceed
  8,000 bytes, the body is cut at a section boundary and ends with
  `(truncated; run smith feedback show <id> for the full report)`. Smith marks
  the report `filed` after the browser hand-off succeeds; it cannot know
  whether the user pressed Submit.

## Risks / Trade-offs

- A pinned tool adds a small schema to every request. Adding it changes the
  cached prefix once; after that it is as stable as the other pinned tools.
- Models may over-report. The per-session limit, the duplicate handling and
  the "when not to report" guidance bound the noise. The owner can switch the
  module off.
- Report text can contain private prompt or code excerpts. It stays local,
  and the only path off the machine is a GitHub page the user reviews and
  submits.

## Migration Plan

None. This is additive. The store directory is created on the first report.

## Open Questions

- Should the startup notice appear in headless plain output too, or only in
  the TUI? The proposal keeps headless output unchanged.
