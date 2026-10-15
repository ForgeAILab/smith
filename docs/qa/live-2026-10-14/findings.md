# Live checks before v0.4.0 (2026-10-14)

Release build of `ac69d5c` against the owner's real `config.toml` and
credentials, copied into a temporary `HOME` without `trust.json`: 0.3.10
cannot read the shared trust file once a newer build has recorded a
`slash_command` decision, which is the failure this release fixes.

## Capability walkthrough (update-capability-discovery 4.3)

`zai/glm-5.3`, `--approval ask`, 110x36. Capture:
`capabilities-activate-deny-110x36.txt`.

- `/capabilities` on a fresh session lists `active (7)`: `edit`, `list`,
  `read`, `search`, `shell`, `task_output`, `task_stop`, then
  `available to activate (42)`.
- `/capabilities deny tool:shell` reports `denied for this session:
  tool:shell`; the next listing shows `active (6)` without `shell` and
  `denied (1)` with `tool:shell · denied by this session`.
- Asked to activate `skill:smith.configuration.image-generation` by name,
  the model called `registry.activate`, got it `staged` for the next
  request, and summarized the skill correctly. Asked whether it had a shell,
  it said no.

No findings.

## Cache A/B against 0.3.10 (update-capability-discovery 4.4)

`docs/qa/grammar-2026-10-03/cache_ab.py`, three turns per model, old =
`smith-0.3.10-release-backup`, new = 0.4.0. Raw rows: `cache-ab.json`.
Cached input tokens per turn:

| Model | 0.3.10 | 0.4.0 |
|---|---|---|
| `zai/glm-5.3` | 6208 / 2816 / 4032 | 15360 / 2944 / 4096 |
| `google/gemini-3.8-flash` | 0 / 0 / 0 | 0 / 0 / 0 |
| `xai/grok-4.3` | 256 / 128 / 3840 | 3200 / 3712 / 3968 |

0.4.0 reads at least as much from cache on every turn. Gemini reported no
cached reads on either build, as in earlier runs.

The script exits 1 because 0.4.0's first `zai` turn ended
`approval_required`. The session log shows why: the model called `shell`,
which is now always active, the headless run refused it for lack of an
approver, and the model fell back to `read` and answered. A rerun of the
same turn ended `ok`. This follows from core tools being always active,
and the release notes say so.

## CLI agent capability bridge (add-cli-agent-capability-bridge 8.1, 8.5)

Debug build of main `1106510`, the owner's real `HOME` and config, Claude
Code 2.1.300 (`~/.local/bin/claude`), Codex 0.162.1. Run after v0.4.0, so
none of this is in a release yet.

Automated (`live_cli_bridge`, opt-in): `claude_live_flow` and
`codex_live_flow` with `LIVE_CHILD_MODEL=zai/glm-5.3` pass, including the
child step (spawn, bounded wait, `PONG` in the committed history). Both
drift canaries pass. The child step had never run and failed three ways
before passing; all three were test bugs, fixed in the same commit as this
note:

- the disposable child profile name was 50 characters; the limit is 32;
- turn two ran with `--approval deny`, which refuses every spawn; it now
  uses `allow-all` only when the child step is on (the CLI's own tools
  follow `allow_own_tools`, not `--approval`);
- `zai/glm-5.3` was written as the profile's bare `model` under provider
  `zai`, which Smith reads as `zai/zai/glm-5.3`.

`LIVE_ADVISOR=1` needs an advisor in the owner's config, which has none,
and the owner's config was not edited. Interactive (tmux, `--approval ask`,
120x40): a scratch project whose `.smith/config.toml` sets
`advisor = "zai/glm-5.3"` and declares a stdio MCP server with one counting
tool. On `cli/claude-code/haiku`, then `cli/codex/gpt-6-astra`:

- The bridge offered relay, `advisor`, `agent`, `registry_search`,
  `registry_activate`, and `artifact_read`.
- The untrusted server stayed undialed (`/mcp`: `untrusted · needs
  approval`) until `/mcp trust docs`, which showed the command, arguments,
  and digest before connecting.
- `mcp__docs__count` was found with `registry_search` and raised Smith's
  approval with the tool's arguments and authority warnings; the counter
  log stayed empty until `y`, then recorded exactly one call per approval.
- `agent` spawn raised the child approval; the `code` child answered
  `PONG` and the wait reported `timed_out: false`.
- `advisor` answered from the in-flight external turn's work ("correct so
  far"), and the exit summary attributed its usage to `advisor zai/glm-5.3`.
- `/quit` then `--resume` on the Claude Code session: the bridge was
  offered again, the next call raised a new approval (count 2), the CLI
  recalled the earlier label, and the child was still listed as durable.

Findings (not fixed):

1. The exit summary prices a CLI model under the profile's provider:
   `price unknown for zai/cli/claude-code/haiku` (and
   `zai/cli/codex/gpt-6-astra`). Cosmetic.
2. A project MCP server that is declared but untrusted gives the CLI turn
   no hint. The model searched the registry, found nothing, and told the
   user to check the server; nothing pointed at `/mcp trust`. The welcome
   screen does not mention untrusted servers either.
3. The profile-name error reads "a agent profile name must contain ...".
