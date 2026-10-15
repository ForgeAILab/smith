## ADDED Requirements

### Requirement: Image-bearing result previews

A tool row whose result holds image parts SHALL show one line per image in
its result preview, with the media type and pixel dimensions when the header
can be decoded, for example `[image/png 1440×900]`. It SHALL NOT render the
image data. A dropped-image text line from the MCP bound SHALL render as
ordinary result text. Live and resumed rows SHALL agree.

#### Scenario: Screenshot result

- **WHEN** an MCP tool returns text and one PNG
- **THEN** the row's preview shows the text excerpt and `[image/png 1440×900]`

#### Scenario: Resumed session

- **GIVEN** the same row rendered live
- **WHEN** the session is resumed
- **THEN** the resumed row shows the same image line

### Requirement: View image rows

The TUI SHALL render a `view_image` call as an in-progress row naming the
image id or path and the question, then as a completed row that shows a
bounded excerpt of the answer, or as a failed row with the error.

#### Scenario: Completed answer

- **WHEN** a `view_image` call completes
- **THEN** the row shows the image id, the question, and the start of the
  vision model's answer
