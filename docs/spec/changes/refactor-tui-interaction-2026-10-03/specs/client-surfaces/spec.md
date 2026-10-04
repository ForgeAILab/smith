## ADDED Requirements

### Requirement: Notices have a kind and a place

Every notice SHALL carry a typed kind that fixes its label and whether it is a
transcript row or keypress feedback. Feedback — a reply to the user's own
keypress that changes nothing — SHALL show in the hint row until the next
key and MUST NOT become a transcript entry. Everything else SHALL stay a
transcript row with its current label.

#### Scenario: A refused command

- **GIVEN** a turn is running
- **WHEN** the user submits `/model`
- **THEN** the hint row says the command needs an idle turn and the draft is
  kept
- **AND** no transcript row is added
- **AND** the next keypress clears the message

#### Scenario: A provider retry

- **GIVEN** a provider request is being retried
- **WHEN** Smith reports the retry
- **THEN** the report is a transcript row as before
