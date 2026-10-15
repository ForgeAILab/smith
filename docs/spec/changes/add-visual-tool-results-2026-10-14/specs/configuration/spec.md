## ADDED Requirements

### Requirement: Model image input is metadata-driven

Smith SHALL decide whether a model accepts image input from that model's
resolved `input_modalities`. `[models."<provider>/<model>"] input_modalities`
SHALL override catalog metadata with field-level precedence. It accepts the
catalog vocabulary: `text`, `image`, `audio`, `video`, `document`. A model
with no `input_modalities` from any source SHALL be treated as text-only. A
command-JSONL provider SHALL be text-only whatever is configured. When a
session on a model without image-input metadata first drops an image, Smith
SHALL state once that the model has no image-input metadata and name the
override key. `smith config explain` SHALL report the winning source of
`input_modalities`.

#### Scenario: Catalog advertises image input

- **GIVEN** the catalog lists `image` among a model's input modalities
- **WHEN** a session starts on that model
- **THEN** the model is treated as accepting images

#### Scenario: Override marks an uncatalogued endpoint as vision-capable

- **GIVEN** a custom provider `local` with no catalog entry for `qwen-vl`
- **AND** `[models."local/qwen-vl"] input_modalities = ["text", "image"]`
- **WHEN** a session starts on `local/qwen-vl`
- **THEN** the model is treated as accepting images
- **AND** `smith config explain` names the user file as the source

#### Scenario: No metadata

- **GIVEN** a model with no catalog entry and no override
- **WHEN** a tool result carrying an image is planned into a request
- **THEN** the image is replaced by a placeholder
- **AND** Smith states once that the model has no image-input metadata and
  names `[models."<provider>/<model>"] input_modalities`

#### Scenario: Command provider override is ignored

- **GIVEN** a command-JSONL provider model with
  `input_modalities = ["text", "image"]`
- **WHEN** the session starts
- **THEN** the model is treated as text-only

### Requirement: Vision model selection

Smith SHALL select the vision model for a main profile from, in increasing
precedence, the top-level `vision` key and the profile's own `vision` key,
inherited through `extends`. The value grammar SHALL be the one advisor
selection uses: a profile name, a `provider/model` reference, or `false`.
Resolution of a profile or model selection SHALL follow the advisor rules.
The selected model MUST accept image input; otherwise Smith SHALL reject the
configuration at the `vision` key before any credential or provider is
constructed. This check SHALL run whatever the main model is. Whether the
selection is used SHALL be decided when the runtime is composed, from the
active model: it is used only while that model is text-only.
`smith config explain vision` SHALL report the winner and every overridden
source.

#### Scenario: Top-level model reference

- **GIVEN** top-level `vision = "google/gemini-3-flash"`
- **AND** the catalog lists `image` input for that model
- **WHEN** profile `glm`, whose model is text-only, starts
- **THEN** the vision model resolves to provider `google` and model
  `gemini-3-flash`

#### Scenario: Selected model cannot see

- **GIVEN** `vision = "glm/glm-4.6"`
- **AND** that model is text-only
- **WHEN** configuration loads
- **THEN** Smith reports at the `vision` key that the model does not accept
  image input
- **AND** no credential is read

#### Scenario: Main model already sees

- **GIVEN** top-level `vision = "google/gemini-3-flash"`
- **WHEN** a profile whose model accepts image input starts
- **THEN** configuration loads
- **AND** the `vision` module reports inactive because the main model
  accepts images

#### Scenario: Unusable selection is rejected even for a vision main

- **GIVEN** `vision = "glm/glm-4.6"`, a text-only model
- **WHEN** a profile whose model accepts image input starts
- **THEN** Smith reports at the `vision` key that the model does not accept
  image input

#### Scenario: Profile disables vision

- **GIVEN** top-level `vision = "sol"`
- **AND** profile `glm` sets `vision = false`
- **WHEN** profile `glm` starts
- **THEN** no vision model is resolved

### Requirement: Image window setting

Smith SHALL resolve `context.inline_images` through the existing
configuration layers. It defaults to 10, with range 1..=100, and is the
image-window size the request image projection uses. A value out of range
SHALL be rejected at load time and name the key.

#### Scenario: Out-of-range value

- **GIVEN** `[context] inline_images = 0`
- **WHEN** configuration loads
- **THEN** Smith rejects it, naming `context.inline_images` and the range
