## ADDED Requirements

### Requirement: Image generation settings

Smith SHALL read `[tools.image_generation]` with `enabled`, `model` (default
`gpt-image-2`), `quality`, and `size` (default `auto` for both), and SHALL
report each value's provenance through `smith config explain`.

#### Scenario: Disable image generation

- **GIVEN** `[tools.image_generation] enabled = false`
- **WHEN** a ChatGPT session builds its tool list
- **THEN** `generate_image` is absent
