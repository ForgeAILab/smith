## ADDED Requirements

### Requirement: File commands expand to an ordinary prompt

Smith SHALL treat a discovered file command as a prompt template. Submitting
`/<name>` followed by optional text MUST submit the template body as an
ordinary user prompt, under the same queueing, reference, and approval rules
as typed text, and MUST NOT execute anything while expanding it. The text the
user typed MUST be what the composer, queue preview, and input history show,
and the expanded prompt MUST be what the committed transcript shows. Every
occurrence of `$ARGUMENTS` in the body MUST be replaced by the text after the
command name in a single pass; when the body has no placeholder and the
arguments are not empty, they MUST be appended after the body.

#### Scenario: Template with a placeholder

- **GIVEN** a user command `audit` whose body is `Audit $ARGUMENTS for bugs.`
- **WHEN** the user submits `/audit src/lib.rs`
- **THEN** the prompt sent to the provider is `Audit src/lib.rs for bugs.`
- **AND** the queue preview and input history show `/audit src/lib.rs`
- **AND** the committed transcript row shows the expanded prompt, live and
  after the session is resumed

#### Scenario: Template without a placeholder

- **GIVEN** a user command `standup` whose body has no `$ARGUMENTS`
- **WHEN** the user submits `/standup yesterday only`
- **THEN** the prompt is the body followed by `yesterday only`

#### Scenario: Arguments are not expanded again

- **GIVEN** a command whose body contains `$ARGUMENTS`
- **WHEN** the user's arguments themselves contain the text `$ARGUMENTS`
- **THEN** that text reaches the provider literally

#### Scenario: Template text is never executed

- **GIVEN** a command body containing shell syntax or a `!` line
- **WHEN** the user invokes the command
- **THEN** the text is sent as prompt content
- **AND** no process is started by the expansion

#### Scenario: Edited user command applies on the next invocation

- **GIVEN** a discovered user command
- **WHEN** its file is edited and the user invokes it again in the same session
- **THEN** the edited body is used without restarting Smith

### Requirement: File commands share command discovery

File commands SHALL appear in slash completion, `Ctrl+P`, and `/help` through
the same discovery surface as built-in commands, each labelled with its source
layer and showing its description and argument hint. A built-in command name
MUST always resolve to the built-in.

#### Scenario: Slash completion lists a file command

- **GIVEN** a user command `audit` with a description
- **WHEN** the user types `/aud`
- **THEN** the menu lists `audit` with its description and a user-layer label
- **AND** `Tab` completes it without submitting a prompt

#### Scenario: Help lists file commands

- **GIVEN** at least one runnable file command
- **WHEN** the user submits `/help`
- **THEN** the file commands are listed in their own group with descriptions

#### Scenario: Built-in name wins

- **GIVEN** a file named `model.md` in a commands directory
- **WHEN** the user submits `/model`
- **THEN** the built-in model command runs
- **AND** the file is reported as a discovery problem

### Requirement: File command visibility and trust command

Smith SHALL provide a built-in `/commands` command that lists every discovered
file command grouped by source layer with its description and whether it can
run. The command MUST state why a command cannot run, MUST show which entries
a higher layer shadowed, MUST report every discovery problem, MUST offer a way
to grant trust to a project command, and MUST offer a way to rebuild the
catalog from disk without restarting.

#### Scenario: Inspect the catalog

- **GIVEN** a session with a user command and a project command
- **WHEN** the user runs `/commands`
- **THEN** each is listed under its layer with its description
- **AND** each entry states whether it can run

#### Scenario: Running a withheld project command

- **GIVEN** a project command nobody has approved
- **WHEN** the user submits it
- **THEN** Smith renders a local message naming `/commands trust <name>`
- **AND** no provider request is issued

#### Scenario: Grant trust from the command

- **GIVEN** a project command awaiting confirmation
- **WHEN** the user runs `/commands trust <name>` and confirms
- **THEN** Smith shows the project-relative path and content identity before
  recording the decision
- **AND** the command runs in the same session without restarting Smith

#### Scenario: Project command changes after approval

- **GIVEN** a project command approved at one content digest
- **WHEN** its file is rewritten and the user submits it
- **THEN** the command does not run
- **AND** it is reported as changed with the command that would re-approve it

#### Scenario: Reload picks up a new file

- **GIVEN** a command file created after the session started
- **WHEN** the user runs `/commands reload` while idle
- **THEN** the new command appears in completion and can be run
- **AND** edited descriptions and argument hints are refreshed

### Requirement: File commands in non-interactive mode

`smith -p` SHALL expand a prompt whose first token is `/<name>` when `<name>`
is a discovered file command, using the same expansion as the TUI, and MUST
pass every other prompt through unchanged, including the names of built-in
commands. The documented literal-slash escape MUST apply in this mode. A
project command that is untrusted or changed MUST fail closed without a
provider request.

#### Scenario: Headless run uses a user command

- **GIVEN** a user command `audit`
- **WHEN** a caller runs `smith -p "/audit src/lib.rs"`
- **THEN** the expanded template is the prompt for the run

#### Scenario: Slash text that names no command

- **GIVEN** no file command named `usr`
- **WHEN** a caller runs `smith -p "/usr/bin/env is missing"`
- **THEN** the text is sent as the prompt unchanged

#### Scenario: Escaped slash in a headless run

- **GIVEN** a user command `audit`
- **WHEN** a caller runs `smith -p "//audit the plan"`
- **THEN** the prompt sent to the provider is `/audit the plan`
- **AND** the template is not expanded

#### Scenario: Untrusted project command in a headless run

- **GIVEN** a project command nobody has approved
- **WHEN** a caller runs `smith -p "/<name>"`
- **THEN** Smith exits non-zero with a diagnostic on stderr naming the command
- **AND** no provider request is issued
