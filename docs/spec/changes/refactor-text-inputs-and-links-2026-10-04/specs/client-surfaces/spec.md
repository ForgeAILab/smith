## ADDED Requirements

### Requirement: Every text field edits like the composer

Smith SHALL let every single-line text field — picker filters, setup fields
including masked API-key fields, and history search — accept the
composer's line-editing keys (Left, Right, Home, End, Ctrl+A, Ctrl+E, Ctrl+U,
Ctrl+K, Ctrl+W, Alt+B, Alt+F, Backspace, Delete) and bracketed paste, with the
cursor shown where the next character goes. A masked field MUST keep masking
every character it holds and MUST NOT reveal its contents through editing.

#### Scenario: Correcting a filter

- **GIVEN** the `/model` picker is open with the filter `glm-5.2-highspeeed`
- **WHEN** the user presses Alt+B, then Backspace
- **THEN** the filter is `glm-5.2-highspeed` with the cursor before the last
  word, and the list refilters
- **AND** Ctrl+U clears the filter

#### Scenario: Editing a masked key

- **GIVEN** the API-key field holds a pasted key with a typo in the middle
- **WHEN** the user moves left with the arrow keys and fixes the character
- **THEN** the field still shows only masking glyphs
- **AND** the corrected key is what setup stores

### Requirement: Links are clickable where the terminal supports it

When the terminal is known to support OSC 8 hyperlinks, Smith SHALL emit
assistant Markdown links and bare `http(s)` URLs in the transcript as OSC 8
hyperlinks, keeping the visible text and layout exactly as without them. When
support is unknown, Smith MUST emit no hyperlink sequences. Smith-owned text
selection and copy MUST yield the visible text only, never escape sequences.
Hyperlinks MUST NOT change cell widths, wrapping, or the frames recorded by
fixtures.

#### Scenario: A supporting terminal

- **GIVEN** the terminal identifies itself as one known to support OSC 8
- **WHEN** an answer contains `[the docs](https://example.com/docs)`
- **THEN** the label `the docs` is rendered underlined as before, followed by
  the dim `(https://example.com/docs)`, and clicking it opens the URL
- **AND** the rows and columns are identical to an unsupported terminal's

#### Scenario: An unknown terminal

- **GIVEN** no known-supporting terminal is detected, or the session runs
  inside a multiplexer that does not pass hyperlinks through
- **WHEN** the same answer renders
- **THEN** no OSC 8 sequence is written

#### Scenario: Copying a link

- **WHEN** the user drag-selects a line containing a hyperlink and copies it
- **THEN** the clipboard holds the visible characters of the line only
