## ADDED Requirements

### Requirement: Read returns image files as images

The `read` tool SHALL return a file whose content is a PNG, JPEG, GIF or WebP
image as an image part followed by one line naming the path, media type,
pixel size and byte size. The type SHALL be recognized from the file's
signature, not its name. An image larger than 3.5 MiB SHALL be refused with
its size and the bound. Any other binary content SHALL keep the existing
refusal. Workspace confinement and approval SHALL be unchanged.

#### Scenario: Image-capable model reads a picture
- **GIVEN** a main model that accepts images and `photo.png` in the project
- **WHEN** the model calls `read` on `photo.png`
- **THEN** the tool result carries the image and a line such as
  `photo.png · image/png · 1024×768 px · 312 KiB`
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

#### Scenario: Oversized image
- **GIVEN** a 9 MiB JPEG
- **WHEN** the model calls `read` on it
- **THEN** the result is an error naming 9 MiB and the 3.5 MiB bound
- **AND** no image part is added
