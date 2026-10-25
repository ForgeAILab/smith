## ADDED Requirements

### Requirement: Ordered model fallback within a profile

Smith SHALL accept an optional `[profiles.<name>.fallback]` table whose
`models` array contains ordered provider/model entries after the profile's
primary `provider` and `model`. Each entry SHALL require a declared provider
and model, and MAY override reasoning, context window, output request, and
first-response timeout. The profile's instructions and authority policy SHALL
remain the same for every candidate.

#### Scenario: Cross-provider chain

- **GIVEN** a profile selects `zai/glm-5.3` and lists ChatGPT and Anthropic model entries in `fallback.models`
- **WHEN** Smith resolves the profile
- **THEN** the primary remains first and the other bindings retain declaration order
- **AND** each binding resolves its own model limits, capabilities, credential source, and reasoning dialect

#### Scenario: No fallback configured

- **WHEN** an existing profile has no fallback table
- **THEN** it retains the current single-model retry and credential-rotation behavior

#### Scenario: Candidate controls differ

- **GIVEN** a fallback model requires different reasoning controls and a different named context window
- **WHEN** its entry explicitly overrides those controls
- **THEN** the candidate resolves using those values and its own catalog metadata
- **AND** unsupported explicit controls fail preflight rather than being silently changed

### Requirement: Configurable first-response budget

Smith SHALL accept a positive `first_response_timeout_ms` in the profile's
fallback table and an optional override on each fallback model entry. Omission
SHALL add no latency cutoff. A configured budget SHALL apply to each logical
provider request on that candidate, including retries and backoff, and SHALL
remain bounded by the original turn deadline.

#### Scenario: Explicit startup cutoff

- **GIVEN** a profile enables fallbacks with `first_response_timeout_ms = 30000`
- **WHEN** its primary has not started a meaningful response after 30 seconds
- **THEN** the runtime applies the safe timeout fallback policy
- **AND** retries or heartbeats cannot reset or satisfy the timer

#### Scenario: Per-candidate cutoff

- **GIVEN** the profile sets 30 seconds and one fallback entry sets 60 seconds
- **WHEN** the runtime selects that entry
- **THEN** its startup budget is 60 seconds, limited by the remaining turn deadline

#### Scenario: Invalid timeout

- **WHEN** a configured first-response timeout is zero, negative, or cannot be represented by the runtime
- **THEN** preflight fails with the candidate/key and source location before provider I/O

### Requirement: Explainable fallback inheritance and validation

Fallback settings SHALL follow normal file/profile precedence and bounded
profile inheritance. A higher-precedence model list SHALL replace the whole
inherited list; an empty list SHALL disable fallback and its startup timer.
Smith SHALL validate every candidate's metadata and credential-reference shape
before provider I/O, retain source provenance, and reject duplicate bindings
or installed-agent harness candidates. Secret-source acquisition MAY remain
lazy until a candidate is selected.

#### Scenario: Child profile disables the inherited chain

- **GIVEN** `plan` extends `code`, which declares fallback models and a timeout
- **WHEN** `plan` declares `fallback.models = []`
- **THEN** the effective plan profile has no fallback or startup cutoff
- **AND** config explanation identifies the inherited and winning sources

#### Scenario: Invalid candidate

- **WHEN** a fallback names an undeclared provider, unresolved model, duplicate binding, invalid explicit control, or `cli/...` harness
- **THEN** preflight fails at the exact fallback entry with source provenance
- **AND** no provider request is sent

#### Scenario: Exact model selection

- **GIVEN** a selected profile has a fallback chain
- **WHEN** a CLI or session override explicitly selects a provider or model
- **THEN** that explicit selection disables the profile chain until the profile is selected again
- **AND** config explanation reports why fallback is inactive
