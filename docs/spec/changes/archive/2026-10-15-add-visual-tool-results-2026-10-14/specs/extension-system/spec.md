## ADDED Requirements

### Requirement: MCP image results reach the model

Smith SHALL deliver an image block in a connected MCP server's tool result to
the model as an image part of that tool result. It SHALL NOT replace the
image with a text description. The image part SHALL be bounded separately
from text output: at most 5 MiB of decoded bytes per image, and at most 8
images per result. An image over either bound, or in a media type other than
PNG, JPEG, GIF, or WebP, SHALL be replaced by a text line that gives its
media type, its size, and the reason it was dropped. Whether the image is
then included in a given request is decided by the request image projection
(see `context-working-set`).

#### Scenario: Screenshot from a computer-use server

- **GIVEN** a trusted MCP server whose `get_window_state` result holds a
  text block and a 900 KB PNG image block
- **AND** the main model accepts image input
- **WHEN** the agent calls that tool
- **THEN** the tool result holds the text and the PNG as an image part
- **AND** the next provider request includes that image in the tool result

#### Scenario: Vision model on a Chat Completions endpoint

- **GIVEN** a custom OpenAI-compatible provider whose model is marked
  `input_modalities = ["text", "image"]`
- **WHEN** an MCP tool result with a PNG is sent to it
- **THEN** the `tool` message carries the result's text
- **AND** a `user` message right after it carries the PNG, labelled with the
  tool call id
- **AND** the request is not rejected

#### Scenario: Image over the size bound

- **GIVEN** a server result with a 12 MiB PNG
- **WHEN** the result is translated
- **THEN** the tool result holds a text line naming `image/png`, the size,
  and that the image exceeded the per-image bound
- **AND** no image part is present

#### Scenario: Text bound does not apply to images

- **GIVEN** a tool output limit of 32 KiB
- **AND** a server result with a 2 MB PNG and 1 KB of text
- **WHEN** the result is bounded
- **THEN** the image part is kept unchanged

#### Scenario: Harness turn receives a bridged image

- **GIVEN** a `cli/claude-code/...` main session with the same server
  declared in Smith
- **WHEN** the CLI calls the server's tool through the Smith bridge
- **THEN** the bridge returns the image to the CLI as an MCP image block
