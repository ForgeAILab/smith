---
description: List the Smith agent profiles configured on this machine
allowed-tools: Bash
---

!`"${CLAUDE_PLUGIN_ROOT}/scripts/smith-profiles"`

Present the table above to the user as-is. Name the default, and say which
profiles are read-only (`plan` / `review` posture) — those never register `edit`
or `shell`, which is a harder guarantee than `--approval deny` declining a call.

This reads local configuration only. It does not prove a profile's credentials
are still valid; a profile can be configured and out of quota. Do not claim a
profile works unless a dispatch actually succeeded on it.
