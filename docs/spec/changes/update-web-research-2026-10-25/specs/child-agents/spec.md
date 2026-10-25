## ADDED Requirements

### Requirement: Read-only children hold every tool that cannot write

A read-only child SHALL hold every tool that a read-only-posture session on the
same route holds, and SHALL be denied only the tools that can modify the
workspace, write outside it, or spawn a process. A child is read-only here when
its tool scope is read-only or its workspace is the read-only view. The tools
kept SHALL include the session's artifact reader and lister, the web tools that
`[agent] read_only_network_modules` admits, image inspection, todo planning,
and question relay to the parent. Configuration SHALL be the only way to narrow
the set further: `read_only_network_modules` and an explicitly named tool scope
apply to children exactly as to the root. Smith SHALL pass the names it admits
to the runtime as the read-only child allowlist, and the runtime MUST still
refuse a listed tool whose permission bound includes a filesystem write, an
external write, or a process spawn. Calls to a kept tool SHALL go through the
parent's approval policy as other child calls do.

#### Scenario: A research child fetches a page

- **GIVEN** the default `read_only_network_modules` and a `plan`-posture child
  profile
- **WHEN** the root spawns it with the default read-only tool scope and
  workspace
- **THEN** `web_fetch`, the configured backend `web_search`, `artifact.read`,
  and `artifact.list` are in the child's view
- **AND** `edit`, `shell`, and unreviewed MCP tools are not

#### Scenario: A child reads its own stored output

- **GIVEN** a read-only child whose `web_fetch` result was stored as an
  artifact
- **WHEN** the child calls `artifact.read` with that artifact's ID
- **THEN** the page is returned to the child

#### Scenario: A build profile spawned read-only

- **GIVEN** the default list and a `build`-posture child profile
- **WHEN** the root spawns it with `tools` of `all` and no workspace
- **THEN** the child's view holds `web_fetch` and `artifact.read`
- **AND** it holds no tool that can modify the workspace

#### Scenario: The owner narrows network access

- **GIVEN** `[agent] read_only_network_modules = []`
- **WHEN** the root spawns a read-only child
- **THEN** neither `web_fetch` nor `web_search` is in the child's view
- **AND** `artifact.read` still is

#### Scenario: A listed tool that writes is still refused

- **GIVEN** a host allowlist naming a tool whose permission bound includes a
  filesystem write
- **WHEN** a read-only child view is built
- **THEN** that tool is not in the view

#### Scenario: A fetch from a child still asks

- **GIVEN** the approval policy `ask` and no rule for the destination host
- **WHEN** a read-only child calls `web_fetch`
- **THEN** the request reaches the parent's approval surface before any
  network request is made

### Requirement: A child's reply read through the agent tool stays inline

When the `agent` tool returns a completed child's reply, Smith SHALL keep that
outcome inline up to the resolved `limits.tool_output_limit_bytes` instead of
offloading it at `context.tool_output_inline_bytes`, so the parent reads a
report as text in one call. A reply larger than that ceiling SHALL follow the
ordinary offloading rule.

#### Scenario: The parent asks for a 30 KB report

- **GIVEN** default limits and a completed child whose reply is 30,000 bytes
- **WHEN** the parent calls `agent` with action `result` for that child
- **THEN** the outcome holds the whole reply inline
- **AND** no artifact reference replaces it

### Requirement: Research delegation asks for bounded reports

When the delegation section is present, Smith's root delegation guidance SHALL
tell the model to ask a research child for its findings as concise text with
one list of the sources it used, leaving bulky data in the artifacts its tools
produced and naming those artifacts by ID, rather than returning a full dossier
with repeated citations.

#### Scenario: The root delegates a research question

- **GIVEN** a root run whose surface includes the `agent` tool
- **WHEN** the context is assembled
- **THEN** the delegation guidance asks research children for concise
  findings, a single source list, and artifact IDs for bulky data

## MODIFIED Requirements

### Requirement: Child-completion delivery is bounded but never refused for size

An automatic `delegation.child-completion` delivery SHALL be built within an
inline budget equal to the resolved `limits.tool_output_limit_bytes`, the
per-call output ceiling, rather than the offloading threshold, because a
child's reply is the result the parent asked for. Each completed child's reply
SHALL be delivered in full while it fits the remaining budget, in the
delivery's existing deterministic order; a reply of at most 1,024 bytes SHALL
always be delivered in full. Every reply longer than 1,024 bytes SHALL be
stored unchanged as an artifact owned by the parent session when the child
completes. A reply that does not fit SHALL be delivered as its first 1,024
bytes together with that artifact's reference. Smith MUST NOT refuse, defer,
or drop an admission because of the size of its replies, and an oversized
outcome MUST NOT delay the delivery of any other outcome.

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

#### Scenario: A research report arrives as text

- **GIVEN** default limits and a child whose final reply is 30,000 bytes
- **WHEN** its outcome is delivered automatically
- **THEN** the delivery contains the whole reply verbatim

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
