## MODIFIED Requirements

### Requirement: Unified agent profile declarations

Smith SHALL use named profiles as the single declarative agent-preset type for
main-agent selection and explicit direct-child creation. A profile MAY contain
bounded description and instructions, an authority-narrowing posture,
main/child availability, provider/model preferences, an inherited delegation
availability switch, and existing profile policy fields; none of those fields
may grant trust, credentials, permissions, approval, workspace scope, or host
capabilities. Omitted delegation availability MUST preserve the existing
enabled behavior for main profiles.

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
- **THEN** Smith treats it as a main-enabled build profile with delegation
  enabled
- **AND** does not expose it for child creation without an explicit child use

#### Scenario: Main profile disables delegation

- **GIVEN** a main-enabled profile declares `delegation = false`
- **WHEN** Smith resolves the effective profile through inheritance and
  precedence
- **THEN** provenance reports delegation as disabled from the winning source
- **AND** the field cannot add a child profile, permission, or authority
