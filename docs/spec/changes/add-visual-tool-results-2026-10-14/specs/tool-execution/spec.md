## ADDED Requirements

### Requirement: View image tool

Smith SHALL provide a `view_image` tool that asks the resolved vision model
one question about one image and returns the model's text answer. The call
SHALL name exactly one of these, and Smith MUST reject a call that names both
or neither:

- `image`: an image id from a placeholder.
- `path`: an image file inside the workspace or Smith's generated-image
  directory.

`question` SHALL be required, with 1 to 2,000 characters. The tool's
description SHALL tell the model that an `[image <id>: …]` placeholder in the
conversation can be inspected by passing that id as `image`. The vision request
SHALL carry a fixed instruction, the image, the image's pixel dimensions, the
name of the tool that produced the image when known, and the question. It
SHALL carry no conversation history. The instruction SHALL ask for positions
as pixel coordinates in the image's own dimensions. The tool SHALL declare
network and data-egress effects to the vision model's endpoint, plus a host
read when `path` is used, so that approval policy applies. Vision usage SHALL
be recorded against the session, the same way advisor usage is.

#### Scenario: Locate a control in a screenshot

- **GIVEN** a GLM main model, `vision = "google/gemini-3-flash"`, and a
  history placeholder `[image 3f9a2c1b: image/png 1440×900 not included in this request]`
- **WHEN** the agent calls `view_image` with `image = "3f9a2c1b"` and
  `question = "Where is the Save button?"`
- **THEN** Smith sends that PNG, its dimensions, and the question to the
  vision model
- **AND** the tool result is the model's text answer

#### Scenario: Workspace image

- **WHEN** the agent calls `view_image` with `path = "docs/mockup.png"` and
  a question
- **THEN** Smith reads the file after approval and sends it to the vision
  model

#### Scenario: Image no longer in history

- **GIVEN** an image id whose message was compacted away
- **WHEN** the agent calls `view_image` with that id
- **THEN** the tool returns an error to the model saying the image is no
  longer in session history
- **AND** no vision request is sent

#### Scenario: Path outside the allowed roots

- **WHEN** the agent calls `view_image` with `path = "/etc/hosts.png"`
- **THEN** the tool returns an error without reading the file or contacting
  the vision model

### Requirement: View image availability

Smith SHALL offer `view_image` through compiled-in module `vision`. At each
composition, the module SHALL mount only when the active model is text-only
and a vision model resolves. Otherwise it SHALL report itself inactive with
the reason. Turning the module off SHALL NOT change the image placeholders
sent to a text-only model.

#### Scenario: Vision main model

- **GIVEN** a main model that accepts image input
- **WHEN** the tool list is built
- **THEN** `view_image` is absent
- **AND** `/modules` reports `vision` inactive because the main model accepts
  images

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
- **WHEN** the tool list is built
- **THEN** `view_image` is absent
- **AND** images still reach the model as placeholders
