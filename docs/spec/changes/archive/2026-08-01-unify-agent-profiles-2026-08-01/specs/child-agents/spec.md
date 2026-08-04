## ADDED Requirements

### Requirement: Profile-based direct child composition

Smith SHALL create an explicit direct child from a child-enabled profile using
the same typed profile resolution and standard runtime composition boundaries
used for the main agent. Before dispatch it MUST show the selected profile,
bounded instruction summary, provider/model, effective limits, read-only
workspace posture, and provider-spend confirmation.

#### Scenario: Invoke a child on the parent's model
- **GIVEN** a child-enabled review profile resolves to the parent's
  provider/model
- **WHEN** the user submits and confirms `@review <task>`
- **THEN** Smith creates one attributed child with that profile's instructions
  and effective limits
- **AND** the child remains depth-one and read-only

#### Scenario: Invoke a child on another declared model
- **GIVEN** a child-enabled profile selects another fully configured declared
  provider/model
- **WHEN** the user confirms the displayed model and spend
- **THEN** Smith runs normal credential, catalog, context, and runtime preflight
  before allocating or dispatching the child
- **AND** no partial child or hidden fallback to the parent model is created

#### Scenario: Profile requests broader child authority
- **GIVEN** a child-enabled profile has build posture or a setting that would
  widen the host child ceiling
- **WHEN** Smith computes the effective child policy
- **THEN** the result remains the intersection of parent authority, the
  depth-one read-only child ceiling, and profile posture
- **AND** any incompatible value is rejected or source-explainably narrowed

### Requirement: Durable child profile identity

Smith SHALL include the effective profile name, revision, placement, provider/
model selection, and authority posture in durable child policy compatibility
evidence. Follow-up and resume MUST retain the same profile identity and MUST
fail closed when exact compatible composition is unavailable.

#### Scenario: Resume with an unchanged profile
- **GIVEN** an interrupted child has a durable checkpoint and its effective
  profile revision remains available
- **WHEN** the user explicitly resumes that child
- **THEN** Smith continues the unfinished turn with the same profile identity
- **AND** does not consume a new task slot or repeat committed work

#### Scenario: Profile changed before resume
- **GIVEN** the selected profile's effective instructions, model, or posture
  changed after checkpoint creation
- **WHEN** the user requests resume
- **THEN** Smith reports an incompatible policy fingerprint
- **AND** does not silently spawn a replacement or run under mixed revisions
