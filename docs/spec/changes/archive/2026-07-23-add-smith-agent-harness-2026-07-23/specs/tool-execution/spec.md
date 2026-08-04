## ADDED Requirements

### Requirement: Minimal built-in coding tools

Smith SHALL provide built-in file read, path/text search, patch application,
and shell command tools implementing Agent Runtime's neutral `Tool` contract.
Every tool MUST publish a stable name, description, effects, input schema, and
bounded structured outcome.

#### Scenario: Apply a project patch

- **GIVEN** the agent submits a valid patch inside the allowed project root
- **WHEN** approval policy permits the write
- **THEN** Smith applies the patch atomically where practical
- **AND** records a structured result and affected paths

### Requirement: Scoped execution context

Every tool call MUST use the shared invocation context carrying workspace,
deadline, cancellation token, output limit, approval decision, and request
identity. A Smith tool MUST NOT infer broader filesystem authority from the
process working directory.

#### Scenario: Tool targets outside its scope

- **GIVEN** a filesystem tool is restricted to one project root
- **WHEN** its resolved target escapes that root
- **THEN** Smith rejects the call before mutation
- **AND** records a permission failure

### Requirement: Approval before material side effects

Smith SHALL inject a configurable approval policy into Agent Runtime. The
shared executor MUST evaluate it before shell execution or filesystem mutation;
an action requiring confirmation remains pending until approved and fails
closed when no allowing policy is available.

#### Scenario: Interactive mutation requires confirmation

- **GIVEN** a patch is classified as requiring approval
- **WHEN** the agent requests it in the TUI
- **THEN** Smith displays the exact tool, scope, and material arguments
- **AND** executes only after user approval

#### Scenario: Headless approval cannot be collected

- **GIVEN** `smith -p` has no TTY and no policy authorizing a shell command
- **WHEN** the agent requests that command
- **THEN** Smith returns an approval-required result and stable non-success
  outcome
- **AND** never hangs waiting for input

### Requirement: Bounded cancellable processes

Shell and monitor commands MUST run in Smith-owned process groups on macOS and
Linux. Smith SHALL enforce deadlines and output limits and MUST terminate the
owned group on cancellation or confirmed shutdown.

#### Scenario: Shell command spawns a child process

- **GIVEN** a shell command starts a subprocess
- **WHEN** the tool call is cancelled
- **THEN** Smith terminates the owned process group within the cleanup grace
  period
- **AND** records whether forced termination was required

### Requirement: Side-effect-aware tool scheduling

Smith SHALL configure Agent Runtime's side-effect-aware scheduler. Independent
read-only tools MAY run concurrently, but shared execution MUST serialize or
reject calls whose declared write scopes overlap unless an explicit conflict
policy allows another deterministic outcome.

#### Scenario: Two patches overlap

- **GIVEN** one model turn requests two patches to the same file
- **WHEN** Smith schedules the calls
- **THEN** it does not apply them concurrently
- **AND** preserves deterministic result order
