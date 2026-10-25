## MODIFIED Requirements

### Requirement: View image availability

Smith SHALL offer `view_image` through compiled-in module `vision`. At each
composition, the module SHALL mount only when the active model is text-only
and a vision model resolves. Otherwise it SHALL report itself inactive with
the reason. When the active model is text-only, built-in tools are enabled,
the active posture admits the `vision` module (a writable posture, or a
read-only posture whose `read_only_network_modules` lists `vision`), and
`vision` is unset, the inactive report SHALL carry a setup hint. The hint names the missing vision model,
`smith.guide.vision`, and the main-enabled profiles whose model accepts
images, in `profile_order` order. When `vision = false`, no hint SHALL be
attached. Turning the module off SHALL NOT change the image placeholders
sent to a text-only model.

#### Scenario: Vision main model

- **GIVEN** a main model that accepts image input
- **WHEN** the tool list is built
- **THEN** `view_image` is absent
- **AND** `/modules` reports `vision` inactive because the main model accepts
  images
- **AND** no setup hint is attached

#### Scenario: Switching to a text-only model mounts the tool

- **GIVEN** a session on a vision model and `vision = "google/gemini-3-flash"`
- **WHEN** the user switches with `/model` to a text-only model
- **THEN** after the rebuild, `view_image` is in the tool list

#### Scenario: Read-only posture

- **GIVEN** a text-only main model, a vision model, and `plan` posture
- **WHEN** the tool list is built
- **THEN** `view_image` is absent, because read-only posture admits no
  network or egress tools
- **AND** `/modules` reports `vision` inactive for read-only posture

#### Scenario: No vision model configured

- **GIVEN** a text-only main model and no `vision` selection
- **AND** profile `visual` is main-enabled with a model that accepts images
- **WHEN** the tool list is built
- **THEN** `view_image` is absent
- **AND** images still reach the model as placeholders
- **AND** the `vision` inactive report carries a setup hint naming
  `smith.guide.vision` and candidate `visual`

#### Scenario: No candidate profile

- **GIVEN** a text-only main model, no `vision` selection, and no
  main-enabled profile whose model accepts images
- **WHEN** the tool list is built
- **THEN** the setup hint names `smith.guide.vision` with no candidates

#### Scenario: Vision explicitly disabled

- **GIVEN** a text-only main model and `vision = false`
- **WHEN** the tool list is built
- **THEN** `view_image` is absent
- **AND** no setup hint is attached

## ADDED Requirements

### Requirement: Image generation setup hint

The `image-generation` module SHALL attach a setup hint to its inactive
report when the active provider has no image binding, built-in tools are
enabled, and the posture is not read-only. The hint names the missing image
binding and `smith.guide.setup`, which explains configuring one.

#### Scenario: Provider without image binding
- **GIVEN** an active provider with no image binding
- **WHEN** the tool list is built
- **THEN** the image generation tool is absent
- **AND** the inactive report carries a setup hint naming `smith.guide.setup`

#### Scenario: Read-only posture has no hint
- **GIVEN** `plan` posture
- **WHEN** the tool list is built
- **THEN** the image-generation inactive report carries no setup hint
