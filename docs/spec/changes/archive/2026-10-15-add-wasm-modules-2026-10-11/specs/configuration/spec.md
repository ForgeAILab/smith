## ADDED Requirements

### Requirement: WASM module capability grants are recorded with the plugin

Smith SHALL record, with a plugin's trust decision, the capabilities
granted to each of its WASM modules, and MUST grant a module no capability
beyond that record. Repository and project configuration MUST NOT grant or
widen a WASM module's capabilities. A user MAY narrow a module's grant in
the user configuration; a narrowed capability MUST be reported in module
listings.

#### Scenario: Project file tries to widen a grant

- **GIVEN** a project configuration file that names a network host for an
  installed WASM module
- **WHEN** configuration resolves
- **THEN** the module's network grant is unchanged
- **AND** the ignored setting is reported

#### Scenario: User narrows a grant

- **GIVEN** a WASM module granted workspace read and network access
- **WHEN** the user configuration removes its network capability
- **THEN** its HTTP host function fails as ungranted
- **AND** the listing shows the capability as withheld by the user
