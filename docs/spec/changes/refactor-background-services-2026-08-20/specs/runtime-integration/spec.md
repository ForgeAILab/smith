## ADDED Requirements

### Requirement: Host services have explicit runtime ownership

Smith's composition path SHALL receive host services explicitly and retain them
for the lifetime they serve. Mutable process-global installation MUST NOT be
used for background tasks or other host adapters, and direct embedders MUST be
able to construct multiple isolated Smith hosts in one process.

#### Scenario: Factory composes background-capable tools

- **GIVEN** the resolved host supplies a background task service
- **WHEN** the one Smith factory assembles built-in tools
- **THEN** every background-capable tool receives that exact service
- **AND** host exit policy and shutdown use the same owned service instance

#### Scenario: Concurrent hosts use different fakes

- **GIVEN** two deterministic tests compose hosts with different fake services
- **WHEN** their turns run concurrently
- **THEN** each fake observes only its own calls
- **AND** construction order does not select a process-wide winner
