## MODIFIED Requirements

### Requirement: Images are fitted to the active provider

Smith SHALL prepare every image file it reads or attaches against the active
main model's provider limits, kept in one table: formats accepted, longest
side, pixel budget, and encoded size. Smith SHALL also fit every image part in
each outgoing provider request to the limits of the provider serving that
request, whatever its source, so a provider switch never sends an image that
provider rejects. An image that already fits SHALL be sent unchanged.
Otherwise Smith SHALL:
- downscale it to fit;
- convert a format the provider does not accept to PNG;
- reduce an animated GIF to its first frame;
- re-encode a PNG without transparency as JPEG at quality 85 when it is still
  too large.

Request-time results SHALL be cached by content and limits. An image that
cannot be prepared at request time SHALL be replaced in that request by
`[image omitted: <reason>]` rather than fail the turn. Canonical history
SHALL be unchanged.

A file over 64 MiB, an image over 100 megapixels, or a corrupt image SHALL be
refused with the reason before any provider request when it is read or
attached.

#### Scenario: Large photo for ChatGPT
- **GIVEN** the `chatgpt` provider and a 4032×3024 JPEG
- **WHEN** it is read or attached
- **THEN** the model receives a JPEG within 2048 px on its longest side and
  within 2,500 tiles of 32 px
- **AND** the description names both sizes

#### Scenario: Small screenshot passes through
- **GIVEN** a 1200×800 PNG within every limit
- **WHEN** it is read
- **THEN** the image part carries the file's original bytes

#### Scenario: Format the provider rejects
- **GIVEN** the `xai` provider and a WebP image
- **WHEN** it is attached
- **THEN** the model receives it as a PNG

#### Scenario: Decompression guard
- **GIVEN** a PNG that declares 20000×20000 pixels
- **WHEN** it is read
- **THEN** the result is an error naming the 100-megapixel limit
- **AND** the image is not decoded

#### Scenario: Switching to a stricter provider
- **GIVEN** a WebP image read while the session used ChatGPT
- **WHEN** the user switches with `/model` to an xAI model and sends a turn
- **THEN** the request to xAI carries that image as a PNG within xAI's limits
- **AND** the turn succeeds

#### Scenario: Repeated image is prepared once
- **GIVEN** an oversized image in history and an unchanged provider
- **WHEN** three consecutive requests carry it
- **THEN** it is decoded and prepared once, and all three requests carry
  identical bytes

#### Scenario: Unpreparable image at request time
- **GIVEN** an image part whose data cannot be decoded
- **WHEN** a request carries it
- **THEN** that request carries `[image omitted: …]` in its place
- **AND** the request is still sent
