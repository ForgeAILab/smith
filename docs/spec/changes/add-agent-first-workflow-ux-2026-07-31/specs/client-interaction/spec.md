## ADDED Requirements

### Requirement: Agent-first idle composer

The interactive TUI SHALL identify the active root agent mode, provider/model,
project/branch, and context confidence at the idle point of action and SHALL
show bounded discovery hints without adding a permanent header or focusable
region. The composer remains the sole persistent focus.

#### Scenario: Empty idle composer at normal width
- **GIVEN** no overlay or turn is active and the composer is empty
- **WHEN** Smith renders at a normal terminal width
- **THEN** it shows the active agent mode beside provider/model and project
- **AND** it exposes `Tab agents`, `Ctrl+P commands`, `@ files/agents`, and
  `! shell` as concise hints

#### Scenario: Empty idle composer at narrow width
- **GIVEN** the terminal is 44 columns wide
- **WHEN** the identity and hints cannot all fit
- **THEN** Smith removes low-priority hint and path detail first
- **AND** retains agent/activity, approval state, model, and honest context
  provenance without relying on color

#### Scenario: Tab cycles an idle root mode
- **GIVEN** the runtime is idle, the composer is empty, and no overlay is open
- **WHEN** the user presses `Tab`
- **THEN** Smith selects the next authorized root agent mode at a safe boundary
- **AND** focus remains in the composer without sending a provider request

#### Scenario: Tab does not replace overlay behavior
- **GIVEN** command completion, reference completion, approval, or a
  questionnaire is open
- **WHEN** the user presses `Tab`
- **THEN** the overlay's documented navigation/completion behavior runs
- **AND** the active root mode does not change

### Requirement: Unified typed reference completion

Smith SHALL provide one `@` completion surface for bounded canonical workspace
files and registered child-agent presets. Resolution MUST occur locally before
provider spend and MUST retain type, provenance, authority, and size metadata.

#### Scenario: Attach a workspace file
- **GIVEN** the user selects `@src/lib.rs` from file completion
- **WHEN** they submit the prompt
- **THEN** Smith prepares and authorizes an exact workspace read
- **AND** contributes bounded content or an artifact reference with file
  provenance to the planned request

#### Scenario: Reference escapes the workspace
- **GIVEN** a draft contains an unresolved or outside-workspace file reference
- **WHEN** the user submits it
- **THEN** Smith keeps the draft and reports a local bounded error
- **AND** performs no provider request or unauthorized read

#### Scenario: Literal at sign
- **GIVEN** the draft contains the documented `@@` escape
- **WHEN** the user submits it
- **THEN** Smith sends one literal leading `@` at that position
- **AND** does not open or resolve a reference

### Requirement: Prepared local shell shortcut

Input beginning with one non-whitespace `!` SHALL execute as a local shell
action only through Smith's canonical prepared tool executor. It MUST use the
same schema validation, exact workspace, broad permission bound, authorization,
approval, deadline, cancellation, scheduling, output bounding, artifacts, and
checkpoint semantics as a model-requested shell call.

#### Scenario: User submits a shell shortcut
- **GIVEN** the composer contains `!cargo test`
- **WHEN** the user submits it
- **THEN** Smith displays the prepared action and applies resolved approval
  policy before execution
- **AND** renders the committed result locally without a provider request

#### Scenario: Shell approval is denied
- **GIVEN** the prepared shortcut requires approval
- **WHEN** the user denies it
- **THEN** no process starts and the transcript records a bounded denial
- **AND** the denial grants no future authority

#### Scenario: Literal exclamation prompt
- **GIVEN** the draft begins with `!!`
- **WHEN** the user submits it
- **THEN** Smith sends a normal user prompt beginning with one `!`
- **AND** starts no local shell action

### Requirement: Replay-equivalent live work summary

Smith SHALL derive one replaceable live work summary from versioned runtime
events, covering plan progress, active tools, validation gates, retries,
changed-path count, and attributed child state. `/details` SHALL toggle bounded
detail without revealing protected arguments, and the terminal evidence row
MUST be equivalent under live reduction and journal replay.

#### Scenario: Multi-step coding turn advances
- **GIVEN** a turn has a plan, active tool, and running child review
- **WHEN** their lifecycle events arrive
- **THEN** one work summary updates in place with current state and attribution
- **AND** the composer remains usable without a permanent plan pane

#### Scenario: Turn reaches a terminal result
- **GIVEN** a work summary is live
- **WHEN** the turn succeeds, fails, is interrupted, or reaches a limit
- **THEN** Smith commits one compact evidence row with terminal plan counts
- **AND** no pending or in-progress state remains presented as active work

#### Scenario: Details remain redaction-safe
- **GIVEN** a prepared tool contains a command, edit body, or sensitive answer
- **WHEN** the user invokes `/details`
- **THEN** Smith shows only the reviewed typed projection and lifecycle evidence
- **AND** never reconstructs raw values from redacted events
