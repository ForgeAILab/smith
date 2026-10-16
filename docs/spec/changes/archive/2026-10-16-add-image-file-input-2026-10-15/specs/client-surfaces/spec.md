## ADDED Requirements

### Requirement: Image file attachments

A user SHALL be able to attach an image file to a prompt.
- In the TUI, an `@FILE` attachment whose content is a PNG, JPEG, GIF or WebP
  image SHALL be sent as a prepared image part, shown like a pasted clipboard
  image.
- Headless runs SHALL accept `--image <PATH>`, repeatable, which adds each
  prepared image to the prompt.

Both SHALL use the same preparation as `read`. A `--image` path SHALL be read
as the caller's own input, without approval and without project confinement.
`--image` with a `cli/*` profile SHALL fail before the run starts rather than
drop the image.

#### Scenario: Headless image prompt
- **GIVEN** the `visual` profile on an image-capable model
- **WHEN** the user runs `smith -p "Label this picture" --image ./cat.jpg`
- **THEN** the first request carries the prompt text and the prepared image
- **AND** the result answers from the picture

#### Scenario: TUI attachment
- **GIVEN** an interactive session and `diagram.png` in the project
- **WHEN** the user submits `@diagram.png what is wrong here?`
- **THEN** the message carries the image part and the text
- **AND** the transcript shows an image attachment rather than an error

#### Scenario: Harness profile
- **GIVEN** a `cli/codex` profile
- **WHEN** the user runs `smith -p "describe" --image ./cat.jpg`
- **THEN** Smith exits with an error that `--image` is not supported for
  installed coding agents yet
- **AND** no turn starts

#### Scenario: Unreadable attachment
- **GIVEN** `--image ./missing.png`
- **WHEN** the run starts
- **THEN** Smith exits with an error naming the path, before any provider
  request
