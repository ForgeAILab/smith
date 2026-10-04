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

## Release cache comparison, 0.3.2 against 0.3.3

`../grammar-2026-10-03/cache_ab.py` ([results](cache-ab.json)). Gemini matched
turn for turn; Z.AI's 0.3.3 run read the file more in turn 1, as in earlier
runs. xAI's 0.3.3 turn 2 was served 192 cached tokens in the A/B run and in
one of two repeats (2,432 in the other), while 0.3.2 got 2,432–2,688 in all
three; turn 3 was normal on both builds every time. xAI serving 192 cached
tokens on an otherwise identical prefix was already recorded on both builds
in the 0.3.0 run.

To rule out the reasoning-provenance change, a throwaway 0.3.3 build dumped
the xAI request bodies: turn 2's request carried both encrypted reasoning
items from turn 1 and extended turn 1's last request as a prefix (system,
user, reasoning, function call, function output, then reasoning, assistant,
user). Same-provider reasoning is kept, and that run read 2,432 cached
tokens. Snapshots from the A/B runs show turn 1 reasoning stamped
`{provider: xai, model: grok-4.3}`, matching the manifest. No miss or
re-billed tokens on any turn.
