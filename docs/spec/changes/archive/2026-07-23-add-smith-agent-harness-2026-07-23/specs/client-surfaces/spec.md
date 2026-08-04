## ADDED Requirements

### Requirement: Basic interactive TUI

Running `smith` without non-interactive flags SHALL open a Ratatui/Crossterm
coding surface containing a scrollable transcript, streaming response,
composer, command access, tool calls/results, approval prompts, provider/model
selection, and session create/resume. The surface MUST be driven by the same
resolved Smith runtime factory and shared events as every other host.

#### Scenario: Run a basic coding turn

- **GIVEN** valid provider configuration
- **WHEN** the user enters a prompt and approves a requested tool
- **THEN** the TUI streams model text and tool state without blocking input
- **AND** persists the canonical turn for resume

### Requirement: Operational status in the TUI

The TUI SHALL display current provider/model, token and provenance status,
cache state, active monitors, direct children, and queued notifications. An
estimated or unknown value MUST be visually distinguishable from a
provider-reported value.

#### Scenario: Provider switch leaves estimated context

- **GIVEN** the user switches to a provider that has not reported usage
- **WHEN** the status line updates
- **THEN** it labels context tokens estimated
- **AND** does not reuse the prior provider's verified cache indicator

### Requirement: Non-interactive prompt mode

Smith SHALL accept `smith -p <prompt>` and `smith -p -` for stdin. Callers MUST
be able to select project, session/resume, provider, model, approval policy, and
background-exit policy through stable arguments or configuration. Headless mode
MUST use the same runtime factory as the TUI.

#### Scenario: Prompt comes from stdin

- **GIVEN** a caller pipes a prompt to `smith -p -`
- **WHEN** the agent finishes successfully
- **THEN** Smith writes the requested output format to stdout
- **AND** uses stderr for progress and diagnostics

### Requirement: Versioned machine output

Non-interactive mode SHALL support `text`, `json`, and `stream-json`. JSON MUST
be one versioned final result envelope; stream JSON MUST be newline-delimited
versioned runtime events followed by a terminal result. Machine-readable stdout
MUST NOT contain progress prose or terminal color escapes.

#### Scenario: External CLI consumes stream JSON

- **GIVEN** `--output-format stream-json`
- **WHEN** the agent streams text, calls a tool, reports usage, and completes
- **THEN** stdout contains one parseable event per line in causal order
- **AND** the final line communicates terminal status and session ID

### Requirement: Fail-closed headless approval

When no TTY is available, an action requiring user approval MUST produce a
structured approval-required outcome and stable non-success exit status unless
the caller supplied an explicit policy authorizing it.

#### Scenario: Headless mutation lacks policy

- **GIVEN** an external caller runs `smith -p` without a TTY or mutation policy
- **WHEN** the model requests a patch
- **THEN** Smith does not modify the file
- **AND** returns an approval-required result rather than waiting indefinitely

### Requirement: Explicit active-work exit policy

The TUI MUST request confirmation before exiting with active monitors or
children. Non-interactive mode SHALL support `error`, `wait`, and `stop`
background-exit policies and MUST default to `error`.

#### Scenario: Headless turn finishes with a persistent monitor

- **GIVEN** the final answer is ready while a persistent monitor remains
- **WHEN** the caller did not choose an exit policy
- **THEN** Smith emits an active-work error describing the monitor
- **AND** does not silently orphan it

### Requirement: Host-appropriate UI contributions

Declarative extension status/widgets SHALL render in the TUI. In
non-interactive mode, presentation-only contributions MUST be omitted
predictably without failing the agent run, while their underlying data events
MAY remain in machine output.

#### Scenario: Status extension runs headlessly

- **GIVEN** a trusted extension registers a TUI status item
- **WHEN** `smith -p` runs
- **THEN** Smith does not attempt terminal rendering
- **AND** the extension's non-visual lifecycle hooks continue according to
  policy

### Requirement: macOS and Linux terminal support

Smith SHALL support macOS and Linux terminals with keyboard-only operation,
visible focus, resize handling, and reduced-motion behavior. Platform-specific
process cleanup MUST be covered by automated tests.

#### Scenario: Terminal resizes during streaming

- **GIVEN** the TUI is displaying a streaming response
- **WHEN** the terminal size changes
- **THEN** content reflows without losing transcript or focus state
