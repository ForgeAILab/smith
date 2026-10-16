## ADDED Requirements

### Requirement: Trust files written by a newer Smith remain readable

Smith SHALL load a persisted trust file whose records include decisions it
does not understand, as long as each such record is a JSON object with string
`kind` and `label` fields. A record that is not understood MUST authorize
nothing, and persisting the store MUST write back every record that was not
replaced with the same fields and values. A version mismatch or a structurally
malformed file MUST still fail loudly.

#### Scenario: Unknown kind does not block startup

- **GIVEN** a trust file with a decision of a kind this Smith does not know,
  next to a decision of a kind it does know
- **WHEN** Smith reads the trust file
- **THEN** the trust file loads
- **AND** the known decision has the same effect as before

#### Scenario: A record of an unknown kind authorizes nothing

- **GIVEN** a trust file holding an `allow` record of a kind this Smith does
  not know
- **WHEN** Smith evaluates any artifact in that project
- **THEN** that record does not make the artifact trusted

#### Scenario: A record with an unknown decision value authorizes nothing

- **GIVEN** a trust file holding a record of a known kind whose decision value
  this Smith does not know
- **WHEN** Smith evaluates the artifact with that kind and label
- **THEN** the artifact is reported as undecided

#### Scenario: Recording a decision preserves records that are not understood

- **GIVEN** a trust file holding a record this Smith does not understand
- **WHEN** the user records a new decision in this Smith
- **THEN** the persisted file still contains that record with the same fields
  and values

#### Scenario: A new decision replaces a record that is not understood

- **GIVEN** a trust file whose record for a known kind and label has a
  decision value this Smith does not know
- **WHEN** the user records a decision for that same kind and label
- **THEN** the persisted file holds exactly one record for that kind and label
- **AND** that record is the new decision

#### Scenario: Forgetting an artifact removes records that are not understood

- **GIVEN** a trust file holding a record of a known kind and label that this
  Smith does not understand
- **WHEN** the user forgets the artifact with that kind and label
- **THEN** the persisted file holds no record for that kind and label
- **AND** records for other kinds and labels are unchanged

#### Scenario: Version mismatch is still refused

- **GIVEN** a trust file whose `version` is not one this Smith supports
- **WHEN** Smith reads the trust file
- **THEN** Smith reports that it does not understand that version
- **AND** no decision is loaded

#### Scenario: A record with no kind or label is still refused

- **GIVEN** a trust file holding a record that is not an object, or has no
  string `kind` or `label`
- **WHEN** Smith reads the trust file
- **THEN** Smith reports the trust file as unreadable
