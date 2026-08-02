## ADDED Requirements

### Requirement: Source-explainable reasoning controls

Smith SHALL resolve reasoning presence separately from adjustable reasoning
controls. Control metadata SHALL identify switch behavior, supported efforts,
optional token-budget support, provider wire dialect, defaults, and provenance;
a boolean reasoning capability alone MUST NOT imply controllability.

#### Scenario: Boolean reasoning remains fixed

- **GIVEN** a catalog record declares only `reasoning = true`
- **WHEN** Smith resolves the model profile
- **THEN** reasoning is recorded as present but fixed
- **AND** no toggle, effort, budget, or wire dialect is inferred

#### Scenario: Rich trusted metadata declares controls

- **GIVEN** trusted metadata for an exact provider/model binding declares a
  toggle, supported effort levels, defaults, and a request dialect
- **WHEN** Smith resolves the model profile
- **THEN** the control profile retains every value and its source
- **AND** `/status` can explain where the effective control came from

### Requirement: Layered reasoning defaults

Smith SHALL allow profiles to declare typed reasoning enabled-state and effort
defaults. Smith MUST validate requested defaults against the selected provider/model controls
before constructing a runtime and MUST preserve provider defaults when no
Smith value is configured.

#### Scenario: Supported profile default resolves

- **GIVEN** a profile requests enabled reasoning at `high` effort
- **AND** the selected provider/model advertises that combination
- **WHEN** Smith resolves the configuration
- **THEN** the runtime policy carries the configured values and source

#### Scenario: Unsupported effort fails before runtime construction

- **GIVEN** a profile requests an effort absent from the capability snapshot
- **WHEN** Smith resolves the configuration
- **THEN** startup fails with the requested value and supported alternatives
- **AND** no credential lookup or provider request is performed

#### Scenario: Omitted reasoning configuration preserves provider behavior

- **GIVEN** no layer configures enabled state or effort
- **WHEN** Smith constructs the runtime
- **THEN** it preserves the provider/model default
- **AND** it does not synthesize `low`, enable reasoning, or disable reasoning

### Requirement: Compatible persisted reasoning override

Smith SHALL persist a session reasoning override additively and revalidate it
against the frozen capability snapshot during resume. Older sessions without
the field MUST preserve provider/model defaults.

#### Scenario: Compatible override resumes

- **GIVEN** a saved session contains a supported thinking and effort override
- **WHEN** the session resumes against a compatible capability snapshot
- **THEN** the override remains effective and source-labelled

#### Scenario: Legacy session has no override

- **GIVEN** a saved session predates the reasoning override field
- **WHEN** the session resumes
- **THEN** deserialization succeeds
- **AND** the provider/model default remains effective
