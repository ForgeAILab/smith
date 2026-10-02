# Live walkthrough against a real provider — 2026-10-02

Binary: `target/debug/smith` built from the uncommitted main tree (base
`58bc098`). Configuration: the owner's real `~/.smith/config.toml`, default
profile `code` (`zai/glm-5.3`), `--approval ask`. Driver: tmux at 100×32, a
scratch Git project with two files. Three model turns, one shell shortcut,
six local commands. Captures are the `live-*.txt` files in
[captures](captures/).

## Blocker found before the walkthrough

Smith does not start with the owner's configuration. The installed release
(`~/.local/bin/smith`, 0.2.15) fails the same way, so this is not caused by
the uncommitted changes ([capture](captures/live-01-start.txt)):

```
smith: starting the Smith session: provider `chatgpt` cannot select a context
window for model `gpt-6.1-sol`: window `272k` is pinned by flat model limit
`models."chatgpt/gpt-6.1-sol".max_input_tokens`; remove that limit to select a
named window
```

Cause, read in the source:

- `~/.smith/config.toml` gives `chatgpt/gpt-6.1-sol` flat `context_tokens`
  and `max_input_tokens`. The trusted catalog record for that model declares
  `default_context_window: Some("272k")` (`smith-config/src/setup.rs`).
- `resolve_context_window_selection`
  (`smith-runtime/src/factory/context_policy.rs:97-133`) drops the built-in
  windows the flat limit outranks, then still takes the built-in default name
  and reports it as a window the user asked for.
- `smith config explain context_window --profile sol` prints "not set" for
  the same configuration, so the configuration resolver and the session
  factory disagree.
- The selected profile was `code`. `sol` is resolved because it is
  child-enabled (`factory/provider/children.rs:13`), and a child profile that
  fails to resolve ends the root session.

The walkthrough below ran with a project-level file that redeclares the
`272k` window for that model. The owner's configuration was not edited.

## Checks

| Scenario | Result |
| --- | --- |
| Plain turn ("reply with ok") | Streams, completes in 4 s ([capture](captures/live-03-plain-turn.txt)) |
| Model-requested shell call under `ask` | Approval prompt appears; draft typed earlier is kept ([capture](captures/live-04b-approval-idle.txt)) |
| `n` on an idle approval | Denied; the model reports the refusal |
| Typing `yan ` every 120 ms across the arrival of an approval, 25 bursts after it appeared | Approval stayed pending; none of the `y`, `a`, `n` keys answered it ([capture](captures/live-06-typed-through-arrival.txt)) |
| `y` after one second of quiet | Allowed once; command ran; turn completed ([capture](captures/live-07-allowed.txt)) |
| `/model`, Esc, next command | Picker closes, composer empty, next command runs |
| `!ls -la` under `ask` | Runs with no approval prompt ([capture](captures/live-13-bang.txt)) |
| `/status`, `/context`, `/timeline`, `/help`, `/diagnostics` | All render; `/timeline` did not fail (no goal turn in this session) |
| Resize to 44×16 and back | Reflows, no corruption |
| Ctrl+C twice | Exit 0, usage line and resume hint printed |

This closes the "covered by tests only" gap for the approval keystroke guard
in [the results file](fix-interaction-defects-results.md). Keys typed while
the guard holds are discarded, not added to the draft: the draft held the 37
bursts typed before the prompt and none of the 25 typed after.

## Defects seen live

Stability, not covered by any open change:

1. The startup failure above. The `gpt-6.1-sol` catalog record arrived in
   `ed253a8` (v0.2.15), so the shipped release has it for any configuration
   that sets flat limits on that model.
2. A child profile that cannot resolve ends the root session instead of
   being reported as unavailable: `prepare_child_profile_routes` propagates
   the first error (`factory/provider/children.rs:41`).

Observed, not explained: `/status` and the exit line report `6 turn(s)` after
three prompts and one shell shortcut. What the counter counts was not checked.

Fluency. The task that already covers each is named; the rest are new.

| Seen | Covered by |
| --- | --- |
| Every model turn opens with `capabilities · activation epoch N: tool:registry.search, skill:smith.configuration…` | new |
| Working row reads `Working… · 2s · ↑ 2s` | grammar 3.1 |
| `changes · Smith turn 1 · contains ambiguous changes only; use /diff` after a read-only `wc -l` | grammar 2.4 |
| `Worked for 20s` stays at the bottom, below every later local command's output | grammar 3.2 |
| `!ls -la` prints four rows (`shell · $ ls -la`, `shell(command, cwd, timeout_ms · details unavailable) · ok`, the change notice, `/shell`) before the output | grammar 2.3 |
| A denied call prints two rows (`Shell(…) · failed / approval declined` and `approval · shell denied`) | grammar 2.2 |
| Approval: 64-character hash in the title, two lines cut at the edge with no ellipsis, raw JSON body, key hints printed twice | grammar 3.4 |
| `/model`: 464 rows, each cut off inside provenance text (`ctx 272k [smith-trusted-models r5] · input 255616 [smith-trus`); opens on `gpt-5.6-luna`, not the current model | grammar 4.2 |
| `/context` and `/diagnostics`: lines broken mid-word, `?` placeholders, one 5-line `cache maintenance:` sentence | grammar 4.3, 4.4 |
| Local command echoed as bare `/status` with no prompt marker | grammar 2.1 |
| Ctrl+U does nothing; one Esc discards a five-line draft | grammar 5.3; the Esc behaviour is new |
| After `/help` the view stays on "following paused" until the next turn | new |

Not exercised: a goal turn (the `/timeline` failure from the fixture work),
child agents, MCP servers, resume, compaction, any provider other than
`zai`.
