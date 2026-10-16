## ADDED Requirements

### Requirement: File commands are content-only contributions

Smith SHALL classify a file command as a content-only contribution that
requires no extension process and no protocol. A file command MUST contribute
prompt text only, MUST NOT receive or imply any filesystem, process, network,
credential, provider, or approval capability, and MUST be recorded with its
source layer in the same contribution records other modules use, so a later
executable extension that contributes a command appears in the same listing.

#### Scenario: File command is recorded without grants

- **GIVEN** a runnable user command
- **WHEN** the session's contributions are resolved
- **THEN** the command is recorded as a command contribution from the user
  layer
- **AND** the record carries no requested or granted capability

#### Scenario: No extension host is needed

- **GIVEN** Node is absent and no executable extension is configured
- **WHEN** the user invokes a file command
- **THEN** the command expands and runs normally
