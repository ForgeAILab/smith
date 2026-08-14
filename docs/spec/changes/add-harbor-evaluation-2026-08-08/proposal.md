---
created_at: 2026-08-08T20:29:08Z
updated_at: 2026-08-08T23:40:01Z
---

## Why

Smith has one disposable coding-workflow evaluation, deterministic provider
tests, and a versioned headless protocol, but no runner for public agent
benchmarks. That makes it impossible to reproduce the Harbor Index experiment
described for Deep Agents, distinguish fixed harness overhead from complete
trajectory consumption, or compare Smith changes with task-paired confidence
intervals.

Harbor already owns task acquisition, isolated environments, executable
verifiers, rollouts, rewards, timing, and result storage. Smith should integrate
as a Harbor agent instead of building a competing benchmark runner. The first
configuration will exercise the exact model requested for this evaluation:
`gpt-5.6-luna` with `max` reasoning through Smith's existing
`chatgpt-responses` OAuth provider.

## What Changes

- Add a pinned, self-contained Harbor evaluation project under
  `benchmarks/harbor/` with a custom installed Smith agent, converter tests,
  frozen run profiles, an analysis command, and a runbook.
- Build revision- and SHA-attributed static Linux Smith binaries once, then
  upload the matching architecture into each Harbor task instead of compiling
  Smith separately in every trial.
- Read one named Smith OAuth entry from the host's owner-only
  `~/.smith/auth.json`, construct an in-memory/minimal schema-v1 auth document,
  and upload it privately to the trial. Other credentials, the source auth
  file, and refreshed trial credentials never enter Harbor logs or artifacts.
  After each serial trial, atomically merge only the selected refreshed entry
  into Smith's supported host auth store while preserving unrelated state and
  rejecting concurrent changes.
- Generate a trial-local Smith configuration pinned to provider `chatgpt`,
  model `gpt-5.6-luna`, effort `max`, conservative reviewed context/output
  limits, disabled persistence, and unattended `allow-all` authority inside
  Harbor's disposable workspace.
- Run Smith through its existing schema-v3 `stream-json` headless protocol and
  convert the events/result to Harbor's ATIF trajectory plus `AgentContext`
  metrics.
- Preserve disjoint Smith usage in trajectory metadata while mapping Harbor's
  aggregate counters explicitly. Subscription/OAuth runs report provider cost
  as unknown; they do not borrow OpenAI Platform API prices or call a fixed
  subscription fee zero marginal cost.
- Commit three frozen profiles: a tiny integration smoke, a representative
  development subset, and the complete Harbor Index 1.0 run with three
  rollouts. The default concurrency is one because every trial receives an
  isolated copy of one renewable OAuth bundle.
- Add an analysis command that compares compatible Harbor jobs by task,
  averages rollouts within each task, and reports deterministic paired
  bootstrap confidence intervals for reward, tokens, and latency. Reported
  Smith success is also cross-tabulated against verifier success.
- Add a separate base-footprint probe. It reports Smith's planned token totals
  by context segment kind and the provider-reported first-request total as two
  different measurements; it never presents estimated component attribution
  as provider-reported usage.

## Impact

- Affected specs: `evaluation-harness` (new)
- Affected code: new `benchmarks/harbor/` project and a release-helper script;
  no production Smith or Agent Runtime contract changes
- External dependency: a pinned Harbor Python package and its transitive
  dependencies, isolated from the Cargo workspace
- Credentials: uses an existing Smith `authfile:chatgpt` OAuth entry and copies
  only the selected entry into an ephemeral trial; serial refresh rotation is
  handed back to that same entry under a private lock
- Compatibility: additive; ordinary Smith, TUI, headless, configuration, and
  release behavior are unchanged
- Cost limitation: ChatGPT subscription/OAuth does not expose per-trajectory
  provider USD cost, so cost remains unknown for this configuration

## Out of Scope

- Claiming Smith is better or worse than Deep Agents from a Smith-only run.
- Comparing an OAuth-backed Smith run with an API-key-backed harness as if the
  provider conditions were identical.
- Implementing Harbor integrations for Deep Agents or Terminus.
- Changing Smith's todo, delegation, prompt, registry-routing, or tool policy.
- Parallel mutation or round-robin synchronization of one OAuth refresh token
  across trial sandboxes.
- Uploading raw Harbor jobs, trajectories, or OAuth material to a public
  service.

## Approval Boundary

Approval authorizes the isolated Harbor benchmark project, static Linux binary
builder, selected-entry OAuth injection and serial refresh handoff, fixed Luna
Max trial configuration,
ATIF/metric conversion, frozen smoke/development/full profiles, base-footprint
probe, paired analysis, tests, and documentation described in `tasks.md`.

It does not authorize a production Smith protocol change, an Agent Runtime
change, an API-key provider path, OAuth material in repository files or job
artifacts, public job upload, policy ablations, or cross-harness performance
claims.
