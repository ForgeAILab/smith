# Stable ready queue

This crate turns a declared task list into deterministic parallel execution
batches. It is intentionally small, but the scheduling behavior is used as a
compatibility boundary by callers.

The current implementation has regressions around validation and ordering.
See the public API documentation and tests for the required behavior.

## Smith workflow evaluation

`scripts/eval-agent-workflow.sh` copies this fixture to a private temporary
directory, commits only the disposable copy, reproduces the failing baseline,
and records Smith/model/reviewer/fixture provenance beside the project. With
`--live`, the task requires todos, bounded inspection, exact editing, fmt/test/
Clippy gates, and a same-model read-only child review. Public behavior and
adversarial validation/cycle tests are rerun outside the agent result, and
generated session or child metadata is rejected if it appears in the fixture
checkout.
