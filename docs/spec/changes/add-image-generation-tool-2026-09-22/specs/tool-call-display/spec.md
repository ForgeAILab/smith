## ADDED Requirements

### Requirement: Image generation rows

The TUI SHALL render a `generate_image` call as an in-progress row, then as
a completed row that shows the saved path and image dimensions, or as a
failed row that shows the provider error.

#### Scenario: Completed image

- **WHEN** an image generation completes
- **THEN** the transcript row shows the saved path and the image's pixel
  dimensions
