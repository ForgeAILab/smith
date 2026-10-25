## MODIFIED Requirements

### Requirement: Artifact retrieval is bounded and authorized
The effective raw page bound SHALL be one quarter of the resolved
`limits.tool_output_limit_bytes`, at least 1 byte and at most the runtime's
maximum artifact read. It SHALL NOT derive from the inline threshold, because
artifact pages are never offloaded. Missing limits SHALL use that bound; valid
larger limits SHALL be clamped during preparation. Source schemas SHALL permit
the accepted input before normalization. Invocation SHALL refuse a prepared
limit outside the current bound and delegate all session-ownership and
source-integrity validation to the existing runtime.

#### Scenario: A saved artifact is read after reopening a session
- GIVEN a valid session-owned artifact retained on disk
- WHEN its owner requests a large page after restart
- THEN the tool returns a bounded page and accurate continuation offset; a
  different session cannot read it and no original tool effect is replayed.

#### Scenario: Default pages fit a long report in a few reads
- **GIVEN** default limits and a 35,000-byte child reply stored as an artifact
- **WHEN** the parent reads it with `artifact.read` from offset zero, following
  each `next_offset`
- **THEN** each page holds up to 16,384 bytes and three reads return the whole
  reply

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
