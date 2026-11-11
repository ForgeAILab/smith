## ADDED Requirements

### Requirement: Agent-filed harness feedback tool

While the `harness-feedback` module is enabled, Smith SHALL pin a
`harness_feedback` tool in every turn without activation. That covers root
sessions, child agents including read-only children, headless runs, and
`cli/*` turns through the CLI bridge. The tool MUST declare no effects, MUST
record a report without a Smith approval prompt in any posture, and MUST
return a short acknowledgement naming the report id. A CLI agent's own
permission settings still apply to bridged calls.

#### Scenario: Agent records a report without a prompt

- **GIVEN** an interactive session under the default posture
- **WHEN** the agent calls `harness_feedback` with a kind, area, title and
  what happened
- **THEN** Smith writes one report to the feedback store
- **AND** no approval prompt is shown
- **AND** the tool result names the new report id

#### Scenario: Restricted turns can still report

- **GIVEN** a read-only child agent, or a headless run with `--approval deny`
- **WHEN** the agent calls `harness_feedback`
- **THEN** the report is recorded

#### Scenario: CLI-backed turns receive the tool

- **GIVEN** a profile that routes turns to a `cli/*` agent
- **WHEN** Smith builds the bridge tool set for that turn
- **THEN** `harness_feedback` is included
- **AND** a bridged call adds no Smith approval prompt

#### Scenario: The tool needs no activation

- **WHEN** a session starts with the module enabled
- **THEN** `harness_feedback` is in the pinned tool set
- **AND** it is not listed in the activatable index

### Requirement: Guidance on when to report

Smith SHALL tell the agent, through the tool description and one prompt
guidance fragment, to report a tool or product failure that was resolved or
abandoned, a missing capability that blocked a reasonable request, user
frustration with Smith, or harness behavior that pushed the agent into a
mistake. The guidance MUST say not to report bugs in the user's own project
or task difficulty Smith did not cause, and MUST tell the agent not to
announce a report or ask the user about it.

#### Scenario: Guidance is present while enabled

- **WHEN** a session starts with the module enabled
- **THEN** the system prompt contains the feedback guidance fragment
- **AND** the tool description states when to report and when not to

#### Scenario: Guidance is absent while disabled

- **GIVEN** `[modules.harness-feedback] enabled = false`
- **WHEN** a session starts
- **THEN** neither the tool nor the guidance fragment is present

### Requirement: Reports carry harness-attached evidence

Each report SHALL include evidence that Smith supplies itself: Smith version
and commit, OS, profile, provider and model, session, turn and call ids, child
id when filed by a child, workspace path, and up to five of the most recent
failed tool calls Smith ran in the session, with their names, call ids and
bounded error text. Tool arguments MUST NOT override these fields. For
`cli/*` turns the failures MUST be limited to bridged calls, and the report
MUST say that the CLI's own tools are not covered.

#### Scenario: A failed tool call is attached

- **GIVEN** a tool call earlier in the session failed with an error
- **WHEN** the agent then calls `harness_feedback`
- **THEN** the report lists the failed call's tool name, call id and error
  text truncated to at most 500 characters

#### Scenario: CLI-backed evidence states its limit

- **GIVEN** a `cli/*` turn
- **WHEN** the agent files a report through the bridge
- **THEN** the evidence lists only failed bridged calls
- **AND** it states that the CLI's own tool calls are not included

#### Scenario: Model-supplied evidence cannot override Smith's fields

- **WHEN** the tool arguments include a field claiming a different Smith
  version or session id
- **THEN** the stored report holds Smith's own values for those fields

### Requirement: Owner-only bounded feedback store

Smith SHALL store each report as one Markdown file with frontmatter under
`~/.smith/feedback/`, with an owner-only directory and files, written
atomically. Titles, text fields and files MUST be size-bounded with visible
truncation markers. Registered secret values MUST be redacted before writing.
A session MUST NOT create more than five new reports. Within one session, a
report matching an earlier one on kind, area and normalized title MUST raise
that report's count instead of creating a file. The store MUST hold at most
500 reports and MUST NOT delete queued reports automatically. Concurrent
writers, such as parallel sessions and child agents, MUST NOT lose a report or
a count.

#### Scenario: Concurrent sessions both record

- **GIVEN** two Smith sessions running at the same time
- **WHEN** each files a distinct report at the same moment
- **THEN** both reports are stored intact

#### Scenario: A repeated issue raises the count

- **GIVEN** the agent has filed a report titled "edit rejects CRLF files" in
  this session
- **WHEN** it files the same kind, area and title again
- **THEN** the existing report's count becomes 2
- **AND** no new file is created

#### Scenario: The per-session limit is reached

- **GIVEN** five new reports were created in this session
- **WHEN** the agent files a sixth distinct report
- **THEN** the tool result says the session limit is reached
- **AND** nothing is written

#### Scenario: A registered secret is redacted

- **WHEN** a report field contains a registered secret value
- **THEN** the stored report shows `[redacted]` in its place

#### Scenario: A full store refuses rather than dropping queued reports

- **GIVEN** the store holds 500 reports, all queued
- **WHEN** the agent files a new report
- **THEN** the tool result says the store is full
- **AND** no queued report is deleted

### Requirement: Reviewing queued reports

Smith SHALL provide `/feedback`, which lists stored reports inline, newest
first, with id, age, status, kind, area, title and count. `/feedback <id>`
SHALL show one full report inline. These results MUST NOT become model
context. After a session starts, Smith SHALL show one transcript notice when
queued reports exist that have not been listed since they were filed.

#### Scenario: Listing queued reports

- **GIVEN** two queued reports and one dismissed report
- **WHEN** the user invokes `/feedback`
- **THEN** all three appear inline, newest first, with their status
- **AND** no provider request is issued

#### Scenario: Startup notice for unviewed reports

- **GIVEN** a report filed in an earlier session that has not been listed
- **WHEN** a new interactive session starts
- **THEN** the transcript shows one line naming the count and `/feedback`
- **AND** the notice does not repeat after the user lists the reports

#### Scenario: Stored reports survive disabling the module

- **GIVEN** stored reports and `[modules.harness-feedback] enabled = false`
- **WHEN** the user invokes `/feedback`
- **THEN** the stored reports are listed
- **AND** no startup notice was shown for them

### Requirement: Filing a report on GitHub after preview

From `/feedback <id>`, Smith SHALL offer an inline select to file the report
on GitHub, dismiss it, or keep it. Filing MUST first show the exact issue
title and body. The body MUST leave out the workspace path and session, turn
and call ids. On confirmation Smith SHALL open a prefilled `issues/new` URL
for `ForgeAILab/smith` in the browser and MUST NOT submit anything itself.
The report becomes `filed` only after the browser hand-off succeeds.

#### Scenario: Preview before opening the browser

- **WHEN** the user chooses to file a report
- **THEN** Smith shows the exact title and body that the URL will carry
- **AND** opens the browser only after the user confirms

#### Scenario: Declining keeps the report queued

- **WHEN** the user declines at the preview
- **THEN** no browser is opened
- **AND** the report stays `queued`

#### Scenario: An oversized body is truncated at a section boundary

- **GIVEN** a report whose encoded URL would exceed 8,000 bytes
- **WHEN** Smith builds the filing URL
- **THEN** the body ends at a section boundary with a line pointing to
  `smith feedback show <id>`

#### Scenario: Browser hand-off fails

- **WHEN** the platform opener cannot be started
- **THEN** Smith shows the URL as copyable text
- **AND** the report stays `queued`

### Requirement: Command-line access for repair agents

Smith SHALL provide `smith feedback list [--json]`, `smith feedback show <id>`
and `smith feedback dismiss <id>`, which read and update the store without
starting a session. JSON output MUST be bounded and MUST label report text as
untrusted data.

#### Scenario: A repair agent lists reports as JSON

- **WHEN** `smith feedback list --json` runs
- **THEN** it prints one bounded JSON document with each report's id, status,
  kind, area, title, count and created time
- **AND** the document marks report text fields as untrusted

#### Scenario: Dismissing a report

- **WHEN** `smith feedback dismiss <id>` runs for a queued report
- **THEN** the report's status becomes `dismissed`
- **AND** it no longer counts toward the startup notice
