## ADDED Requirements

### Requirement: Bounded root agent modes

Smith SHALL provide a small host-owned registry of root agent modes whose
prompt policy and capability posture can only narrow the resolved run. Mode
selection MUST NOT change provider, model, credentials, trust, approval, or
authoritative permission policy implicitly.

#### Scenario: Select plan mode
- **GIVEN** the current run permits editing
- **WHEN** the user selects the built-in `plan` mode
- **THEN** Smith rebuilds the safe-boundary view without mutation abilities
- **AND** a provider-generated edit or shell-write call fails closed

#### Scenario: Repository defines a privileged mode
- **GIVEN** project configuration claims a mode grants write or shell authority
- **WHEN** Smith resolves modes
- **THEN** it rejects or narrows that claim under project-trust policy
- **AND** repository text cannot authorize a side effect

### Requirement: Explicit user-invoked child presets

Registered `@agent` references SHALL map to bounded host-controlled depth-one
child presets. Before dispatch Smith MUST show inherited provider/model,
workspace posture, limits, expected result, and provider-spend confirmation;
the preset cannot widen parent authority.

#### Scenario: Invoke a read-only reviewer
- **GIVEN** the user submits `@review` with a bounded task
- **WHEN** they confirm provider spend
- **THEN** Smith creates one attributed read-only child on the parent model
- **AND** the child has no write, shell-mutation, or child-management ability

#### Scenario: Child preset attempts nesting
- **GIVEN** a direct child returns a call shaped like delegation
- **WHEN** Smith authorizes it
- **THEN** the existing depth-one rule rejects the call
- **AND** the new UX creates no grandchild path

### Requirement: Child timeline navigation

Smith SHALL expose current and completed direct children through `/agent` and
`/timeline` with stable previous, next, and parent navigation. Child inspection
MUST be a temporary read-only view and MUST NOT move persistent input focus
away from the root composer.

#### Scenario: Inspect consecutive child results
- **GIVEN** two attributed children completed
- **WHEN** the user opens one and requests next
- **THEN** Smith displays the next child's bounded lifecycle and result
- **AND** returning to parent restores the same composer draft and scroll state

#### Scenario: No child is selected
- **GIVEN** no direct child exists
- **WHEN** the user opens child navigation
- **THEN** Smith reports the empty state locally with available preset hints
- **AND** does not fabricate a session or provider request
