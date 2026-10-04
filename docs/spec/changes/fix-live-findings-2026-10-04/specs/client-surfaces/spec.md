## ADDED Requirements

### Requirement: Line editing works in a slash draft

The TUI SHALL apply the composer's line-editing keys (Ctrl+A, Ctrl+E,
Ctrl+U, Ctrl+K, Ctrl+W, Alt+B, Alt+F, Left, Right, Home, End, Delete) to a
draft that starts with `/` exactly as to any other draft, while the command
menu stays open and refreshes its matches.

#### Scenario: Clearing a refused command

- **GIVEN** the draft `/agentx` was refused as an unknown command and is
  still in the composer
- **WHEN** the user presses Ctrl+U
- **THEN** the draft is empty
- **AND** typing `/agent` produces the draft `/agent`

### Requirement: Child details state each fact once

The agent inspector and `/agent` SHALL name each fact about a child once,
label every count, use Smith's compact token form, render the child's result
as Markdown, and offer `/agent resume` only when an exact checkpoint exists.
The `/agent` output MUST be headed by the command the user typed.

#### Scenario: Inspecting a finished child without a checkpoint

- **GIVEN** a resumed session whose child finished and has no exact
  checkpoint
- **WHEN** the user runs `/agent child-1`
- **THEN** the session id, durability, turn count, and token count each
  appear once, with the turn count labelled
- **AND** the result's Markdown is rendered, not shown as `**` and
  backticks
- **AND** no `/agent resume` command is offered

### Requirement: Capability activation stays in detail

The TUI SHALL show capability activation notices only when work detail is
expanded and in `/diagnostics`, never in the default transcript.

#### Scenario: A tool is activated through the registry

- **WHEN** a turn activates tools through `registry.search`
- **THEN** the default transcript shows no `activation epoch` line
- **AND** Ctrl+O and `/diagnostics` still show the activated capabilities

### Requirement: Child-agent approvals read like the spawn row

The approval for starting a child agent SHALL name the operation, the task,
and the child's tools and workspace in the words the spawn row uses, and
SHALL list a turn, token, or time limit only when one is set. It MUST NOT
print null values, the unlimited sentinel, internal field names, or internal
target or permission identifiers.

#### Scenario: Approving a write-capable child

- **GIVEN** the model asks to start a child with all tools in the shared
  workspace and no limits
- **WHEN** the approval opens
- **THEN** it reads as starting a child agent with the task, `tools all`,
  and `workspace shared`
- **AND** no `null`, `4294967295`, `deadline_ms`, `delegation.spawn`, or
  `child-agent:session-` text appears
