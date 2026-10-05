## MODIFIED Requirements

### Requirement: Session listing is readable on a terminal

`smith sessions list` SHALL print a header row, aligned columns, and the
last-updated time in local time when standard output is a terminal. When
standard output is not a terminal it MUST keep the existing tab-separated
row format. The prompt column SHALL be labelled for what it holds, the
latest user prompt.

#### Scenario: Listing on a terminal

- **GIVEN** the project has saved sessions
- **WHEN** the user runs `smith sessions list` in a terminal
- **THEN** each column has a heading, and the prompt column reads
  `LATEST PROMPT`
- **AND** the update time is a local date and time, not a millisecond count

#### Scenario: Listing through a pipe

- **WHEN** the output of `smith sessions list` is piped to another program
- **THEN** each session that holds a user message is one tab-separated row
  with the fields and order documented for the Claude Code plugin
- **AND** the plugin's documentation names the last field the latest prompt

### Requirement: Sessions without a user message are not offered

Smith SHALL NOT offer a session that holds no user message for resumption:
the resume pickers and both forms of `smith sessions list` MUST omit it, and
the exit report MUST NOT print a `resume with …` line for it. When the
interactive surface ends a session that holds no user message, Smith SHALL
remove that session's files. A session whose user message failed before any
provider usage SHALL still be offered and kept.

#### Scenario: Start and quit without typing

- **GIVEN** the user starts Smith and quits without submitting a prompt
- **WHEN** Smith exits and the user later opens `/resume` or runs
  `smith sessions list` in a terminal
- **THEN** the exit report has no `resume with …` line
- **AND** that session is not listed
- **AND** its snapshot, journal, and lock files are gone from the project's
  session directory

#### Scenario: A prompt that failed is still offered

- **GIVEN** the user submitted a prompt and the provider request failed
  before reporting usage
- **WHEN** the user opens `/resume`
- **THEN** the session is listed with its prompt as the row's name

#### Scenario: Piped listing

- **GIVEN** a project holds one session with a prompt and one without
- **WHEN** `smith sessions list` is piped to another program
- **THEN** only the session with a prompt is printed

## ADDED Requirements

### Requirement: Child sessions are reached through their parent

Smith SHALL NOT list a child agent's session as a resumable session: the
resume pickers and both forms of `smith sessions list` MUST omit sessions
that belong to a parent session. A child stays reachable through its parent
(`/agent`, follow-up), and an explicit `--resume <ID>` keeps its current
behaviour.

#### Scenario: A session that spawned a child

- **GIVEN** a session that spawned one durable child agent
- **WHEN** the user runs `smith sessions list` or opens `/resume`
- **THEN** the parent session is listed once
- **AND** no `child-session-…` entry is listed

