## ADDED Requirements

### Requirement: Consequential prompts require deliberate input

The TUI SHALL NOT resolve an approval, trust, rotation, or recovery prompt
from keystrokes the user was already entering when the prompt appeared. The
composer draft MUST be preserved across the prompt, and a session-wide grant
MUST NOT result from input typed before the prompt was visible.

#### Scenario: An approval arrives while the user is typing

- **GIVEN** the user is typing a steering message during an active turn
- **WHEN** an approval appears and further characters, including a decision
  letter, arrive without a pause
- **THEN** Smith records no decision
- **AND** the prompt stays open with its controls visible
- **AND** the draft contains the text typed before the prompt appeared

#### Scenario: A deliberate decision

- **GIVEN** an approval has been visible for at least 500 ms and no key has
  arrived during that window
- **WHEN** the user presses a decision key
- **THEN** Smith records exactly that decision for exactly that prompt

### Requirement: Cancelling a command picker restores the composer

The composer SHALL be left empty when the user cancels a picker that a slash
command opened. The cancelled command text MUST NOT remain as a draft prefix
for later input.

#### Scenario: Model picker is cancelled

- **GIVEN** the user submitted `/model` and the model picker is open
- **WHEN** the user presses Escape
- **THEN** the picker closes and the composer is empty
- **AND** submitting `/status` next runs `/status`

### Requirement: A confirmation states its own action

Every confirmation dialog SHALL name the action it will perform in its body
and in its control hints. Smith MUST NOT reuse another action's wording.

#### Scenario: Trusting an MCP server

- **GIVEN** the user invokes `/mcp trust NAME`
- **WHEN** the confirmation is shown
- **THEN** the body describes trusting that server and what it permits
- **AND** no text refers to a reverse patch, undo, or revert

#### Scenario: Redo

- **GIVEN** the user invokes `/redo`
- **WHEN** the confirmation is shown
- **THEN** the affirmative control reads as applying the redo

### Requirement: Prepared action text keeps its structure

The approval surface SHALL render a multi-line prepared action with its line
breaks intact. Adjacent lines MUST NOT be joined without a separator.

#### Scenario: Shell action with an access note

- **GIVEN** a prepared shell action whose text is the command followed by a
  host-access note on a second line
- **WHEN** the approval is rendered
- **THEN** the command and the note appear on separate lines

## MODIFIED Requirements

### Requirement: Prepared local shell shortcut

Input beginning with one non-whitespace `!` SHALL execute as a local shell
action only through Smith's canonical prepared tool executor. It MUST use the
same schema validation, exact workspace, broad permission bound, deadline,
cancellation, scheduling, output bounding, artifacts, and checkpoint semantics
as a model-requested shell call. Submitting the shortcut SHALL be the
authorization for exactly that prepared command, once; Smith MUST NOT ask the
user to approve a command the user just typed, and the submission MUST NOT
grant authority to any other call.

#### Scenario: User submits a shell shortcut
- **GIVEN** the composer contains `!cargo test`
- **WHEN** the user submits it
- **THEN** Smith runs the prepared action without an approval prompt
- **AND** renders the committed result locally without a provider request

#### Scenario: The submission authorizes only itself
- **GIVEN** the user ran `!ls` in a session whose policy is to ask
- **WHEN** the model later requests a shell call, including the same command
- **THEN** Smith applies the resolved approval policy to that call
- **AND** the earlier shortcut granted no session or target authority

#### Scenario: Literal exclamation prompt
- **GIVEN** the draft begins with `!!`
- **WHEN** the user submits it
- **THEN** Smith sends a normal user prompt beginning with one `!`
- **AND** starts no local shell action
