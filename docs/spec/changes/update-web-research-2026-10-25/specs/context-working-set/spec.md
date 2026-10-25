## ADDED Requirements

### Requirement: Text artifact pages never split a character
`artifact.read` SHALL return a text page as text when the page begins or ends
inside a multi-byte UTF-8 character, leaving out the partial character, and
SHALL report the offsets of the bytes it returned, so
that following `next_offset` reads every character exactly once. A page that
is not UTF-8 inside its bounds, or that is shorter than one character, SHALL
be returned as bytes unchanged. Smith's reader cannot see the artifact's media
type, so a binary page whose only invalid bytes are a leading partial
character reads as text; the runtime reader SHALL decide by media type.

#### Scenario: A page edge falls inside a character
- **GIVEN** a stored artifact of two-byte characters
- **WHEN** a page is requested whose last byte is the first byte of a character
- **THEN** the page is returned as text without that byte
- **AND** `next_offset` points at the start of that character

#### Scenario: A requested offset falls inside a character
- **GIVEN** the same artifact
- **WHEN** a page is requested at an offset one byte into a character
- **THEN** the page is returned as text starting at the next character
- **AND** the reported offset is that character's start

#### Scenario: Binary content stays bytes
- **GIVEN** an artifact whose bytes are not UTF-8
- **WHEN** any page is read
- **THEN** the page is returned as bytes, unchanged
