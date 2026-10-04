## ADDED Requirements

### Requirement: Headless results are folded by a testable value

Smith SHALL compute a headless run's usage, cache, interaction, lifecycle,
and status from runtime events in a value that takes one event at a time and
can be driven without a host. The headless flow MUST only set up the run,
feed events to that value, and build the result from it after shutdown.

#### Scenario: Testing the result accounting

- **GIVEN** a recorded sequence of runtime events for one headless turn
- **WHEN** a unit test feeds them to the fold
- **THEN** it can assert the turn's usage, cache output, and whether the run
  continues, without starting a host

#### Scenario: Output is unchanged

- **WHEN** the headless fixtures run after the fold is extracted
- **THEN** every stream and result output is byte-identical

### Requirement: Picker entries are built per picker

Smith SHALL build each picker's entries in its own function, so that no
single function builds every picker.

#### Scenario: Changing one picker

- **WHEN** a developer changes the rows of the `/model` picker
- **THEN** the change is inside that picker's function
- **AND** the other pickers' entries are unchanged
