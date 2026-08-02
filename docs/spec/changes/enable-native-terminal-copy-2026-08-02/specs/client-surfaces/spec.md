## ADDED Requirements

### Requirement: Native terminal text selection

Smith terminal surfaces SHALL leave pointer text selection and copying under
the terminal emulator's ownership by default. Smith MUST NOT enable global
mouse reporting merely to provide optional click or wheel behavior, and all
required interactions SHALL remain available from the keyboard.

#### Scenario: User copies visible transcript text

- **GIVEN** Smith is showing stable transcript content in an interactive
  terminal
- **WHEN** the user drags across visible text and invokes the terminal's copy
  command
- **THEN** the terminal can perform native text selection and copy
- **AND** Smith does not consume the pointer gesture as an application mouse
  event

#### Scenario: User selects outside the composer

- **GIVEN** Smith is showing footer, picker, modal, or setup text
- **WHEN** the user begins a pointer selection outside the composer
- **THEN** Smith does not intercept the initial click or drag
- **AND** selection behavior is consistent with transcript selection

#### Scenario: Keyboard operation remains complete

- **GIVEN** Smith does not enable mouse reporting
- **WHEN** the user edits the composer, scrolls the transcript, navigates a
  picker, or answers a modal
- **THEN** the documented keyboard controls provide the complete interaction
- **AND** bracketed paste continues to work independently of mouse reporting
