## ADDED Requirements

### Requirement: Ordered provider credential pools

A provider declaration SHALL accept an ordered pool of credential references
in place of a single reference. Each pool entry MUST be a reviewed
`CredentialRef` form with unchanged parsing, layering, provenance, and
redaction semantics, entries MUST be distinct, and the first entry is the
default active member. A legacy single `credential` declaration SHALL resolve
as a pool of one with identical behavior.

#### Scenario: Declare a pool of two accounts

- **GIVEN** a provider declares `credentials = ["keychain:xai/personal", "keychain:xai/work"]`
- **WHEN** configuration resolves
- **THEN** the provider resolves with an ordered two-member pool
- **AND** each member reports its own source provenance

#### Scenario: Legacy single credential remains valid

- **GIVEN** a provider declares only `credential = "env:XAI_API_KEY"`
- **WHEN** configuration resolves
- **THEN** the provider resolves as a pool of one
- **AND** no configuration migration or warning is required

#### Scenario: Invalid pool entry fails preflight

- **GIVEN** a pool entry is not a parseable credential reference
- **WHEN** setup or factory preflight validates the provider
- **THEN** resolution fails before any terminal or provider I/O
- **AND** the error names the offending entry and its declaration source
- **AND** no credential value appears in the error

#### Scenario: Duplicate pool entries are rejected

- **GIVEN** a pool lists the same credential reference twice
- **WHEN** configuration resolves
- **THEN** resolution fails with a configuration error naming the duplicate
