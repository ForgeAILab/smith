## ADDED Requirements

### Requirement: Registered composer material is edited atomically

Smith SHALL treat each complete composer placeholder backed by registered
large-paste or clipboard-image material as one logical editing unit. A
registered placeholder MUST expose cursor positions only at its start and end,
MUST be removed as a whole by adjacent backward or forward deletion, and MUST
retain its complete compact label while editable. When input is committed,
Smith MUST replace registered pasted-text labels with their exact stored text
in the user transcript while retaining registered image labels.

#### Scenario: Move across a pasted-text placeholder

- **GIVEN** the composer contains text followed by a registered
  `[Pasted text #N +L lines]` placeholder followed by more text
- **WHEN** the user moves horizontally across the placeholder
- **THEN** one `Left` or `Right` press moves between its end and start boundary
- **AND** the cursor never stops inside the label

#### Scenario: Delete a pasted-text placeholder backward

- **GIVEN** the cursor is immediately after a registered pasted-text
  placeholder
- **WHEN** the user presses `Backspace`
- **THEN** Smith removes the complete placeholder from the composer
- **AND** no fragment of its label or raw content remains in that draft

#### Scenario: Delete an image placeholder forward

- **GIVEN** the cursor is immediately before a registered clipboard-image
  placeholder
- **WHEN** the user presses `Delete`
- **THEN** Smith removes the complete placeholder from the composer
- **AND** that image is not included when the edited draft is prepared

#### Scenario: Adjacent placeholders remain distinct units

- **GIVEN** two registered placeholders are adjacent in the composer
- **WHEN** the user moves or deletes at their shared boundary
- **THEN** Smith targets exactly the placeholder indicated by the movement or
  deletion direction
- **AND** leaves the other placeholder and its material unchanged

#### Scenario: Ordinary text remains character-addressable

- **GIVEN** the composer contains Unicode text, an image path, or text shaped
  like an unregistered paste or image placeholder
- **WHEN** the user moves through or deletes that text
- **THEN** Smith applies ordinary Unicode-safe character editing
- **AND** does not expand paste content or attach an image for that text

#### Scenario: Commitment expands text but retains image labels

- **GIVEN** a draft contains registered text-paste and clipboard-image
  placeholders that were not deleted
- **WHEN** Smith commits the input and renders its user transcript entry
- **THEN** each pasted-text label is replaced by its exact stored content in
  that transcript entry and in provider text
- **AND** each image label remains visible in that transcript entry
- **AND** each real clipboard image is submitted as image content in
  placeholder order

#### Scenario: Uncommitted projections stay compact

- **GIVEN** registered pasted-text or image material is still editable, queued,
  or recalled from composer history
- **WHEN** Smith renders that uncommitted input
- **THEN** Smith keeps its registered compact labels instead of expanding raw
  pasted text
- **AND** the labels retain atomic movement and deletion behavior while their
  material remains registered
