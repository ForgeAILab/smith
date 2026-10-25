## ADDED Requirements

### Requirement: Read-only children keep reviewed network-read tools

A read-only child SHALL still receive the module tools that `[agent]
read_only_network_modules` admits to a read-only posture, and only those. A
child is read-only here when its tool scope is read-only or its workspace is
the read-only view.
Smith SHALL pass these tool names to the runtime as the read-only child
allowlist; it MUST NOT add a tool with any filesystem or external write, an
unreviewed MCP tool, or a plugin tool. Calls to an admitted tool SHALL go
through the parent's approval policy exactly as other child calls do. An empty
`read_only_network_modules` SHALL leave read-only children with no network
tools.

#### Scenario: A research child fetches a page

- **GIVEN** the default `read_only_network_modules` and a `plan`-posture child
  profile
- **WHEN** the root spawns it with the default read-only tool scope and
  workspace
- **THEN** `web_fetch` and the configured backend `web_search` are in the
  child's view
- **AND** `edit`, `shell`, and unreviewed MCP tools are not

#### Scenario: A full scope over a read-only view

- **GIVEN** the default list and a `build`-posture child profile
- **WHEN** the root spawns it with `tools` of `all` and no workspace
- **THEN** the child's view holds `web_fetch` and no tool that can modify the
  workspace

#### Scenario: The owner restores the strict filter

- **GIVEN** `[agent] read_only_network_modules = []`
- **WHEN** the root spawns a read-only child
- **THEN** neither `web_fetch` nor `web_search` is in the child's view

#### Scenario: A fetch from a child still asks

- **GIVEN** the approval policy `ask` and no rule for the destination host
- **WHEN** a read-only child calls `web_fetch`
- **THEN** the request reaches the parent's approval surface before any
  network request is made

### Requirement: Research delegation asks for bounded reports

When the delegation section is present, Smith's root delegation guidance SHALL
tell the model to ask a research child for concise findings with one list of
the sources it used, rather than a full dossier with repeated citations, and to
read a long reply through its artifact only when the excerpt is not enough.

#### Scenario: The root delegates a research question

- **GIVEN** a root run whose surface includes the `agent` tool
- **WHEN** the context is assembled
- **THEN** the delegation guidance asks research children for concise
  findings and a single source list
