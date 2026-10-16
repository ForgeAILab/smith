## MODIFIED Requirements

### Requirement: On-disk skill discovery for the user and workspace layers

Smith SHALL discover skills from a fixed directory layout under the user
state root and its sibling `.agents` directory, and under the active
project's `.smith` and `.agents` directories. It SHALL declare each skill
into the corresponding source layer of the existing skill catalog. A skill's
identity MUST come from its directory name rather than from its file content.
Discovery MUST NOT change the resolution order, shadowing rules, or
activation authority the catalog already defines.

When both locations of one layer hold a skill with the same name, the
`.smith` skill SHALL be declared and the `.agents` skill SHALL be reported as
a named discovery problem that names both paths. It SHALL NOT be a duplicate
declaration. A project `.agents` skill SHALL be subject to the same
containment, content pinning, hash-bound trust, and bounds as a project
`.smith` skill. The per-layer skill bound SHALL apply to each location
separately.

#### Scenario: User and project both supply a skill

- **GIVEN** a `SKILL.md` under the user state root's `skills/<name>/` and
  another under the project's `.smith/skills/<other>/`
- **WHEN** a session is composed
- **THEN** both appear in the bounded index with their authored descriptions
- **AND** each is attributed to the user and workspace layer respectively

#### Scenario: Shared locations are discovered

- **GIVEN** `<home>/.agents/skills/pdf/SKILL.md` and
  `<project>/.agents/skills/deploy/SKILL.md`
- **WHEN** a session is composed
- **THEN** `pdf` is attributed to the user layer
- **AND** `deploy` is attributed to the workspace layer under project trust

#### Scenario: Same name in both locations of a layer

- **GIVEN** `<project>/.smith/skills/review/SKILL.md` and
  `<project>/.agents/skills/review/SKILL.md`
- **WHEN** a session is composed
- **THEN** exactly one workspace `review` is declared, from `.smith`
- **AND** the `.agents` copy is reported as shadowed, naming both paths
- **AND** composition does not fail

#### Scenario: Project symlink escapes

- **GIVEN** `<project>/.agents/skills/x/SKILL.md` is a symlink to a file
  outside the project
- **WHEN** discovery runs
- **THEN** `x` is refused and reported, as for `.smith/skills`

#### Scenario: Frontmatter cannot rename a skill

- **GIVEN** a skill directory named `rust-review` whose frontmatter declares a
  different `name`
- **WHEN** discovery runs
- **THEN** the skill is not registered
- **AND** the mismatch is reported as a named discovery problem
- **AND** no skill is registered under the frontmatter's name

#### Scenario: Nothing on disk

- **GIVEN** none of the four locations contains a `skills` directory
- **WHEN** a session is composed
- **THEN** the catalog contains exactly the built-in harness reference set
- **AND** no directory is created
