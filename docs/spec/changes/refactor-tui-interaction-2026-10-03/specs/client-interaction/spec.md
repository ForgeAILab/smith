## ADDED Requirements

### Requirement: One confirmation component

The interactive TUI SHALL present every confirmation that is not an approval
or questionnaire through one component with a title, an optional warning, a
body, and accept and cancel keys. The body MUST be scrollable to its end, `y`
SHALL accept, `n` and Esc SHALL cancel, and Enter MUST NOT choose either.

#### Scenario: Reviewing a long undo patch

- **GIVEN** an undo preview longer than the dialog
- **WHEN** the confirmation opens
- **THEN** the user can scroll the patch to its last line
- **AND** nothing is applied until the user presses `y`

### Requirement: Prompts are never replaced

The TUI SHALL open overlays through one policy. A prompt that needs an answer
(an approval, a questionnaire, or a confirmation) MUST NOT be replaced by any
other overlay; prompts SHALL queue in arrival order. Pickers, the palette,
history search, and the shortcuts panel SHALL close when a prompt arrives and
MUST NOT open over a prompt.

#### Scenario: A confirmation arrives during an approval

- **GIVEN** an approval is open
- **WHEN** a credential-rotation confirmation is requested
- **THEN** the approval stays open
- **AND** the confirmation opens after the approval is answered

#### Scenario: A prompt arrives while a picker is open

- **GIVEN** the model picker is open
- **WHEN** an approval arrives
- **THEN** the picker closes and the approval opens
