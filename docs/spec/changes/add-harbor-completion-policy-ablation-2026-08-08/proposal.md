---
created_at: 2026-08-09T01:06:40Z
updated_at: 2026-08-09T07:02:00Z
---

## Why

The first Luna Max Harbor smoke proved the runner, OAuth handoff, trajectory
conversion, and artifact boundaries, but scored zero of three tasks. The
failures were actionable: one final artifact contradicted Smith's claimed
validation, one ARC answer misplaced one of five markers after lossy
delegation, and one long proof timed out before creating its required artifact.

Smith needs a controlled, task-agnostic ablation before any full benchmark run.
The experiment must distinguish the value of an artifact-first completion
policy from the value or cost of delegation without adding task-specific hints
or tuning against verifier answers.

## What Changes

- Add an inherited, source-explainable profile field that can disable root
  delegation. When disabled, Smith omits both the `agent` tool and delegation
  prompt fragment while leaving every unrelated tool and policy unchanged.
- Add three versioned Harbor variants over one Smith binary:
  `current`, `artifact-first-v1`, and `artifact-first-v1-no-delegation`.
- Contribute the artifact-first policy as an additive profile developer
  instruction. Preserve the original Harbor task instruction byte-for-byte as
  the user message and record the policy text digest in job provenance.
- Run the frozen 26-task development manifest three times per variant. Use a
  deterministic three-round cyclic order so provider-time drift is not fully
  confounded with one variant, producing 234 total trajectories.
- Extend analysis to group the three one-rollout jobs for each variant, retain
  every timeout and verifier failure, and compare reward, tokens, and latency
  with task-paired bootstrap intervals.
- Bound a root `agent.wait` operation so one unfinished child cannot occupy the
  rest of an otherwise-unlimited Smith turn without returning control to the
  model.
- Make the serial Harbor launcher terminate its child process group on operator
  interruption, and make non-zero Smith exits report an exact exit/signal
  classification without exposing process output or credentials.
- Document the required Rosetta execution path when an Apple-Silicon Colima
  host runs the Harbor Index's AMD64 task images.
- Report the two primary contrasts separately:
  `artifact-first-v1 - current` and
  `artifact-first-v1-no-delegation - artifact-first-v1`.

## Impact

- Affected specs: `configuration`, `child-agents`, `evaluation-harness`
- Affected code: Smith profile parsing/resolution and runtime composition;
  bounded delegation wait behavior; `benchmarks/harbor/` variant, launcher,
  provenance, diagnostics, and analysis code
- Compatibility: additive; existing profiles default to delegation enabled
- Experiment size: 26 tasks x 3 rollouts x 3 variants = 234 serial OAuth
  trajectories, potentially requiring multiple days and resumable execution
- Cost: subscription/OAuth USD cost remains unknown; provider usage and rate
  windows are recorded without treating a subscription as zero marginal cost

## Out of Scope

- Adding task-specific primer, ARC, math, verifier, or expected-answer hints.
- Increasing benchmark task deadlines or changing task/verifier contents.
- Using the three inspected smoke tasks as unbiased evidence of improvement.
- Running or tuning on the full 82-task profile in this change.
- Claiming Deep Agents, Smith, or any model is universally better from this
  internal policy ablation.
- Adding a general production deadline scheduler or synthetic mid-turn steer.
- Treating strict verifier misses, incorrect exact answers, or incomplete task
  implementations as infrastructure failures.

## Approval Boundary

Approval authorizes the additive profile delegation switch, three Harbor
variants, deterministic 234-trajectory development experiment, grouped paired
analysis, tests, OAuth-backed execution, and sanitized local reports described
in `tasks.md`.

It does not authorize task-specific prompt tuning, changed Harbor verifiers,
concurrent copies of the OAuth credential, public job upload, a full 82-task
run, or a cross-harness performance claim.
