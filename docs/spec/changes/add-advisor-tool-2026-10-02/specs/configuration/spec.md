## ADDED Requirements

### Requirement: Advisor profile selection

Smith SHALL accept `advisor` as a profile placement in `use`, and SHALL
select the advisor for a main profile from, in increasing precedence, the
top-level `advisor` key and the profile's own `advisor` key (inherited
through `extends`). The value SHALL be a profile name or `false`; `false`
SHALL disable the advisor for that profile. A profile SHALL NOT resolve
itself as its advisor: a top-level default naming the active profile SHALL be
skipped for that profile, and a profile-level key naming the profile itself
SHALL be rejected. A profile's own advisor selection SHALL be ignored while it
serves as an advisor, because an advisor request carries no tools. Smith SHALL
reject, before credential or provider construction, an advisor name that does
not exist or names a profile whose `use` lacks `advisor`. `smith config
explain advisor` SHALL report the winner and every overridden source.

#### Scenario: Top-level default

- **GIVEN** top-level `advisor = "sol"` and profile `sol` with
  `use = ["main", "child", "advisor"]`
- **WHEN** profile `code` starts without its own `advisor` key
- **THEN** `code` resolves advisor `sol`

#### Scenario: Profile disables the advisor

- **GIVEN** top-level `advisor = "sol"`
- **AND** profile `glm` sets `advisor = false`
- **WHEN** profile `glm` starts
- **THEN** no advisor is resolved

#### Scenario: Target not placed as an advisor

- **GIVEN** `advisor = "plan"` and profile `plan` with `use = ["main"]`
- **WHEN** configuration loads
- **THEN** Smith reports an error naming the `advisor` key and the missing
  `advisor` placement
- **AND** no credential is read

#### Scenario: Advisor profile used as the main profile

- **GIVEN** top-level `advisor = "sol"` and profile `sol` with
  `use = ["main", "child", "advisor"]`
- **WHEN** profile `sol` starts as the main profile
- **THEN** configuration loads and `sol` has no advisor
- **AND** profile `code` still resolves advisor `sol`

#### Scenario: Unknown advisor

- **GIVEN** `advisor = "missing"`
- **WHEN** configuration loads
- **THEN** Smith reports an error listing the profiles placed as advisors
