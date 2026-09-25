## ADDED Requirements

### Requirement: Mandatory Anthropic effort selection is capability-driven

Smith SHALL make its existing `/think`, `/effort`, `--effort`, `/status`, and
`/context` surfaces use the resolved capability snapshot for an exact binding
configured with mandatory `anthropic-effort` controls, without provider
probing or special-casing the model name. They MUST NOT expose raw reasoning
content.

#### Scenario: Fable 5.1 effort picker opens locally

- **GIVEN** the idle binding advertises `low`, `medium`, `high`, `xhigh`, and
  `max` through trusted Anthropic metadata
- **WHEN** the user opens `/effort`
- **THEN** the picker lists those levels in advertised order plus provider
  default
- **AND** opening or choosing the setting sends no provider request

#### Scenario: Mandatory adaptive thinking cannot be disabled

- **GIVEN** the binding marks Anthropic adaptive thinking mandatory
- **WHEN** the user opens `/think` or submits `/think off`
- **THEN** the off choice is unavailable with a written reason
- **AND** the direct command fails locally without provider I/O

#### Scenario: Invocation effort keeps ordinary provenance

- **GIVEN** the user starts the exact binding with `--effort xhigh`
- **WHEN** Smith explains or displays the effective reasoning state
- **THEN** the selection is `xhigh` and retains command-line provenance
- **AND** status never renders the model's raw reasoning content
