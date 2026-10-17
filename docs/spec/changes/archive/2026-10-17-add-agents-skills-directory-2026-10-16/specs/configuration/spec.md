## MODIFIED Requirements

### Requirement: Fixed skill directory layout

Smith SHALL read skills from `skills/<name>/SKILL.md` in four fixed places,
with the containing directory's name as the skill's name. For the user layer
these are beneath the user state root (`<home>/.smith`) and beneath
`<home>/.agents`. For the workspace layer they are beneath the project's
`.smith/` directory and beneath the project's `.agents/` directory. Within
one layer, the `.smith` location SHALL win over the `.agents` location for
the same name. The layout MUST be fixed rather than configurable, and Smith
MUST NOT create any of these directories.

#### Scenario: Author adds a user skill

- **GIVEN** the user creates `skills/rust-review/SKILL.md` under the user state
  root with a `description` in its frontmatter
- **WHEN** Smith next starts in any project
- **THEN** the skill is available in that session

#### Scenario: Skill installed in the shared user location

- **GIVEN** a skill installer wrote `<home>/.agents/skills/pdf/SKILL.md`
- **WHEN** Smith next starts in any project
- **THEN** skill `pdf` is available in that session from the user layer

#### Scenario: Skill installed in the shared project location

- **GIVEN** the project contains `.agents/skills/deploy/SKILL.md`
- **WHEN** a session starts in that project
- **THEN** skill `deploy` is listed in the workspace layer
- **AND** it activates only after its content is trusted

#### Scenario: Layout is not configurable

- **GIVEN** configuration that attempts to relocate the skills directory
- **WHEN** configuration is resolved
- **THEN** the setting is not recognized
- **AND** discovery still reads the fixed locations
