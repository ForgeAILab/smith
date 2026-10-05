## Context

The initial smoke had three distinct failure modes:

- The primer artifact failed the verifier's exact Tm reconstruction even
  though Smith's terminal answer claimed a passing check.
- The ARC artifact had correct dimensions and marker count but placed one of
  five markers at the wrong coordinate. Smith spawned two children; one lacked
  sufficient task context, requested input, and was stopped.
- OmniMath emitted hundreds of reasoning events but reached the 1,200-second
  boundary without creating the required response artifact.

These are completion and validation failures, not Harbor or OAuth failures.
Because the smoke tasks and their verifier diagnostics have now been inspected,
they are calibration evidence only.

## Goals / Non-Goals

- Goals:
  - Test a generic completion policy without modifying Harbor user tasks.
  - Isolate the incremental effect of suppressing delegation.
  - Keep model, effort, artifact, tools other than delegation, authority,
    timeout, network, resources, OAuth policy, tasks, and rollouts fixed.
  - Preserve failures and use task-paired uncertainty intervals.
  - Make a multi-day serial run resumable at one-rollout job boundaries.
- Non-Goals:
  - Guarantee a positive reward change.
  - Tune to the three observed answers or hidden verifier implementation.
  - Add a universal Smith deadline feature.
  - Select a final production default from one development run alone.

## Decisions

### 1. Use a profile instruction, not a rewritten task

`artifact-first-v1` is an additive Smith profile developer fragment. Harbor's
original task remains the exact user instruction stored in `instruction.txt`
and represented in ATIF. The policy is versioned and hashed in provenance:

```text
Create every required deliverable at its exact path early in the run, then
refine it. Treat the final bytes on disk as the source of truth. After the last
edit, reread the deliverable and run an independent check derived only from the
task instruction. For example-based transformations, replay the inferred rule
against every training example before applying it. Reserve the final portion
of the task budget for validation and leave the best complete artifact in
place if time becomes uncertain. Report success only after checks against the
final artifact pass; otherwise report the remaining failure honestly.
```

The policy contains no task identity, answer, verifier path, hidden assertion,
or benchmark-family special case. Its exact text and SHA-256 are stable inputs
to the experiment.

### 2. Disable delegation at composition time

A new optional profile field, `delegation`, defaults to `true`. A main profile
with `delegation = false` makes the factory's single delegation-eligibility
predicate return false. That predicate controls both prompt contribution and
`agent` tool registration, preventing the misleading state where instructions
and actual capabilities drift.

The field narrows capability only. Project configuration cannot use it to add
delegation, permissions, child profiles, credentials, or authority beyond the
host's existing policy. Child surfaces remain non-delegating regardless of the
field.

### 3. Compare exactly three variants

- `current`: no completion-policy fragment; delegation uses its existing
  default.
- `artifact-first-v1`: the versioned fragment is active; delegation remains
  enabled.
- `artifact-first-v1-no-delegation`: the same fragment is active and the
  profile sets `delegation = false`.

All variants run from the same Smith revision and artifact digest. Variant
identity, effective delegation state, policy revision, policy digest, and
comparison axis are recorded in job and ATIF provenance.

### 4. Rotate order across three serial rounds

One OAuth bundle requires serial execution. To reduce complete confounding
between variant and wall-clock/provider state, the experiment launcher runs
one rollout per task in three rounds using this cyclic order:

```text
round 1: current, artifact-first-v1, no-delegation
round 2: artifact-first-v1, no-delegation, current
round 3: no-delegation, current, artifact-first-v1
```

Each cell is a normal resumable 26-task Harbor job. The launcher writes its
schedule before execution, refuses a mismatched existing job, and resumes only
the missing or incomplete cell. OAuth concurrency stays one.

### 5. Group jobs before paired bootstrap analysis

The analyzer accepts a manifest containing three compatible one-rollout jobs
per variant. It validates that every job shares the serving and task invariants
and differs only on the declared variant axis. It then averages each variant's
three rollout observations within task before task-paired bootstrapping.

Primary contrasts are:

```text
artifact-first-v1 - current
no-delegation - artifact-first-v1
```

The first estimates the policy effect. The second estimates the incremental
effect of suppressing delegation under the same policy. Reward, tokens, and
latency are reported independently; missing metrics make only that metric
unavailable. No improvement/reduction wording is allowed when the interval
crosses zero.

## Risks / Trade-offs

- A 234-trajectory serial experiment can take days and can encounter provider
  quota or OAuth interruptions. Round/cell resume boundaries limit rework.
- The policy adds fixed prompt tokens. Any trajectory savings must exceed this
  overhead and are measured end to end.
- `delegation = false` is an additive production configuration surface created
  for a controlled ablation. Its default preserves current behavior.
- The development set is a tuning set, not a holdout. A later full report must
  identify the 56 non-development tasks separately if used for confirmation.
- Three rollouts reduce but do not eliminate model stochasticity or temporal
  provider drift.

## Migration Plan

1. Add and contract-test the delegation profile field with a default of true.
2. Add variant configuration, provenance, and fake-agent tests.
3. Add the cyclic experiment manifest/launcher and grouped analyzer.
4. Validate all local tests and build one common static Smith artifact.
5. Run one canary per distinct policy/delegation composition.
6. Execute and audit the nine serial development jobs.
7. Produce paired JSON/Markdown reports for the two primary contrasts.

## Open Questions

- None. The full Harbor Index run remains a separate approval after this
  development ablation is interpreted.

## Paused-run failure hardening

The first development cell was paused after ten zero-reward trials. Four were
valid Smith completions rejected by strict task verifiers: two incorrect exact
answers, one unsupported circuit instruction, and one repository patch with
672 of 673 required tests passing. Those remain model or task-solution
failures and are not retried or reclassified. The other six exposed completion,
contract, provider, or host-runtime failures rather than ordinary verifier
rejections:

- two task-boundary timeouts, including one root blocked in `agent.wait` for a
  still-running child;
- one model-emitted shell call that violated the advertised JSON schema and was
  correctly rejected fail-closed;
- one logical provider request whose three attempts each reached the existing
  300-second transport deadline;
- one immediate exit 139 from an AMD64 task container on an ARM64 Colima VM
  using generic foreign-architecture emulation; and
- one AMD64 Qt verifier in which every process-spawning test failed with a
  `QProcess` fork error under generic emulation.

The exit-139 case did not reproduce on an identical focused rerun. The local
Harbor engine had Rosetta disabled even though every pulled Index task and
verifier image was AMD64, so the dedicated Colima profile now uses Rosetta.
This is environment hardening, not a change to Smith, the model, or a task.
The preserved Qt task workspace was then replayed against the unchanged
official verifier under Rosetta: all 42 required tests passed. Its original
zero is therefore retained in the paused job but classified as invalid
host-emulation evidence, not a Luna patch failure.

`agent.wait` remains a blocking convenience, but an omitted timeout now uses a
bounded default and an explicit timeout is capped. Expiry reports that the
child is still running and returns control to the root; it does not stop or
alter the child. The enclosing invocation deadline and cancellation remain
authoritative.

The experiment launcher owns a Harbor subprocess group. On interruption it
forwards SIGINT, waits a bounded cleanup interval, escalates to SIGTERM and
then SIGKILL only if needed, and does not leave a detached Harbor runner.
Smith non-zero exits include the numeric code and conventional signal name in
the exception while retaining the existing redaction boundary.
