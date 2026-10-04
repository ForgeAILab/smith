## ADDED Requirements

### Requirement: Fixed skill directory layout

Smith SHALL read skills from `skills/<name>/SKILL.md` beneath the user state
root and beneath the project's `.smith/` directory, with the containing
directory's name as the skill's name. The layout MUST be fixed rather than
configurable, and Smith MUST NOT create either directory.

#### Scenario: Author adds a user skill

- **GIVEN** the user creates `skills/rust-review/SKILL.md` under the user state
  root with a `description` in its frontmatter
- **WHEN** Smith next starts in any project
- **THEN** the skill is available in that session

#### Scenario: Layout is not configurable

- **GIVEN** configuration that attempts to relocate the skills directory
- **WHEN** configuration is resolved
- **THEN** the setting is not recognized
- **AND** discovery still reads the fixed locations

### Requirement: Project skills are hash-bound executable trust

Smith's project-trust model SHALL cover project-supplied skill instructions as
a distinct kind of executable authority, decided per file content and persisted
alongside the existing kinds. Recording a decision MUST show the artifact's
project-relative path and content identity before it is recorded.

#### Scenario: Decision binds path and content together

- **GIVEN** a decision recorded for a project skill
- **WHEN** the same content appears at a different project path, or different
  content appears at the same path
- **THEN** the earlier decision does not authorize it

#### Scenario: Existing trust files remain readable

- **GIVEN** a persisted trust file written before skills were trustable
- **WHEN** Smith reads it
- **THEN** existing decisions load unchanged
- **AND** project skills are undecided rather than approved
