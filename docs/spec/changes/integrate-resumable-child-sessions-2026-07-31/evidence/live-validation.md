# Live validation: durable child continuity and exact resume

Date: 2026-07-31 (America/Toronto)

This evidence used the real Smith binary, the configured Z.AI Coding Plan
endpoint, and a disposable Git repository under `/tmp`. No provider or
checkpoint secret is recorded here.

## Resolved production composition

`smith config explain` resolved:

- profile: `zai-glm-5-2`;
- provider: `zai`;
- model: `glm-5.2`;
- provider credential: the owner-only user configuration's inline `api_key`
  (the explanation surface rendered the value as `[redacted]`); and
- protected checkpoint key: the dedicated `SMITH_CHECKPOINT_KEY` environment
  source, rendered as `[redacted]`.

The test supplied a disposable sessions directory through
`SMITH_PERSISTENCE_SESSIONS_DIR`. Because the checkpoint key resolved from the
environment and the provider key resolved from inline owner-only
configuration, Smith did not initialize or query Keychain.

The coordinated Agent Runtime implementation was committed as
`e562ca21481403fe731d9de6ed88ffa16e1435a5` before Smith's cross-repository CI
gate was run.

Commands below use `<checkpoint-key>` in place of the 64-hex environment
value:

```sh
SMITH_CHECKPOINT_KEY=<checkpoint-key> \
SMITH_PERSISTENCE_SESSIONS_DIR=<disposable-sessions> \
target/debug/smith -p '<task>' \
  --project <disposable-project> \
  --profile zai-glm-5-2 \
  --approval allow-all \
  --background-exit wait \
  --output-format json --no-color --no-motion
```

## Real coding task and restart follow-up

The disposable project contained an intentional duration parser defect: the
hours branch multiplied by 60 instead of 3600. The first Smith process was
required to spawn one durable child, follow up that same child, implement the
fix, and run the focused tests.

Observed terminal result:

- parent session: `session-88c14e25-acb2-418c-86cc-c3d760e9ac4e`;
- child: `child-1`;
- child session: `child-session-2c6cb496-19df-41d0-9d07-5d2b434b00c5`;
- durable child turns after spawn plus in-process follow-up: 2;
- cumulative child tokens: 9,533; and
- test result: 3 tests passed.

A fresh Smith process then resumed the same parent and was instructed to use
`agent follow_up` on `child-1`, never `spawn`. The child recalled the exact
prior bug and fix, inspected the current file, and retained the same child
session ID. Its cumulative accounting advanced monotonically to 3 turns and
18,446 tokens. The parent remained
`session-88c14e25-acb2-418c-86cc-c3d760e9ac4e`; no replacement child or spawn
event appeared.

The final disposable project status contained only the intended `calc.py`
change, and `python3 -m unittest -v` reported all three tests `ok`.

## Abrupt-process exact resume

A second disposable session used one durable all-tools child and a controlled
probe. The probe created a marker only after Smith had entered the shell tool,
then blocked. On observing that marker, the test sent `SIGKILL` to Smith and
the probe process. There was no orderly shutdown or coordinator flush.

Stable identities were:

- parent session: `session-e6be3f56-aeaa-4233-b40b-351caa7e042a`;
- child: `child-1`; and
- child session: `child-session-98510700-b8ea-4b90-b067-679354e80f6a`.

On restart, the parent turn's own exact checkpoint first recovered its pending
`agent wait`. The model then listed `child-1` as `interrupted` and
`resumable: true`, invoked:

```json
{"action":"resume","child_id":"child-1"}
```

and received:

```json
{"mode":"exact_checkpoint","resumed":"child-1"}
```

The runtime did not replay the indeterminate pre-crash shell invocation. It
committed the bounded canonical error result for that uncommitted slot, after
which the child model deliberately issued a new shell call. The resumed child
then completed under the same IDs and reported all three focused tests `ok`.
The parent transcript contains no second spawn.

The schema-v9 journal recorded these attributed boundaries:

| Sequence | Event | Result |
| ---: | --- | --- |
| 513 | `child_spawned` | `child-1`, max turns 2 |
| 563-564 | catalog recovery then exact-checkpoint reconciliation | same child session, interrupted, resumable became true |
| 791 | `resume_started` | same child session |
| 834 | `child_completed` | 3 tests passed |

This live run exposed that the protected child checkpoint can be newer than
the parent catalog after `SIGKILL`. The final implementation therefore performs
an asynchronous, provider-free checkpoint reconciliation before Smith accepts
delegation commands. The final event refinement emits one authoritative
`Recovered { state: Interrupted, resumable: true }` transition; the
conformance fixture asserts that exact cardinality. The two live lines above
were captured immediately before that observability-only de-duplication.

After completion, a second `resume child-1` was rejected because the child was
already idle and non-resumable. This proves the exact resume is single-use and
does not create a replacement implicitly.
