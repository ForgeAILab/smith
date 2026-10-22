## ADDED Requirements

### Requirement: Explicit user skill activation uses canonical admission

Smith SHALL admit an explicitly selected skill through its existing resolved
skill catalog, authorized capability scope, activation policy, pinned content
verification, and context budgets before the next provider request. It MUST
resolve the effective winning declaration, preserve its provenance, and grant
no additional tools, permissions, approvals, or credentials. Activation SHALL
use the normal live activation epoch and checkpoint lifecycle.

#### Scenario: Select the winning declaration

- **GIVEN** multiple layers declare the same skill name
- **WHEN** the user explicitly activates the name
- **THEN** the existing highest admitted declaration is selected
- **AND** withheld higher layers do not shadow an admitted lower layer
- **AND** the shown source matches the activated descriptor

#### Scenario: Complete admission before changing the session

- **GIVEN** a current usable provider-backed session
- **WHEN** a skill is explicitly selected
- **THEN** Smith verifies scope, trust, content identity, and budget before
  replacing that session's execution view
- **AND** its identity, transcript, workspace, and approval policy are retained
- **AND** a failed admission leaves the previous session usable

#### Scenario: A skill changes after discovery

- **GIVEN** a skill was indexed at one digest and its file is later rewritten
- **WHEN** the user explicitly invokes it
- **THEN** activation refuses the changed content
- **AND** neither old trust nor the slash selection approves the new bytes
- **AND** the rewritten body does not enter model context

#### Scenario: The profile denies the selected skill

- **GIVEN** a skill's descriptor is indexed but the profile or session denies
  its capability id
- **WHEN** the user invokes its shortcut
- **THEN** the denial is reported rather than silently skipping the pin
- **AND** no instructions are admitted through an alternative context lane

#### Scenario: The selected skill exceeds budget

- **GIVEN** the selected skill cannot fit normal activation/context admission
- **WHEN** the user invokes it
- **THEN** Smith reports the budget refusal before any provider call
- **AND** no truncated body is installed and the previous session is retained

#### Scenario: Repeat or resume a successful activation

- **GIVEN** a skill is active in the session's canonical activation state
- **WHEN** the user selects it again
- **THEN** no duplicate instruction fragment is appended
- **WHEN** the session resumes or is reconfigured
- **THEN** the existing activation lifecycle revalidates scope and content
  identities before those instructions can be used
