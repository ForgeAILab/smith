## MODIFIED Requirements

### Requirement: Layered reasoning defaults

Smith SHALL allow profiles to declare typed reasoning enabled-state and effort
defaults, and SHALL additionally accept one invocation-scoped reasoning effort
supplied on the command line. The invocation value MUST resolve at the
command-line layer, so it outranks the selected profile's declared effort and an
environment-supplied effort, and is outranked by an explicit in-session
selection. Smith MUST validate a requested effort against the selected
provider/model controls before constructing a runtime, whichever layer supplied
it, and MUST preserve provider defaults when no Smith value is configured.

#### Scenario: Supported profile default resolves

- **GIVEN** a profile requests enabled reasoning at `high` effort
- **AND** the selected provider/model advertises that combination
- **WHEN** Smith resolves the configuration
- **THEN** the runtime policy carries the configured values and source

#### Scenario: Invocation effort outranks the selected profile

- **GIVEN** the selected profile declares a reasoning effort
- **AND** the invocation supplies a different advertised effort
- **WHEN** Smith resolves the configuration
- **THEN** the invocation value is effective
- **AND** explaining `reasoning.effort` identifies the command line as its
  source and keeps the profile value visible as an overridden entry

#### Scenario: Unsupported effort fails before runtime construction

- **GIVEN** a profile requests an effort absent from the capability snapshot
- **WHEN** Smith resolves the configuration
- **THEN** startup fails with the requested value and supported alternatives
- **AND** no credential lookup or provider request is performed

#### Scenario: Unsupported invocation effort fails the same way

- **GIVEN** the invocation supplies an effort absent from the capability
  snapshot
- **WHEN** Smith resolves the configuration
- **THEN** startup fails with the requested value and supported alternatives
- **AND** no credential lookup or provider request is performed
- **AND** the run does not fall back to a profile, environment, or provider
  value it was told to override

#### Scenario: Omitted reasoning configuration preserves provider behavior

- **GIVEN** no layer configures enabled state or effort
- **WHEN** Smith constructs the runtime
- **THEN** it preserves the provider/model default
- **AND** it does not synthesize `low`, enable reasoning, or disable reasoning

### Requirement: Compatible persisted reasoning override

Smith SHALL persist a session reasoning override additively and revalidate it
against the frozen capability snapshot during resume. Older sessions without
the field MUST preserve provider/model defaults. An invocation-scoped effort
supplied when a session resumes MUST take effect for that run in place of the
persisted effort, and MUST NOT overwrite or discard the persisted value; a
later resume without the invocation value MUST see the persisted override
unchanged. The persisted thinking state MUST be unaffected by an
invocation-scoped effort.

#### Scenario: Compatible override resumes

- **GIVEN** a saved session contains a supported thinking and effort override
- **WHEN** the session resumes against a compatible capability snapshot
- **THEN** the override remains effective and source-labelled

#### Scenario: Invocation effort shadows the persisted override

- **GIVEN** a saved session carries an effort override
- **AND** the session is resumed with a different advertised invocation effort
- **WHEN** Smith resolves the resumed configuration
- **THEN** the run uses the invocation effort
- **AND** the saved session's persisted thinking state remains effective

#### Scenario: Shadowed override survives the run

- **GIVEN** a session was resumed with an invocation effort shadowing its
  persisted effort
- **WHEN** the session is saved and later resumed without an invocation effort
- **THEN** the persisted effort is effective again with its original value
- **AND** it was never rewritten to the invocation value

#### Scenario: Legacy session has no override

- **GIVEN** a saved session predates the reasoning override field
- **WHEN** the session resumes
- **THEN** deserialization succeeds
- **AND** the provider/model default remains effective
