# Advisor live walkthrough — 2026-10-03

Binary: `target/debug/smith` from `feat/advisor-tool` (on top of
`refactor/client-structure`). Configuration: the owner's real
`~/.smith/config.toml`, default profile `code` (`zai/glm-5.3`), plus a
project-local `.smith/config.toml` in a scratch Git project:

```toml
advisor = "sol"

[profiles.sol]
use = ["main", "child", "advisor"]
```

The owner's configuration was not edited. `smith config explain advisor`
reported `advisor = sol` from the project file.

## Run 1

Prompt: read `calc.py` (an `add` that subtracts), consult the advisor, do not
edit.

- The first activation epoch listed `tool:advisor`; the user did not name a
  tool for it to be offered.
- `glm-5.3` read both files, then called `Advisor()`; `sol` answered and the
  main model used the advice in its reply.
- `/status` showed the advisor separately: `advisor: input 269 · output 137 ·
  reasoning 516`, and `advisor price unknown` (ChatGPT has no catalog price)
  ([capture](live-01-status.txt)).
- Defect: the advisor read the trailing `Tool call: advisor {}` as a pending
  call and advised "wait for the advisor's result".

## Fix and run 2

The current consultation's call is now marked in the transcript, and the
advisor prompt says the final `advisor` call is the request being answered.
Same prompt again: the advisor confirmed the diagnosis and gave the one-line
fix directly ([capture](live-02-turn-after-fix.txt)). Both sessions exited 0.

## Selection by profile or model

After the owner asked for the advisor to be picked like a profile or a
model, `use = ["advisor"]` was removed and `advisor` accepts a profile name
or `provider/model`. Binary: release build of `feat/advisor-tool` at
`2ca17da`, same owner configuration, no edits to it. Each scratch project
carried a one-line project-local `.smith/config.toml`.

### Run 3: `advisor = "sol"`, no `use` change, glm-5.3

`smith config explain advisor` reported `advisor = sol` from the project
file. Two headless turns (`--resume` for the second): `glm-5.3` called
`advisor` on both; the advice came back and was used. The second turn's
stream showed `tool_call_requested advisor` → `tool_call_completed
advisor (ok)`, an advisor usage record (`purpose: advisor`, 630 input,
111 output, 682 reasoning), and main-model cache reads of 2,304 and 2,752
tokens on the resumed turn.

### Run 4: `advisor = "chatgpt/gpt-6.1-sol"`, gemini-3.8-flash

`smith config explain advisor --profile gemini` reported the model
reference. Two headless turns: `advisor` was called on each and returned
advice. In the TUI ([capture](live-03-model-advisor-tui.txt)) the row read
`Advisor() · ok` with the advice as its preview, and `/status` showed
`advisor: input 292 · output 71` beside the root usage, with
`advisor price unknown` in the cost line.

### Cache, 0.2.16 against this build

Three headless turns on a project with a 50 KB file (read in turn 1, two
follow-ups via `--resume`), no advisor, each binary on the same prompts:

| Model | Build | Turn 1 cached | Turn 2 cached | Turn 3 cached |
|---|---|---|---|---|
| zai/glm-5.3 | 0.2.16 | 9,728 | 1,664 | 31,616 |
| zai/glm-5.3 | this | 35,328 | 1,664 | 31,872 |
| google/gemini-3.8-flash | 0.2.16 | 0 | 0 | 0 |
| google/gemini-3.8-flash | this | 0 | 0 | 0 |
| xai/grok-4.3 | 0.2.16 | 2,112 | 3,264 | 192 |
| xai/grok-4.3 | this | 8,000 | 5,632 | 5,824 |

Turn 1 differs because the model chose a different number of reads. Both
builds show the same pattern: Z.AI misses on the first resumed turn and hits
the full prefix on the next; Gemini reports no cache reads on either build
(state `unsupported`); xAI hits most requests and occasionally serves only
192 cached tokens on an otherwise identical prefix, on both builds. No miss
or re-billed tokens were reported.

The xAI runs needed a fresh login first: the stored token had expired on
2026-08-14 and xAI rejected its refresh on both builds.

### Run 5: grok-4.3 with each advisor form

Two headless turns each, `--provider xai --model grok-4.3`. With
`advisor = "sol"` and with `advisor = "chatgpt/gpt-6.1-sol"`, grok called
`advisor` once in turn 1 (advisor usage 332/113/320 and 294/88), used the
advice, and answered correctly in turn 2. Main-model cache reads were 5,632
and 5,248 tokens in turn 1.

### Run 6: Claude Code plugin with gemini-3.8-flash

With the 0.3.0 candidate installed as `~/.local/bin/smith`, the Claude Code
Smith plugin listed all 13 profiles through `scripts/smith-profiles`, and its
`smith-delegate` agent dispatched a read-only task with
`--profile gemini --approval deny`, then resumed the same session. Both
envelopes returned `status: ok` from `gemini-3.8-flash`, with correct answers
(the parser `AdvisorTarget::parse`, and the test
`advisor_target_parsing_splits_on_the_first_slash`).
