## ADDED Requirements

### Requirement: Independently revisioned project-instruction fragment

Smith SHALL contribute a present project-instruction snapshot as one required,
provenance-bearing developer-instruction fragment whose revision derives from
its exact source identity and content. It MUST remain distinct from Smith's
stable product-policy sections, optional retrieval-style project context,
canonical conversation history, and executable skill activation. Exact prompt
and cache identity MUST change when a newly constructed runtime captures
different project instructions.

#### Scenario: The same snapshot is reused by a child

- **GIVEN** a root runtime and direct child use the same captured project
  instruction snapshot
- **WHEN** Smith constructs their prompt and child-policy fingerprints
- **THEN** both identify the same project-instruction revision
- **AND** child construction performs no second filesystem read

#### Scenario: Instructions change before a later runtime

- **GIVEN** one runtime was built from project-instruction revision A
- **WHEN** a later runtime captures changed content as revision B
- **THEN** the project fragment and exact full prompt/cache identity differ
- **AND** unchanged Smith product fragments retain their own prior revisions
- **AND** Smith does not report the old exact cache identity as applicable

#### Scenario: File changes without runtime reconstruction

- **GIVEN** an active runtime has already planned with a captured project
  instruction revision
- **WHEN** the underlying file changes
- **THEN** the runtime's fragments and cache identity remain unchanged
- **AND** no provider request is sent merely because of the filesystem change

#### Scenario: Complete host prompt override is supplied

- **GIVEN** a direct embedder supplies Smith's complete system-prompt override
- **WHEN** the factory composes the runtime
- **THEN** the override retains its existing complete-replacement semantics
- **AND** Smith does not append an implicit project-instruction fragment
