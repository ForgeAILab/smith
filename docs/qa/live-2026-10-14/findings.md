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
