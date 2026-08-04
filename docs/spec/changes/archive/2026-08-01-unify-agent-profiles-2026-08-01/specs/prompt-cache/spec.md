## ADDED Requirements

### Requirement: Exact agent profile prompt identity

Smith SHALL derive a deterministic revision for the effective agent-profile
fragment from its resolved behavior, instructions, placement, and source
identity, and SHALL include that revision in root prompt plans and child policy
fingerprints. Smith MUST keep stable host, project-instruction, skill, memory,
and profile revisions independently attributable.

#### Scenario: Reuse an unchanged profile
- **GIVEN** two equivalent compositions resolve the same effective profile and
  instruction bytes
- **WHEN** Smith plans their prompt and policy identities
- **THEN** the profile fragment has the same exact revision
- **AND** unrelated stable Smith fragments retain their own revisions

#### Scenario: Profile instructions change
- **GIVEN** a newly constructed runtime resolves changed profile instructions
- **WHEN** Smith plans provider context
- **THEN** the profile revision and exact full prompt identity change
- **AND** Smith does not claim reuse under the prior exact identity

#### Scenario: Debug profile identity
- **GIVEN** a profile contains private or sensitive instruction text
- **WHEN** status, debug, journal, or compatibility diagnostics render it
- **THEN** they show bounded name, revision, placement, and provenance only
- **AND** do not copy the raw instruction body into canonical user history
