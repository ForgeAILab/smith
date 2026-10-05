## ADDED Requirements

### Requirement: Memory command

The TUI SHALL provide `/memory`, which shows the project's memory directory and
lists each memory's name, type, and description from the current files. It
MUST work when memory is off, so the user can still find and review the files.

#### Scenario: User reviews project memory

- **GIVEN** two saved memories in the current project
- **WHEN** the user runs `/memory`
- **THEN** Smith shows the memory directory path
- **AND** lists both memories with their type and description
