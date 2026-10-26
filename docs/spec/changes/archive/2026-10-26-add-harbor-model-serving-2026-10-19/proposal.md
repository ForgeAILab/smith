---
created_at: 2026-10-19T04:20:00Z
updated_at: 2026-10-26T07:05:49Z
---

## Why

The evaluation bridge pins one serving identity — ChatGPT OAuth with
`gpt-5.6-luna` at `max` effort — into `constants.py`, the generated trial
configuration, the installed agent, the canary gate, and the credential audit.
Every other question the harness exists to answer (which model completes
Harbor Index tasks with fewer tokens and less wall-clock time) requires a
second serving, and today there is no supported way to run one. The immediate
need is a baseline comparison of `gpt-5.6-luna` against `glm-5.3` served by
the z.ai OpenAI-compatible coding API, which the operator already uses daily
through the `zai` provider.

The z.ai path is an API-key provider, not a product OAuth entry, so the
existing OAuth isolation machinery (selected `authfile:chatgpt` entry,
refresh-token rotation handoff) does not apply and must not be silently
reused: the key must reach the sandbox through a channel that keeps it out of
repository files, Harbor configuration, command strings, logs, trajectories,
and artifacts, and completed jobs must be auditable for it.

## What Changes

- Add an explicit, versioned **serving** table to the bridge. A serving pins
  provider name and kind, base URL, credential reference scheme, model catalog
  limits, reasoning effort policy, allowed egress hosts, and the canary prompt.
- Ship two servings:
  - `luna-max` (existing behavior, becomes the default):
    `chatgpt` / `gpt-5.6-luna` / `max`, `chatgpt-responses`,
    `credential = "authfile:chatgpt"`, egress `auth.openai.com` +
    `chatgpt.com`.
  - `glm-5.3`: `zai` / `glm-5.3`, `openai-compatible`,
    `base_url = "https://api.z.ai/api/coding/paas/v4"`,
    `credential = "env:ZAI_API_KEY"`, `response.reasoning_only =
    "text"`, no effort flag, egress `api.z.ai`.
- Keep the frozen profiles (`smoke`, `dev`, `full`) serving-independent; a run
  selects exactly one serving through a `--serving` CLI flag recorded in job
  provenance and run invariants.
- Pin identical per-turn **input** budgets across servings (255,616 max input)
  so the baseline compares model and provider behavior, not different context
  budgets. Output ceilings are each serving's native model limit (16,384 for
  `gpt-5.6-luna`, 131,072 for `glm-5.3`), and each serving's context is
  exactly its input budget plus its output reserve. The first GLM smoke
  attempt pinned 16,384 for both servings and GLM's text-mode reasoning —
  which consumes the output budget — truncated mid-attempt on two of three
  tasks; the operator chose native ceilings for the baseline on that evidence.
- For API-key servings, resolve the key at launch — from
  `ZAI_API_KEY`, falling back to the operator's
  `~/.smith/config.toml` `[providers.zai]` block — into process memory only,
  and deliver it to the sandboxed Smith process through the environment-exec
  channel. No OAuth auth-file upload or refresh handoff occurs for these
  servings.
- Extend the post-job credential audit to scan for the resolved API-key value
  on API-key servings, exactly as it scans for the OAuth entry today.
- **Finding from the first GLM dev job:** environment delivery is visible to
  the sandboxed process (for example a core dump captured the agent
  environment into a collected task artifact; the audit caught it and the
  artifact was removed). Follow-up hardening: deliver the key through an
  owner-only file reference outside the task workspace, mirroring the OAuth
  auth-file pattern, instead of the environment-exec channel.
- Parameterize the live canary gate by serving: identity (provider/model,
  effort when the serving pins one), terminal `ok` status, provider-reported
  usage, and credential-leak-free output.
- Cross-serving analysis is explicitly descriptive: the analyzer's paired
  bootstrap requires matching serving invariants, and jobs whose serving
  differs must be compared with `--descriptive-only`.
- Cost provenance stays per serving and is never mixed: OAuth subscription
  cost remains `unknown`; the API-key serving reports only what Smith's
  provider-reported usage accounting produces.

## Non-Goals

- No change to Smith runtime/provider code, the model catalog tooling, or the
  frozen Harbor Index dataset, task sets, or profile definitions.
- No multi-serving job (one serving per job), no cross-serving paired
  significance claims, and no provider price injection for the OAuth serving.
- No concurrency change: API-key servings still run serially because the
  frozen profiles pin concurrency one and the analyzer requires matching
  rollout policy.
- Not widening the GLM serving's **input** budget beyond the shared 255,616
  pin even though GLM's native context is larger; only the output ceiling is
  native.

## Rollout Plan

1. Implement the serving table, config generation, job plumbing, canary, and
   audit changes with unit tests.
2. Rebuild and verify the musl Smith artifacts from a clean tree.
3. Recreate the Apple-Silicon Colima `harbor` profile with Virtualization
   framework Rosetta (the VM no longer exists) and verify AMD64 execution.
4. Pass the live canary for both servings.
5. Run `smoke` on `glm-5.3` as the integration gate, then `dev` (26 tasks × 1
   rollout) on both servings as the baseline.
6. Produce the descriptive baseline comparison (reward, tokens, latency) and
   audit both jobs for credential material.
