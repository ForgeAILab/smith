---
created_at: 2026-10-04T19:35:32Z
updated_at: 2026-10-04T19:35:32Z
---

## Why

A session that switches model prices every token at the last model's rates
and still calls the figure exact. The 0.3.5 live pass printed
`$0.012 exact · google/gemini-3.8-flash` for a session that ran GLM first:
`SessionCost::compute` multiplies the session's cumulative totals by one
`PriceReference`, `Status::switch_model` keeps the totals while replacing the
price, and the label returns to exact after the new model's first report
(`docs/qa/live-2026-10-04b/findings.md`, "Exit summary with two models").
Delegated children have the same flaw by rule: their usage is priced at the
root's rate whatever model they ran.

## What Changes

- Root usage is kept per provider/model binding. A model switch closes the
  current binding's counters and starts the next; earlier counters are never
  repriced.
- Delegated usage is kept per child binding, resolved from the child's
  profile when it is spawned, and priced at that binding's own catalog rates.
  A child whose binding cannot be resolved is unpriced.
- Session cost is the sum of each binding's counters at its own price. Any
  binding without a price leaves its tokens out of the figure and labels it
  estimated, naming the binding (`price unknown for …`), as the advisor
  already does. With no priced binding at all there is no cost line.
- The exit report and `/status` name every binding that contributed, with its
  share: `$0.034 exact · zai/glm-5.3 $0.022 · google/gemini-3.8-flash
  $0.012`. A single-binding session prints exactly what it prints today.
- The usage log records the per-binding counters (schema version 5); the
  existing top-level provider and model fields keep the last binding so older
  readers still parse it.

## Impact

- Affected specs: usage-accounting.
- Affected code: `smith-client` `status.rs` (`Status`, `SessionUsage`,
  `SessionCost`, `PriceReference::render_sources`), `usage_log.rs`;
  `smith-tui` `app/state/children.rs`; `smith-cli` `tui_driver.rs`
  (price resolution on switch and spawn), `runtime_host.rs`
  (`render_exit_cost_line`, `report_session_usage`), `local_command.rs`
  (`render_status_cost`).
- Headless output is unchanged: it reports counters, not a session price.
- Usage log readers accept versions 1–5.
