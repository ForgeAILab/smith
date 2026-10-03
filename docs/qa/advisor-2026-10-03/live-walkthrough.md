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
