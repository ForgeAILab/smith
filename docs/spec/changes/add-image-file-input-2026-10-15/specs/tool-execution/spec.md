## ADDED Requirements

### Requirement: Read returns image files as images

The `read` tool SHALL return a file whose content is a PNG, JPEG, GIF or WebP
image as a prepared image part followed by one line naming the path, the
original pixel size, and the size, format and byte count sent. The type SHALL
be recognized from the file's signature, not its name. Any other binary
content SHALL keep the existing refusal. Workspace confinement and approval
SHALL be unchanged.

#### Scenario: Image-capable model reads a picture
- **GIVEN** a main model that accepts images and `photo.png` in the project
- **WHEN** the model calls `read` on `photo.png`
- **THEN** the tool result carries the image and a line describing it
- **AND** the next request sends the image to the model

#### Scenario: Text-only model reads a picture
- **GIVEN** a text-only main model and a configured vision model
- **WHEN** the model calls `read` on `photo.png`
- **THEN** the model receives the runtime's `[image <id>: …]` placeholder
- **AND** `view_image` can answer a question about that id

#### Scenario: A file named like an image is not one
- **GIVEN** `fake.png` whose bytes are not an image signature
- **WHEN** the model calls `read` on it
- **THEN** it is handled as before: text if it is text, a binary refusal
  otherwise

### Requirement: Images are fitted to the active provider

Smith SHALL prepare every image file it reads or attaches against the active
main model's provider limits, kept in one table: formats accepted, longest
side, pixel budget, and encoded size. An image that already fits SHALL be
sent unchanged. Otherwise Smith SHALL:
- downscale it to fit;
- convert a format the provider does not accept to PNG;
- reduce an animated GIF to its first frame;
- re-encode a PNG without transparency as JPEG at quality 85 when it is still
  too large.

A file over 64 MiB, an image over 100 megapixels, or a corrupt image SHALL be
refused with the reason before any provider request.

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
