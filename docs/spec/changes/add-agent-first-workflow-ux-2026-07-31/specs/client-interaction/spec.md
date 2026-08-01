## ADDED Requirements

### Requirement: Agent-first idle composer

The interactive TUI SHALL identify the active root agent mode, provider/model,
project/branch, and context confidence at the idle point of action without
adding a permanent header, shortcut strip, or focusable region. The composer
remains the sole persistent focus. `?` from an empty composer and `/help` SHALL
render the same bounded local command/composer guide without a provider request
or canonical history entry.

#### Scenario: Empty idle composer at normal width
- **GIVEN** no overlay or turn is active and the composer is empty
- **WHEN** Smith renders at a normal terminal width
- **THEN** it shows the active agent mode beside provider/model and project
- **AND** it does not render a persistent shortcut strip

#### Scenario: Empty idle composer at narrow width
- **GIVEN** the terminal is 44 columns wide
- **WHEN** the identity cannot all fit
- **THEN** Smith removes low-priority path detail first
- **AND** retains agent/activity, approval state, model, and honest context
  provenance without relying on color

#### Scenario: Local help is available on demand
- **GIVEN** no overlay is open and the composer is empty
- **WHEN** the user presses `?` or submits `/help`
- **THEN** Smith renders the same bounded command and composer guide locally
- **AND** creates no provider request or canonical model-history entry

#### Scenario: Interrupted draft is recoverable before exit
- **GIVEN** the composer contains an unsent draft
- **WHEN** the user presses `Ctrl+C` once
- **THEN** Smith clears the composer and stores the draft in bounded local
  recall history
- **AND** `Up` restores the draft without a provider request

#### Scenario: Consecutive control-C presses exit
- **GIVEN** the user has pressed `Ctrl+C` once and no intervening key
- **WHEN** the user presses `Ctrl+C` again within one second
- **THEN** Smith exits from either idle or active work

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

### Requirement: Replay-equivalent anchored todo pane

Smith SHALL derive one replaceable todo projection from versioned runtime
events. Public items SHALL render in a bounded, non-focusable pane immediately
above the composer and MUST NOT enter transcript history. Sensitive plan item
text MUST NOT render. `/details` SHALL toggle bounded tool lifecycle detail
without revealing protected arguments. Live reduction and journal replay MUST
produce the same todo projection. A compact picker SHALL temporarily replace
the todo presentation in the anchored pane without changing that projection.

#### Scenario: Multi-step coding turn advances
- **GIVEN** a turn has a public multi-step plan
- **WHEN** plan lifecycle events arrive
- **THEN** the authored todo items update in place immediately above the
  composer
- **AND** the transcript retains only the quiet working timer and ordinary
  attributed events

#### Scenario: Turn reaches a terminal result
- **GIVEN** an anchored todo pane is visible
- **WHEN** the turn succeeds, fails, is interrupted, or reaches a limit
- **THEN** the reconciled terminal todo remains visible until the next turn
  starts
- **AND** Smith commits no aggregate `work` row to the transcript

#### Scenario: Compact interaction replaces the todo presentation
- **GIVEN** an anchored public todo projection is visible
- **WHEN** the user opens command or resource completion
- **THEN** the compact picker replaces the todo presentation directly above
  the fixed composer
- **AND** closing the picker restores the unchanged todo projection
- **AND** the temporary picker controls disappear with it

#### Scenario: Details remain redaction-safe
- **GIVEN** a prepared tool contains a command, edit body, or sensitive answer
- **WHEN** the user invokes `/details`
- **THEN** Smith shows only the reviewed typed projection and lifecycle evidence
- **AND** never reconstructs raw values from redacted events
