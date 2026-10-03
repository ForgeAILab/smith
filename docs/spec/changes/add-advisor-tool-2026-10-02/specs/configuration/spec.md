## ADDED Requirements

### Requirement: Advisor profile selection

Smith SHALL accept `advisor` as a profile placement in `use`, and SHALL
select the advisor for a main profile from, in increasing precedence, the
top-level `advisor` key and the profile's own `advisor` key (inherited
through `extends`). The value SHALL be a profile name or `false`; `false`
SHALL disable the advisor for that profile. Smith SHALL reject, before
credential or provider construction, an advisor name that does not exist,
names a profile whose `use` lacks `advisor`, or names a profile that itself
resolves an advisor. `smith config explain advisor` SHALL report the winner
and every overridden source.

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

#### Scenario: Unknown advisor

- **GIVEN** `advisor = "missing"`
- **WHEN** configuration loads
- **THEN** Smith reports an error listing the profiles placed as advisors
