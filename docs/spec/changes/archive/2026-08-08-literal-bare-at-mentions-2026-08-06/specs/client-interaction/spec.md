## MODIFIED Requirements

### Requirement: Unified typed reference completion

Smith SHALL provide one `@` completion surface for bounded canonical workspace
files and registered child-agent presets. Resolution MUST occur locally before
provider spend and MUST retain type, provenance, authority, and size metadata.
A bare `@token` that matches a known file or child agent resolves as that
reference. A bare `@token` that matches neither SHALL pass through as literal
text, so ordinary prose containing scoped package names, social handles, or
other leading-at signs sends without an attachment error. Explicit typed
prefixes (`@file:`, `@agent:`) that fail to resolve, and ambiguous names that
collide between files and agents, MUST still report a local bounded error.

#### Scenario: Attach a workspace file

- **GIVEN** the user selects `@src/lib.rs` from file completion
- **WHEN** they submit the prompt
- **THEN** Smith prepares and authorizes an exact workspace read
- **AND** contributes bounded content or an artifact reference with file
  provenance to the planned request

#### Scenario: Unresolvable bare at sign is literal text

- **GIVEN** a draft contains a bare `@token` that matches no workspace file or
  child agent
- **WHEN** the user submits it
- **THEN** Smith sends the prompt with the `@token` as literal text
- **AND** performs no provider request, attachment, or unauthorized read beyond
  the ordinary prompt

#### Scenario: Explicit typed reference escapes the workspace

- **GIVEN** a draft contains an explicit `@file:` or `@agent:` reference that
  does not resolve, or an ambiguous name that is both a file and an agent
- **WHEN** the user submits it
- **THEN** Smith keeps the draft and reports a local bounded error
- **AND** performs no provider request or unauthorized read

#### Scenario: Literal at sign

- **GIVEN** the draft contains the documented `@@` escape
- **WHEN** the user submits it
- **THEN** Smith sends one literal leading `@` at that position
- **AND** does not open or resolve a reference
