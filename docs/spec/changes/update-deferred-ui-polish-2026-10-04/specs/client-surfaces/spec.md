## ADDED Requirements

### Requirement: Child details read as words

The agent inspector and `/agents` SHALL name a child's workspace in the same
words the spawn row uses and SHALL say in words whether an exact resume is
available. They MUST NOT print debug-formatted values.

#### Scenario: Inspecting a read-only child

- **GIVEN** a child running in a read-only view of the project
- **WHEN** the user opens its inspector
- **THEN** the workspace reads as read only, not `ReadOnlyView`
- **AND** resumability reads as words, not `resumable false`

### Requirement: Setup previews are readable to the end

First-run setup SHALL let the user scroll a review or configuration-collision
preview taller than its frame to its last line, with the footer keys always
visible.

#### Scenario: A long merge preview at 44x16

- **GIVEN** a configuration-collision preview longer than the frame
- **WHEN** the user presses Down or PageDown
- **THEN** the preview scrolls to its last line
- **AND** the footer keys remain on screen

### Requirement: Streaming tables do not reflow

The TUI SHALL show a Markdown table that is still arriving at the end of a
streaming answer as its header and a dim note that the table is being
received, and SHALL draw it in full once it is complete. Rows already on
screen MUST NOT move while the table streams.

#### Scenario: A table arrives row by row

- **GIVEN** an answer whose last element is a table still receiving rows
- **WHEN** a later row is wider than the earlier ones
- **THEN** nothing already drawn shifts
- **AND** the complete table is drawn aligned once the table ends or the
  answer commits
