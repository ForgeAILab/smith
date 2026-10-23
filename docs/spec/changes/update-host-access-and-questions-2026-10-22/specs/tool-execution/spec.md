## MODIFIED Requirements

### Requirement: Handle-relative workspace enforcement

Built-in filesystem tools SHALL use directory capabilities and handle-relative
operations. Project operations SHALL retain their project capability. External host paths
SHALL be prepared as host filesystem resources and routed through the configured
approval policy, with `allow-all` granting permission automatically. The project
root SHALL remain the base for relative paths. Canonical pathname
validation MAY provide display and preliminary diagnostics but MUST NOT be the
final containment primitive, and a bounded read MUST inspect and consume one
opened object through a `max_bytes + 1` limiter.

#### Scenario: Symlink is swapped after validation

- **GIVEN** an ordinary confined run and a repository path validated beneath the workspace
- **AND** an adversary replaces a traversed component with an escaping symlink
- **WHEN** the built-in tool opens or mutates the target
- **THEN** the handle-relative operation refuses the escape
- **AND** no object outside the workspace is read or changed

#### Scenario: Absolute path is outside the project

- **GIVEN** a built-in read, list, search, or edit call names an absolute path
  outside the workspace
- **WHEN** Smith prepares the ordinary filesystem tool
- **THEN** it requests permission for the canonical external filesystem target
- **AND** approval allows the operation and denial returns no external content
- **AND** an unattended run without authority reports approval required

#### Scenario: File grows after metadata inspection

- **GIVEN** a file whose initial metadata is within the read limit
- **WHEN** the object grows while Smith reads it
- **THEN** Smith reads no more than `max_bytes + 1`
- **AND** returns the oversized-file outcome without retaining the full object

#### Scenario: Explicit allow-all authorizes host filesystem tools

- **GIVEN** the resolved invocation approval mode is `allow-all`
- **WHEN** an available filesystem tool names an external absolute path, relative traversal, or symlink target
- **THEN** it can access that host path within operating-system permissions
- **AND** relative paths and omitted tool paths still start from the project
- **AND** prepared external resources identify the host mount and absolute target
- **AND** reads remain bounded and destructive edits remain version-checked

#### Scenario: External image reaches the serving model

- **GIVEN** an explicit allow-all run with an image-capable serving model
- **WHEN** `read` opens a supported image outside the project
- **THEN** it returns validated, provider-sized image content and its description
- **AND** the next provider request contains the typed image content

#### Scenario: Text-only main model uses vision for an external image

- **GIVEN** an explicit allow-all run with a text-only main model and configured vision route
- **WHEN** `view_image` names an external image path
- **THEN** the configured vision route can read and inspect that image
