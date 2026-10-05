## ADDED Requirements

### Requirement: File-backed project memory store

Smith SHALL store project memory as owner-only Markdown files under
`~/.smith/memory/<project-id>/`, keyed by the same project identity as
sessions. Each memory MUST be one file with `name`, `description`, and `type`
frontmatter, where `type` is one of `user`, `feedback`, `project`, or
`reference`. Smith SHALL generate `MEMORY.md` from that frontmatter, beginning
with the workspace path, and MUST NOT require the user to maintain it.

#### Scenario: Memory written in one session exists in the store

- **GIVEN** the agent writes a memory named `release-pipeline`
- **WHEN** the write completes
- **THEN** `release-pipeline.md` exists in the project's memory directory with
  owner-only permissions
- **AND** `MEMORY.md` lists it with its description

#### Scenario: Another project does not see it

- **GIVEN** a memory saved in project A
- **WHEN** a session starts in project B
- **THEN** project B's index does not contain it

#### Scenario: Unsafe name is refused

- **WHEN** the agent writes a memory named `../config`
- **THEN** the write fails and no file outside the memory directory changes

### Requirement: Memory index enters context once per session

At session start, resume, and host rebuild, Smith SHALL snapshot the generated
index into the runtime memory lane within the runtime and Smith bounds. The
snapshot MUST NOT change during a session, so memories written mid-session
appear from the next session. When the index exceeds the bounds, the snapshot
MUST end with a line stating how many entries were omitted. Memory content
MUST NOT be copied into canonical history or audit metadata.

#### Scenario: Mid-session write keeps the prompt stable

- **GIVEN** a running session whose first request carried the memory index
- **WHEN** the agent writes a new memory and the next request is built
- **THEN** the memory records in that request are unchanged

#### Scenario: Next session sees the new memory

- **GIVEN** a memory written in an earlier session
- **WHEN** a new session starts in the same project
- **THEN** the index in its first request lists that memory

### Requirement: Memory tool

Smith SHALL provide a `memory` tool with `list`, `read`, `write`, and `delete`
that addresses memories only by safe single-component names. The main agent in
the `build` posture MUST be offered all four without an approval prompt;
`plan` and `review` postures and child agents MUST be offered `list` and
`read` only. A `write` whose content contains a registered secret value MUST
be refused.

#### Scenario: Child agent cannot write memory

- **GIVEN** a delegated child agent
- **WHEN** its tool view is composed
- **THEN** the `memory` tool offers only `list` and `read`

#### Scenario: Secret content is refused

- **GIVEN** a registered credential value
- **WHEN** the agent writes a memory containing it
- **THEN** the write fails without creating a file
