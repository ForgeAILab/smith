## ADDED Requirements

### Requirement: Inline prompts share one numbered select

Every interactive prompt SHALL render in the anchored pane, in place of the
composer while it is open and keeping its draft: tool approval, confirmation (provider spend, undo, revert, redo,
trust, child follow-up and resume, account rotation), and agent-originated
questionnaire, with no frame and no centering, using one grammar: an
indented heading-tone title, indented evidence, and numbered `❯ N. Label`
choice rows. Up/Down SHALL move the highlight, Enter SHALL answer the
highlighted row, and the digit SHALL answer directly where it is not already
bound to other meaning. Long evidence SHALL scroll with Shift+Up/Down while the
choices stay visible. A prompt MAY take the transcript and composer rows; a pane too
short for the list SHALL show the choices on one row. Every prompt SHALL open
behind the quiet window that swallows keys typed before it appeared. An
approval's highlight SHALL start on `Yes`; a confirmation's highlight SHALL
start on its cancel row, so Enter alone never applies a consequential action.

#### Scenario: An approval reads like the questionnaire

- **GIVEN** a shell tool requests approval
- **WHEN** the prompt opens
- **THEN** the anchored pane shows the title, command, place, and
  warning, then `Do you want to proceed?` and `❯ 1. Yes`, `2.` session
  allowance, `3. No (esc)`
- **AND** no bordered or centered box is drawn

#### Scenario: Enter after the quiet window answers the highlighted row

- **GIVEN** an approval has been open past its quiet window
- **WHEN** the user presses Down then Enter
- **THEN** the session allowance is granted
- **AND** an Enter typed before the prompt appeared grants nothing

#### Scenario: A confirmation defaults to cancel

- **GIVEN** `/undo` opens its confirmation
- **WHEN** the user presses Enter without moving the highlight
- **THEN** the undo is cancelled and the workspace is unchanged

#### Scenario: A minimum-size terminal keeps the prompt answerable

- **GIVEN** a 40×10 terminal and an edit approval
- **WHEN** the prompt renders
- **THEN** the title, target, and all three choices are visible

## MODIFIED Requirements

### Requirement: Inline informational command results

Smith SHALL render read-only local command results as attributed blocks in the
normal transcript instead of opening a blocking viewer. These blocks MUST keep
composer input available, participate in normal transcript scrolling and
follow behavior, remain bounded, and MUST NOT be sent to the provider or added
to canonical model conversation history.

#### Scenario: Status appears in the conversation

- **GIVEN** the interactive composer is available
- **WHEN** the user invokes `/status`
- **THEN** a titled status block is appended to the transcript
- **AND** the composer remains immediately available without a close step
- **AND** no provider request is issued

#### Scenario: Consecutive local results remain visible

- **GIVEN** one informational command result is already in the transcript
- **WHEN** the user invokes another informational command
- **THEN** Smith appends the new titled result after the earlier result
- **AND** does not replace, cover, or dismiss the earlier result

#### Scenario: Diff states render inline

- **WHEN** `/diff` produces a patch, empty result, non-Git outcome, binary
  notice, oversized notice, or conflict
- **THEN** Smith renders the bounded result inline with its state stated in
  text
- **AND** normal transcript scrolling remains available

#### Scenario: Interactive safety surfaces are inline prompts

- **WHEN** Smith needs command selection, tool approval, provider-spend
  confirmation, undo confirmation, or revert confirmation
- **THEN** Smith opens the corresponding inline prompt in the anchored pane with
  explicit controls, never a centered or framed modal
- **AND** informational command results themselves never require dismissal

#### Scenario: Local results do not become model context

- **GIVEN** an informational result is visible in the transcript
- **WHEN** the user sends the next provider prompt or resumes the session
- **THEN** Smith does not represent that local result as a user or assistant
  conversation message
- **AND** protected local status or patch detail is not exposed to the
  provider merely because it was displayed

### Requirement: Account usage visibility and manual switching

When the active provider has a credential pool, the TUI SHALL show which pool
member is active, offer an accessible picker listing every member with its
usage meter, cooldown state, and provenance-safe display name, and accept an
explicit manual switch to any member not currently in cooldown. A rotation
offered after limit exhaustion SHALL be presented as an inline prompt the user answers,
stating the cache cost of switching, and its outcome SHALL be recorded in the
transcript. Headless runs SHALL select their member once at session start and
keep it for the whole run, projecting the active member and the typed
exhaustion outcome through the versioned machine output without ever prompting
or rotating.

#### Scenario: Inspect pool usage

- **GIVEN** the active provider declares a two-member pool
- **WHEN** the user opens the account picker
- **THEN** both members appear in pool order with usage percentage or unknown,
  cooldown state, and which one is active
- **AND** no credential value or secret fragment is displayed

#### Scenario: Manually switch the active account

- **GIVEN** the picker is open and the second member is eligible
- **WHEN** the user selects it
- **THEN** subsequent attempts use the second member
- **AND** the sticky selection persists for future sessions
- **AND** the transcript records the manual switch

#### Scenario: Rotation is offered as an inline prompt and announced

- **GIVEN** an attempt hits limit exhaustion mid-task with an eligible member
  available
- **WHEN** the runtime offers rotation
- **THEN** an inline prompt names the outgoing and incoming members, the outgoing
  member's reset time, and warns that switching resends the turn without the
  provider-side prompt cache
- **AND** confirming it replays the attempt and writes a rotation notice to the
  transcript
- **AND** declining it writes the exhaustion outcome to the transcript instead

#### Scenario: A headless run never rotates

- **GIVEN** `smith -p` starts with a two-member pool and exhausts its member
  mid-run
- **WHEN** the attempt fails with the typed limit-exhaustion error
- **THEN** the run fails with that error and the earliest reset time
- **AND** no prompt is rendered and no member switch occurs
- **AND** machine output names the member the run started on
