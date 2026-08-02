## ADDED Requirements

### Requirement: Agent profile instruction composition

Smith SHALL compose the selected profile's identity, posture semantics, and
optional bounded instructions as one independently revisioned developer-
instruction fragment after stable host policy. Normal profile configuration
MUST NOT replace the Smith system identity or change fragment priority, kind,
source, cache class, tool authority, trust, or approval policy.

#### Scenario: Main profile supplies instructions
- **GIVEN** the selected main profile contains bounded UTF-8 instructions
- **WHEN** Smith plans provider context
- **THEN** the instructions appear in a dedicated attributed profile fragment
- **AND** stable Smith security/workflow fragments remain independently present

#### Scenario: Profile asks for unauthorized mutation
- **GIVEN** a plan or review profile instructs the model to modify the workspace
- **WHEN** the model requests a mutating capability
- **THEN** the effective read-only ability view rejects the request
- **AND** prompt text is not interpreted as permission or approval

#### Scenario: Direct embedder uses a complete override
- **GIVEN** a direct embedder deliberately supplies the existing complete
  system-prompt override
- **WHEN** the runtime is composed
- **THEN** the override retains its explicit replacement semantics
- **AND** ordinary configuration profiles cannot access that replacement path
