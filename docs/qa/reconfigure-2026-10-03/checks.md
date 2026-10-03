# Reconfigure keeps the screen — checks

`refactor-tui-state` and `update-reconfigure-keeps-screen`, branch
`feat/tui-state-and-reconfigure`, debug build, 2026-10-03.

## PTY checks

`../grammar-2026-10-03/final_checks.py` gained a reconfigure surface: a
first turn, `/status`, then `/model` to `local/small-model`. Against 0.3.1
it fails at 100x32 and 44x16: after the switch the transcript area is empty
and Up recalls nothing. Against this build all 67 checks pass; after the
switch the `/status` rows and a `provider · changed` notice are on screen and
Up recalls the earlier prompt ([100x32](sweep-model-switch-keeps-screen-100x32.txt),
[44x16](sweep-model-switch-keeps-screen-44x16.txt)).

## Live walkthrough

The owner's configuration (`zai/glm-5.3`, `--approval ask`, 100x32):

- After a first turn and `/status`, `/model` to `google/gemini-3.8-flash`
  kept both on screen and appended `provider · changed · zai/glm-5.3 →
  google/gemini-3.8-flash` and a reasoning notice, because the two models
  differ in thinking state ([capture](live-model-switch-keeps-screen-100x32.txt)).
- `/effort low` appended one reasoning notice; the footer read
  `think on · effort low`.
- `/connect`, `dddai`, opened guided setup in its own screen
  ([capture](live-connect-flow-100x32.txt)). Esc returned to the same
  transcript and composer ([capture](live-after-connect-cancel-100x32.txt)),
  and `~/.smith/config.toml` was byte-identical before and after.
- A second turn ran on Gemini. The quit report counted both turns, so usage
  carried across the rebuild, and printed after the terminal was restored.
- `--resume` of that session failed to start on 0.3.1 and on this build
  before task 4.1: the saved `effort low` cannot apply to the default
  `zai/glm-5.3`. After 4.1 it starts, clears the override with a notice, and
  names the model change ([capture](live-resume-clears-override-100x32.txt)).

## Known issue, not addressed here

After that resume, the next turn on `zai/glm-5.3` fails with `OpenAI Chat
Completions cannot represent one or more message content parts`, because the
history holds a Gemini turn. 0.3.1 fails the same way headless (a Gemini turn,
then `--resume` with `--provider zai`). It is a provider-history limit in the
runtime adapters, not part of this change.

## Cache, 0.3.1 against this build

`../grammar-2026-10-03/cache_ab.py`, same method as the 0.3.1 run, release
build of `1e1056d` ([results](cache-ab.json)):

| Model | Build | Turn 1 cached | Turn 2 cached | Turn 3 cached | Turn 2 / 3 uncached |
|---|---|---|---|---|---|
| zai/glm-5.3 | 0.3.1 | 11,648 | 1,728 | 4,672 | 2,990 / 104 |
| zai/glm-5.3 | this | 5,824 | 1,792 | 2,880 | 1,140 / 107 |
| google/gemini-3.8-flash | 0.3.1 | 0 | 0 | 0 | 1,992 / 2,073 |
| google/gemini-3.8-flash | this | 0 | 0 | 0 | 1,990 / 2,134 |
| xai/grok-4.3 | 0.3.1 | 384 | 2,432 | 2,688 | 288 / 235 |
| xai/grok-4.3 | this | 2,048 | 2,432 | 2,688 | 309 / 229 |

The 0.3.1 glm-5.3 run read the file more often in turn 1 (5,044 uncached
against 1,264), which carries into its later turns. Grok and Gemini match
turn for turn after turn 1. No miss or re-billed tokens on any turn.
