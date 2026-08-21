## ADDED Requirements

### Requirement: Prepared-call-scoped automatic approval

Smith SHALL express automatic approval as versioned typed rules over immutable
prepared calls rather than lists of tool names. A rule MUST constrain a
module-qualified tool identity, allowed operations, a permission ceiling, a
risk ceiling, and a concrete resource pattern; it MAY additionally expire or
limit uses. Matching MUST fail closed for unknown fields, operations,
permissions, risks, resource kinds, or revisions, and repository-controlled
layers MUST remain unable to supply approval authority.

#### Scenario: Workspace replacement matches a narrow rule

- **GIVEN** user configuration authorizes `smith/edit` `replace` beneath
  `src/**` with only filesystem read and write permissions
- **WHEN** an immutable prepared replacement targets `src/lib.rs` within that
  ceiling
- **THEN** Smith may approve it without prompting
- **AND** the exact prepared call fingerprint remains the action invoked

#### Scenario: Edit operation widens to delete

- **GIVEN** a rule authorizes `replace` and `create` but not `delete`
- **WHEN** an `edit` prepared call requests delete authority
- **THEN** the rule does not match
- **AND** Smith consults the configured fallback approval policy

#### Scenario: Rule encounters host or egress authority

- **GIVEN** a prepared call includes a host-root resource, arbitrary process
  execution, network, credential use, data egress, or an unclassified resource
- **WHEN** Smith evaluates scoped automatic approval
- **THEN** no scoped rule authorizes the call
- **AND** only an explicit broader approval policy can allow it

### Requirement: Tool-name auto-approval migration fails closed

The legacy `approval.auto_approve = ["tool"]` shape SHALL NOT be interpreted as
a grant. A non-empty legacy value MUST fail preflight with a bounded migration
diagnostic before session state, terminal entry, provider I/O, or tool
execution; absent or empty legacy values MAY be accepted as inert during the
migration window.

#### Scenario: Legacy edit allowlist is present

- **GIVEN** user configuration contains `approval.auto_approve = ["edit"]`
- **WHEN** Smith resolves approval policy
- **THEN** preflight rejects the ambiguous grant and names the typed replacement
- **AND** no edit is automatically approved

#### Scenario: Project supplies a typed automatic rule

- **GIVEN** repository-controlled configuration contains a syntactically valid
  typed automatic approval rule
- **WHEN** Smith preflights the run
- **THEN** startup fails under the existing self-authorization prohibition
- **AND** the rule grants no authority merely because the project is trusted
