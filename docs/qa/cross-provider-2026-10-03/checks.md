# Cross-provider history — checks

`update-cross-provider-history` with Agent Runtime
`fix/smith-cross-provider-reasoning` at `b1d1974`, release build, the owner's
real configuration, 2026-10-03.

## Method

`xmatrix2.sh <binary> <out> math|tool`: for every ordered pair of Gemini 3.8
Flash, Anthropic Messages (`dddai`, claude-fable-5-1), Z.AI glm-5.3, and xAI
grok-4.3, one headless turn on the first, then `--resume` on the second.
`math` asks a reasoning question with no tools; `tool` makes turn 1 read
`notes.txt` with the read tool. `xpair.sh` reruns one pair.

## Before (0.3.2)

`math`: Gemini → Z.AI failed locally ("OpenAI Chat Completions cannot
represent one or more message content parts"); Gemini → xAI and Anthropic →
Gemini were rejected by the provider; xAI → Gemini failed with "Corrupted
thought signature". The request to Anthropic after a Gemini turn carried
Gemini's signature as `redacted_thinking`.

## After

`math`: all 12 pairs answered 392.

`tool`: every pair whose first turn completed answered 42. Gemini continuing
after another provider's tool calls first failed locally ("Gemini signed
continuation is incomplete or out of order"); the runtime now requires
Gemini's signed thought only before tool calls in the active continuation.
That rule was probed live before it was implemented: Z.AI, xAI, and
Anthropic tool turns then continued on Gemini.

The Anthropic provider (`dddai`) was unreliable throughout: 424 "service
unavailable" responses, first turns failing, and occasional replies to the
system prompt instead of the conversation ("I am Smith, a terminal-first
coding agent"). Its own session (Anthropic → Anthropic) shows the same
behaviour, so those results are not attributed to Smith.

## Cache, 0.3.2 against this build

`../grammar-2026-10-03/cache_ab.py` ([results](cache-ab.json)), then Z.AI
twice more to separate variance:

| Run | Build | Turn 1 uncached | Turn 2 cached / uncached | Turn 3 cached / uncached |
|---|---|---|---|---|
| A/B | 0.3.2 | 1,271 | 1,728 / 1,197 | 2,880 / 108 |
| A/B | this | 2,360 | 1,728 / 2,352 | 4,032 / 106 |
| repeat 1 | 0.3.2 | 1,360 | 1,792 / 1,137 | 2,880 / 121 |
| repeat 1 | this | 1,220 | 1,792 / 1,127 | 2,880 / 101 |
| repeat 2 | 0.3.2 | 2,366 | 1,792 / 1,922 | 3,712 / 67 |
| repeat 2 | this | 1,268 | 1,792 / 1,126 | 2,880 / 90 |

Turn 2's uncached input follows how many times turn 1 read the file, on
either build. Gemini and xAI matched turn for turn in the A/B run. No miss or
re-billed tokens on any turn. Same-producer requests are also covered by
byte-for-byte tests in the runtime adapters.
