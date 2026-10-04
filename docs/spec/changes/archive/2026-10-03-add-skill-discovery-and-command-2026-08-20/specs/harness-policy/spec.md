## ADDED Requirements

### Requirement: On-disk skill discovery for the user and workspace layers

Smith SHALL discover skills from a fixed directory layout under the user state
root and under the active project, and declare each into the corresponding
source layer of the existing skill catalog. A skill's identity MUST come from
its directory name rather than from its file content, and discovery MUST NOT
change the resolution order, shadowing rules, or activation authority the
catalog already defines.

#### Scenario: User and project both supply a skill

- **GIVEN** a `SKILL.md` under the user state root's `skills/<name>/` and
  another under the project's `.smith/skills/<other>/`
- **WHEN** a session is composed
- **THEN** both appear in the bounded index with their authored descriptions
- **AND** each is attributed to the user and workspace layer respectively

#### Scenario: Frontmatter cannot rename a skill

- **GIVEN** a skill directory named `rust-review` whose frontmatter declares a
  different `name`
- **WHEN** discovery runs
- **THEN** the skill is not registered
- **AND** the mismatch is reported as a named discovery problem
- **AND** no skill is registered under the frontmatter's name

#### Scenario: Nothing on disk

- **GIVEN** neither the user state root nor the project contains a `skills`
  directory
- **WHEN** a session is composed
- **THEN** the catalog contains exactly the built-in harness reference set
- **AND** no directory is created

### Requirement: Discovered skill bodies are pinned to the discovered bytes

Smith SHALL digest each discovered `SKILL.md` at discovery and declare the
skill pinned to that digest, in every layer. Activation MUST load only those
exact bytes and MUST fail closed when the file no longer matches, and building
the bounded index MUST still open no skill body.

#### Scenario: Body is rewritten after the session starts

- **GIVEN** a discovered and activatable skill
- **WHEN** its `SKILL.md` is rewritten and the agent then activates the skill
- **THEN** activation fails
- **AND** the rewritten text does not enter privileged context

#### Scenario: Indexing stays lazy after discovery

- **GIVEN** discovered user and workspace skills
- **WHEN** the session's bounded index is constructed
- **THEN** each entry exposes name, description, source layer, and estimated
  cost without any body being read at index time

### Requirement: Workspace skill bodies require hash-bound project trust

Smith SHALL resolve each discovered workspace skill's trust status against the
project before offering it to the catalog, binding the decision to the exact
file content. A workspace `SKILL.md` that resolves outside the project root
MUST be refused rather than covered by the project's trust, and an untrusted,
changed, or refused declaration MUST remain visible as bounded index metadata
without activating or shadowing a lower layer.

#### Scenario: Project ships an unreviewed skill

- **GIVEN** a project supplies a skill nobody has approved
- **WHEN** a session is composed
- **THEN** the skill appears in the index as not activatable with its reason
- **AND** its body never enters privileged context
- **AND** a user skill of the same name still activates

#### Scenario: Approved skill is edited by a later commit

- **GIVEN** a workspace skill approved at one content digest
- **WHEN** its `SKILL.md` changes and a session is composed
- **THEN** the earlier decision authorizes nothing
- **AND** the skill is reported as changed rather than silently re-approved

#### Scenario: Project skill is a symlink out of the project

- **GIVEN** a project `SKILL.md` that canonicalizes outside the project root
- **WHEN** discovery runs
- **THEN** the skill is not registered
- **AND** the escape is reported as a discovery problem

#### Scenario: Granting trust admits the exact reviewed content

- **GIVEN** an untrusted workspace skill
- **WHEN** the user approves it and the catalog is resolved again
- **THEN** the skill activates
- **AND** the digest the decision recorded is the digest activation enforces

### Requirement: Skill discovery fails closed per skill and reports why

A skill file Smith cannot use SHALL be excluded from the catalog and reported
as a named discovery problem carried alongside the index. Discovery MUST be
bounded in the number of skills, the size of a body, and the size of an indexed
description, and MUST NOT fail session start because a skill file is malformed.

#### Scenario: One malformed skill among several

- **GIVEN** three skill directories, one of whose `SKILL.md` has no
  `description`
- **WHEN** a session is composed
- **THEN** the session starts
- **AND** the two well-formed skills are indexed
- **AND** the malformed one is reported by name with its reason

#### Scenario: Non-skill content beside the skills

- **GIVEN** a `skills` directory containing a loose file and a directory with
  no `SKILL.md`
- **WHEN** discovery runs
- **THEN** neither is registered
- **AND** neither is reported as a problem

#### Scenario: A bound is exceeded

- **GIVEN** a skill body larger than the read bound, or more skill directories
  in one layer than the count bound
- **WHEN** discovery runs
- **THEN** the excluded skills are not indexed
- **AND** the exclusion is reported rather than silently truncated
