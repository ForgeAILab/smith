---
description: Continue a previous Smith session with its history intact
argument-hint: "<SESSION_ID> [--write] [what to do next]"
context: fork
allowed-tools: Bash(smith:*)
---

Route this to the `smith:smith-delegate` subagent as a resumed dispatch.

Raw user request:
$ARGUMENTS

The first argument is the session id. Pass it as `--resume <SESSION_ID>` and
strip it from the task text.

Resuming is preferred over restating context in a fresh prompt: Smith still
holds the prior conversation, so a follow-up costs a fraction of what
re-describing the problem would, and it cannot drift from what actually
happened.

If no session id was given, run `smith sessions list` and ask which to continue
rather than guessing at the most recent one.
