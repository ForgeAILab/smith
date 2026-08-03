---
description: List resumable Smith sessions for this project
argument-hint: ""
context: fork
allowed-tools: Bash(smith:*)
---

Run `smith sessions list` and present the result as a short table.

Each row is tab-separated: session id, last updated, turn count, model, and the
opening prompt. Show the id in full — it is the argument `/smith:resume` needs.

If the list is empty, say so plainly. Do not start a session to populate it.
