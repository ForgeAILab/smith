## ADDED Requirements

### Requirement: Recovery patches show only what changed

Smith SHALL preview undo, redo, and revert as line diffs grouped in hunks
with three lines of context, naming each file relative to the project when
it is inside the project. The preview MUST be the same text whose
fingerprint is journaled for that decision.

#### Scenario: Undoing a small addition

- **GIVEN** a turn that added a four-line function to a file
- **WHEN** the user opens `/undo`
- **THEN** the patch shows the four removed lines with up to three lines of
  context around them, not every line of the file
- **AND** the file is named by its project-relative path
- **AND** applying the undo checks the fingerprint of that same text
