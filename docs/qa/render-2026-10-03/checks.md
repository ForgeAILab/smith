# Cached transcript rendering — checks

`refactor-transcript-rendering`, branch `feat/render-incremental`,
2026-10-03.

## Frame time

`crates/smith-tui/tests/transcript_perf.rs` (ignored by default) builds 5,000
mixed blocks (prompts, Markdown answers, tool rows with previews, notices,
errors, local results, shell echoes), then draws 100 frames at 100x32 through
the public `smith_tui::render::draw`, appending a streamed delta before each
frame. It uses only APIs that existed before the change, so the same file ran
on the pre-change commit `74391e6`:

    cargo test --release --locked -p smith-tui --test transcript_perf -- --ignored --nocapture

| Build | Mean frame time |
|---|---|
| `74391e6` (before) | 23.731 ms |
| this change | 0.167 ms |

## Output unchanged

All render tests and local-command fixtures pass unedited. A property test
compares cached and uncached rendering for a long mixed transcript at 44, 80,
and 100 columns, folded and expanded, after appends, tool status and preview
updates, streaming deltas, expand toggles, history replacement, and resizes.
`../grammar-2026-10-03/final_checks.py`: 67/67 at 100x32, 80x24, 44x16, and
no-colour, including the reconfigure surface.
