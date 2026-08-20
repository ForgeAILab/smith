## ADDED Requirements

### Requirement: Skill visibility and trust command

Smith SHALL provide a built-in command that lists every skill in the session's
bounded index grouped by source layer, showing each skill's name, description,
and whether it can activate. The command MUST state the reason a skill cannot
activate, MUST show which entries a higher layer shadowed, MUST report every
skill-discovery problem, and MUST offer a way to grant trust to a workspace
skill awaiting confirmation.

#### Scenario: Inspect the catalog

- **GIVEN** a session with built-in skills, a user skill, and a workspace skill
- **WHEN** the user runs the skills command
- **THEN** each skill is listed under its source layer with its description
- **AND** each entry states whether it can activate

#### Scenario: A workspace skill is withheld

- **GIVEN** a project skill nobody has approved
- **WHEN** the user runs the skills command
- **THEN** the entry states that it needs approval
- **AND** names the command that would grant it

#### Scenario: A skill file could not be used

- **GIVEN** a skill directory whose `SKILL.md` is malformed
- **WHEN** the user runs the skills command
- **THEN** the problem is listed with the skill's name and the reason
- **AND** the remaining skills are still listed

#### Scenario: A higher layer shadows a name

- **GIVEN** a user skill and a built-in skill with the same name
- **WHEN** the user runs the skills command
- **THEN** both entries are shown
- **AND** the display identifies which one activates

#### Scenario: Grant trust from the command

- **GIVEN** a workspace skill awaiting confirmation
- **WHEN** the user grants trust through the command
- **THEN** Smith displays the skill's project-relative path and content
  identity before recording the decision
- **AND** the skill becomes activatable in the same session without the user
  restarting Smith

#### Scenario: Newly trusted skill joins at a safe boundary

- **GIVEN** the user granted trust to a workspace skill
- **WHEN** a turn is in progress
- **THEN** the catalog is not exchanged until the session is idle
- **AND** the session keeps its identity and transcript when it is
