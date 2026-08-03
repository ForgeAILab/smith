---
description: Hand a coding task to a local Smith agent and return its answer verbatim
argument-hint: "[--write] [--resume <SESSION_ID>] [--profile <NAME>] [--model <MODEL>] [what Smith should do]"
context: fork
allowed-tools: Bash(smith:*)
---

Route this request to the `smith:smith-delegate` subagent.
The final user-visible response must be Smith's output verbatim.

Raw user request:
$ARGUMENTS

Routing:

- `--write` selects `--approval allow-all` for this dispatch. Without it, the
  run is read-only (`--approval deny`) and Smith cannot edit files or run
  commands. Do not add `--write` on your own initiative.
- `--resume <SESSION_ID>` continues an existing Smith session.
- `--profile`, `--model`, and `--project` pass through to `smith` unchanged.
- Every one of those is a routing control. Strip them from the task text.

Do not investigate the repository before dispatching. Smith reads the workspace
itself, and a read here is both duplicated cost and a chance to hand it a
picture that is already stale.
