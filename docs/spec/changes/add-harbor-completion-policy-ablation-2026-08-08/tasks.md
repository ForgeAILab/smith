---
created_at: 2026-08-09T01:06:40Z
updated_at: 2026-08-09T07:02:00Z
completed_at:
---

## 0. Approval and baseline

- [x] 0.1 Approve this proposal before implementation.
- [x] 0.2 Retain the completed smoke job as diagnostic evidence without using
  its inspected task answers as an optimization target.
- [x] 0.3 Freeze the three variant identities, policy text/revision/digest, and
  cyclic three-round execution order.

## 1. Profile-controlled delegation

- [x] 1.1 Add an inherited, source-explainable `delegation` profile field that
  defaults to enabled for existing profiles.
- [x] 1.2 Make one eligibility predicate control both delegation prompt text
  and `agent` tool registration.
- [x] 1.3 Prove `delegation = false` removes the prompt/tool without changing
  unrelated tools, authority, provider selection, or child-surface behavior.

## 2. Harbor variants

- [x] 2.1 Add `current`, `artifact-first-v1`, and
  `artifact-first-v1-no-delegation` as typed, versioned variants.
- [x] 2.2 Inject the completion policy only as a profile developer fragment;
  retain the original task bytes as the user instruction.
- [x] 2.3 Record variant, effective delegation, policy revision/digest, Smith
  artifact, and all existing run invariants in job and trajectory provenance.
- [x] 2.4 Test that no task name, expected answer, verifier path, or credential
  content enters the policy or committed reports.

## 3. Resumable experiment execution

- [x] 3.1 Add a deterministic experiment manifest for 26 frozen development
  tasks, three variants, three one-rollout rounds, and concurrency one.
- [x] 3.2 Execute cells in the specified cyclic order and refuse existing jobs
  whose manifest or invariants do not match.
- [x] 3.3 Resume incomplete cells without rerunning completed trials or
  widening OAuth concurrency.
- [x] 3.4 Document the nine job names, run/resume commands, expected trajectory
  count, quota interruption handling, and explicit prohibition on upload.

## 4. Grouped analysis

- [x] 4.1 Load three one-rollout jobs per variant and validate identical task,
  serving, timeout, resource, network, approval, and artifact invariants.
- [x] 4.2 Permit only the declared policy/delegation variant axis to differ.
- [x] 4.3 Average three rollout observations per task and run deterministic
  task-paired bootstrap analysis with at least 10,000 resamples.
- [x] 4.4 Report policy and delegation contrasts separately for reward, tokens,
  latency, failures, and reported/verifier success without overstating
  intervals that cross zero.

## 5. Verification and execution

- [x] 5.1 Run Smith format, Clippy, configuration, factory, prompt, and headless
  contract tests relevant to profile-controlled delegation.
- [x] 5.2 Run nested Harbor formatting, lint, type checks, unit tests, profile
  validation, artifact verification, and strict spec validation.
- [x] 5.3 Build and verify one common static Smith artifact and run live Luna
  Max canaries for the three effective compositions.
- [x] 5.4 Classify the paused first-cell failures as verifier/model,
  provider-transport, Smith runtime, or host-emulation evidence without
  changing task rewards.
- [x] 5.5 Bound `agent.wait` with a default and maximum wait, returning a
  structured still-running outcome without stopping the child.
- [x] 5.6 Make run/resume interruption terminate the owned Harbor process group
  and add exact redaction-safe Smith exit/signal diagnostics.
- [x] 5.7 Document and verify Rosetta for the dedicated Apple-Silicon Colima
  engine used to run the Index's AMD64 images.
- [x] 5.8 Re-run focused unit/contract tests and calibrated Luna canaries before
  deciding whether to replace or resume the development manifest.
- [ ] 5.9 Execute all nine serial development cells, retaining timeouts and
  verifier failures, for 234 expected trajectories.
- [ ] 5.10 Audit every job for OAuth material and validate every emitted ATIF
  trajectory.
- [ ] 5.11 Generate sanitized JSON/Markdown reports for both primary contrasts
  and state whether each result is statistically clear.
