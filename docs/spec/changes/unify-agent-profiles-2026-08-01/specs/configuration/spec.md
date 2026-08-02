## ADDED Requirements

### Requirement: Unified agent profile declarations

Smith SHALL use named profiles as the single declarative agent-preset type for
main-agent selection and explicit direct-child creation. A profile MAY contain
bounded description and instructions, an authority-narrowing posture,
main/child availability, provider/model preferences, and existing profile
policy fields; none of those fields may grant trust, credentials, permissions,
approval, workspace scope, or host capabilities.

#### Scenario: Use one profile on the main agent
- **GIVEN** a valid profile is available for `main`
- **WHEN** startup, `--profile`, or `/profile` selects it
- **THEN** Smith resolves its instructions, posture, provider/model, and limits
  through the normal typed profile-precedence layer
- **AND** applies the effective profile atomically at a safe runtime boundary

#### Scenario: Expose one profile to both placements
- **GIVEN** a valid profile declares `use = ["main", "child"]`
- **WHEN** Smith builds its local profile inventory
- **THEN** the same named declaration is eligible for main selection and
  explicit child invocation
- **AND** each placement independently intersects the profile with host policy

#### Scenario: Existing profile omits availability
- **GIVEN** a pre-change profile contains only existing runtime fields
- **WHEN** the transition release resolves it
- **THEN** Smith treats it as a main-enabled build profile
- **AND** does not expose it for child creation without an explicit child use

### Requirement: Deterministic agent profile inheritance

Smith SHALL allow a profile to extend at most one named profile and SHALL
resolve the effective fields with bounded, acyclic, source-explainable
inheritance before provider, credential, session, or terminal I/O. Child fields
replace inherited scalar or section fields, and instruction bodies MUST NOT be
implicitly concatenated.

#### Scenario: Reuse a provider and model baseline
- **GIVEN** `plan` extends a valid `work` profile and overrides posture and
  instructions
- **WHEN** Smith resolves `plan`
- **THEN** it inherits the provider/model and other unmodified fields from
  `work`
- **AND** provenance identifies the winning source for every effective field

#### Scenario: Inheritance contains a cycle
- **GIVEN** two or more profiles form an inheritance cycle
- **WHEN** configuration preflight resolves any affected profile
- **THEN** Smith fails before credential, provider, session, or terminal I/O
- **AND** the diagnostic identifies the bounded cycle and source declarations

### Requirement: Legacy agent configuration migration

Smith SHALL accept existing root-mode and child-preset declarations through an
explicit one-release compatibility adapter, emit source-explainable migration
guidance, and fail closed when a legacy declaration conflicts with a new
profile of the same effective name. Smith MUST NOT silently select a winner or
change a legacy run profile into a child-enabled profile.

#### Scenario: Load an existing child preset
- **GIVEN** configuration declares a valid read-only `[child_agents.inspect]`
- **WHEN** the transition release builds the profile inventory
- **THEN** it exposes an equivalent deprecated child-only preset
- **AND** reports the replacement profile shape without changing authority

#### Scenario: Legacy and new names collide
- **GIVEN** a legacy mode or child preset conflicts with a new profile of the
  same effective name
- **WHEN** Smith resolves declarations
- **THEN** resolution fails with both source locations
- **AND** no map order, file order, or precedence rule hides the collision
