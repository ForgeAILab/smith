## ADDED Requirements

### Requirement: The composer inserts line breaks in every terminal

The composer SHALL insert a newline at the cursor, without submitting, for
Ctrl+J in every terminal, and for Shift+Enter and Alt+Enter where the
terminal reports them. Typing `\` and then Enter SHALL replace the backslash
with a newline. In the command palette, Ctrl+J, Shift+Enter and Alt+Enter
SHALL also insert a newline and MUST NOT run a command. Submitting a
multi-line draft MUST send its newlines intact. While an idle draft is
non-empty and no overlay is open, the hint row SHALL name a newline key. The
shortcuts panel and `/help` SHALL list every newline key. When a draft is
taller than the composer, the composer SHALL scroll so that the cursor row
stays visible.

#### Scenario: Ctrl+J in a terminal without keyboard enhancements

- **GIVEN** a terminal that sends Shift+Enter as a plain Enter
- **AND** the composer holds `first`
- **WHEN** the user presses Ctrl+J, types `second`, and presses Enter
- **THEN** Smith submits `first\nsecond` as one message

#### Scenario: Shift+Enter in a kitty-protocol terminal

- **GIVEN** a terminal that answers the keyboard-protocol query, such as
  Ghostty (and cmux) or kitty
- **WHEN** the user presses Shift+Enter in a non-empty draft
- **THEN** a newline is inserted at the cursor and nothing is submitted

#### Scenario: A newline key in the command palette

- **GIVEN** the composer holds `/status` with the command palette open
- **WHEN** the user presses Ctrl+J, Shift+Enter or Alt+Enter
- **THEN** the draft becomes `/status` followed by a newline
- **AND** no command runs and the palette stays open

#### Scenario: Newline keys are discoverable

- **GIVEN** an idle composer with a non-empty draft and no overlay
- **WHEN** Smith renders the hint row
- **THEN** it reads `ctrl+j for newline`
- **AND** the shortcuts panel lists Shift+Enter, Alt+Enter, Ctrl+J and
  `\` then Enter

#### Scenario: A tall draft keeps the cursor in view

- **GIVEN** a twelve-line draft in a composer limited to eight rows
- **WHEN** the cursor is on the last line
- **THEN** the composer shows the last eight rows with the cursor on the last
  one
- **AND** pressing Up moves within the draft and scrolls it, without recalling
  history

### Requirement: Keyboard disambiguation never outlives the screen

When the terminal answers the kitty keyboard-protocol query, Smith SHALL
push `DISAMBIGUATE_ESCAPE_CODES` after it enters the alternate screen. It
MUST NOT request release or repeat events. Smith SHALL query at most once per
process, at the first entry, before any terminal input reader exists. Every
exit path that leaves the alternate screen SHALL pop the flags before it
leaves: normal quit, error, panic, the exit-deadline backstop, and a second
stop signal. Smith MUST pop only flags it pushed, and at most once. With
disambiguation on, Ctrl+[, Ctrl+I and Ctrl+M SHALL keep their legacy meanings
of Esc, Tab and Enter, and every other existing binding SHALL behave as it
does without the protocol.

#### Scenario: Quitting restores the shell's keyboard mode

- **GIVEN** Smith pushed keyboard flags in a kitty-protocol terminal
- **WHEN** the user quits
- **THEN** Smith writes the pop before it leaves the alternate screen
- **AND** the shell receives Shift+Enter in its own encoding again

#### Scenario: A terminal without the protocol

- **GIVEN** a terminal that answers the device-attributes query but not the
  keyboard query
- **WHEN** Smith enters and leaves the alternate screen
- **THEN** Smith writes neither a push nor a pop

#### Scenario: Restoring twice pops once

- **GIVEN** Smith pushed keyboard flags
- **WHEN** a panic hook and then the terminal guard both restore the terminal
- **THEN** exactly one pop is written
