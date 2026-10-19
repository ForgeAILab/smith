## ADDED Requirements

### Requirement: Child-completion delivery is bounded but never refused for size

An automatic `delegation.child-completion` delivery SHALL be built within an
inline budget equal to the resolved `context.tool_output_inline_bytes`. Each
completed child's reply SHALL be delivered in full while it fits the
remaining budget, in the delivery's existing deterministic order; a reply of
at most 1,024 bytes SHALL always be delivered in full. Every reply longer
than 1,024 bytes SHALL be stored unchanged as an artifact owned by the parent
session when the child completes. A reply that does not fit SHALL be
delivered as its first 1,024 bytes together with that artifact's reference.
Smith MUST NOT refuse, defer, or drop an
admission because of the size of its replies, and an oversized outcome MUST
NOT delay the delivery of any other outcome.

#### Scenario: A long report is delivered as excerpt and reference

- **GIVEN** a parent that does not read its child's result
- **AND** the child's final reply is larger than the inline budget
- **WHEN** the child completes and the parent is idle
- **THEN** one child-completion turn starts without a user turn
- **AND** it carries the start of the reply and a parent-owned artifact
  reference
- **AND** `artifact.read` on that reference returns the exact reply

#### Scenario: A reply within the budget is delivered whole

- **GIVEN** a child whose final reply fits the inline budget
- **WHEN** its outcome is delivered automatically
- **THEN** the delivery contains the reply verbatim
- **AND** the delivery carries no excerpt marker or artifact reference for it

#### Scenario: A long reply does not block a later child

- **GIVEN** a long-reply child completed while its delivery was pending
- **WHEN** a second child with a short reply completes
- **THEN** both outcomes are delivered
- **AND** the short reply is inline

#### Scenario: Several replies together exceed the budget

- **GIVEN** three children each with a reply under the budget but together
  over it
- **WHEN** their outcomes are delivered in one batch
- **THEN** replies are inlined in deterministic order while each fits the
  remaining budget
- **AND** each reply that does not fit is delivered as an excerpt and
  reference

#### Scenario: The session has no artifact store

- **GIVEN** a non-persistent session without an artifact store
- **WHEN** a reply larger than the budget is delivered
- **THEN** the delivery carries a bounded excerpt and points the parent to
  that child's result, which `agent result` returns under the ordinary
  output limits
- **AND** the admission is not refused

### Requirement: Delivered outcomes name their artifacts

Every completed outcome in an automatic delivery SHALL list the artifacts
transferred to the parent from that child's turn. Each entry SHALL give the
parent-owned artifact id, the child-owned id it was copied from, the media
type, and the byte length, so the parent can read it with `artifact.read` and
match it to any id the child's reply mentions. An outcome with no transferred
artifacts SHALL list none.

#### Scenario: A child's large tool output reaches the parent

- **GIVEN** a child whose turn offloaded a large tool output to an artifact
- **WHEN** its completed outcome is delivered automatically
- **THEN** the delivery names the parent-owned copy and its child-owned
  source
- **AND** `artifact.read` on the parent-owned id succeeds for the parent

#### Scenario: The parent reads the result itself

- **GIVEN** the parent reads the outcome with `agent wait` or `agent result`
- **WHEN** the tool result is returned
- **THEN** it lists the same artifact references
- **AND** no automatic delivery repeats the outcome
