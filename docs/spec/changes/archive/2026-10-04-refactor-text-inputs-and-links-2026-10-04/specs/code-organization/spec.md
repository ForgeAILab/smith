## ADDED Requirements

### Requirement: One single-line text input

Smith SHALL implement single-line text editing once, in `smith-tui`, and use
it for picker filters, setup fields (plain and masked), and history search.
The composer's own line operations MUST be the same implementation applied to
its current line, so a key behaves identically in every field.

#### Scenario: Changing a line-editing key

- **WHEN** a developer changes what Ctrl+W deletes
- **THEN** the change is in one place
- **AND** the composer, every picker filter, setup fields, and history search
  behave the new way

#### Scenario: One test drives every field

- **WHEN** the workspace tests run
- **THEN** one table of line-editing cases is driven through the composer, a
  picker filter, a plain setup field, a masked setup field, and history
  search, and each produces the same text and cursor
