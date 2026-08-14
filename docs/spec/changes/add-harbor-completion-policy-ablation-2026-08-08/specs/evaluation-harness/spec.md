## ADDED Requirements

### Requirement: Versioned completion-policy ablations

The Harbor evaluation SHALL support typed harness variants that contribute a
versioned task-agnostic policy as additive developer instructions while
preserving the original Harbor task as the exact user instruction. Every job
MUST record the variant identity, policy revision and digest, effective
delegation state, and unchanged serving invariants.

#### Scenario: Run the current baseline

- **GIVEN** the `current` variant is selected
- **WHEN** Smith receives a Harbor task
- **THEN** no completion-policy fragment is added
- **AND** delegation retains its existing default behavior

#### Scenario: Run artifact-first without delegation

- **GIVEN** `artifact-first-v1-no-delegation` is selected
- **WHEN** Smith composes the task runtime
- **THEN** the versioned artifact-first developer fragment is active
- **AND** the delegation prompt and `agent` tool are both absent
- **AND** the original Harbor task bytes remain the user instruction

### Requirement: Controlled development ablation

The evaluation SHALL run the frozen 26-task development manifest for three
rollouts under each of the three approved variants. Execution MUST remain
serial, MUST rotate variant order deterministically across rounds, MUST be
resumable at job boundaries, and MUST retain all verifier failures and
timeouts.

#### Scenario: Execute the complete development matrix

- **GIVEN** one verified Smith artifact and a usable OAuth entry
- **WHEN** the operator starts the approved policy experiment
- **THEN** nine one-rollout jobs produce up to 234 task trajectories
- **AND** each round follows the committed cyclic variant order
- **AND** no cell uses OAuth concurrency above one

#### Scenario: Resume after a quota interruption

- **GIVEN** some experiment cells completed before execution stopped
- **WHEN** the operator resumes the experiment
- **THEN** completed compatible cells are retained
- **AND** only missing or incomplete work resumes
- **AND** a mismatched existing manifest fails closed

### Requirement: Declared-axis grouped comparison

The analyzer SHALL group three compatible one-rollout jobs per variant and
compare variants only when policy/delegation is the sole declared differing
axis. It MUST average rollouts within task before paired bootstrap analysis and
MUST report the policy and delegation contrasts separately.

#### Scenario: Compare artifact-first with current

- **GIVEN** three complete compatible rounds for both variants
- **WHEN** the policy contrast is analyzed
- **THEN** reward, tokens, latency, failures, and success cross-tabs use paired
  task observations
- **AND** policy-improvement language appears only when its interval excludes
  zero

#### Scenario: An undeclared invariant differs

- **GIVEN** two variant groups differ in model, effort, artifact, timeout,
  tasks, resources, network, approval, or rollout count
- **WHEN** grouped comparison is requested
- **THEN** the analyzer refuses the paired claim
- **AND** identifies only the non-secret invariant names that differ

### Requirement: Calibration evidence is not holdout evidence

The evaluation MUST label the smoke and development tasks as inspected or
tuning evidence after their diagnostics influence harness policy. It MUST NOT
present their post-change reward as an unbiased holdout result.

#### Scenario: Report the development ablation

- **GIVEN** the completion policy was motivated or selected using smoke and
  development evidence
- **WHEN** a comparison report is generated
- **THEN** it labels the result as an internal development ablation
- **AND** makes no full-suite or cross-harness generalization

### Requirement: Interruption-safe local execution diagnostics

The local evaluation launcher SHALL own and terminate the Harbor subprocess
group it starts when operator interruption occurs. Smith non-zero exits MUST
retain an exact redaction-safe numeric exit classification and, when the code
maps conventionally to a signal, the signal identity. Runtime hardening MUST
NOT alter task reward or turn a verifier failure into infrastructure success.

#### Scenario: Operator interrupts a running experiment cell

- **GIVEN** the launcher has an active Harbor subprocess group
- **WHEN** the operator interrupts the experiment
- **THEN** the launcher forwards interruption and waits a bounded cleanup
  interval
- **AND** escalates termination only while the owned subprocess group remains
  alive
- **AND** preserves completed trial directories for later resume or audit

#### Scenario: Smith exits after a native signal

- **GIVEN** Smith exits with the conventional status for a native signal
- **WHEN** the bridge raises the trial exception
- **THEN** the exception identifies the numeric exit code and signal
- **AND** includes no command, prompt, credential, raw provider body, or tool
  argument content
