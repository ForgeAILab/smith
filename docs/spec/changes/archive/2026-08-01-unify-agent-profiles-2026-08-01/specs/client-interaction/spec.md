## ADDED Requirements

### Requirement: Unified profile selection surfaces

Smith SHALL present one local profile inventory with clear main, child, or both
placement labels. Startup and `/profile` SHALL select a main-enabled profile;
`@name <task>` SHALL select a child-enabled profile; retained child IDs MUST
remain visibly distinct from profile-based spawn choices.

#### Scenario: Switch the main agent profile
- **GIVEN** Smith is idle and a main-enabled profile is locally valid
- **WHEN** the user selects it with `/profile`
- **THEN** Smith previews or applies one atomic safe-boundary rebuild with the
  profile's behavior and runtime summary
- **AND** status identifies the active profile, posture, provider/model, and
  provenance without exposing raw instructions or secrets

#### Scenario: Select a child-enabled profile
- **GIVEN** the `@` picker contains a child-enabled profile and a retained child
  with a different stable identity
- **WHEN** the user filters or selects an entry
- **THEN** the picker labels the profile as a new child preset and the retained
  identity as a follow-up target
- **AND** it never interprets one as the other

#### Scenario: Profile is unavailable for the requested placement
- **GIVEN** a profile is valid only for `main`
- **WHEN** the user attempts to invoke it as a child
- **THEN** Smith fails locally with its placement and available alternatives
- **AND** preserves the draft before provider spend or child allocation

### Requirement: Main profile cycle order

Smith SHALL replace root-mode cycling with a validated order of main-enabled
profiles. Cycling MUST occur only while idle with an empty composer and no
overlay, and each change MUST use the same safe-boundary profile application
as `/profile`.

#### Scenario: Cycle configured profiles
- **GIVEN** `profile_order` contains distinct valid main-enabled profiles
- **WHEN** the user cycles from an empty idle composer
- **THEN** Smith selects the next profile in declared order
- **AND** atomically updates prompt, posture, provider/model, limits, and status

#### Scenario: Ordered profile is child-only
- **GIVEN** `profile_order` names a profile not enabled for main use
- **WHEN** configuration is resolved
- **THEN** Smith rejects the order with the profile declaration source
- **AND** does not create a partially selectable cycle
