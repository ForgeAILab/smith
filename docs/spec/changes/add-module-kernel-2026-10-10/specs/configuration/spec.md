## ADDED Requirements

### Requirement: Module selection is configuration

Smith SHALL decide which compiled-in modules mount from `modules.<id>.enabled`,
resolved through the ordinary layered resolution with provenance, so that a
profile, a project file, or a session override can carry its own module set.
When no layer sets the key, the module's declared default applies. A key
naming an id that no known module has MUST be rejected with the list of
known ids. A module that is switched on but not compiled into the running
binary MUST be reported and MUST NOT be treated as mounted.

#### Scenario: Module value is explained

- **GIVEN** `modules.image-generation.enabled = false` in the user
  configuration
- **WHEN** configuration resolves
- **THEN** `image-generation` is off
- **AND** its provenance names the user file and that key

#### Scenario: Profile carries its own module set

- **GIVEN** the user file turns `image-generation` on
- **AND** the selected profile turns it off
- **WHEN** configuration resolves
- **THEN** `image-generation` is off
- **AND** its provenance names the profile

#### Scenario: Unknown module id

- **GIVEN** a configuration file sets `modules.nope.enabled = true`
- **WHEN** configuration resolves
- **THEN** resolution fails naming `nope`
- **AND** lists the known module ids

#### Scenario: Module is on but not compiled in

- **GIVEN** a binary built without the `image-generation` module
- **AND** `modules.image-generation.enabled = true`
- **WHEN** Smith starts
- **THEN** the module is reported as not built
- **AND** the session starts without it

### Requirement: Ported features keep their existing switch

Smith SHALL keep a ported feature's existing switch working. When an
existing feature becomes a module, the configuration key that already
switched it SHALL remain a valid spelling of that module's
`enabled` key, with the same default. Setting the existing key and the
module key to different values in the same layer MUST be an error naming
both keys; across layers the normal layer precedence selects one winner,
as for the existing idle-compaction alias. Every other setting of the feature MUST keep its existing key.

#### Scenario: Existing key still switches the feature

- **GIVEN** `tools.image_generation.enabled = false` in the user
  configuration and no `modules.image-generation` table
- **WHEN** configuration resolves
- **THEN** the `image-generation` module is off
- **AND** its provenance names `tools.image_generation.enabled`

#### Scenario: Both spellings disagree in one file

- **GIVEN** one file sets `tools.image_generation.enabled = true` and
  `modules.image-generation.enabled = false`
- **WHEN** configuration resolves
- **THEN** resolution fails naming both keys and the file
