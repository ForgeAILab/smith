## ADDED Requirements

### Requirement: Trusted ChatGPT models grant OpenAI-effort controls

Smith SHALL treat the exact ChatGPT endpoint as trusted reasoning evidence
for its trusted ChatGPT model records. Without explicit model settings, such
a binding SHALL resolve the OpenAI-effort dialect with reasoning on by
default and no `off`, the efforts `low`, `medium`, `high`, `xhigh`, `max`,
and `ultra`, and a default effort of `medium`. Explicit
`[models."chatgpt/<model>".reasoning]` settings SHALL take precedence. A
model id on that endpoint that is not a trusted record SHALL NOT receive
inferred controls.

#### Scenario: Connected ChatGPT model without a models block

- **GIVEN** a `chatgpt` provider on the ChatGPT endpoint and no
  `[models."chatgpt/gpt-5.6-sol"]` table
- **WHEN** a profile selecting `gpt-5.6-sol` resolves
- **THEN** reasoning resolves the OpenAI-effort dialect, mandatory on, with
  efforts `low` through `ultra` and default effort `medium`
- **AND** selecting `xhigh` emits that effort in the request

#### Scenario: Explicit settings win

- **GIVEN** `[models."chatgpt/gpt-5.6-sol".reasoning]` sets
  `default_effort = "high"`
- **WHEN** the profile resolves
- **THEN** the default effort is `high`

#### Scenario: Untrusted model on the ChatGPT endpoint

- **GIVEN** a model id on the ChatGPT endpoint that is not a trusted
  ChatGPT record
- **WHEN** a session asks to control reasoning
- **THEN** Smith refuses before provider I/O, as for any unknown binding
