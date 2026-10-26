# evaluation-harness Specification

## Purpose
Reproducible benchmark evaluation of Smith (Harbor), with isolated credentials and honest metrics and trajectories.
## Requirements
### Requirement: Reproducible Harbor evaluation profiles

Smith SHALL provide an isolated Harbor integration that pins the Harbor
version, Harbor Index dataset revision, Smith binary revision and digest,
provider path, model, reasoning effort, timeout, resource policy, network
policy, approval policy, task identities, and rollout count. It MUST provide
separate frozen smoke, development, and complete profiles and MUST keep raw job
output out of version control by default.

#### Scenario: Run the Luna Max smoke profile

- **GIVEN** verified Smith artifacts and a usable selected ChatGPT OAuth entry
- **WHEN** the operator runs the frozen smoke profile
- **THEN** Harbor executes its fixed Harbor Index 1.0 tasks serially with
  `gpt-5.6-luna` at `max` effort
- **AND** the job records every pinned run invariant needed to reproduce it

#### Scenario: Task image activates a project interpreter at login

- **GIVEN** a Harbor task image exposes its project interpreter or test runner
  through a Bash login profile
- **WHEN** the bridge launches Smith and Smith invokes its shell tool
- **THEN** Smith and its child commands inherit that task login environment
- **AND** a minimal image without Bash falls back to a POSIX shell

#### Scenario: Run the complete evaluation

- **GIVEN** the live canary, smoke profile, and development profile have passed
- **WHEN** the operator runs the complete profile
- **THEN** Harbor executes all 82 Harbor Index 1.0 tasks with three rollouts
- **AND** it uses the same serving and policy invariants for every trial

### Requirement: OAuth credential isolation

The Harbor bridge MUST read only one explicitly selected Smith OAuth entry from
an owner-only schema-v1 auth document and MUST upload only a minimal private auth
document into the ephemeral trial. Credential values and Smith user-state paths
MUST NOT enter commands, logs, errors, trajectories, metadata, verifier input,
or collected artifacts. After each serial trial, the bridge MUST validate the
trial document and atomically merge only the selected refreshed entry into the
host document under a private lock. It MUST preserve unrelated host fields and
credentials and MUST reject a concurrent selected-entry change.

#### Scenario: Host auth file contains several products

- **GIVEN** the host auth document contains `chatgpt`, another ChatGPT account,
  and a non-ChatGPT credential
- **WHEN** the bridge prepares a trial for entry `chatgpt`
- **THEN** the uploaded document contains only the schema version and selected
  `chatgpt` entry
- **AND** no other credential name or value appears in the trial or job output

#### Scenario: A trial rotates the selected refresh token

- **GIVEN** a serial trial started from the current selected `chatgpt` entry
- **WHEN** Smith refreshes that entry before the trial ends
- **THEN** the bridge atomically replaces only the matching host entry
- **AND** all unrelated credentials and document fields remain unchanged

#### Scenario: The host entry changes during a trial

- **GIVEN** a trial started from one selected entry value
- **WHEN** another writer changes that host entry before refresh handoff
- **THEN** the bridge rejects the handoff without overwriting either value
- **AND** no credential content appears in the diagnostic

#### Scenario: Auth file is unsafe or missing

- **GIVEN** the configured auth path is missing, oversized, malformed,
  non-regular, or not owner-only
- **WHEN** the bridge performs preflight
- **THEN** it fails before uploading files or starting Smith
- **AND** its diagnostic contains no auth-file content

### Requirement: Honest Harbor metrics and ATIF trajectory

The bridge SHALL convert Smith's schema-v3 headless stream into an ATIF v1.7
trajectory and Harbor `AgentContext` metrics while preserving Smith's disjoint
usage counters in metadata. Harbor input MUST equal uncached input plus cached
input plus cache-write tokens, Harbor cache MUST equal cached input, and Harbor
output MUST equal visible output plus reasoning tokens. OAuth/subscription cost
MUST remain unknown unless the provider reports a trajectory-attributable cost.

#### Scenario: Tool-assisted run reports every counter

- **GIVEN** Smith completes a turn with uncached input, a cache read, a cache
  write, reasoning, visible output, and tool continuations
- **WHEN** the bridge converts the stream
- **THEN** Harbor receives the specified aggregate token mapping
- **AND** ATIF metadata retains each original counter, request, attempt, and
  tool lifecycle separately

#### Scenario: Subscription run has no cost observation

- **GIVEN** a ChatGPT OAuth trial reports tokens but no per-trajectory USD cost
- **WHEN** the bridge populates Harbor metrics
- **THEN** `cost_usd` remains absent
- **AND** the bridge does not substitute Platform API pricing or zero cost

#### Scenario: Smith fails after partial output

- **GIVEN** Smith emitted attempt-scoped output and usage before a failure or
  timeout
- **WHEN** Harbor completes the trial
- **THEN** the partial trajectory and usage remain available for diagnosis
- **AND** the terminal Smith status is not presented as success

### Requirement: Fixed overhead and trajectory consumption remain distinct

The evaluation SHALL report planned base-harness component tokens separately
from provider-observed first-request usage and complete task trajectory usage.
Estimated segment attribution MUST remain labelled as planned/estimated and
MUST NOT be transformed into provider-reported component counts by subtraction.

#### Scenario: Probe a fresh minimal session

- **GIVEN** a fresh Smith session receives the fixed base probe input
- **WHEN** its first request completes
- **THEN** the report includes planned token totals by context segment kind
- **AND** separately includes the provider-observed first-attempt input/cache
  totals and their provenance

#### Scenario: Report a complete Harbor task

- **GIVEN** one task used retries, tools, compaction, or delegation
- **WHEN** the trial report is produced
- **THEN** its trajectory total includes every provider-attributed attempt and
  continuation
- **AND** no fixed-overhead percentage is presented as total task savings

### Requirement: Task-paired statistical comparison

The evaluation SHALL compare compatible jobs by averaging rollouts within each
task and bootstrapping paired task differences with a fixed seed and at least
10,000 resamples. It MUST report 95 percent percentile intervals and MUST use
improvement or reduction language only when the corresponding interval excludes
zero.

#### Scenario: Compatible jobs are compared

- **GIVEN** two jobs share dataset tasks, provider path, model, effort, timeout,
  and rollout policy
- **WHEN** the analyzer compares reward, token usage, and latency
- **THEN** it reports task-paired point estimates and 95 percent intervals
- **AND** cross-tabulates Smith-reported success against verifier success

#### Scenario: Serving conditions differ

- **GIVEN** two jobs use different models, efforts, provider paths, task sets,
  or execution policies
- **WHEN** a paired comparison is requested
- **THEN** the analyzer refuses the unlabelled paired claim
- **AND** may produce only an explicitly labelled descriptive comparison

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

### Requirement: Live provider conformance suite

Smith SHALL provide an ignored, opt-in live test that drives the built (or a
named) `smith` binary headlessly against routes from the owner's real
configuration, named in `SMITH_LIVE_CONFORMANCE_MODELS`. For every route it
SHALL run a two-read tool turn, a resumed tool-using continuation, a turn
killed after its first tool result and then resumed, and, for routes named in
`SMITH_LIVE_CONFORMANCE_SEARCH`, a hosted-search-plus-tool turn and a resumed
follow-up; for every ordered pair of distinct routes it SHALL run a tool turn
on the first and a resumed tool-using turn on the second. It MUST NOT move
the owner's home or configuration, MUST remove only the session files its own
runs created, and MUST write `report.json` and `report.md` with each
scenario's status, session, usage, cache-read percentage, and on failure
Smith's full error text and the stderr tail. Cache percentages SHALL be
reported and never asserted.

#### Scenario: A provider rejects a resumed turn
- **GIVEN** a route whose provider rejects the resumed half of a scenario
- **WHEN** the suite finishes
- **THEN** the report marks that scenario failed and quotes Smith's error,
  including the provider's checked reason
- **AND** the remaining scenarios still run and the test fails at the end

#### Scenario: Nothing is configured
- **WHEN** the suite runs without `SMITH_LIVE_CONFORMANCE_MODELS`
- **THEN** it spends no provider quota and does nothing

### Requirement: Pinned model servings

The bridge SHALL define an explicit serving table. Each serving pins its
provider name, provider kind, base URL, credential reference scheme, model
catalog limits, reasoning effort policy (including no effort), allowed egress
hosts, and canary prompt. Frozen run profiles stay serving-independent; a run
selects exactly one serving, and the serving identity plus its invariants are
recorded in job configuration, run invariants, and job provenance. The
initial table contains `luna-max` (ChatGPT OAuth `gpt-5.6-luna` at `max`) and
`glm-5.3` (z.ai OpenAI-compatible `glm-5.3`, no effort flag). Both share the
same max-input budget; each serving's output ceiling is its model's native
limit and its context is exactly its input budget plus its output reserve.

#### Scenario: A GLM trial configuration carries no credential value

- **GIVEN** the `glm-5.3` serving is selected for a run
- **WHEN** the bridge generates the trial `config.toml`
- **THEN** the provider block references the key as `env:ZAI_API_KEY`
- **AND** the document contains no API-key or OAuth credential value
- **AND** the provider base URL and `response.reasoning_only = "text"` policy
  match the serving descriptor

#### Scenario: Serving and profile constants stay consistent

- **GIVEN** an operator selects a serving for a frozen profile
- **WHEN** job configuration is built
- **THEN** the agent model name, terminal-result identity check, allowed
  egress hosts, and provenance all derive from that one serving descriptor
- **AND** a mismatch between the requested serving and the agent model name
  fails the run before any trial starts

#### Scenario: Unknown serving fails closed

- **GIVEN** a serving name that is not in the serving table
- **WHEN** any bridge command resolves it
- **THEN** the command fails with a bounded error naming the unknown serving
- **AND** no job, config, or trial is created

### Requirement: Environment credential isolation for API-key servings

The bridge SHALL resolve environment-reference API keys at launch from the
documented environment variable, falling back to the operator's Smith
configuration provider block, into process memory only. It reaches the
sandboxed Smith process exclusively through the
environment-exec channel. Credential material MUST NOT be written into a
repository file, Harbor configuration, command string, log, or trajectory,
and completed jobs are audited for the resolved key value. The OAuth
auth-file upload and refresh handoff do not run for these servings.

#### Scenario: Missing key material fails before trials start

- **GIVEN** neither the environment variable nor the operator configuration
  provides the API key
- **WHEN** the bridge preflights the run
- **THEN** preflight fails with a message that identifies the expected
  variable and provider block
- **AND** the message contains no credential material

#### Scenario: No OAuth handoff on API-key servings

- **GIVEN** a trial running on an API-key serving
- **WHEN** the trial finishes or is interrupted
- **THEN** no auth-file refresh handoff runs against the operator's auth store

### Requirement: Per-serving live canary gate

Before spending on Harbor tasks, the isolated live canary SHALL support any
serving in the table. The gate asserts the serving identity the run requested
(provider, model, and effort when the serving pins one), a successful terminal
result, provider-reported usage, and output free of every resolved credential
value. Canary reports record which serving produced them.

#### Scenario: GLM canary passes with provider-reported usage

- **GIVEN** the `glm-5.3` serving with resolvable key material
- **WHEN** the isolated canary runs
- **THEN** the terminal result reports provider `zai` and model `glm-5.3`
- **AND** the canary report records provider-attributed usage and the serving
  name

#### Scenario: Canary output containing credential material aborts

- **GIVEN** a canary run on either serving
- **WHEN** stdout or stderr contains any resolved credential value
- **THEN** the canary fails closed with a bounded message
- **AND** no report is written that contains the credential

### Requirement: Cross-serving descriptive comparison

The analyzer's task-paired bootstrap comparison SHALL require matching serving
invariants. Jobs run on different servings are compared only through an
explicitly descriptive summary that keeps reward, token, and latency evidence
per serving, never claims paired significance across servings, and keeps cost
provenance per serving rather than mixing subscription-unknown and metered
observations.

#### Scenario: Paired comparison refuses across servings

- **GIVEN** one job run on `luna-max` and another on `glm-5.3`
- **WHEN** they are analyzed without `--descriptive-only`
- **THEN** the analyzer refuses the comparison, naming the differing serving
  invariants

#### Scenario: Descriptive baseline keeps evidence separate

- **GIVEN** the same two jobs
- **WHEN** they are analyzed with `--descriptive-only`
- **THEN** the report presents reward, token, and latency summaries per
  serving side by side
- **AND** it labels the comparison descriptive rather than statistically
  paired
