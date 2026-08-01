## ADDED Requirements

### Requirement: Default immutable root project instructions

Standard Smith hosts SHALL discover exactly `AGENTS.md` at the canonical
project root before runtime construction and SHALL activate a valid present
file as bounded project guidance in both interactive and headless runs. The
host SHALL capture one immutable snapshot per constructed runtime, direct
children SHALL inherit that exact snapshot, and repository guidance MUST NOT
grant authority or weaken higher-priority Smith policy.

#### Scenario: Root instructions are present

- **GIVEN** the canonical project root contains a regular non-symlinked UTF-8
  `AGENTS.md` within the configured size bound
- **WHEN** Smith constructs a standard interactive or headless runtime
- **THEN** the exact bounded snapshot is activated as project instructions
- **AND** the source and content revision are available as composition evidence
- **AND** the body is not fabricated as a canonical user message

#### Scenario: Root instructions are absent

- **GIVEN** the canonical project root contains no `AGENTS.md`
- **WHEN** Smith constructs a standard runtime
- **THEN** construction continues without a project-instruction fragment
- **AND** Smith does not invent or search parent and nested directories for one

#### Scenario: Present instructions are unsafe to load exactly

- **GIVEN** root `AGENTS.md` is a symlink, non-regular file, unreadable,
  non-UTF-8, outside the canonical root, or over 32 KiB
- **WHEN** standard host preflight evaluates it
- **THEN** startup fails with a bounded path-specific diagnostic before provider
  I/O or terminal entry
- **AND** Smith does not silently skip, truncate, or partially activate it

#### Scenario: Instructions change during an active runtime

- **GIVEN** a runtime captured one valid root instruction snapshot
- **WHEN** the file changes after construction
- **THEN** the active runtime and every direct child retain the captured
  revision
- **AND** Smith performs no automatic watch, reload, or context mutation

#### Scenario: Project guidance requests broader authority

- **GIVEN** activated `AGENTS.md` text asks for an out-of-workspace write or
  unapproved command
- **WHEN** the agent attempts the requested side effect
- **THEN** the normal prepared workspace, authorization, and approval policy
  still applies
- **AND** the repository text grants no permission or trust decision
