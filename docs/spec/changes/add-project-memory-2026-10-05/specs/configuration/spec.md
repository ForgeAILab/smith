## ADDED Requirements

### Requirement: Memory setting is owner-controlled

Smith SHALL resolve `memory.enabled`, defaulting to true, from the user file,
environment, or command line. A project or project-local file that sets it
MUST fail resolution naming the key. When off, Smith MUST contribute no memory
index and register no `memory` tool, and MUST NOT delete memory files.

#### Scenario: Project file cannot set memory

- **GIVEN** a project file containing `memory.enabled = true`
- **WHEN** configuration resolves
- **THEN** resolution fails naming the project-layer key

#### Scenario: Memory turned off keeps files

- **GIVEN** saved memories and `memory.enabled = false` in the user file
- **WHEN** a session starts
- **THEN** no memory index or `memory` tool is present
- **AND** the memory files still exist
