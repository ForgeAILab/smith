## ADDED Requirements

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
