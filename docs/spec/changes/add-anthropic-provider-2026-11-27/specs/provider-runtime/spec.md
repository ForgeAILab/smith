## ADDED Requirements

### Requirement: Catalog-governed Anthropic requests

For the official `anthropic` binding, Smith SHALL compose Agent Runtime's
existing Anthropic Messages adapter with the profile resolved from the frozen
catalog and SHALL NOT add a parallel provider path. Catalog absence or invalid
metadata MUST fail before credential resolution or provider I/O.

#### Scenario: Selected effort reaches the adapter

- **GIVEN** the binding is `anthropic/claude-opus-5-5` with a catalog effort
  ladder
- **WHEN** a turn is accepted at `low` effort
- **THEN** the native request contains `thinking.type = "adaptive"` and
  `output_config.effort = "low"`
- **AND** it carries the `x-api-key` header and no OpenAI, OpenRouter, Z.AI,
  or Gemini reasoning extension

#### Scenario: Default effort applies without a selection

- **GIVEN** the binding has a catalog effort ladder containing `high` and no
  effort is selected
- **WHEN** a turn is accepted
- **THEN** the native request contains `thinking.type = "adaptive"` and
  `output_config.effort = "high"`
- **AND** status attributes the level to the Smith catalog binding default

#### Scenario: No thinking budget from catalog metadata

- **GIVEN** a catalog model advertises only a toggle
- **WHEN** Smith prepares any request for it without explicit reasoning
  metadata
- **THEN** the native request contains no `budget_tokens`
- **AND** no `thinking` field is sent

#### Scenario: Model metadata is incomplete

- **GIVEN** the selected Anthropic model lacks valid limits or tool capability
  in the frozen catalog and no explicit override supplies them
- **WHEN** Smith prepares the runtime
- **THEN** construction fails before credential lookup or Anthropic I/O
- **AND** Smith does not infer values from the model name

### Requirement: Official Anthropic requests tolerate history edits

At the exact official endpoint, Smith SHALL configure the Anthropic adapter to
drop thinking blocks whose conversation prefix no longer matches, instead of
letting the request fail. Every request on that provider that carries adaptive
thinking MUST include `thinking.block_binding.prefix_mismatch_behavior =
"drop_block"` and the `thinking-binding-controls-2026-08-01` beta. Gateway
endpoints MUST NOT receive this control.

#### Scenario: Main turn carries the binding

- **GIVEN** the official binding with any supported effort, including the
  default
- **WHEN** a turn is sent
- **THEN** the request carries `block_binding` with `drop_block`
- **AND** the `anthropic-beta` header lists
  `thinking-binding-controls-2026-08-01` once

#### Scenario: Summary call replays history safely

- **GIVEN** a session on the official binding has replayable thinking blocks
- **WHEN** Smith sends a semantic-summary or other one-shot request through the
  same provider without its own reasoning config
- **THEN** that request carries the binding's effort, adaptive thinking, and
  `drop_block`
- **AND** a request that already has a reasoning config keeps it unchanged

#### Scenario: Gateway request is unchanged

- **GIVEN** a provider of kind `anthropic-messages` at a gateway endpoint
- **WHEN** a turn is sent with an explicit `anthropic-effort` selection
- **THEN** the request carries no `block_binding` and no binding beta
