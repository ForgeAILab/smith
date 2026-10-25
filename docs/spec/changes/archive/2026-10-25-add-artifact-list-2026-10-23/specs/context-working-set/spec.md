## ADDED Requirements

### Requirement: Session artifacts can be listed

When Smith's own artifact store backs a root or child runtime, Smith SHALL
register an `artifact.list` tool beside `artifact.read`. It SHALL return only
artifacts whose protected owner is the invoking session, taken from the
runtime-owned invocation context and never from model arguments. Each row
SHALL carry metadata only: ID, purpose, media type, byte length, the
originating turn and call when recorded, the child that produced it, and the
source session of an artifact copied from a child. The child SHALL come from
the artifact's provenance when Runtime recorded it, otherwise from resolving a
copy's lineage session against the session's own children; it is absent when
neither names one. Rows SHALL be ordered newest first, with ties broken by ID,
and returned in pages of at most 100 rows (default 20) with a total count and a
continuation offset. Optional exact `purpose` and `child_id` filters SHALL
narrow the rows. Listing SHALL request the `artifact.read` permission over a
`session-artifact` resource and SHALL skip unreadable or malformed metadata
records instead of failing the page.

#### Scenario: A session lists its own artifacts

- **GIVEN** a store holding artifacts owned by the session and by another
  session
- **WHEN** the session calls `artifact.list`
- **THEN** only its own artifacts are listed, newest first, without content
- **AND** an `artifact.read` of any listed ID succeeds

#### Scenario: Listing narrows by purpose and pages

- **GIVEN** the session owns more `delegation.child-result` artifacts than one
  page holds and one `delegation.child-reply` artifact
- **WHEN** it lists with `purpose = "delegation.child-reply"`
- **THEN** exactly the reply artifact is returned with `total = 1`
- **AND** an unfiltered first page returns the default page size with a
  continuation offset

#### Scenario: Listing finds one child's reply

- **GIVEN** a child whose reply was stored as a parent-owned copy, and a copied
  artifact whose lineage names that child's session
- **WHEN** the parent lists with that `child_id`
- **THEN** both rows are returned, each naming the child
- **AND** the parent's own tool outputs are excluded and name no child

#### Scenario: No Smith store backs the runtime

- **GIVEN** a runtime whose artifact store was supplied without a listing
  catalog
- **WHEN** Smith registers its tools
- **THEN** `artifact.list` is not registered and no error names it

### Requirement: A missing artifact names how to find real ones

When `artifact.read` fails because the ID does not exist, Smith SHALL return an
error that says the ID does not exist in this session and that artifact IDs
are copied from tool results or host messages, never constructed. When
`artifact.list` is registered for the same runtime, the error SHALL name it.
Ownership and integrity failures SHALL keep Runtime's own errors.

#### Scenario: A model reads an invented ID

- **GIVEN** `artifact.list` is registered
- **WHEN** the model reads an ID that no artifact has
- **THEN** the tool error states the ID does not exist and names
  `artifact.list`
- **AND** no artifact content or other session's metadata is revealed
