## ADDED Requirements

### Requirement: Image generation tool

Smith SHALL provide a `generate_image` tool that creates an image from a
prompt, or edits reference images, through the active provider's Images API.
It SHALL accept at most five references, given either as `reference_paths`
or as a count of `recent_images`, and MUST reject a call that supplies both.
Reference paths MUST resolve inside the workspace or Smith's generated-image
directory.

#### Scenario: Generate a new image

- **GIVEN** the active provider is ChatGPT
- **WHEN** the model calls `generate_image` with only a prompt
- **THEN** Smith posts to `{base}/images/generations` with the configured
  image model
- **AND** it saves the PNG under
  `~/.smith/generated_images/<session>/<call>.png`
- **AND** the tool result contains the image and its saved path

#### Scenario: Edit a referenced image

- **WHEN** the model calls `generate_image` with a prompt and one workspace
  image path
- **THEN** Smith posts to `{base}/images/edits` with that image as a data URL

#### Scenario: Both reference kinds supplied

- **WHEN** a call supplies both `reference_paths` and `recent_images`
- **THEN** the tool returns an error to the model without contacting the
  provider

### Requirement: Image tool availability

Smith SHALL register `generate_image` only when the active provider is ChatGPT
or the OpenAI Platform endpoint and image generation is enabled. The tool
MUST reuse that provider's credential lease and MUST NOT be offered in `plan`
posture.

#### Scenario: Unsupported provider

- **GIVEN** the active provider is a GLM OpenAI-compatible endpoint
- **WHEN** the tool list is built
- **THEN** `generate_image` is absent

#### Scenario: Plan posture

- **GIVEN** the active profile uses `plan` posture
- **WHEN** the tool list is built
- **THEN** `generate_image` is absent

### Requirement: Image tool results survive output bounding

The tool-output bound SHALL measure an image part by its image cost, not by
the length of its data URL. A generated image that fits the bound MUST
reach the model intact.

#### Scenario: Large data URL

- **GIVEN** a tool result with a 2 MB PNG data URL and a 32 KiB output limit
- **WHEN** the result is bounded
- **THEN** the image part is kept unchanged
