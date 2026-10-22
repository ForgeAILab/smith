---
created_at: 2026-10-19T04:20:00Z
updated_at: 2026-10-19T04:20:00Z
completed_at:
---

## 1. Serving table

- [x] 1.1 Add a frozen `Serving` descriptor (name, provider, provider kind,
  base URL, credential reference, model catalog limits, effort policy, allowed
  hosts, canary prompt, OAuth flag) with `luna-max` and `glm-5.3` entries and
  a `load_serving` lookup that fails closed on unknown names.
- [x] 1.2 Generate the trial `config.toml` from the serving: provider block
  and credential reference per serving, `response.reasoning_only = "text"`
  only for `glm-5.3`, identical context/input/output limits for both, and no
  credential value anywhere in the document.
- [x] 1.3 Unit tests: both servings render the expected provider, credential
  reference, base URL, reasoning policy, and limits; an unknown serving name
  is rejected.

## 2. Job plumbing

- [x] 2.1 `run_invariants` and `job_config` record the serving name,
  provider, provider kind, model, effort (or `none`), credential policy, and
  serving-specific `agent_extra_allowed_hosts` (`api.z.ai` for `glm-5.3`).
- [x] 2.2 `preflight` requires the ChatGPT OAuth entry only for OAuth
  servings and requires resolvable API-key material for API-key servings,
  without logging it.
- [x] 2.3 `SmithAgent` validates `model_name` against the selected serving,
  skips auth-file upload and refresh handoff for API-key servings, and passes
  the API key through the environment-exec channel only.
- [x] 2.4 The `run` command omits `--effort` for servings without one and the
  terminal-result invariant check matches the serving identity actually
  requested.
- [x] 2.5 Job provenance records the serving name and credential policy.
- [x] 2.6 Key resolution helper: `ZAI_API_KEY` first, then the
  operator's `~/.smith/config.toml` `[providers.zai]` `api_key`, held in
  memory; missing material fails with a message that prints no key.
- [x] 2.7 `audit-job` scans job files for the resolved API-key value on
  API-key servings and for the OAuth entry on OAuth servings.
- [x] 2.8 Unit tests covering 2.1–2.7, including that generated job config
  and provenance never contain key material.

## 3. Canary and CLI

- [x] 3.1 `local_run` executes the isolated canary for either serving with
  per-serving identity, prompt, and usage gates; OAuth refresh handoff only
  for OAuth servings; the credential-leak scan covers every resolved secret.
- [x] 3.2 CLI gains `--serving` on `canary`, `probe`, `run`,
  `validate-profile`, and `audit-job`, defaulting to `luna-max`; `resume`
  recovers the serving from job provenance.
- [x] 3.3 The analyzer refuses a paired comparison across differing servings
  without `--descriptive-only`, and the descriptive report keeps reward,
  token, and latency summaries per serving.
- [x] 3.4 Unit tests for the CLI flag surface and the cross-serving refusal.

## 4. Bridge quality gates

- [x] 4.1 `uv run pytest`, `uv run ruff check`, and `uv run mypy` pass in
  `benchmarks/harbor`.
- [x] 4.2 Re-pin `RUNTIME_EVENT_SCHEMA_VERSION` to the pinned runtime's
  current schema (16) and accept the interleaved `cache_controller` and
  `module_notification` envelopes; the bridge at repository HEAD could not
  parse streams from the Smith binary at repository HEAD (pre-existing
  breakage found by the canary gate).

## 5. Live baseline execution

- [x] 5.1 Rebuild and verify both musl artifacts from a clean tree
  (`smith-harbor build`, `smith-harbor verify-artifacts`); built from a
  `git worktree` of HEAD because the operator's tree carries unrelated
  in-progress TUI edits. Manifest records `dirty: false`, revision `ad292ae`.
- [x] 5.2 Recreate the Colima `harbor` profile with `--vz-rosetta`, mount the
  durable jobs volume, and verify an AMD64 container reports `VirtualApple`.
- [x] 5.3 Live canary passes for `luna-max` and `glm-5.3`
  (`reports/canary-luna-max-baseline.json`, `reports/canary-glm-53-baseline.json`).
- [x] 5.4 `smoke` profile passes on `glm-5.3` as the integration gate
  (`smith-glm53-smoke-baseline-001`): agent loop, verifier, and audit path all
  work end to end. Two of three tasks hit the initially pinned 16,384 output
  ceiling mid-reasoning; the operator chose native output ceilings
  (131,072 for GLM) for the baseline on that evidence.
- [ ] 5.5 `dev` profile (26 tasks × 1 rollout) completes on both servings
  (`smith-glm53-dev-baseline-001`, then `smith-luna-max-dev-baseline-001`).
- [ ] 5.6 Descriptive baseline comparison report produced with
  `analyze --descriptive-only`; both jobs audited for credential material.
