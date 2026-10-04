## ADDED Requirements

### Requirement: Model output budget visibility

Smith's model-selection surfaces SHALL distinguish a model's advertised output
ceiling from the effective request output budget. The picker MUST identify an
automatically derived budget without presenting it as catalog metadata or a
user-authored override.

#### Scenario: Automatic budget makes a catalog model selectable

- **GIVEN** a catalog model advertises equal 500,000-token context and output
  ceilings
- **AND** Smith derives a 32,768-token automatic request budget
- **WHEN** the user filters `/model` to that entry
- **THEN** the row is selectable and shows both the 500,000-token ceiling and
  32,768-token automatic request budget
- **AND** it does not tell the user to add a local model-limit override

#### Scenario: Configured budget is distinguishable

- **GIVEN** an explicit profile or session value supplies the effective request
  output budget
- **WHEN** Smith renders model details or a reserve diagnostic
- **THEN** it labels the value as configured rather than automatic
- **AND** keeps catalog provenance attached only to the advertised ceiling
