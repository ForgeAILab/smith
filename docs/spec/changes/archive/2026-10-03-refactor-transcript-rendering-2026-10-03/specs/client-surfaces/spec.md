## ADDED Requirements

### Requirement: The transcript renders only what changed

The interactive TUI SHALL render a transcript block again only when that
block, the width, the fold state, or the theme changed, and SHALL draw only
the rows in view. The rendered output MUST be identical to rendering every
block from scratch.

#### Scenario: A long conversation while a turn streams

- **GIVEN** a transcript of thousands of blocks and a streaming answer
- **WHEN** frames are drawn while the answer grows
- **THEN** only the streaming block is rendered again on each frame
- **AND** the screen is identical to a full render

#### Scenario: Expanding tool output

- **GIVEN** a cached transcript
- **WHEN** the user presses Ctrl+O
- **THEN** every block is rendered with the new fold state
- **AND** the screen is identical to a full render

### Requirement: Scrolling has no row ceiling

The transcript SHALL scroll to any row of a transcript of any length.
Transcript scroll offsets MUST NOT be limited to 65,535 rows.

#### Scenario: A very long transcript

- **GIVEN** a transcript taller than 65,535 rows
- **WHEN** the user scrolls to its start
- **THEN** the first row is shown

### Requirement: Drawing does not change application state

The TUI SHALL compute layout and scroll bounds in a step before drawing, and
drawing SHALL read the application without changing it.

#### Scenario: Two draws of the same state

- **GIVEN** an application state after layout was applied
- **WHEN** it is drawn twice
- **THEN** both frames are identical and the state is unchanged
