## ADDED Requirements

### Requirement: Explicit allow-all shorthand

Smith SHALL accept valueless `--yolo` as an explicit invocation-level alias
for `--approval allow-all`. The alias MUST pass through the same typed approval
selection and runtime policy as the long form, MUST NOT create a distinct
approval mode, and MUST NOT widen the selected profile's tool or permission
set.

#### Scenario: Trusted run uses the shorthand

- **GIVEN** a selected build profile exposes a prepared mutating tool
- **WHEN** the user explicitly starts Smith with `--yolo`
- **THEN** Smith resolves the invocation approval mode as `allow-all`
- **AND** applies the same central authorization and execution path as
  `--approval allow-all`

#### Scenario: Plan remains read-only

- **GIVEN** the selected plan profile removes edit and shell capabilities
- **WHEN** the user explicitly starts Smith with `--yolo`
- **THEN** the plan profile still cannot request or execute edit or shell
- **AND** approval policy does not restore any removed capability

#### Scenario: Approval spellings conflict

- **WHEN** one invocation supplies both `--yolo` and `--approval`, repeats
  `--yolo`, or assigns a value to `--yolo`
- **THEN** Smith rejects the invocation before runtime construction
- **AND** does not silently choose an approval policy by argument order
