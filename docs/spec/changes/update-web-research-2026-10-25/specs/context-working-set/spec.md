## ADDED Requirements

### Requirement: Text artifact pages never split a character
`artifact.read` SHALL return a text page as text when the page begins or ends
inside a multi-byte UTF-8 character, leaving out the partial character, and
SHALL report the offsets of the bytes it returned, so
that following `next_offset` reads every character exactly once. A page that
is not UTF-8 inside its bounds, or that is shorter than one character, SHALL
be returned as bytes unchanged. Text SHALL be decided by the artifact's media
type, so a binary artifact is never decoded as text.

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

### Requirement: A fetched page stays inline up to the output ceiling
Smith SHALL keep a `web_fetch` outcome inline when its serialization fits the
resolved `limits.tool_output_limit_bytes`, regardless of
`context.tool_output_inline_bytes`. A larger text outcome SHALL be stored
unchanged as an artifact, and the model SHALL receive the reference with a
preview holding the page's beginning up to that ceiling, less room for the
reference, followed by a note giving the `artifact.read` offset in the stored
outcome at which the unshown text starts. When that offset cannot be
determined, the note SHALL say to read the artifact from its start. Other
tools SHALL keep the inline threshold.

#### Scenario: A 30 KB page under default settings
- **GIVEN** `tool_output_inline_bytes = 8192` and the default 64 KiB output
  ceiling
- **WHEN** `web_fetch` returns a 30 KB page
- **THEN** the model receives the whole page inline
- **AND** no artifact is stored for it

#### Scenario: A 200 KB page
- **GIVEN** the default 64 KiB output ceiling
- **WHEN** `web_fetch` returns a 200 KB source file
- **THEN** the complete outcome is stored as an artifact
- **AND** the model receives the file's first part, up to the ceiling, with
  the artifact reference
- **AND** reading the artifact at the offset the note gives returns the text
  that follows the shown part

#### Scenario: Another tool's 30 KB result
- **GIVEN** the same settings
- **WHEN** a shell command prints 30 KB
- **THEN** the result is offloaded with a bounded excerpt as before
