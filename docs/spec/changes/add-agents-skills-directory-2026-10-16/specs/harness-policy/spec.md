## MODIFIED Requirements

### Requirement: On-disk skill discovery for the user and workspace layers

Smith SHALL discover skills from a fixed directory layout under the user
state root and its sibling `.agents` directory, and under the active
project's `.smith` and `.agents` directories. It SHALL declare each skill
into the corresponding source layer of the existing skill catalog. A skill's
identity MUST come from its directory name rather than from its file content:
a frontmatter `name` that differs from the directory SHALL be ignored, not
refused, because installers commonly prefix the directory.
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

- **GIVEN** a skill directory named `chatcut-voice` whose frontmatter declares
  `name: voice`
- **WHEN** discovery runs
- **THEN** the skill registers as `chatcut-voice`
- **AND** no skill is registered under `voice`
- **AND** no discovery problem is reported for it

#### Scenario: Nothing on disk

- **GIVEN** none of the four locations contains a `skills` directory
- **WHEN** a session is composed
- **THEN** the catalog contains exactly the built-in harness reference set
- **AND** no directory is created

## ADDED Requirements

### Requirement: Skill frontmatter accepts block-scalar descriptions

Smith SHALL accept the skill frontmatter forms common in installed skills
without adopting general YAML. A `description` value MAY be a block scalar:
`>` folds its indented lines with single spaces, `|` keeps their newlines,
and either may carry a `-` or `+` chomping indicator. A plain value MAY also
continue on indented lines, which fold. A value wrapped in matching single or
double quotes SHALL be unquoted. An indented line SHALL belong to the
preceding key and SHALL never set a top-level field. Indented lines under
keys Smith does not interpret SHALL be skipped. The empty and length checks
SHALL apply to the folded, trimmed description. Any other line that is not
`key: value` SHALL still be refused, and the directory-name rule SHALL be
unchanged.

#### Scenario: Folded description

- **GIVEN** a `SKILL.md` whose frontmatter has `description: >` followed by
  two indented lines
- **WHEN** discovery runs
- **THEN** the skill registers with the two lines joined by one space

#### Scenario: Literal description

- **GIVEN** `description: |` followed by two indented lines
- **WHEN** discovery runs
- **THEN** the skill registers with the two lines separated by a newline

#### Scenario: Nested map cannot set the description

- **GIVEN** a frontmatter with `description: Real` and a `metadata:` key
  whose indented child is `description: Fake`
- **WHEN** discovery runs
- **THEN** the skill's description is `Real`

#### Scenario: Quoted value

- **GIVEN** `description: "Review Rust code"`
- **WHEN** discovery runs
- **THEN** the description is `Review Rust code` without quotes
