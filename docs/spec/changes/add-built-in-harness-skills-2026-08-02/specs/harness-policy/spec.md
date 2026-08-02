## ADDED Requirements

### Requirement: Built-in harness reference skills

Smith SHALL ship built-in skills that document the harness itself, initially
covering configuration and agent profiles, the headless protocol, persistence
and recovery, and the security model. Each skill body MUST be embedded at
compile time from the shipped reference document it mirrors, and each skill
MUST be indexed with an authored name, description, and keywords so
descriptor-first retrieval selects it without reading any body.

#### Scenario: Agent is asked to configure Smith in a foreign workspace
- **GIVEN** a Smith agent running in a workspace that contains no Smith
  documentation
- **WHEN** the user asks it to add a profile to `.smith/config.toml`
- **THEN** the agent can activate the built-in configuration skill and
  receive the shipped configuration reference for its binary revision
- **AND** no workspace or network read is required to obtain it

#### Scenario: Embedded body matches shipped documentation
- **GIVEN** a Smith binary built from a repository revision
- **WHEN** a built-in harness reference skill is activated
- **THEN** its instructions are byte-identical to that revision's shipped
  reference document

#### Scenario: Descriptor resolution stays lazy
- **GIVEN** the resolved skill catalog includes the built-in reference set
- **WHEN** the catalog index is constructed for a session
- **THEN** built-in entries expose name, description, source layer, and
  estimated instruction cost without materializing any body

### Requirement: One built-in reference set across Smith hosts

The interactive TUI and headless `smith -p` SHALL expose the same built-in
harness reference skills through the shared `smith-runtime` composition path.
A direct embedder that supplies its own skill sources replaces the set
entirely and receives no implicit built-in entries.

#### Scenario: TUI and headless expose one index
- **GIVEN** the same resolved configuration
- **WHEN** a session is composed interactively and through `smith -p`
- **THEN** both sessions index an identical built-in reference skill set

#### Scenario: Embedder overrides the skill sources
- **GIVEN** a direct embedder constructs the runtime with explicit skill
  sources
- **WHEN** the catalog resolves
- **THEN** only the embedder's declarations appear

### Requirement: Built-in reference skills carry no authority

Activating a built-in harness reference skill SHALL contribute bounded
instructions only. It MUST NOT grant a tool, permission, approval,
credential, executable trust, or wider workspace, and built-in entries remain
the lowest-precedence layer that user, trusted-workspace, and session
declarations may shadow by name.

#### Scenario: User shadows a built-in reference skill
- **GIVEN** a user profile declares a skill with a built-in skill's name
- **WHEN** the catalog resolves
- **THEN** the user declaration activates in place of the built-in body
- **AND** the built-in entry remains visible in the bounded index

#### Scenario: Reference text requests wider access
- **GIVEN** an activated built-in reference body describes privileged
  operations
- **WHEN** the agent acts on that guidance
- **THEN** every action still passes the unchanged approval, trust, and
  authorization checks
