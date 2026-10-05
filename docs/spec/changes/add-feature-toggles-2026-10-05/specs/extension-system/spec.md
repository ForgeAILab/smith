## ADDED Requirements

### Requirement: Built-in features are recorded as modules

Each enabled built-in feature SHALL be recorded in composition evidence as a
module with `BuiltIn` provenance and its contributions, the same record shape
used for MCP servers and future extensions. A disabled feature MUST contribute
no module. A module record MUST NOT grant any capability its feature did not
already have.

#### Scenario: Disabled feature leaves no module

- **GIVEN** `image-generation` is off
- **WHEN** the runtime composition resolves
- **THEN** no `smith/image-generation` module is recorded
- **AND** `generate_image` is not registered
