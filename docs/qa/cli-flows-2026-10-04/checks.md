# Headless fold and picker builders — checks

`refactor-cli-flows`, branch `refactor/cli-flows`, 2026-10-04.

- Gate with the macOS runner's temp path: fmt, strict Clippy, 2,033
  workspace tests, `cargo deny --locked check all`. Every headless and
  local-command fixture unchanged.
- `HeadlessFold::exit` consults the goal state only when a finish exists and
  the immediate exit did not fire, and pending child work only after every
  earlier guard holds — the same cases and order as the former loop.
- The fold's six tests run without a host and fail when the logic they
  cover breaks: counting synthetic usage failed
  `synthetic_usage_is_excluded_while_real_usage_merges`; never marking a
  child completion pending failed
  `a_child_completion_after_root_finish_waits_for_delivery`. Both edits
  were reverted.
- `runtime_resources` is 47 lines; the longest picker builder
  (`model_entries`) is 94.
- `../grammar-2026-10-03/final_checks.py`: 67/67.
